//! The GPU canvas against the CPU renderer: pixel parity, the fallback predicate, read-back and
//! frame timings (see `docs/gpu-rasterizer.md`).

use std::sync::Arc;

use figcraft_color::{BlendMode, Color, Gradient, GradientKind, Paint};
use figcraft_doc::{Document, Effect, EffectKind, Glass, NodeKind, StrokeAlign, TextData};
use figcraft_render::{Options, Pixmap, View, render_view};
use wgpu::{Device, Queue};

use crate::canvas::CanvasGpu;
use crate::scene;
use crate::scene::Layers as _;

/// The machine's adapter (none in a headless CI container: the tests then skip).
fn device() -> Option<(Arc<Device>, Arc<Queue>)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((Arc::new(device), Arc::new(queue)))
}

/// Shapes, corners, gradients, opacity, blend and a stroke (no text, images or effects).
fn parity_doc() -> Document {
    let mut d = Document::new("gpu parity");
    let p = d.first_page().unwrap();
    let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::WHITE);

    let solid = d.create(p, NodeKind::Rectangle, "solid", 24.0, 24.0, 160.0, 100.0).unwrap();
    d.get_mut(solid).unwrap().fills = vec![Paint::solid(hex("E03030"))];

    let rounded = d.create(p, NodeKind::Rectangle, "rounded", 220.0, 24.0, 160.0, 100.0).unwrap();
    {
        let n = d.get_mut(rounded).unwrap();
        n.corners.radii = [28.0; 4];
        n.fills = vec![Paint::solid(hex("2040E0"))];
    }

    let gradient = d.create(p, NodeKind::Rectangle, "gradient", 416.0, 24.0, 160.0, 100.0).unwrap();
    {
        let g = Gradient {
            kind: GradientKind::Linear,
            stops: vec![
                figcraft_color::ColorStop { position: 0.0, color: hex("F0C830") },
                figcraft_color::ColorStop { position: 1.0, color: hex("20B0D0") },
            ],
            from: (0.0, 0.5),
            to: (1.0, 0.5),
            width: (0.0, -0.5),
        };
        d.get_mut(gradient).unwrap().fills = vec![Paint::gradient(g)];
    }

    let ellipse =
        d.create(p, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "ellipse", 24.0, 160.0, 160.0, 100.0).unwrap();
    d.get_mut(ellipse).unwrap().fills = vec![Paint::solid(hex("20C060"))];

    let half = d.create(p, NodeKind::Rectangle, "half", 220.0, 160.0, 160.0, 100.0).unwrap();
    {
        let n = d.get_mut(half).unwrap();
        n.opacity = 0.5;
        n.fills = vec![Paint::solid(hex("A040E0"))];
    }

    let stroke = d.create(p, NodeKind::Rectangle, "stroke", 416.0, 160.0, 160.0, 100.0).unwrap();
    {
        let n = d.get_mut(stroke).unwrap();
        n.fills.clear();
        n.corners.radii = [16.0; 4];
        n.strokes = vec![Paint::solid(hex("303030"))];
        n.stroke.weight = 6.0;
        n.stroke.align = StrokeAlign::Center;
    }

    let blended = d.create(p, NodeKind::Rectangle, "blended", 300.0, 90.0, 120.0, 80.0).unwrap();
    {
        let n = d.get_mut(blended).unwrap();
        n.blend = BlendMode::Multiply;
        n.fills = vec![Paint::solid(hex("40E0A0"))];
    }
    d
}

/// The parity canvas: 600 × 300 device pixels at 100%.
const PARITY: (u32, u32) = (600, 300);

/// The CPU render of `view` and the GPU one, through the same doc.
fn both(d: &Document, view: &View) -> Option<(Pixmap, Pixmap)> {
    let p = d.first_page()?;
    let cpu = render_view(d, p, view, &Options::default())?;
    let (device, queue) = device()?;
    let scene = scene::build(d, p, view).ok()?;
    let mut canvas = CanvasGpu::new(&device, view.width, view.height)?;
    canvas.render(&device, &queue, &scene, scene::background(d, p))?;
    let gpu = canvas.read(&device, &queue)?;
    Some((cpu, gpu))
}

/// The worst channel difference and the mean absolute difference (0–255 scale), with where the
/// worst pixel is and how many pixels differ by more than 8 in a channel.
fn diff(a: &Pixmap, b: &Pixmap) -> (u8, f64, (u32, u32), usize) {
    let (mut worst, mut sum, mut n) = (0u8, 0f64, 0usize);
    let mut at = (0, 0);
    let mut over = 0;
    let w = a.width().max(1) as usize;
    for (i, (x, y)) in a.data().iter().zip(b.data()).enumerate() {
        let d = x.abs_diff(*y);
        if d > worst {
            worst = d;
            at = ((i / 4 % w) as u32, (i / 4 / w) as u32);
        }
        sum += f64::from(d);
        n += 1;
    }
    for px in 0..(a.data().len() / 4).min(b.data().len() / 4) {
        let (sa, sb) = (a.data().get(px * 4..px * 4 + 4), b.data().get(px * 4..px * 4 + 4));
        if let (Some(sa), Some(sb)) = (sa, sb)
            && sa.iter().zip(sb).any(|(x, y)| x.abs_diff(*y) > 8)
        {
            over += 1;
        }
    }
    (worst, sum / n.max(1) as f64, at, over)
}

#[test]
fn the_gpu_canvas_matches_the_cpu_render() {
    if device().is_none() {
        eprintln!("no GPU adapter: skipping");
        return;
    }
    let d = parity_doc();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let (cpu, gpu) = both(&d, &view).expect("both renderers drew the page");
    assert_eq!((cpu.width(), cpu.height()), (gpu.width(), gpu.height()));
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    if std::env::var("FIGCRAFT_PARITY_PNG").is_ok() {
        let dir = std::env::temp_dir();
        let _ = cpu.save_png(dir.join("figcraft-parity-cpu.png"));
        let _ = gpu.save_png(dir.join("figcraft-parity-gpu.png"));
        println!("wrote {}", dir.display());
    }
    let px = |p: &Pixmap, (x, y): (u32, u32)| {
        let i = ((y as usize * p.width() as usize) + x as usize) * 4;
        p.data().get(i..i + 4).map(<[u8]>::to_vec)
    };
    println!(
        "parity: worst channel difference {worst} at {at:?} (CPU {:?}, GPU {:?}), mean {mean:.3}, \
         {over} pixels differ by more than 8 (0–255)",
        px(&cpu, at),
        px(&gpu, at)
    );
    assert!(worst <= 96, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 0.5, "the mean difference is {mean:.3}");
    assert!(over < (PARITY.0 * PARITY.1) as usize / 200, "{over} pixels differ by more than 8");
}

#[test]
fn unsupported_pages_are_refused() {
    let mut d = parity_doc();
    let p = d.first_page().unwrap();
    assert_eq!(scene::supported(&d, p), Ok(()));

    // A drop shadow on an opaque card is vello's blurred rounded rect, drawn under the card.
    let id = d.node(p).and_then(|n| n.children.first().copied()).unwrap();
    d.get_mut(id).unwrap().effects = vec![Effect::drop_shadow()];
    assert_eq!(scene::supported(&d, p), Ok(()));
    // A translucent card casts a shadow as faint as it paints, hidden where it covers: the card's
    // silhouette, clipped outside its own outline.
    let opaque = d.node(id).and_then(|n| n.fills.first().cloned()).unwrap();
    d.get_mut(id).unwrap().fills[0].opacity = 0.5;
    assert_eq!(scene::supported(&d, p), Ok(()));
    // Beside a stroke of its own the card's shadow is cast from its pixels, one layer.
    d.get_mut(id).unwrap().strokes = vec![Paint::solid(Color::WHITE)];
    d.get_mut(id).unwrap().stroke.weight = 4.0;
    assert_eq!(scene::supported(&d, p), Ok(()));
    d.get_mut(id).unwrap().strokes.clear();
    d.get_mut(id).unwrap().stroke.weight = 0.0;
    // Nor when the fill fades: the shadow is cast from the card's own pixels.
    d.get_mut(id).unwrap().fills = vec![Paint::gradient(Gradient {
        kind: GradientKind::Linear,
        stops: vec![
            figcraft_color::ColorStop { position: 0.0, color: Color::WHITE },
            figcraft_color::ColorStop { position: 1.0, color: Color { a: 0.0, ..Color::WHITE } },
        ],
        from: (0.0, 0.5),
        to: (1.0, 0.5),
        width: (0.0, 1.0),
    })];
    assert_eq!(scene::supported(&d, p), Ok(()));
    d.get_mut(id).unwrap().fills[0] = opaque;
    // A shadow that spreads grows the card's own silhouette; one that shows behind is hidden by an
    // opaque card either way; one with a blend mode composites as a layer. None needs a layer.
    d.get_mut(id).unwrap().effects = vec![Effect::drop_shadow()];
    if let Some(figcraft_doc::EffectKind::DropShadow(s)) = d.get_mut(id).ok().and_then(|n| n.effects.first_mut()).map(|e| &mut e.kind) {
        s.spread = 6.0;
    }
    assert_eq!(scene::supported(&d, p), Ok(()));
    if let Some(figcraft_doc::EffectKind::DropShadow(s)) = d.get_mut(id).ok().and_then(|n| n.effects.first_mut()).map(|e| &mut e.kind) {
        s.spread = 0.0;
        s.show_behind = true;
        s.blend = BlendMode::Multiply;
    }
    assert_eq!(scene::supported(&d, p), Ok(()));
    // A gradient fill covers the card as opaquely as a solid one (its stops are all opaque).
    let solid = d.node(id).and_then(|n| n.fills.first().cloned()).unwrap();
    let mut g = Gradient {
        kind: GradientKind::Linear,
        stops: vec![
            figcraft_color::ColorStop { position: 0.0, color: Color::from_hex("F0C830").unwrap_or(Color::WHITE) },
            figcraft_color::ColorStop { position: 1.0, color: Color::from_hex("20B0D0").unwrap_or(Color::WHITE) },
        ],
        from: (0.0, 0.5),
        to: (1.0, 0.5),
        width: (0.0, 1.0),
    };
    d.get_mut(id).unwrap().fills = vec![Paint::gradient(g.clone())];
    assert_eq!(scene::supported(&d, p), Ok(()));
    // A linear gradient whose width handle isn't square to its axis (imported files can skew one)
    // draws different bands in vello's projection: the CPU has it.
    g.from = (0.0, 0.0);
    g.to = (1.0, 1.0);
    g.width = (1.0, 0.0);
    d.get_mut(id).unwrap().fills = vec![Paint::gradient(g)];
    assert_eq!(scene::supported(&d, p), Err(scene::Unsupported::Paint));
    d.get_mut(id).unwrap().fills = vec![solid];
    // An opaque stroke grows the silhouette; a translucent one is cast from the card's pixels.
    d.get_mut(id).unwrap().strokes = vec![Paint::solid(Color::WHITE)];
    d.get_mut(id).unwrap().stroke.weight = 4.0;
    d.get_mut(id).unwrap().stroke.align = StrokeAlign::Outside;
    assert_eq!(scene::supported(&d, p), Ok(()), "an outside stroke is the renderer's band");
    d.get_mut(id).unwrap().strokes[0].opacity = 0.5;
    assert_eq!(scene::supported(&d, p), Ok(()), "a translucent band casts from the pixels");
    d.get_mut(id).unwrap().strokes.clear();
    d.get_mut(id).unwrap().stroke.weight = 0.0;
    d.get_mut(id).unwrap().effects = vec![Effect::drop_shadow()];
    // One layer blur is drawn on the GPU (its own layer, blurred there).
    d.get_mut(id).unwrap().effects = vec![Effect::layer_blur()];
    assert_eq!(scene::supported(&d, p), Ok(()));
    // One layer of its own: a blur and a shadow together, in the CPU's order.
    d.get_mut(id).unwrap().effects = vec![Effect::drop_shadow(), Effect::layer_blur()];
    assert_eq!(scene::supported(&d, p), Ok(()));
    // A background blur reads what lies under the node: drawn on the GPU from a backdrop render.
    d.get_mut(id).unwrap().effects = vec![Effect::background_blur()];
    assert_eq!(scene::supported(&d, p), Ok(()));
    d.get_mut(id).unwrap().effects = vec![Effect::background_blur(), Effect::drop_shadow()];
    assert_eq!(scene::supported(&d, p), Err(scene::Unsupported::Effect));
    // Glass is a pane over the backdrop, drawn by the GPU's glass shader.
    d.get_mut(id).unwrap().effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
    assert_eq!(scene::supported(&d, p), Ok(()));
    d.get_mut(id).unwrap().effects.clear();

    // An image fill draws from the renderer's own decoded, filtered, cached pixmap.
    d.get_mut(id).unwrap().fills = vec![Paint::image("hash", figcraft_color::ImageScaleMode::Fill)];
    assert_eq!(scene::supported(&d, p), Ok(()), "an image fill is the renderer's pixmap");
    // A paint's own blend mode composites in a layer of its own; plus-darker has no vello mode.
    let mut p0 = Paint::solid(Color::WHITE);
    p0.blend = BlendMode::Multiply;
    d.get_mut(id).unwrap().fills = vec![p0];
    assert_eq!(scene::supported(&d, p), Ok(()));
    d.get_mut(id).unwrap().fills[0].blend = BlendMode::PlusDarker;
    assert_eq!(scene::supported(&d, p), Err(scene::Unsupported::Paint));
    // An inner shadow of a shape is one blurred buffer clipped to it; a group's is cast from its
    // own pixels, in a layer of the group.
    let inner = figcraft_doc::Shadow { radius: 10.0, ..Default::default() };
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::WHITE)];
    d.get_mut(id).unwrap().effects = vec![Effect { kind: EffectKind::InnerShadow(inner), visible: true }];
    assert_eq!(scene::supported(&d, p), Ok(()));
    d.get_mut(id).unwrap().kind = NodeKind::Group;
    assert_eq!(scene::supported(&d, p), Ok(()), "a group's inner shadow is cast from its pixels");
}

/// Two cards, the upper one rounded and blurred as a layer.
fn blurred_doc() -> Document {
    let mut d = Document::new("gpu layer blur");
    let p = d.first_page().unwrap();
    let under = d.create(p, NodeKind::Rectangle, "under", 120.0, 60.0, 220.0, 160.0).unwrap();
    d.get_mut(under).unwrap().fills = vec![Paint::solid(Color::from_hex("2040E0").unwrap_or(Color::WHITE))];
    let card = d.create(p, NodeKind::Rectangle, "blurred", 300.0, 80.0, 200.0, 140.0).unwrap();
    {
        let n = d.get_mut(card).unwrap();
        n.corners.radii = [24.0; 4];
        n.fills = vec![Paint::solid(Color::from_hex("E03030").unwrap_or(Color::WHITE))];
        n.effects = vec![Effect::layer_blur()];
    }
    d
}

#[test]
fn the_gpu_layer_blur_matches_the_cpu_render() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = blurred_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    };
    let built = built.expect("the GPU draws a layer blur");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("layer blur parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
}

/// A rounded white card with a drop shadow over a light ground.
fn shadow_doc() -> Document {
    let mut d = Document::new("gpu shadow");
    let p = d.first_page().unwrap();
    let ground = d.create(p, NodeKind::Rectangle, "ground", 60.0, 40.0, 480.0, 220.0).unwrap();
    d.get_mut(ground).unwrap().fills = vec![Paint::solid(Color::from_hex("E8E8E8").unwrap_or(Color::WHITE))];
    let card = d.create(p, NodeKind::Rectangle, "card", 200.0, 90.0, 200.0, 130.0).unwrap();
    {
        let n = d.get_mut(card).unwrap();
        n.corners.radii = [16.0; 4];
        n.fills = vec![Paint::solid(Color::WHITE)];
        n.effects = vec![Effect::drop_shadow()];
    }
    d
}

#[test]
fn the_gpu_drop_shadow_matches_the_cpu_render() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = shadow_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    };
    let built = built.expect("the GPU draws a drop shadow");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("drop shadow parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
}

/// Cards whose edges are scattered by a texture effect, one of them inside a longer effect list.
fn texture_doc() -> Document {
    let mut d = Document::new("gpu texture");
    let p = d.first_page().unwrap();
    let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::WHITE);
    let ground = d.create(p, NodeKind::Rectangle, "ground", 40.0, 30.0, 520.0, 240.0).unwrap();
    d.get_mut(ground).unwrap().fills = vec![Paint::solid(hex("E8E8E8"))];
    let texture = |radius: f64, size: f64, clip: bool| Effect {
        kind: EffectKind::Texture(figcraft_doc::Texture { radius, size, clip_to_shape: clip, ..figcraft_doc::Texture::default() }),
        visible: true,
    };
    let card = d.create(p, NodeKind::Rectangle, "rough", 80.0, 70.0, 160.0, 140.0).unwrap();
    {
        let n = d.get_mut(card).unwrap();
        n.corners.radii = [12.0; 4];
        n.fills = vec![Paint::solid(hex("E03030"))];
        n.effects = vec![texture(10.0, 0.8, false)];
    }
    let clipped = d.create(p, NodeKind::Rectangle, "clipped", 300.0, 70.0, 140.0, 140.0).unwrap();
    {
        let n = d.get_mut(clipped).unwrap();
        n.corners.radii = [24.0; 4];
        n.fills = vec![Paint::solid(hex("20C060"))];
        n.effects = vec![texture(9.0, 0.5, true)];
    }
    // In a list: the blur, then the scatter, then the shadow cast from that result.
    let list = d.create(p, NodeKind::Rectangle, "blur, texture and shadow", 470.0, 80.0, 120.0, 120.0).unwrap();
    {
        let n = d.get_mut(list).unwrap();
        n.corners.radii = [16.0; 4];
        n.fills = vec![Paint::solid(hex("2040E0"))];
        n.effects = vec![Effect::layer_blur(), texture(8.0, 0.6, false), Effect::drop_shadow()];
    }
    d
}

#[test]
fn the_gpu_draws_the_texture_effect() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = texture_doc();
    let p = d.first_page().unwrap();
    assert_eq!(scene::supported(&d, p), Ok(()), "a texture effect stays on the GPU");
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    };
    let built = built.expect("the GPU draws the texture effect");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("texture parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 1.0, "the mean difference is {mean:.3}");
}

/// Text in one style: the renderer's layout turns it into glyph outlines.
fn text_doc() -> Document {
    let mut d = Document::new("gpu text");
    let p = d.first_page().unwrap();
    let mut td = TextData::new("Hello, FigCraft");
    td.style.font_size = 40.0;
    let id = d.create(p, NodeKind::Text(td), "title", 30.0, 70.0, 400.0, 80.0).unwrap();
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let mut sub = TextData::new("smaller text 123");
    sub.style.font_size = 18.0;
    let id = d.create(p, NodeKind::Text(sub), "body", 30.0, 160.0, 400.0, 40.0).unwrap();
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex("404040").unwrap_or(Color::BLACK))];
    d
}

#[test]
fn the_gpu_text_matches_the_cpu_render() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = text_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = scene::build_with(&d, p, &view, &mut scene::NoLayers).expect("the GPU draws text");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("text parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 160, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 1.0, "the mean difference is {mean:.3}");
}

/// Coloured stripes with a rounded pane over them that blurs what it covers.
fn backdrop_doc() -> Document {
    let mut d = Document::new("gpu backdrop");
    let p = d.first_page().unwrap();
    let colours = ["E03030", "2040E0", "20C060", "F0C830", "A040E0", "20B0D0"];
    for i in 0..8u32 {
        let id = d.create(p, NodeKind::Rectangle, "stripe", 20.0 + f64::from(i) * 70.0, 60.0, 62.0, 200.0).unwrap();
        d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(colours[i as usize % colours.len()]).unwrap_or(Color::WHITE))];
    }
    let pane = d.create(p, NodeKind::Rectangle, "pane", 150.0, 90.0, 240.0, 150.0).unwrap();
    {
        let n = d.get_mut(pane).unwrap();
        n.corners.radii = [28.0; 4];
        n.fills = vec![Paint::solid(Color { a: 0.15, ..Color::WHITE })];
        n.effects = vec![Effect::background_blur()];
    }
    d
}

#[test]
fn the_gpu_background_blur_matches_the_cpu_render() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = backdrop_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    }
    .expect("the GPU draws a background blur");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("background blur parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
}

/// Coloured stripes with a glass pane over them.
fn glass_doc() -> Document {
    let mut d = Document::new("gpu glass");
    let p = d.first_page().unwrap();
    let colours = ["E03030", "2040E0", "20C060", "F0C830"];
    for i in 0..12u32 {
        let id = d.create(p, NodeKind::Rectangle, "stripe", f64::from(i) * 50.0, 0.0, 50.0, 300.0).unwrap();
        d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(colours[i as usize % colours.len()]).unwrap_or(Color::WHITE))];
    }
    let pane = d.create(p, NodeKind::Rectangle, "glass", 120.0, 60.0, 300.0, 180.0).unwrap();
    {
        let n = d.get_mut(pane).unwrap();
        n.corners.radii = [40.0; 4];
        n.fills = vec![Paint::solid(Color { a: 0.1, ..Color::WHITE })];
        n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
    }
    d
}

/// Backdrop effects inside containers: a glass pane in a group, a blurred card in a frame.
fn nested_backdrop_doc() -> Document {
    let mut d = Document::new("gpu nested backdrops");
    let p = d.first_page().unwrap();
    let colours = ["E03030", "2040E0", "20C060", "F0C830"];
    for i in 0..18u32 {
        let id = d.create(p, NodeKind::Rectangle, "stripe", f64::from(i) * 50.0, 0.0, 50.0, 400.0).unwrap();
        d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(colours[i as usize % colours.len()]).unwrap_or(Color::WHITE))];
    }
    let group = d.create(p, NodeKind::Group, "card group", 60.0, 40.0, 520.0, 320.0).unwrap();
    let card = d.create(group, NodeKind::Rectangle, "card", 0.0, 0.0, 520.0, 320.0).unwrap();
    d.get_mut(card).unwrap().fills = vec![Paint::solid(Color::from_hex("FFFFFF").unwrap_or(Color::WHITE))];
    let pane = d.create(group, NodeKind::Rectangle, "nested glass", 40.0, 40.0, 240.0, 160.0).unwrap();
    {
        let n = d.get_mut(pane).unwrap();
        n.corners.radii = [32.0; 4];
        n.fills = vec![Paint::solid(Color { a: 0.1, ..Color::WHITE })];
        n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
    }
    let frame = d.create(p, NodeKind::Frame, "blurred card", 600.0, 40.0, 240.0, 200.0).unwrap();
    {
        let n = d.get_mut(frame).unwrap();
        n.corners.radii = [24.0; 4];
        n.fills = vec![Paint::solid(Color::from_hex("F06030").unwrap_or(Color::WHITE))];
        n.clip = true;
    }
    let blurred = d.create(frame, NodeKind::Rectangle, "nested blur", 30.0, 30.0, 160.0, 100.0).unwrap();
    {
        let n = d.get_mut(blurred).unwrap();
        n.corners.radii = [16.0; 4];
        let b = figcraft_doc::Blur { radius: 24.0, ..Default::default() };
        n.effects = vec![Effect { kind: EffectKind::BackgroundBlur(b), visible: true }];
    }
    d
}

#[test]
fn the_gpu_draws_nested_backdrops() {
    if device().is_none() {
        eprintln!("no GPU adapter: skipping");
        return;
    }
    let d = nested_backdrop_doc();
    let Some((worst, mean, over)) = segmented_parity(&d, "nested backdrops") else { return };
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
    assert!(over < (PARITY.0 * PARITY.1) as usize / 20, "{over} pixels differ by more than 8");
}

/// Four stripes, a glass pane of its own, then a group whose nested pane reads the stripes under
/// it: the plan is `Nodes(stripes), Backdrop(pane), Nodes(group)`.
fn pane_and_nested_glass_doc() -> Document {
    let mut d = Document::new("gpu retained steps");
    let p = d.first_page().unwrap();
    let colours = ["E03030", "2040E0", "20C060", "F0C830"];
    for i in 0..4u32 {
        let id = d.create(p, NodeKind::Rectangle, "stripe", f64::from(i) * 60.0, 0.0, 60.0, 300.0).unwrap();
        d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(colours[i as usize]).unwrap_or(Color::WHITE))];
    }
    let pane = d.create(p, NodeKind::Rectangle, "pane", 40.0, 180.0, 200.0, 120.0).unwrap();
    {
        let n = d.get_mut(pane).unwrap();
        n.corners.radii = [24.0; 4];
        n.fills = vec![Paint::solid(Color { a: 0.1, ..Color::WHITE })];
        n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
    }
    // The nested pane overlaps both the stripes and the pane before it: what it refracts includes
    // them, so its pixels are not its own subtree's.
    let group = d.create(p, NodeKind::Group, "group", 20.0, 60.0, 300.0, 160.0).unwrap();
    let nested = d.create(group, NodeKind::Rectangle, "nested pane", 30.0, 20.0, 200.0, 100.0).unwrap();
    {
        let n = d.get_mut(nested).unwrap();
        n.corners.radii = [16.0; 4];
        n.fills = vec![Paint::solid(Color { a: 0.1, ..Color::WHITE })];
        n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
    }
    d
}

/// The ids of a plan's first ordinary step (the layers before the page's first backdrop node).
fn first_step(plan: &[scene::Step]) -> Option<Vec<figcraft_doc::NodeId>> {
    plan.iter().find_map(|s| match s {
        scene::Step::Nodes(ids) => Some(ids.clone()),
        scene::Step::Backdrop(_) | scene::Step::PlusDarker(_) => None,
    })
}

/// Draw `d`'s first page through the segmented path on `canvas` and read the frame back.
fn segmented_pixels(canvas: &mut CanvasGpu, device: &Device, queue: &Queue, d: &Document, view: &View) -> Option<Pixmap> {
    let p = d.first_page()?;
    let plan = scene::plan(d, p).ok()?;
    crate::canvas::segmented_frame(device, queue, canvas, d, p, view, &plan, scene::background(d, p))?;
    canvas.read(device, queue)
}

/// A cloned document with `id`'s paint changed: what the engine hands the canvas after an edit
/// (the nodes are new `Arc`s there, so the step's key changes).
fn recoloured(d: &Document, id: figcraft_doc::NodeId, hex: &str) -> Document {
    let mut edited = d.clone();
    if let Ok(n) = edited.get_mut(id) {
        n.fills = vec![Paint::solid(Color::from_hex(hex).unwrap_or(Color::WHITE))];
    }
    edited
}

#[test]
fn the_gpu_keeps_an_unchanged_step_between_frames() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = pane_and_nested_glass_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let plan = scene::plan(&d, p).unwrap();
    let stripe = d.node(p).and_then(|p| p.children.first().copied()).expect("a stripe");
    let ids = first_step(&plan).expect("the stripes are a step of their own");
    assert!(scene::step_is_self_contained(&d, &ids), "the stripes read no backdrop");
    let key = scene::step_key(&d, &view, &ids);
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let first = segmented_pixels(&mut canvas, &device, &queue, &d, &view).expect("drawn");
    assert!(canvas.step_kept(0, key).is_some(), "the unchanged step is kept");
    // The same document again: the step is laid over the page from its kept segment.
    let again = segmented_pixels(&mut canvas, &device, &queue, &d, &view).expect("drawn");
    assert_eq!(first.data(), again.data(), "a kept step draws the same pixels");
    // An edit inside the step gives it a new key: it is drawn again, and the frame matches one
    // from a canvas that never drew this document before.
    let edited = recoloured(&d, stripe, "102030");
    let new_key = scene::step_key(&edited, &view, &ids);
    assert_ne!(key, new_key, "an edited step has a new key");
    let changed = segmented_pixels(&mut canvas, &device, &queue, &edited, &view).expect("drawn");
    let mut fresh = CanvasGpu::new(&device, w, h).unwrap();
    let expected = segmented_pixels(&mut fresh, &device, &queue, &edited, &view).expect("drawn");
    assert_eq!(changed.data(), expected.data(), "the changed step is not taken from the kept segment");
}

#[test]
fn the_gpu_keeps_a_step_while_a_later_backdrop_moves() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = pane_and_nested_glass_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let plan = scene::plan(&d, p).unwrap();
    let ids = first_step(&plan).expect("the stripes are a step of their own");
    let key = scene::step_key(&d, &view, &ids);
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    segmented_pixels(&mut canvas, &device, &queue, &d, &view).expect("drawn");
    // The pane moves: the stripes did not change, so their step stays kept.
    let pane = d.node(p).and_then(|p| p.children.get(4).copied()).expect("the pane");
    let moved_by = figcraft_geom::Affine::translate((10.0, 6.0));
    let mut edited = d.clone();
    let was = edited.get(pane).map(|n| n.transform).unwrap_or(figcraft_geom::Affine::IDENTITY);
    if let Ok(n) = edited.get_mut(pane) {
        n.transform = moved_by * was;
    }
    assert_eq!(scene::step_key(&edited, &view, &ids), key, "the stripes did not change");
    let moved = segmented_pixels(&mut canvas, &device, &queue, &edited, &view).expect("drawn");
    assert!(canvas.step_kept(0, key).is_some(), "the unchanged step is kept");
    let mut fresh = CanvasGpu::new(&device, w, h).unwrap();
    let expected = segmented_pixels(&mut fresh, &device, &queue, &edited, &view).expect("drawn");
    assert_eq!(moved.data(), expected.data(), "a moved pane draws over the kept step correctly");
}

#[test]
fn a_step_that_reads_the_page_is_drawn_every_frame() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = pane_and_nested_glass_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let plan = scene::plan(&d, p).unwrap();
    let group = first_step(&plan).expect("the stripes");
    assert!(scene::step_is_self_contained(&d, &group), "the stripes read no backdrop");
    let nested = plan
        .iter()
        .filter_map(|s| match s {
            scene::Step::Nodes(ids) => Some(ids.clone()),
            scene::Step::Backdrop(_) | scene::Step::PlusDarker(_) => None,
        })
        .nth(1)
        .expect("the group is a step of its own");
    assert!(!scene::step_is_self_contained(&d, &nested), "the group's nested pane reads the stripes under it");
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    segmented_pixels(&mut canvas, &device, &queue, &d, &view).expect("drawn");
    // The stripes change under the nested pane: its step must be drawn again, not kept.
    let stripe = d.node(p).and_then(|p| p.children.get(2).copied()).expect("a stripe under the pane");
    let edited = recoloured(&d, stripe, "A040E0");
    let changed = segmented_pixels(&mut canvas, &device, &queue, &edited, &view).expect("drawn");
    let mut fresh = CanvasGpu::new(&device, w, h).unwrap();
    let expected = segmented_pixels(&mut fresh, &device, &queue, &edited, &view).expect("drawn");
    assert_eq!(changed.data(), expected.data(), "a step that reads the page is not kept");
}

/// A scene that draws an effect layer's image is not kept between frames: the canvas reuses those
/// images (and their atlas slots), so a kept scene would later draw whatever pixels the image holds
/// then. What it does keep is the layer itself, under the layer's own key.
#[test]
fn a_scene_that_draws_a_layer_image_is_not_kept() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = progressive_card_doc(120.0);
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let first = single_scene_pixels(&mut canvas, &device, &queue, &d, &view).expect("drawn");
    assert_eq!(canvas.kept_node_scenes(), 1, "the ground's scene is kept, the card's (it draws its blurred layer) is not");
    let again = single_scene_pixels(&mut canvas, &device, &queue, &d, &view).expect("drawn");
    assert_eq!(again.data(), first.data(), "the same frame draws the same pixels");
    assert!(canvas.kept_shadow_hits() > 0, "the blurred layer itself was kept");
}

/// Draw `d`'s first page as the single-scene path does (what `canvas::present` uses when the page
/// has no top-level backdrop node) and read it back.
fn single_scene_pixels(canvas: &mut CanvasGpu, device: &Device, queue: &Queue, d: &Document, view: &View) -> Option<Pixmap> {
    let p = d.first_page()?;
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas, device, queue };
        scene::build_with(d, p, view, &mut layers).ok()?
    };
    canvas.render(device, queue, &built, scene::background(d, p))?;
    canvas.read(device, queue)
}

/// Scenes are kept between frames unless they draw a layer image (an effect layer's image is the
/// canvas's to reuse, so a scene that kept one could later draw another layer's pixels). The cards
/// here cast their shadows from their outlines (a spread), which vello draws as blurred rects: no
/// image, so their scenes are kept.
#[test]
fn a_drag_reuses_the_encoded_scenes_of_the_layers_it_did_not_move() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = shadow_cards_with(12, 2.0);
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let first = single_scene_pixels(&mut canvas, &device, &queue, &d, &view).expect("drawn");
    assert_eq!(canvas.kept_node_scenes(), 12, "every layer's scene is kept");
    assert_eq!(canvas.node_hits(), 0, "the first frame encodes them all");
    let again = single_scene_pixels(&mut canvas, &device, &queue, &d, &view).expect("drawn");
    assert_eq!(again.data(), first.data(), "a kept scene draws the same pixels");
    assert_eq!(canvas.node_hits(), 12, "an unchanged frame re-encodes nothing");
    // One card moves: only its scene is encoded again, and the frame matches a fresh canvas's.
    let card = d.node(p).and_then(|p| p.children.get(5).copied()).expect("a card");
    let by = figcraft_geom::Affine::translate((7.0, 0.0));
    let mut moved = d.clone();
    let was = moved.get(card).map(|n| n.transform).unwrap_or(figcraft_geom::Affine::IDENTITY);
    if let Ok(n) = moved.get_mut(card) {
        n.transform = by * was;
    }
    let changed = single_scene_pixels(&mut canvas, &device, &queue, &moved, &view).expect("drawn");
    assert_eq!(canvas.node_hits(), 12 + 11, "eleven scenes were kept, the moved card's was encoded");
    let mut fresh = CanvasGpu::new(&device, w, h).unwrap();
    let expected = single_scene_pixels(&mut fresh, &device, &queue, &moved, &view).expect("drawn");
    assert_eq!(changed.data(), expected.data(), "the moved card is drawn from the document, not a kept scene");
}

#[test]
fn a_container_with_a_backdrop_effect_is_not_kept() {
    let d = nested_backdrop_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let children = d.node(p).map(|p| p.children.clone()).unwrap_or_default();
    let stripes = children.first().copied().expect("a stripe");
    let group = children.iter().copied().find(|id| d.node(*id).is_some_and(|n| n.name == "card group")).expect("the group");
    assert!(scene::node_scene_key(&d, &view, stripes).is_some(), "a stripe is its own");
    assert!(scene::node_scene_key(&d, &view, group).is_none(), "the group's glass reads the page under it");
}

/// Coloured stripes under a translucent card that darkens them (plus-darker), shadow and all.
fn plus_darker_doc() -> Document {
    let mut d = Document::new("gpu plus-darker");
    let p = d.first_page().unwrap();
    let colours = ["E03030", "2040E0", "20C060", "F0C830", "A040E0", "20B0D0"];
    for (i, c) in colours.iter().enumerate() {
        let x = f64::from(u32::try_from(i).unwrap_or(0)) * 60.0 + 20.0;
        let id = d.create(p, NodeKind::Rectangle, "stripe", x, 20.0, 60.0, 260.0).unwrap();
        d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(c).unwrap_or(Color::WHITE))];
    }
    let card = d.create(p, NodeKind::Rectangle, "plus card", 50.0, 50.0, 260.0, 160.0).unwrap();
    {
        let n = d.get_mut(card).unwrap();
        n.corners.radii = [24.0; 4];
        n.fills = vec![Paint::solid(Color { a: 0.55, ..Color::from_hex("C8C8C8").unwrap_or(Color::WHITE) })];
        n.blend = BlendMode::PlusDarker;
        n.effects = vec![Effect::drop_shadow()];
    }
    d
}

#[test]
fn the_gpu_draws_a_plus_darker_layer() {
    if device().is_none() {
        eprintln!("no GPU adapter: skipping");
        return;
    }
    let d = plus_darker_doc();
    let p = d.first_page().unwrap();
    let plan = scene::plan(&d, p).expect("the page is drawn in segments");
    assert!(plan.iter().any(|s| matches!(s, scene::Step::PlusDarker(_))), "the plus-darker card is a step of its own");
    let Some((worst, mean, over)) = segmented_parity(&d, "plus-darker") else { return };
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
    assert!(over < (PARITY.0 * PARITY.1) as usize / 20, "{over} pixels differ by more than 8");
}

#[test]
fn the_gpu_glass_matches_the_cpu_canvas() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = glass_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    }
    .expect("the GPU draws glass");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("glass parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
}

/// Draw `d` through the segmented path and compare with the CPU render; returns the statistics.
fn segmented_parity(d: &Document, label: &str) -> Option<(u8, f64, usize)> {
    let (device, queue) = device()?;
    let p = d.first_page()?;
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(d, p, &view, &Options::default())?;
    let plan = scene::plan(d, p).ok()?;
    let mut canvas = CanvasGpu::new(&device, w, h)?;
    crate::canvas::segmented_frame(&device, &queue, &mut canvas, d, p, &view, &plan, scene::background(d, p))?;
    let gpu = canvas.read(&device, &queue)?;
    if std::env::var("FIGCRAFT_PARITY_PNG").is_ok() {
        let dir = std::env::temp_dir();
        let name = label.replace(' ', "-");
        let _ = cpu.save_png(dir.join(format!("figcraft-seg-{name}-cpu.png")));
        let _ = gpu.save_png(dir.join(format!("figcraft-seg-{name}-gpu.png")));
    }
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    let (mut lo, mut hi) = ((u32::MAX, u32::MAX), (0u32, 0u32));
    let w0 = cpu.width().max(1) as usize;
    let mut first = None;
    for (i, (a, b)) in cpu.data().iter().zip(gpu.data()).enumerate() {
        if a.abs_diff(*b) > 8 {
            let (x, y) = ((i / 4 % w0) as u32, (i / 4 / w0) as u32);
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
            if first.is_none() {
                first = Some((x, y));
            }
        }
    }
    println!("{label} (segmented): worst {worst} at {at:?}, mean {mean:.3}, {over} differ >8, span {lo:?}..{hi:?}, first {first:?}");
    let px = |p: &Pixmap, (x, y): (u32, u32)| {
        let i = ((y as usize * p.width() as usize) + x as usize) * 4;
        p.data().get(i..i + 4).map(<[u8]>::to_vec)
    };
    println!("  at {at:?}: CPU {:?}, GPU {:?}", px(&cpu, at), px(&gpu, at));
    Some((worst, mean, over))
}

#[test]
fn the_segmented_page_matches_the_cpu_with_one_glass_pane() {
    if device().is_none() {
        eprintln!("no GPU adapter: skipping");
        return;
    }
    let d = glass_doc();
    let (worst, mean, _) = segmented_parity(&d, "one glass pane").expect("drawn");
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
}

#[test]
fn the_segmented_page_matches_the_cpu_with_one_background_blur() {
    if device().is_none() {
        eprintln!("no GPU adapter: skipping");
        return;
    }
    let d = backdrop_doc();
    let (worst, mean, _) = segmented_parity(&d, "one background blur").expect("drawn");
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
}

#[test]
fn the_gpu_page_matches_with_several_backdrop_layers() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = heavy_effects_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let plan = scene::plan(&d, p).expect("the page is drawn in segments");
    assert!(plan.iter().filter(|s| matches!(s, scene::Step::Backdrop(_))).count() >= 2, "several backdrop layers");
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let drawn = crate::canvas::segmented_frame(&device, &queue, &mut canvas, &d, p, &view, &plan, scene::background(&d, p));
    assert!(drawn.is_some(), "the segmented page drew");
    let gpu = canvas.read(&device, &queue).unwrap();
    if std::env::var("FIGCRAFT_PARITY_PNG").is_ok() {
        let dir = std::env::temp_dir();
        let _ = cpu.save_png(dir.join("figcraft-seg-cpu.png"));
        let _ = gpu.save_png(dir.join("figcraft-seg-gpu.png"));
    }
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    let px = |p: &Pixmap, (x, y): (u32, u32)| {
        let i = ((y as usize * p.width() as usize) + x as usize) * 4;
        p.data().get(i..i + 4).map(<[u8]>::to_vec)
    };
    println!(
        "segmented page parity: worst {worst} at {at:?} (CPU {:?}, GPU {:?}), mean {mean:.3}, {over} pixels differ by more than 8",
        px(&cpu, at),
        px(&gpu, at)
    );
    assert!(worst <= 160, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 2.0, "the mean difference is {mean:.3}");
}

/// The shadow variations the fast path now draws without a layer: spread, several shadows, a
/// gradient fill, an opaque stroke, a clipping frame with children, a rotation and a shadow with a
/// blend mode of its own.
fn shadow_breadth_doc() -> Document {
    let mut d = Document::new("gpu shadow breadth");
    let p = d.first_page().unwrap();
    let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::WHITE);
    let stops =
        |a: Color, b: Color| vec![figcraft_color::ColorStop { position: 0.0, color: a }, figcraft_color::ColorStop { position: 1.0, color: b }];
    let shadow = |radius: f64, spread: f64, offset: (f64, f64), show_behind: bool, blend: BlendMode| {
        let mut e = Effect::drop_shadow();
        if let EffectKind::DropShadow(s) = &mut e.kind {
            s.radius = radius;
            s.spread = spread;
            s.offset = offset;
            s.show_behind = show_behind;
            s.blend = blend;
        }
        e
    };

    let spread = d.create(p, NodeKind::Rectangle, "spread", 24.0, 20.0, 140.0, 90.0).unwrap();
    {
        let n = d.get_mut(spread).unwrap();
        n.corners.radii = [18.0; 4];
        let mut g =
            Gradient { kind: GradientKind::Linear, stops: stops(hex("F0C830"), hex("E03030")), from: (0.0, 0.0), to: (1.0, 1.0), width: (-0.5, 0.5) };
        g.square_linear_width(140.0, 90.0);
        n.fills = vec![Paint::gradient(g)];
        n.effects = vec![shadow(10.0, 6.0, (4.0, 6.0), false, BlendMode::Normal)];
    }

    let two = d.create(p, NodeKind::Rectangle, "two shadows", 200.0, 20.0, 140.0, 90.0).unwrap();
    {
        let n = d.get_mut(two).unwrap();
        n.corners.radii = [12.0; 4];
        n.fills = vec![Paint::solid(hex("2040E0"))];
        n.effects = vec![shadow(8.0, 0.0, (0.0, 4.0), false, BlendMode::Normal), shadow(14.0, 2.0, (2.0, 8.0), false, BlendMode::Multiply)];
    }

    let stroked = d.create(p, NodeKind::Rectangle, "stroked", 380.0, 20.0, 150.0, 90.0).unwrap();
    {
        let n = d.get_mut(stroked).unwrap();
        n.corners.radii = [8.0; 4];
        n.fills = vec![Paint::solid(hex("20C060"))];
        n.strokes = vec![Paint::solid(hex("FFFFFF"))];
        n.stroke.weight = 6.0;
        n.stroke.align = StrokeAlign::Outside;
        n.effects = vec![shadow(6.0, 0.0, (0.0, 3.0), false, BlendMode::Normal)];
    }

    // A clipping frame: its children paint over the shadow, and cannot reach past the frame.
    let frame = d.create(p, NodeKind::Frame, "clipped card", 24.0, 150.0, 160.0, 110.0).unwrap();
    {
        let n = d.get_mut(frame).unwrap();
        n.corners.radii = [16.0; 4];
        n.fills = vec![Paint::solid(hex("FFFFFF"))];
        n.clip = true;
        n.effects = vec![shadow(6.0, -2.0, (0.0, 2.0), false, BlendMode::Normal)];
    }
    let child = d.create(frame, NodeKind::Rectangle, "child", 96.0, 60.0, 120.0, 90.0).unwrap();
    d.get_mut(child).unwrap().fills = vec![Paint::solid(hex("20B0D0"))];

    let turned = d.create(p, NodeKind::Rectangle, "turned", 240.0, 160.0, 120.0, 80.0).unwrap();
    {
        let n = d.get_mut(turned).unwrap();
        let turn = 12f64.to_radians();
        n.transform = figcraft_geom::Affine::new([turn.cos(), turn.sin(), -turn.sin(), turn.cos(), 260.0, 170.0]);
        n.corners.radii = [10.0; 4];
        n.fills = vec![Paint::solid(hex("A040E0"))];
        n.effects = vec![shadow(8.0, 3.0, (3.0, 5.0), false, BlendMode::Normal)];
    }

    let behind = d.create(p, NodeKind::Rectangle, "shows behind", 400.0, 160.0, 150.0, 90.0).unwrap();
    {
        let n = d.get_mut(behind).unwrap();
        n.corners.radii = [20.0; 4];
        n.fills = vec![Paint::solid(hex("F0C830"))];
        n.effects = vec![shadow(9.0, 1.0, (0.0, 6.0), true, BlendMode::Normal)];
    }

    // A translucent card: its shadow is as faint as it paints and hidden where it covers.
    let faint = d.create(p, NodeKind::Rectangle, "faint card", 455.0, 240.0, 125.0, 50.0).unwrap();
    {
        let n = d.get_mut(faint).unwrap();
        n.corners.radii = [12.0; 4];
        n.fills = vec![Paint::solid(Color { a: 0.35, ..hex("20B0D0") })];
        // One shadow cast from the layer's pixels (as faint as the card) and one from its outline.
        n.effects = vec![shadow(10.0, 0.0, (3.0, 6.0), false, BlendMode::Normal), shadow(6.0, 3.0, (0.0, 2.0), false, BlendMode::Normal)];
    }
    d
}

#[test]
fn the_gpu_draws_the_shadow_variations() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = shadow_breadth_doc();
    let p = d.first_page().unwrap();
    assert_eq!(scene::supported(&d, p), Ok(()), "a page of shadow variations stays on the GPU");
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = scene::build_with(&d, p, &view, &mut scene::NoLayers).expect("the GPU draws the shadows");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    if std::env::var("FIGCRAFT_PARITY_PNG").is_ok() {
        let dir = std::env::temp_dir();
        let _ = cpu.save_png(dir.join("figcraft-shadows-cpu.png"));
        let _ = gpu.save_png(dir.join("figcraft-shadows-gpu.png"));
    }
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    let px = |p: &Pixmap, (x, y): (u32, u32)| {
        let i = ((y as usize * p.width() as usize) + x as usize) * 4;
        p.data().get(i..i + 4).map(<[u8]>::to_vec)
    };
    println!(
        "shadow breadth parity: worst {worst} at {at:?} (CPU {:?}, GPU {:?}), mean {mean:.3}, {over} pixels differ by more than 8",
        px(&cpu, at),
        px(&gpu, at)
    );
    assert!(worst <= 96, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 1.0, "the mean difference is {mean:.3}");
}

/// A photo-like PNG: a diagonal gradient with a checker of squares, opaque.
fn photo_png(w: u32, h: u32) -> Vec<u8> {
    use image::ImageEncoder as _;
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let (fx, fy) = (f64::from(x) / f64::from(w), f64::from(y) / f64::from(h));
        let t = fx * 0.6 + fy * 0.4;
        let band = if (x / 16 + y / 16) % 2 == 0 { 1.0 } else { 0.86 };
        let c = |a: f64, b: f64| ((a + (b - a) * t) * band * 255.0).round().clamp(0.0, 255.0) as u8;
        *px = image::Rgba([c(0.08, 0.95), c(0.30, 0.72), c(0.85, 0.18), 255]);
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8).expect("png");
    out
}

/// Every paint the GPU now draws as the renderer's own pixmap or in a layer of its own: a photo in
/// each scale mode (1:1, fitted, cropped, tiled, rotated, translucent), a filtered one, a diamond
/// gradient, a pattern fill and paints with their own blend modes.
fn paints_doc() -> Document {
    let mut d = Document::new("gpu paints");
    let p = d.first_page().unwrap();
    let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::WHITE);
    let photo = "photo".to_string();
    d.images.insert(photo.clone(), figcraft_doc::ImageData { bytes: Arc::new(photo_png(320, 200)), width: 320, height: 200 });

    let image = |mode: figcraft_color::ImageScaleMode, tune: &dyn Fn(&mut figcraft_color::ImagePaint)| {
        let mut paint = Paint::image(photo.clone(), mode);
        if let figcraft_color::PaintKind::Image(img) = &mut paint.kind {
            tune(img);
        }
        paint
    };
    // 1:1, fitted (letterboxed), cropped, tiled and rotated.
    fn add(d: &mut Document, p: figcraft_doc::NodeId, name: &str, at: (f64, f64), size: (f64, f64), paints: Vec<Paint>) -> figcraft_doc::NodeId {
        let id = d.create(p, NodeKind::Rectangle, name, at.0, at.1, size.0, size.1).unwrap();
        d.get_mut(id).unwrap().fills = paints;
        id
    }

    add(&mut d, p, "photo 1:1", (20.0, 20.0), (320.0, 200.0), vec![image(figcraft_color::ImageScaleMode::Fill, &|_| {})]);
    add(&mut d, p, "photo fit", (360.0, 20.0), (180.0, 200.0), vec![image(figcraft_color::ImageScaleMode::Fit, &|_| {})]);
    add(
        &mut d,
        p,
        "photo crop",
        (560.0, 20.0),
        (180.0, 200.0),
        vec![image(figcraft_color::ImageScaleMode::Crop, &|i| i.crop = Some([2.0, 0.0, 0.0, 1.6, -0.5, -0.3]))],
    );
    add(&mut d, p, "photo tile", (20.0, 240.0), (180.0, 180.0), vec![image(figcraft_color::ImageScaleMode::Tile, &|i| i.scale = 0.5)]);
    add(&mut d, p, "photo turned", (220.0, 240.0), (200.0, 180.0), vec![image(figcraft_color::ImageScaleMode::Fill, &|i| i.rotation = 1)]);
    // A filtered photo, and a translucent one (the sampler's own alpha).
    add(&mut d, p, "photo tuned", (440.0, 240.0), (140.0, 180.0), vec![image(figcraft_color::ImageScaleMode::Fill, &|i| i.filters.exposure = 0.3)]);
    let mut faint = image(figcraft_color::ImageScaleMode::Fill, &|_| {});
    faint.opacity = 0.55;
    add(&mut d, p, "photo faint", (600.0, 240.0), (140.0, 180.0), vec![faint]);

    // A diamond gradient: the renderer's 256 × 256 tile through the handle frame.
    let diamond = Gradient {
        kind: GradientKind::Diamond,
        stops: vec![
            figcraft_color::ColorStop { position: 0.0, color: hex("FFF3C0") },
            figcraft_color::ColorStop { position: 0.5, color: hex("F0A030") },
            figcraft_color::ColorStop { position: 1.0, color: hex("6030A0") },
        ],
        from: (0.0, 0.0),
        to: (1.0, 1.0),
        width: (0.0, 1.0),
    };
    add(&mut d, p, "diamond", (760.0, 20.0), (120.0, 200.0), vec![Paint::gradient(diamond)]);

    // A pattern fill: a hidden 24 × 24 layer tiled at its own size.
    let tile = add(&mut d, p, "tile source", (0.0, 0.0), (24.0, 24.0), vec![Paint::solid(hex("F0C830"))]);
    d.get_mut(tile).unwrap().visible = false;
    d.get_mut(tile).unwrap().kind = NodeKind::Frame;
    let dot = d.create(tile, NodeKind::Rectangle, "dot", 7.0, 7.0, 10.0, 10.0).unwrap();
    d.get_mut(dot).unwrap().fills = vec![Paint::solid(hex("2040E0"))];
    add(&mut d, p, "pattern", (760.0, 240.0), (120.0, 180.0), vec![Paint::pattern(tile.0)]);

    // Paints with their own blend modes, in a layer each.
    add(&mut d, p, "ground", (20.0, 440.0), (300.0, 40.0), vec![Paint::solid(hex("40A060"))]);
    let mut over = Paint::solid(hex("E03030"));
    over.blend = BlendMode::Multiply;
    add(&mut d, p, "multiplied", (120.0, 450.0), (260.0, 40.0), vec![over]);
    let mut screen = Paint::solid(hex("2040E0"));
    screen.blend = BlendMode::Screen;
    add(&mut d, p, "screened", (420.0, 450.0), (200.0, 40.0), vec![screen]);
    d
}

#[test]
fn the_gpu_draws_the_paint_variations() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = paints_doc();
    let p = d.first_page().unwrap();
    assert_eq!(scene::supported(&d, p), Ok(()), "a page of images, patterns and paint blends stays on the GPU");
    let (w, h) = (900u32, 500u32);
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = scene::build_with(&d, p, &view, &mut scene::NoLayers).expect("the GPU draws the paints");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    if std::env::var("FIGCRAFT_PARITY_PNG").is_ok() {
        let dir = std::env::temp_dir();
        let _ = cpu.save_png(dir.join("figcraft-paints-cpu.png"));
        let _ = gpu.save_png(dir.join("figcraft-paints-gpu.png"));
    }
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    let px = |p: &Pixmap, (x, y): (u32, u32)| {
        let i = ((y as usize * p.width() as usize) + x as usize) * 4;
        p.data().get(i..i + 4).map(<[u8]>::to_vec)
    };
    println!(
        "paint breadth parity: worst {worst} at {at:?} (CPU {:?}, GPU {:?}), mean {mean:.3}, {over} pixels differ by more than 8",
        px(&cpu, at),
        px(&gpu, at)
    );
    assert!(worst <= 128, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 1.5, "the mean difference is {mean:.3}");
}

/// Drop shadows of shapes that aren't one rounded rect (an ellipse, a star, a polygon, per-corner
/// radii, text, a group, a frame whose children reach past it) and inner shadows on shapes.
fn shadow_shape_doc() -> Document {
    let mut d = Document::new("gpu shadow shapes");
    let p = d.first_page().unwrap();
    let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::WHITE);
    let shadow = |radius: f64, spread: f64, offset: (f64, f64)| {
        let mut e = Effect::drop_shadow();
        if let EffectKind::DropShadow(s) = &mut e.kind {
            s.radius = radius;
            s.spread = spread;
            s.offset = offset;
        }
        e
    };
    let inner = |radius: f64, spread: f64, offset: (f64, f64)| Effect {
        kind: EffectKind::InnerShadow(figcraft_doc::Shadow { radius, spread, offset, ..figcraft_doc::Shadow::default() }),
        visible: true,
    };
    let add = |d: &mut Document, kind: NodeKind, name: &str, at: (f64, f64), size: (f64, f64), fill: &str| {
        let id = d.create(p, kind, name, at.0, at.1, size.0, size.1).unwrap();
        d.get_mut(id).unwrap().fills = vec![Paint::solid(hex(fill))];
        id
    };

    let ellipse =
        add(&mut d, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "ellipse", (20.0, 16.0), (160.0, 120.0), "E03030");
    d.get_mut(ellipse).unwrap().effects = vec![shadow(12.0, 0.0, (0.0, 6.0))];

    let star = add(&mut d, NodeKind::Star { count: 6, inner_ratio: 0.45 }, "star", (210.0, 16.0), (140.0, 120.0), "2040E0");
    d.get_mut(star).unwrap().effects = vec![shadow(10.0, 4.0, (4.0, 6.0))];

    let polygon = add(&mut d, NodeKind::Polygon { count: 7 }, "polygon", (380.0, 16.0), (130.0, 120.0), "20C060");
    d.get_mut(polygon).unwrap().effects = vec![shadow(9.0, -3.0, (3.0, 4.0))];

    let corners = add(&mut d, NodeKind::Rectangle, "per-corner card", (540.0, 16.0), (150.0, 120.0), "F0C830");
    {
        let n = d.get_mut(corners).unwrap();
        n.corners.radii = [0.0, 40.0, 8.0, 30.0];
        n.effects = vec![shadow(10.0, 0.0, (3.0, 5.0))];
    }

    let mut td = TextData::new("Shadowed");
    td.style.font_size = 34.0;
    let text = d.create(p, NodeKind::Text(td), "text", 20.0, 210.0, 200.0, 60.0).unwrap();
    d.get_mut(text).unwrap().fills = vec![Paint::solid(hex("303030"))];
    d.get_mut(text).unwrap().effects = vec![shadow(8.0, 2.0, (2.0, 3.0))];

    // A group: its shadow is cast from its pixels, not one outline.
    let group = d.create(p, NodeKind::Group, "group", 240.0, 190.0, 150.0, 120.0).unwrap();
    d.get_mut(group).unwrap().effects = vec![shadow(11.0, 0.0, (2.0, 8.0))];
    for (name, at, size, colour) in [("a", (0.0, 0.0), (80.0, 70.0), "A040E0"), ("b", (60.0, 45.0), (80.0, 70.0), "20B0D0")] {
        let c = d.create(group, NodeKind::Rectangle, name, at.0, at.1, size.0, size.1).unwrap();
        d.get_mut(c).unwrap().fills = vec![Paint::solid(hex(colour))];
    }

    // A frame whose child reaches past it: also cast from the pixels.
    let frame = d.create(p, NodeKind::Frame, "overflowing frame", 450.0, 190.0, 160.0, 120.0).unwrap();
    {
        let n = d.get_mut(frame).unwrap();
        n.fills = vec![Paint::solid(hex("FFFFFF"))];
        n.corners.radii = [18.0; 4];
        n.clip = false;
        n.effects = vec![shadow(10.0, 3.0, (2.0, 6.0))];
    }
    let reach = d.create(frame, NodeKind::Rectangle, "reaching child", 100.0, 40.0, 120.0, 100.0).unwrap();
    d.get_mut(reach).unwrap().fills = vec![Paint::solid(hex("F06030"))];

    // Inner shadows: a card, one with a spread, and an ellipse.
    let inner_card = add(&mut d, NodeKind::Rectangle, "inner card", (20.0, 380.0), (150.0, 120.0), "F0F0F0");
    {
        let n = d.get_mut(inner_card).unwrap();
        n.corners.radii = [24.0; 4];
        n.effects = vec![inner(14.0, 0.0, (0.0, 4.0))];
    }
    let inner_spread = add(&mut d, NodeKind::Rectangle, "inner spread", (200.0, 380.0), (150.0, 120.0), "E8E8F8");
    {
        let n = d.get_mut(inner_spread).unwrap();
        n.corners.radii = [16.0; 4];
        n.effects = vec![inner(10.0, 6.0, (4.0, 4.0))];
    }
    let inner_ellipse = add(
        &mut d,
        NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 },
        "inner ellipse",
        (380.0, 380.0),
        (150.0, 120.0),
        "D0E8F0",
    );
    d.get_mut(inner_ellipse).unwrap().effects = vec![inner(8.0, 0.0, (2.0, 2.0))];

    // A group's inner shadow: cast from around what its children draw together, over them.
    let inner_group = d.create(p, NodeKind::Group, "inner group", 540.0, 380.0, 150.0, 120.0).unwrap();
    d.get_mut(inner_group).unwrap().effects = vec![inner(12.0, 0.0, (3.0, 4.0))];
    for (name, at, size, colour) in [("a", (0.0, 0.0), (100.0, 80.0), "A040E0"), ("b", (50.0, 40.0), (100.0, 80.0), "20B0D0")] {
        let c = d.create(inner_group, NodeKind::Rectangle, name, at.0, at.1, size.0, size.1).unwrap();
        d.get_mut(c).unwrap().fills = vec![Paint::solid(hex(colour))];
    }

    // Effects together, in the CPU's order: inner shadows, the layer blur and grain, the shadows
    // cast from that result.
    let blurred = add(&mut d, NodeKind::Rectangle, "blur and shadow", (20.0, 530.0), (150.0, 110.0), "F0C830");
    {
        let n = d.get_mut(blurred).unwrap();
        n.corners.radii = [20.0; 4];
        n.effects = vec![Effect::layer_blur(), shadow(10.0, 0.0, (2.0, 6.0))];
    }
    let inner_and_shadow = add(&mut d, NodeKind::Rectangle, "inner and shadow", (200.0, 530.0), (150.0, 110.0), "FFFFFF");
    {
        let n = d.get_mut(inner_and_shadow).unwrap();
        n.corners.radii = [18.0; 4];
        n.effects = vec![inner(12.0, 0.0, (0.0, 4.0)), shadow(8.0, 0.0, (3.0, 7.0))];
    }
    let grain = add(&mut d, NodeKind::Rectangle, "blur, grain and shadow", (380.0, 530.0), (150.0, 110.0), "2040E0");
    {
        let n = d.get_mut(grain).unwrap();
        n.corners.radii = [24.0; 4];
        let nz = figcraft_doc::Noise { size: 1.0, density: 0.6, ..Default::default() };
        n.effects = vec![Effect::layer_blur(), Effect { kind: EffectKind::Noise(nz), visible: true }, shadow(10.0, 0.0, (2.0, 5.0))];
    }
    let ramp = add(&mut d, NodeKind::Rectangle, "progressive blur", (560.0, 530.0), (150.0, 110.0), "20C060");
    {
        let n = d.get_mut(ramp).unwrap();
        n.corners.radii = [20.0; 4];
        let blur = figcraft_doc::Blur {
            radius: 24.0,
            blur_type: figcraft_doc::BlurType::Progressive,
            start_radius: 0.0,
            start: (0.5, 0.0),
            end: (0.5, 1.0),
        };
        n.effects = vec![Effect { kind: EffectKind::LayerBlur(blur), visible: true }, shadow(8.0, 0.0, (2.0, 4.0))];
    }
    d
}

#[test]
fn the_gpu_draws_shadows_of_any_shape() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = shadow_shape_doc();
    let p = d.first_page().unwrap();
    assert_eq!(scene::supported(&d, p), Ok(()), "shadows of every shape stay on the GPU");
    let (w, h) = (900u32, 660u32);
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    }
    .expect("the GPU draws the shadows");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    if std::env::var("FIGCRAFT_PARITY_PNG").is_ok() {
        let dir = std::env::temp_dir();
        let _ = cpu.save_png(dir.join("figcraft-shadow-shapes-cpu.png"));
        let _ = gpu.save_png(dir.join("figcraft-shadow-shapes-gpu.png"));
    }
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    let px = |p: &Pixmap, (x, y): (u32, u32)| {
        let i = ((y as usize * p.width() as usize) + x as usize) * 4;
        p.data().get(i..i + 4).map(<[u8]>::to_vec)
    };
    println!(
        "shadow shapes parity: worst {worst} at {at:?} (CPU {:?}, GPU {:?}), mean {mean:.3}, {over} pixels differ by more than 8",
        px(&cpu, at),
        px(&gpu, at)
    );
    if std::env::var("FIGCRAFT_REGIONS").is_ok() {
        for (name, (rx, ry, rw, rh)) in [
            ("ellipse", (20u32, 16u32, 160u32, 120u32)),
            ("star", (210, 16, 140, 120)),
            ("polygon", (380, 16, 130, 120)),
            ("corners", (540, 16, 150, 120)),
            ("text", (20, 210, 200, 60)),
            ("group", (240, 190, 150, 120)),
            ("frame", (450, 190, 160, 120)),
            ("reaching child", (540, 220, 150, 130)),
            ("inner card", (20, 380, 150, 120)),
            ("inner spread", (200, 380, 150, 120)),
            ("inner ellipse", (380, 380, 150, 120)),
            ("inner group", (540, 380, 150, 120)),
            ("blur and shadow", (20, 530, 150, 110)),
            ("inner and shadow", (200, 530, 150, 110)),
            ("blur, grain", (380, 530, 150, 110)),
            ("progressive", (560, 530, 150, 110)),
        ] {
            let (mut sum, mut n, mut w) = (0f64, 0usize, 0u8);
            let mut wp = (rx, ry);
            for y in ry..(ry + rh).min(cpu.height()) {
                for x in rx..(rx + rw).min(cpu.width()) {
                    let i = ((y as usize * cpu.width() as usize) + x as usize) * 4;
                    for c in 0..4 {
                        let d = cpu.data()[i + c].abs_diff(gpu.data()[i + c]);
                        sum += f64::from(d);
                        n += 1;
                        if d > w {
                            w = d;
                            wp = (x, y);
                        }
                    }
                }
            }
            println!("  {name:14} worst {w:3} at {wp:?} (CPU {:?}, GPU {:?}), mean {:.3}", px(&cpu, wp), px(&gpu, wp), sum / n.max(1) as f64);
        }
    }
    assert!(worst <= 96, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 0.5, "the mean difference is {mean:.3}");
    assert!(over < (w * h) as usize / 100, "{over} pixels differ by more than 8");
    // Drawn again with the shadows kept: the same pixels, and an edit makes a new one.
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    }
    .unwrap();
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let again = canvas.read(&device, &queue).unwrap();
    let (w1, m1, _, o1) = diff(&gpu, &again);
    let shadowed =
        d.nodes().filter(|n| n.effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::DropShadow(_) | EffectKind::InnerShadow(_)))).count();
    println!("kept shadows: worst {w1}, mean {m1:.3}, {o1} differ by more than 8, {} kept of {shadowed} shadowed nodes", canvas.kept_shadows());
    if std::env::var("FIGCRAFT_REGIONS").is_ok() {
        let (w2, m2, _, o2) = diff(&cpu, &again);
        println!("  the second frame against the CPU: worst {w2}, mean {m2:.3}, {o2} differ by more than 8");
        for (name, (rx, ry, rw, rh)) in [("ellipse", (20u32, 16u32, 160u32, 120u32)), ("group", (240, 190, 150, 120))] {
            let (mut sum, mut n) = (0f64, 0usize);
            for y in ry..(ry + rh).min(gpu.height()) {
                for x in rx..(rx + rw).min(gpu.width()) {
                    let i = ((y as usize * gpu.width() as usize) + x as usize) * 4;
                    for c in 0..4 {
                        sum += f64::from(gpu.data()[i + c].abs_diff(again.data()[i + c]));
                        n += 1;
                    }
                }
            }
            println!("    {name}: frame 1 against frame 2 mean {:.3}", sum / n.max(1) as f64);
        }
        let mut span = ((u32::MAX, u32::MAX), (0u32, 0u32));
        for (i, (a, b)) in gpu.data().iter().zip(again.data()).enumerate() {
            if a.abs_diff(*b) > 8 {
                let (x, y) = ((i / 4 % gpu.width() as usize) as u32, (i / 4 / gpu.width() as usize) as u32);
                span = ((span.0.0.min(x), span.0.1.min(y)), (span.1.0.max(x), span.1.1.max(y)));
            }
        }
        println!("  kept differ span {span:?}");
    }
    assert_eq!((w1, o1), (0, 0), "a kept shadow draws the same pixels");
    assert!(canvas.kept_shadows() >= 10, "the page's shadows are kept");
    // An edit is a new node, and so a new shadow.
    let before = canvas.kept_shadows();
    let mut changed = d.clone();
    let star = changed.node(p).and_then(|n| n.children.get(1).copied()).unwrap();
    changed.get_mut(star).unwrap().fills = vec![Paint::solid(Color::from_hex("20C060").unwrap_or(Color::WHITE))];
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&changed, p, &view, &mut layers)
    }
    .unwrap();
    assert!(canvas.render(&device, &queue, &built, scene::background(&changed, p)).is_some());
    assert_eq!(canvas.kept_shadows(), before + 1, "a changed node's shadow is made again");
}

/// `n` rounded cards with a shadow no fast path draws straight into the scene (an ellipse), and
/// `n` per-corner cards (their shadows are cast from a layer).
fn shadow_layer_cards(n: u32, inner: bool) -> Document {
    let mut d = Document::new("gpu shadow layers");
    let p = d.first_page().unwrap();
    let colours = ["FFFFFF", "E03030", "2040E0", "20C060", "F0C830", "A040E0"];
    for i in 0..n {
        let (x, y) = (f64::from(i % 6) * 150.0 + 10.0, f64::from(i / 6) * 150.0 + 10.0);
        let kind = if inner { NodeKind::Rectangle } else { NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 } };
        let id = d.create(p, kind, "Card", x, y, 130.0, 130.0).unwrap();
        let node = d.get_mut(id).unwrap();
        if inner {
            node.corners.radii = [0.0, 34.0, 12.0, 26.0];
            let s = figcraft_doc::Shadow { radius: 14.0, offset: (0.0, 4.0), ..Default::default() };
            node.effects = vec![Effect { kind: EffectKind::InnerShadow(s), visible: true }];
        } else {
            node.effects = vec![Effect::drop_shadow()];
        }
        node.fills = vec![Paint::solid(Color::from_hex(colours[i as usize % colours.len()]).unwrap_or(Color::WHITE))];
    }
    d
}

/// The same if only `n` per-corner cards with drop shadows.
fn shadow_layer_cards_rounded(n: u32, shadow: bool) -> Document {
    let mut d = Document::new("gpu shadow rounded");
    let p = d.first_page().unwrap();
    for i in 0..n {
        let (x, y) = (f64::from(i % 6) * 150.0 + 10.0, f64::from(i / 6) * 150.0 + 10.0);
        let id = d.create(p, NodeKind::Rectangle, "Card", x, y, 130.0, 130.0).unwrap();
        let node = d.get_mut(id).unwrap();
        node.corners.radii = [0.0, 34.0, 12.0, 26.0];
        node.fills = vec![Paint::solid(Color::WHITE)];
        if shadow {
            node.effects = vec![Effect::drop_shadow()];
        }
    }
    d
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn shadow_pass_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let mut canvas = CanvasGpu::new(&device, 904, 860).unwrap();
    let mut scene = vello::Scene::new();
    use figcraft_geom::Shape as _;
    let circle = figcraft_geom::kurbo::Circle::new((65.0, 65.0), 60.0).to_path(0.1);
    let white: vello::peniko::Brush = vello::peniko::Brush::Solid(vello::peniko::Color::new([1.0, 1.0, 1.0, 1.0]));
    scene.fill(vello::peniko::Fill::NonZero, vello::kurbo::Affine::IDENTITY, &white, None, &circle);
    let color = Color { a: 0.25, ..Color::BLACK };
    let n = 60;
    {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        let _ = layers.shadow(&scene, 130, 130, 5.5, color, None);
    }
    let t = std::time::Instant::now();
    {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        for _ in 0..n {
            let _ = layers.shadow(&scene, 130, 130, 5.5, color, None);
        }
    }
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    println!("shadow pass, 130 × 130: {:.3} ms a shadow", t.elapsed().as_secs_f64() * 1000.0 / f64::from(n));
    // The same with only the render (no blur, no tint).
    let t = std::time::Instant::now();
    {
        for _ in 0..n {
            let texture = canvas.render_layer(&device, &queue, &scene, 130, 130).unwrap();
            let _ = &texture;
        }
    }
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    println!("  render only: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / f64::from(n));
    // What the pieces cost on their own.
    let make = |w: u32, h: u32| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::canvas::FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    let t = std::time::Instant::now();
    for _ in 0..n {
        let _ = make(130, 130);
    }
    println!("  a texture: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / f64::from(n));
    let t = std::time::Instant::now();
    for _ in 0..n {
        let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        queue.submit([encoder.finish()]);
    }
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    println!("  an empty submit: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / f64::from(n));
    let t = std::time::Instant::now();
    for _ in 0..n {
        let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        queue.submit([encoder.finish()]);
    }
    println!("  an empty submit (unpolled): {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / f64::from(n));
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn shadow_layer_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    for (label, d) in [
        ("30 ellipse shadows", shadow_layer_cards(30, false)),
        ("30 per-corner shadows", shadow_layer_cards_rounded(30, true)),
        ("30 per-corner cards", shadow_layer_cards_rounded(30, false)),
        ("30 inner shadows", shadow_layer_cards(30, true)),
    ] {
        for (w, h) in [(904u32, 860u32), (2400, 1600)] {
            let p = d.first_page().unwrap();
            let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
            let _ = render_view(&d, p, &view, &Options::default());
            let cpu = frame_ms(|| render_view(&d, p, &view, &Options::default()).is_some(), 3);
            let background = scene::background(&d, p);
            let draw = |canvas: &mut CanvasGpu| {
                let built = {
                    let mut layers = crate::canvas::GpuLayers { canvas, device: &device, queue: &queue };
                    scene::build_with(&d, p, &view, &mut layers)
                };
                let ok = built.is_ok_and(|s| canvas.render(&device, &queue, &s, background).is_some());
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                ok
            };
            // The first frame: every shadow is made (and its pipelines compile).
            let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
            let t = std::time::Instant::now();
            assert!(draw(&mut canvas));
            let cold = t.elapsed().as_secs_f64() * 1000.0;
            // Then the frame as the app draws it: a drag redraws the page, the shadows are kept.
            let frame = frame_ms(|| draw(&mut canvas), 5);
            println!("{label}, {w} × {h}: CPU {cpu:.2} ms | GPU first frame {cold:.2} ms, then {frame:.2} ms");
        }
    }
}

/// Text with a styled run (its own fill) and text with a stroke.
fn styled_text_doc() -> Document {
    let mut d = Document::new("gpu styled text");
    let p = d.first_page().unwrap();
    let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::WHITE);
    let mut td = TextData::new("plain and coloured words");
    td.style.font_size = 28.0;
    td.runs = vec![figcraft_doc::StyleRun {
        start: 10,
        end: 18,
        style: figcraft_doc::TextStyle { fills: Some(vec![Paint::solid(hex("E03030"))]), ..td.style.clone() },
    }];
    let run = d.create(p, NodeKind::Text(td), "runs", 24.0, 40.0, 460.0, 80.0).unwrap();
    d.get_mut(run).unwrap().fills = vec![Paint::solid(hex("202020"))];

    let mut stroked = TextData::new("Outlined");
    stroked.style.font_size = 44.0;
    let id = d.create(p, NodeKind::Text(stroked), "stroked", 24.0, 150.0, 400.0, 90.0).unwrap();
    {
        let n = d.get_mut(id).unwrap();
        n.fills = vec![Paint::solid(hex("F0C830"))];
        n.strokes = vec![Paint::solid(hex("2040E0"))];
        n.stroke.weight = 3.0;
        n.stroke.align = StrokeAlign::Outside;
    }
    d
}

#[test]
fn the_gpu_draws_styled_text() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = styled_text_doc();
    let p = d.first_page().unwrap();
    assert_eq!(scene::supported(&d, p), Ok(()), "styled runs and stroked text stay on the GPU");
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = scene::build_with(&d, p, &view, &mut scene::NoLayers).expect("the GPU draws the text");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    let px = |p: &Pixmap, (x, y): (u32, u32)| {
        let i = ((y as usize * p.width() as usize) + x as usize) * 4;
        p.data().get(i..i + 4).map(<[u8]>::to_vec)
    };
    println!(
        "styled text parity: worst {worst} at {at:?} (CPU {:?}, GPU {:?}), mean {mean:.3}, {over} pixels differ by more than 8",
        px(&cpu, at),
        px(&gpu, at)
    );
    assert!(worst <= 160, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 1.0, "the mean difference is {mean:.3}");
}

/// Masks: a layer clipped by an ellipse above it, a vector mask, and a mask inside a group.
fn mask_doc() -> Document {
    let mut d = Document::new("gpu masks");
    let p = d.first_page().unwrap();
    let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::WHITE);
    let frame = d.create(p, NodeKind::Frame, "masked", 20.0, 20.0, 300.0, 200.0).unwrap();
    d.get_mut(frame).unwrap().fills = vec![Paint::solid(hex("FFFFFF"))];
    let card = |d: &mut Document, parent, name: &str, x: f64, y: f64, w: f64, h: f64, color: &str| {
        let id = d.create(parent, NodeKind::Rectangle, name, x, y, w, h).unwrap();
        d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(color).unwrap_or(Color::WHITE))];
        d.get_mut(id).unwrap().corners.radii = [12.0; 4];
        id
    };
    card(&mut d, frame, "under", 0.0, 0.0, 300.0, 200.0, "E8F0FF");
    let mask = d.create(frame, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "mask", 40.0, 20.0, 220.0, 160.0).unwrap();
    d.get_mut(mask).unwrap().fills = vec![Paint::solid(hex("FFFFFF"))];
    d.get_mut(mask).unwrap().is_mask = true;
    card(&mut d, frame, "over", 0.0, 0.0, 300.0, 200.0, "F06030");

    // A vector mask: the outline of the node clips what is above it.
    let group = d.create(p, NodeKind::Group, "vector masked", 360.0, 20.0, 220.0, 160.0).unwrap();
    card(&mut d, group, "vector content", 0.0, 0.0, 220.0, 160.0, "30C080");
    let vector = d.create(group, NodeKind::Rectangle, "vector mask", 20.0, 20.0, 180.0, 120.0).unwrap();
    {
        let n = d.get_mut(vector).unwrap();
        n.fills = vec![Paint::solid(hex("FFFFFF"))];
        n.corners.radii = [30.0; 4];
        n.is_mask = true;
        n.mask_type = figcraft_doc::MaskType::Vector;
    }
    card(&mut d, group, "vector over", 0.0, 0.0, 220.0, 160.0, "2050C0");

    // A mask inside a group, masking a card above it.
    let inner = d.create(p, NodeKind::Group, "nested", 360.0, 220.0, 220.0, 160.0).unwrap();
    card(&mut d, inner, "nested content", 0.0, 0.0, 220.0, 160.0, "FFD040");
    let star = d.create(inner, NodeKind::Rectangle, "nested mask", 20.0, 20.0, 160.0, 120.0).unwrap();
    {
        let n = d.get_mut(star).unwrap();
        n.fills = vec![Paint::solid(hex("FFFFFF"))];
        n.corners.radii = [60.0; 4];
        n.is_mask = true;
    }
    card(&mut d, inner, "nested over", 0.0, 0.0, 220.0, 160.0, "8040E0");

    // A luminance mask: a white-to-black gradient, so the card above it fades out across the run.
    let luma_frame = d.create(p, NodeKind::Frame, "luminance masked", 20.0, 400.0, 300.0, 160.0).unwrap();
    d.get_mut(luma_frame).unwrap().fills = vec![Paint::solid(hex("FFFFFF"))];
    card(&mut d, luma_frame, "luma content", 0.0, 0.0, 300.0, 160.0, "3060E0");
    let luma = d.create(luma_frame, NodeKind::Rectangle, "luma mask", 20.0, 20.0, 260.0, 120.0).unwrap();
    {
        let g = Gradient {
            kind: GradientKind::Linear,
            stops: vec![
                figcraft_color::ColorStop { position: 0.0, color: hex("FFFFFF") },
                figcraft_color::ColorStop { position: 1.0, color: hex("000000") },
            ],
            from: (0.0, 0.5),
            to: (1.0, 0.5),
            width: (0.0, -0.5),
        };
        let n = d.get_mut(luma).unwrap();
        n.fills = vec![Paint::gradient(g)];
        n.is_mask = true;
        n.mask_type = figcraft_doc::MaskType::Luminance;
    }
    card(&mut d, luma_frame, "luma over", 0.0, 0.0, 300.0, 160.0, "F06030");
    d
}

#[test]
fn the_gpu_draws_masks() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = mask_doc();
    let p = d.first_page().unwrap();
    assert_eq!(scene::supported(&d, p), Ok(()), "masks stay on the GPU");
    // A taller canvas than `PARITY`: the mask cases (the luminance one in particular) sit below it.
    let (w, h) = (900u32, 660u32);
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    }
    .expect("the GPU draws the masks");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    let px = |p: &Pixmap, (x, y): (u32, u32)| {
        let i = ((y as usize * p.width() as usize) + x as usize) * 4;
        p.data().get(i..i + 4).map(<[u8]>::to_vec)
    };
    println!(
        "mask parity: worst {worst} at {at:?} (CPU {:?}, GPU {:?}), mean {mean:.3}, {over} pixels differ by more than 8",
        px(&cpu, at),
        px(&gpu, at)
    );
    if std::env::var("FIGCRAFT_REGIONS").is_ok() {
        for (x, y) in [(164u32, 140u32), (30, 30), (170, 120), (460, 100), (470, 300)] {
            println!("  mask {x},{y}: CPU {:?} GPU {:?}", px(&cpu, (x, y)), px(&gpu, (x, y)));
        }
    }
    assert!(worst <= 96, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 0.6, "the mean difference is {mean:.3}");
    // The luminance mask over the run at y 420..540: its white left end shows the run's card, its
    // black right end fades it out to the card below — and both match the CPU.
    for (point, label) in [((60u32, 480u32), "bright"), ((290, 480), "dark")] {
        let (cpu_px, gpu_px) = (px(&cpu, point).unwrap_or_default(), px(&gpu, point).unwrap_or_default());
        assert_eq!(cpu_px.len(), 4, "the {label} end of the luminance mask was drawn");
        for c in 0..3 {
            let d = i16::from(cpu_px[c]) - i16::from(gpu_px[c]);
            assert!(d.abs() <= 16, "the {label} end: channel {c} is {gpu_px:?} on the GPU, {cpu_px:?} on the CPU");
        }
    }
    let dark = px(&gpu, (290, 480)).unwrap_or_default();
    assert!(dark.get(2).is_some_and(|b| *b >= 150), "the mask's black end fades the card out to the one below: {dark:?}");
}

#[test]
fn the_gpu_canvas_reads_back() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = parity_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let scene = scene::build(&d, p, &view).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    assert_eq!(canvas.size(), (w, h));
    assert!(canvas.render(&device, &queue, &scene, scene::background(&d, p)).is_some());
    let pix = canvas.read(&device, &queue).unwrap();
    assert_eq!((pix.width(), pix.height()), (w, h));
    // The page is opaque and its background is drawn: the corner pixel is the page colour.
    assert_eq!(pix.data().get(3), Some(&255), "the read-back is opaque");
    canvas.resize(&device, 320, 200);
    assert_eq!(canvas.size(), (320, 200));
}

/// A progressive blur's levels are blurred at the resolution their own sigma allows: a narrow one
/// keeps the layer's resolution (the ramp's sharp end stays sharp), and a wide one drops until its
/// box passes fit the one-dispatch halo — the scan path's dozen dispatches a level are what made a
/// 600% drag lag.
#[test]
fn wide_blur_levels_are_blurred_smaller_than_narrow_ones() {
    use crate::canvas::level_scale;
    // A 0 → 300 ramp on a 200 × 300 card, its layer clipped to the window: 150 design pixels of
    // sigma, 900 device pixels at 600% — a quarter of the pixels is a 225 box radius, inside the
    // halo. (The same blur in the half-resolution preview is 450 px, so half of *that* layer.)
    let (w, h) = (2281u32, 1625u32);
    assert_eq!(level_scale(900.0, (w, h)), 0.25);
    assert_eq!(level_scale(450.0, (1141, 813)), 0.5);
    // A narrow level keeps the layer at its own resolution: this is the sharp end of the ramp.
    assert_eq!(level_scale(1.0, (w, h)), 1.0);
    assert_eq!(level_scale(200.0, (400, 300)), 1.0);
    // A crop that would be too many pixels is cut by the budget even when the blur is narrow-ish.
    assert_eq!(level_scale(300.0, (w, h)), 0.5);
}

/// An interaction frame (fewer pixels each way) fills the canvas texture's top-left corner at its
/// own resolution: the target keeps the canvas's size — so a preview and the frame after it
/// reallocate nothing — and the read-back is the frame, not the canvas.
#[test]
fn a_smaller_frame_fills_the_canvas_corner() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = parity_doc();
    let p = d.first_page().unwrap();
    let (w, h) = PARITY;
    let small = View { origin: (0.0, 0.0), scale: 0.5, width: w / 2, height: h / 2 };
    let scene = scene::build(&d, p, &small).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    canvas.set_frame(small.width, small.height);
    assert_eq!(canvas.size(), (w, h), "the canvas itself is not reallocated");
    assert!(canvas.render(&device, &queue, &scene, scene::background(&d, p)).is_some());
    let pix = canvas.read(&device, &queue).unwrap();
    assert_eq!((pix.width(), pix.height()), (small.width, small.height), "the read-back is the frame");
    // The corner holds what a canvas of the frame's own size draws for the same view.
    let mut direct = CanvasGpu::new(&device, small.width, small.height).unwrap();
    assert!(direct.render(&device, &queue, &scene, scene::background(&d, p)).is_some());
    let want = direct.read(&device, &queue).unwrap();
    let (mut worst, mut at) = (0u8, (0u32, 0u32));
    for y in 0..small.height {
        for x in 0..small.width {
            let i = ((y * small.width + x) * 4) as usize;
            for c in 0..4 {
                let d = i16::from(pix.data()[i + c]) - i16::from(want.data()[i + c]);
                if d.unsigned_abs() as u8 > worst {
                    worst = d.unsigned_abs() as u8;
                    at = (x, y);
                }
            }
        }
    }
    assert!(worst <= 1, "the frame differs by {worst} at {at:?}");
}

/// A page of `n` rounded, gradient-filled cards at 2400 × 1600: enough geometry to show what the
/// rasterizer costs, without effects.
fn heavy_doc(n: u32) -> Document {
    let mut d = Document::new("gpu frame");
    let p = d.first_page().unwrap();
    let colours = ["E03030", "2040E0", "20C060", "F0C830", "A040E0", "20B0D0"];
    let stops = |a: &str, b: &str| {
        vec![
            figcraft_color::ColorStop { position: 0.0, color: Color::from_hex(a).unwrap_or(Color::WHITE) },
            figcraft_color::ColorStop { position: 1.0, color: Color::from_hex(b).unwrap_or(Color::WHITE) },
        ]
    };
    for i in 0..n {
        let (x, y) = (f64::from(i % 6) * 380.0 + 20.0, f64::from(i / 6) * 150.0 + 20.0);
        let id = d.create(p, NodeKind::Rectangle, "Card", x, y, 340.0, 120.0).unwrap();
        let node = d.get_mut(id).unwrap();
        node.corners.radii = [24.0; 4];
        let g = Gradient {
            kind: GradientKind::Linear,
            stops: stops(colours[i as usize % colours.len()], colours[(i as usize + 3) % colours.len()]),
            from: (0.0, 0.0),
            to: (1.0, 1.0),
            width: (-0.5, 0.5),
        };
        node.fills = vec![Paint::gradient(g)];
    }
    d
}

fn frame_ms(mut f: impl FnMut() -> bool, n: u32) -> f64 {
    let t = std::time::Instant::now();
    for _ in 0..n {
        assert!(f());
    }
    t.elapsed().as_secs_f64() * 1000.0 / f64::from(n)
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn layer_blur_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = blurred_doc();
    let p = d.first_page().unwrap();
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 1200, height: 900 };
    let mut canvas = CanvasGpu::new(&device, view.width, view.height).unwrap();
    let _ = render_view(&d, p, &view, &Options::default());
    let cpu = frame_ms(|| render_view(&d, p, &view, &Options::default()).is_some(), 5);
    // Warm-up: pipelines, atlas.
    for _ in 0..2 {
        let built = {
            let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
            scene::build_with(&d, p, &view, &mut layers)
        }
        .unwrap();
        let _ = canvas.render(&device, &queue, &built, scene::background(&d, p));
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
    let build = frame_ms(
        || {
            let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
            scene::build_with(&d, p, &view, &mut layers).is_ok()
        },
        5,
    );
    let frame = frame_ms(
        || {
            let built = {
                let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
                scene::build_with(&d, p, &view, &mut layers)
            }
            .unwrap();
            let ok = canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            ok
        },
        5,
    );
    println!("layer blur, 1200 × 900: CPU {cpu:.2} ms | GPU build {build:.2} ms, frame {frame:.2} ms");
}

/// `n` shadowed cards (opaque, rounded) at `w` × `h`.
fn shadow_cards(n: u32) -> Document {
    shadow_cards_with(n, 0.0)
}

/// `n` shadowed cards whose shadows spread by `spread` document units.
fn shadow_cards_with(n: u32, spread: f64) -> Document {
    let mut d = Document::new("gpu shadows");
    let p = d.first_page().unwrap();
    let colours = ["FFFFFF", "E03030", "2040E0", "20C060", "F0C830", "A040E0"];
    for i in 0..n {
        let (x, y) = (f64::from(i % 6) * 150.0 + 10.0, f64::from(i / 6) * 150.0 + 10.0);
        let id = d.create(p, NodeKind::Rectangle, "Card", x, y, 130.0, 130.0).unwrap();
        let node = d.get_mut(id).unwrap();
        node.corners.radii = [16.0; 4];
        node.fills = vec![Paint::solid(Color::from_hex(colours[i as usize % colours.len()]).unwrap_or(Color::WHITE))];
        node.effects = vec![Effect::drop_shadow()];
        if let Some(figcraft_doc::EffectKind::DropShadow(s)) = node.effects.first_mut().map(|e| &mut e.kind) {
            s.radius = 12.0;
            s.spread = spread;
        }
    }
    d
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn shadow_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    for (spread, label) in [(0.0, "30 shadowed cards"), (6.0, "30 spread shadows")] {
        for (w, h) in [(904u32, 860u32), (2320, 1440)] {
            let d = shadow_cards_with(30, spread);
            let p = d.first_page().unwrap();
            let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
            let _ = render_view(&d, p, &view, &Options::default());
            let cpu = frame_ms(|| render_view(&d, p, &view, &Options::default()).is_some(), 5);
            let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
            let _ = canvas.render(&device, &queue, &scene::build(&d, p, &view).unwrap(), scene::background(&d, p));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let frame = frame_ms(
                || {
                    let s = scene::build(&d, p, &view).unwrap();
                    let ok = canvas.render(&device, &queue, &s, scene::background(&d, p)).is_some();
                    let _ = device.poll(wgpu::PollType::wait_indefinitely());
                    ok
                },
                8,
            );
            println!("{label}, {w} × {h}: CPU {cpu:.2} ms | GPU frame {frame:.2} ms");
        }
    }
}

/// The page the owner drags in: thirty shadowed cards, three glass panes, a background-blurred
/// card and a layer-blurred one.
fn heavy_effects_doc() -> Document {
    let mut d = shadow_cards(30);
    let p = d.first_page().unwrap();
    for (x, y, w, h) in [(80.0, 80.0, 300.0, 220.0), (360.0, 260.0, 300.0, 220.0), (600.0, 480.0, 280.0, 200.0)] {
        let id = d.create(p, NodeKind::Rectangle, "Glass Pane", x, y, w, h).unwrap();
        let n = d.get_mut(id).unwrap();
        n.corners.radii = [36.0; 4];
        n.fills = vec![Paint::solid(Color { a: 0.08, ..Color::WHITE })];
        n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
    }
    let card = d.create(p, NodeKind::Rectangle, "Blurred card", 200.0, 560.0, 260.0, 160.0).unwrap();
    d.get_mut(card).unwrap().effects = vec![Effect::background_blur()];
    let card2 = d.create(p, NodeKind::Rectangle, "Layer blur", 520.0, 640.0, 220.0, 140.0).unwrap();
    d.get_mut(card2).unwrap().effects = vec![Effect::layer_blur()];
    d
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn heavy_scene_gpu_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    for (name, d) in [("one glass pane", glass_doc()), ("one background blur", backdrop_doc())] {
        let p = d.first_page().unwrap();
        let view = View { origin: (0.0, 0.0), scale: 1.0, width: 904, height: 860 };
        let _ = render_view(&d, p, &view, &Options::default());
        let cpu = frame_ms(|| render_view(&d, p, &view, &Options::default()).is_some(), 5);
        let mut canvas = CanvasGpu::new(&device, view.width, view.height).unwrap();
        let frame = frame_ms(
            || {
                let built = {
                    let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
                    scene::build_with(&d, p, &view, &mut layers)
                };
                let ok = built.is_ok_and(|s| canvas.render(&device, &queue, &s, scene::background(&d, p)).is_some());
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                ok
            },
            5,
        );
        println!("{name}, 904 × 860: CPU {cpu:.2} ms | GPU frame {frame:.2} ms");
        // Pipelined, as the app draws: submit frame after frame, wait only for the last.
        let t = std::time::Instant::now();
        for _ in 0..8 {
            let built = {
                let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
                scene::build_with(&d, p, &view, &mut layers)
            };
            if let Ok(s) = built {
                let _ = canvas.render(&device, &queue, &s, scene::background(&d, p));
            }
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        println!("{name}: pipelined {:.2} ms a frame", t.elapsed().as_secs_f64() * 1000.0 / 8.0);
    }
    for (w, h) in [(904u32, 860u32), (2400, 1600)] {
        let d = heavy_effects_doc();
        let p = d.first_page().unwrap();
        let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
        let _ = render_view(&d, p, &view, &Options::default());
        let cpu = frame_ms(|| render_view(&d, p, &view, &Options::default()).is_some(), 5);
        let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
        let build = frame_ms(
            || {
                let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
                scene::build_with(&d, p, &view, &mut layers).is_ok()
            },
            5,
        );
        let frame = frame_ms(
            || {
                let built = {
                    let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
                    scene::build_with(&d, p, &view, &mut layers)
                };
                let ok = built.is_ok_and(|s| canvas.render(&device, &queue, &s, scene::background(&d, p)).is_some());
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                ok
            },
            5,
        );
        println!("heavy scene {w} × {h}: CPU {cpu:.2} ms | GPU build {build:.2} ms, frame {frame:.2} ms");
    }
}

/// The segmented renderer (pages with backdrop effects) against the CPU, pipelined as the app
/// draws: frames submitted back to back, waiting only for the last.
#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn segmented_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let docs = [("one glass pane", glass_doc()), ("one background blur", backdrop_doc()), ("heavy", heavy_effects_doc())];
    for (name, d) in docs {
        // The second view is the app's canvas after View ▸ Zoom to fit on the heavy page: a
        // 904 × 860 canvas at 34.6%, the whole page in view.
        for (w, h, scale, origin) in
            [(904u32, 860u32, 1.0, (0.0, 0.0)), (2400, 1600, 1.0, (0.0, 0.0)), (904, 860, 0.3464285714285714, (-164.6, -841.2))]
        {
            let p = d.first_page().unwrap();
            let view = View { origin, scale, width: w, height: h };
            let Ok(plan) = scene::plan(&d, p) else { continue };
            let _ = render_view(&d, p, &view, &Options::default());
            let cpu = frame_ms(|| render_view(&d, p, &view, &Options::default()).is_some(), 3);
            let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
            let background = scene::background(&d, p);
            // Warm-up: the pipelines, the glass geometry.
            for _ in 0..2 {
                let _ = crate::canvas::segmented_frame(&device, &queue, &mut canvas, &d, p, &view, &plan, background);
            }
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let t = std::time::Instant::now();
            for _ in 0..6 {
                assert!(crate::canvas::segmented_frame(&device, &queue, &mut canvas, &d, p, &view, &plan, background).is_some());
            }
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let frame = t.elapsed().as_secs_f64() * 1000.0 / 6.0;
            println!("segmented {name} {w} × {h} at {:.0}%: CPU {cpu:.2} ms | GPU {frame:.2} ms a frame (pipelined)", scale * 100.0);
        }
    }
}

/// A frame as the app draws it: the segmented path when the page has a backdrop node, the single
/// scene otherwise (what `canvas::present` chooses).
fn gpu_frame(canvas: &mut CanvasGpu, device: &Device, queue: &Queue, d: &Document, view: &View, background: figcraft_color::Color) -> bool {
    let Some(p) = d.first_page() else { return false };
    let Ok(plan) = scene::plan(d, p) else { return false };
    if plan.iter().any(|s| matches!(s, scene::Step::Backdrop(_))) {
        return crate::canvas::segmented_frame(device, queue, canvas, d, p, view, &plan, background).is_some();
    }
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas, device, queue };
        scene::build_with(d, p, view, &mut layers)
    };
    built.is_ok_and(|s| canvas.render(device, queue, &s, background).is_some())
}

/// The middle of a set of frame times (the machine's GPU throughput moves with what else runs).
fn median(times: &mut [f64]) -> f64 {
    if times.is_empty() {
        return 0.0;
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    times[times.len() / 2]
}

/// A single-layer drag as the app draws it: one card moves a pixel a frame (a new document
/// revision each frame) on the corpus pages. The CPU's own canvas path (`Retained`: damaged
/// pixels only, effect layers kept) is the baseline the GPU has to meet.
#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn segmented_drag_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let pages = [
        ("plain", shadow_cards(30)),
        ("layer blur", blurred_doc()),
        ("one glass pane", glass_doc()),
        ("heavy", heavy_effects_doc()),
        ("nested backdrops", nested_backdrop_doc()),
    ];
    let mut doc_id = 1u64;
    for (name, d) in pages {
        let p = d.first_page().unwrap();
        for (w, h) in [(904u32, 860u32), (2400, 1600)] {
            doc_id += 1;
            let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
            let background = scene::background(&d, p);
            let card = d.node(p).and_then(|p| p.children.first().copied());
            let moved = |i: u32| {
                let mut e = d.clone();
                if let Some(id) = card {
                    let by = figcraft_geom::Affine::translate((f64::from(i), 0.0));
                    let was = e.get(id).map(|n| n.transform).unwrap_or(figcraft_geom::Affine::IDENTITY);
                    if let Ok(n) = e.get_mut(id) {
                        n.transform = by * was;
                    }
                }
                e
            };
            // The GPU: warm up (pipelines, glass geometry), then ten frames of the drag, each
            // waited for (what the window shows is the finished frame).
            let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
            for _ in 0..2 {
                assert!(gpu_frame(&mut canvas, &device, &queue, &d, &view, background));
            }
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let mut drag = Vec::new();
            for i in 0..10 {
                let t = std::time::Instant::now();
                assert!(gpu_frame(&mut canvas, &device, &queue, &moved(i), &view, background));
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                drag.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            // A frame with nothing changed: every step kept.
            let mut still = Vec::new();
            for _ in 0..10 {
                let t = std::time::Instant::now();
                assert!(gpu_frame(&mut canvas, &device, &queue, &d, &view, background));
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                still.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            // The CPU's canvas: its retained layers and damaged pixels on the same frames (the
            // options `render_worker` gives it).
            let mut retained = figcraft_render::Retained::default();
            let opts = Options { lod: true, doc_id, ..Options::default() };
            let run = |retained: &mut figcraft_render::Retained, i: u32| {
                let doc = Arc::new(moved(i));
                retained.render(&doc, p, &view, &opts).is_some()
            };
            assert!(run(&mut retained, 0));
            assert!(run(&mut retained, 1));
            let mut cpu_frames = Vec::new();
            for i in 0..10 {
                let t = std::time::Instant::now();
                assert!(run(&mut retained, i));
                cpu_frames.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            println!(
                "{name:16} {w} × {h}: CPU retained {:5.2} ms | GPU drag {:5.2} ms, still {:5.2} ms (medians)",
                median(&mut cpu_frames),
                median(&mut drag),
                median(&mut still)
            );
        }
    }
}

/// A progressive blur whose layer is past the levels' budget (2 M pixels) blurs its levels at half
/// resolution and reads them upscaled: the same picture as the CPU's, since a level is a blur.
#[test]
fn the_gpu_draws_a_big_progressive_blur_at_half_resolution() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    // A 0 → 420 ramp: the card's layer is 1820 × 2110 (3.8 M pixels, past the 2 M budget), and
    // the view is panned so the view does not clip it.
    let d = progressive_card_doc(420.0);
    let p = d.first_page().unwrap();
    let card = d.node(p).and_then(|p| p.children.get(1).copied()).expect("the card");
    let (w, h) = (2000u32, 2200u32);
    let view = View { origin: (-700.0, -800.0), scale: 1.0, width: w, height: h };
    let (_, _, lw, lh) = scene::layer_rect_for(&d, card, &view).expect("a layer");
    println!("the layer is {lw} × {lh} ({} pixels)", u64::from(lw) * u64::from(lh));
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    }
    .expect("the GPU draws the big progressive blur");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("big progressive blur parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 96, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 1.5, "the mean difference is {mean:.3}");
}

/// The 0 → 300 progressive blur against the CPU: its layer is 1100 × 1200, and each level is
/// blurred in its own band, so the crops and their blend are checked end to end.
#[test]
fn the_gpu_draws_a_progressive_blur() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = progressive_card_doc(300.0);
    let p = d.first_page().unwrap();
    let (w, h) = (904u32, 860u32);
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    }
    .expect("the GPU draws a progressive blur");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("progressive blur parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 96, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 1.5, "the mean difference is {mean:.3}");
}

/// The same ramp across the card instead of down it: the levels' bands are then columns, and the
/// blend reads crops that start a different way across.
#[test]
fn the_gpu_draws_a_sideways_progressive_blur() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let mut d = progressive_card_doc(300.0);
    let p = d.first_page().unwrap();
    let card = d.node(p).and_then(|p| p.children.get(1).copied());
    if let Some(id) = card
        && let Ok(n) = d.get_mut(id)
        && let Some(figcraft_doc::EffectKind::LayerBlur(b)) = n.effects.first_mut().map(|e| &mut e.kind)
    {
        (b.start, b.end) = ((0.0, 0.5), (1.0, 0.5));
    }
    let (w, h) = (904u32, 860u32);
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
    let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
    let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
    let built = {
        let mut layers = crate::canvas::GpuLayers { canvas: &mut canvas, device: &device, queue: &queue };
        scene::build_with(&d, p, &view, &mut layers)
    }
    .expect("the GPU draws a sideways progressive blur");
    assert!(canvas.render(&device, &queue, &built, scene::background(&d, p)).is_some());
    let gpu = canvas.read(&device, &queue).unwrap();
    let (worst, mean, at, over) = diff(&cpu, &gpu);
    println!("sideways progressive blur parity: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
    assert!(worst <= 96, "a channel differs by {worst} (mean {mean:.3})");
    assert!(mean <= 1.5, "the mean difference is {mean:.3}");
}

/// The progressive blur's levels are blurred only where their band is (plus their reach): a
/// 0 → 300 ramp down a 200 × 300 card keeps most of its levels a fraction of the layer's height.
#[test]
fn a_progressive_level_blurs_only_its_band() {
    let levels = figcraft_render::blur::levels(0.0, 150.0);
    let (w, h) = (1100u32, 1200u32);
    // The card's ramp in layer pixels: its top edge's middle to its bottom edge's.
    let (a, b) = ((100.0, 0.0), (100.0, 300.0));
    let mut total = 0u64;
    for (k, d) in levels.iter().enumerate() {
        let reach = (d * 3.0).ceil().clamp(0.0, 8192.0) as u32 + 3;
        let (x, y, cw, ch) = crate::canvas::level_crop(&levels, k, 0.0, 150.0, a, b, (w, h), reach);
        assert!(x + cw <= w && y + ch <= h, "level {k}'s crop {x},{y} {cw}×{ch} is inside the layer");
        assert!(cw > 0 && ch > 0, "level {k} has an empty crop");
        total += u64::from(cw) * u64::from(ch);
    }
    let whole = u64::from(w) * u64::from(h) * levels.len() as u64;
    println!("progressive crops: {total} of {whole} pixels ({:.0}%)", 100.0 * total as f64 / whole as f64);
    assert!(total * 3 < whole, "the bands cover {total} of {whole} pixels");
    // A ramp with no direction keeps the whole layer (there is no band to crop).
    let (x, y, cw, ch) = crate::canvas::level_crop(&levels, 3, 0.0, 150.0, (0.0, 0.0), (0.0, 0.0), (w, h), 10);
    assert_eq!((x, y, cw, ch), (0, 0, w, h));
}

/// A layer's key leaves out where the node sits: moving it — by a whole device pixel or by part
/// of one, at any zoom — gives the same key (so its blurred layer is kept), while anything that
/// changes its pixels does not.
#[test]
fn a_whole_pixel_move_keeps_the_layer_key() {
    let d = progressive_card_doc(300.0);
    let p = d.first_page().unwrap();
    let card = d.node(p).and_then(|p| p.children.get(1).copied()).expect("the card");
    let key_of = |doc: &Document, zoom: f64| {
        // A panned view, big enough that it does not clip the card's layer (see
        // `a_layer_bigger_than_the_view_is_cropped_to_it`): the origin is a page coordinate no one
        // picked to be whole, which is where composing the placement with the view rounds.
        let view = View { origin: (-321.37, -371.11), scale: zoom, width: (1143.0 * zoom).ceil() as u32, height: (1243.0 * zoom).ceil() as u32 };
        let n = doc.node(card).expect("the card");
        let (x0, y0, ..) = scene::layer_rect_for(doc, card, &view).expect("a layer");
        let xf = view.transform() * n.transform;
        let layer_xf = figcraft_geom::Affine::translate((-x0, -y0)) * xf;
        scene::layer_key_for(doc, n, u64::MAX, layer_xf)
    };
    let moved = |dx: f64, dy: f64| {
        let mut e = d.clone();
        if let Ok(n) = e.get_mut(card) {
            n.transform = figcraft_geom::Affine::translate((dx, dy)) * n.transform;
        }
        e
    };
    // Any zoom, any drag: 100% moves the node a whole device pixel, 86% and 244% move it part of
    // one, and the layer's content does not depend on either.
    for zoom in [1.0, 0.86, 2.44] {
        let base = key_of(&d, zoom);
        for step in [0.5, 1.0, 2.0, 3.7] {
            assert_eq!(key_of(&moved(step, step * 0.5), zoom), base, "a {step} px move at {zoom:.2}× changed the layer key");
        }
    }
    // A changed fill is a changed layer.
    let mut repainted = d.clone();
    if let Ok(n) = repainted.get_mut(card) {
        n.fills = vec![Paint::solid(Color::from_hex("E02040").unwrap_or(Color::BLACK))];
    }
    assert_ne!(key_of(&repainted, 1.0), key_of(&d, 1.0), "a repainted layer kept its key");
    // So is a resize (its pixels are laid out again).
    let mut resized = d.clone();
    if let Ok(n) = resized.get_mut(card) {
        n.height = 320.0;
    }
    assert_ne!(key_of(&resized, 1.0), key_of(&d, 1.0), "a resized layer kept its key");
}

/// A drag keeps the blurred layer at every zoom — not just at 100%, where a drag is a whole
/// number of device pixels — and the moved frame matches the CPU's render of the moved card.
#[test]
fn a_dragged_card_keeps_its_blur() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let d = progressive_card_doc(300.0);
    let p = d.first_page().unwrap();
    let card = d.node(p).and_then(|p| p.children.get(1).copied());
    let moved = |dx: f64| {
        let mut e = d.clone();
        if let Some(id) = card
            && let Ok(n) = e.get_mut(id)
        {
            n.transform = figcraft_geom::Affine::translate((dx, 0.0)) * n.transform;
        }
        e
    };
    // 100% and 86%: at the second a drag of a page pixel moves the card about 0.86 device pixels,
    // which is where the layer used to be re-blurred every frame. The view holds the card's layer
    // (its bounds grown by the blur's margin) at each zoom, as a real one does while the card is
    // on screen: a layer the view crops is drawn again as the node moves.
    for (zoom, step) in [(1.0f64, 1.0f64), (0.86, 1.0), (2.0, 0.5)] {
        let (w, h) = ((1143.0 * zoom).ceil() as u32, (1243.0 * zoom).ceil() as u32);
        // A panned view, as in the app: see `a_whole_pixel_move_keeps_the_layer_key`.
        let view = View { origin: (-321.37, -371.11), scale: zoom, width: w, height: h };
        let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
        assert!(gpu_frame(&mut canvas, &device, &queue, &d, &view, scene::background(&d, p)));
        let first = canvas.kept_shadow_hits();
        for i in 1..8u32 {
            let was = canvas.kept_shadow_hits();
            assert!(gpu_frame(&mut canvas, &device, &queue, &moved(f64::from(i) * step), &view, scene::background(&d, p)));
            println!("  {zoom:.2}× move {i}: kept {}", canvas.kept_shadow_hits() - was);
        }
        let hits = canvas.kept_shadow_hits() - first;
        println!("{zoom:.2}×: seven moves found the kept layer {hits} times");
        assert!(hits >= 7, "a drag at {zoom:.2}× re-blurred its layer ({hits} of 7 kept)");
        // And the moved frame is the CPU's: the kept layer is drawn where it belongs (its place
        // snapped to a whole device pixel, so a fractional drag is off by less than one).
        let d7 = moved(7.0 * step);
        let cpu = render_view(&d7, p, &view, &Options::default()).unwrap();
        let gpu = canvas.read(&device, &queue).unwrap();
        let (worst, mean, at, over) = diff(&cpu, &gpu);
        println!("  dragged card parity at {zoom:.2}×: worst {worst} at {at:?}, mean {mean:.3}, {over} pixels differ by more than 8");
        assert!(worst <= 160, "a channel differs by {worst} (mean {mean:.3})");
        assert!(mean <= 3.0, "the mean difference is {mean:.3}");
    }
}

/// The downscale a large layer's blur levels are made with: a texel is the average of the four it
/// covers, the image keeps its orientation, and premultiplied pixels stay premultiplied.
#[test]
fn a_downscale_halves_the_layer_the_right_way_up() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let Some(compose) = crate::compose::Compose::new(&device) else {
        eprintln!("no compose pipelines");
        return;
    };
    let (w, h) = (64u32, 48u32);
    // Every 2 × 2 block one colour — (x / 2, y / 2) — so a half-size copy has to read back as
    // ((x, y, 0)), and any flip, offset or extra premultiply shows.
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y as usize * w as usize) + x as usize) * 4;
            let (r, g) = ((x / 2) as u8, (y / 2) as u8);
            // A translucent block stays as it is: the source is already premultiplied.
            let (r, g) = if x < 2 && y < 2 { (64, 32) } else { (r, g) };
            let a = if x < 2 && y < 2 { 128 } else { 255 };
            pixels[i..i + 4].copy_from_slice(&[r, g, 0, a]);
        }
    }
    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("downscale test source"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &source, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &pixels,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let (hw, hh) = (w / 2, h / 2);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("downscale test target"),
        size: wgpu::Extent3d { width: hw, height: hh, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    compose.downscale(&device, &queue, &target_view, &source_view);
    let out = crate::canvas::read(&device, &queue, &target, (hw, hh)).expect("the downscaled pixels");
    for y in [0u32, 1, 6, 12, 18, 23] {
        let i = ((y as usize * hw as usize) + 1) * 4;
        println!("  row {y:2} at x=1: {:?} (want g {y})", out.data().get(i..i + 4));
    }
    let mut worst = 0u8;
    for y in 0..hh {
        for x in 0..hw {
            let i = ((y as usize * hw as usize) + x as usize) * 4;
            let got = out.data().get(i..i + 4).unwrap_or(&[0; 4]).to_vec();
            let want: [u8; 4] = if x < 1 && y < 1 { [64, 32, 0, 128] } else { [x as u8, y as u8, 0, 255] };
            for c in 0..4 {
                let d = got[c].abs_diff(want[c]);
                if d > worst {
                    worst = d;
                    if d > 2 {
                        println!("  at ({x}, {y}) channel {c}: {:?}, want {want:?}", got);
                    }
                }
            }
        }
    }
    println!("downscale: worst channel difference {worst}");
    assert!(worst <= 2, "a channel differs by {worst}");
}

/// A layer is clipped to the view, as the renderer clips its own: at 600% the card's layer would be
/// 6600 × 7200 (47 M pixels, past any budget and bigger than the window), and only what the window
/// shows is ever drawn.
#[test]
fn a_layer_bigger_than_the_view_is_cropped_to_it() {
    let d = progressive_card_doc(300.0);
    let p = d.first_page().unwrap();
    let card = d.node(p).and_then(|p| p.children.get(1).copied()).expect("the card");
    let view = View { origin: (10.0, -20.0), scale: 6.0, width: 2280, height: 1624 };
    let (x0, y0, w, h) = scene::layer_rect_for(&d, card, &view).expect("a layer");
    println!("600%: the layer is {w} × {h} at ({x0}, {y0})");
    assert_eq!((x0, y0), (0.0, 0.0), "the near edges are the view's");
    assert!(u64::from(w) * u64::from(h) <= 2281 * 1625, "the layer is {w} × {h}, past the view");
    assert!(w as f64 >= 2280.0 && h as f64 >= 1624.0, "the layer misses part of the view: {w} × {h}");
}

/// A 200 × 300 card with a progressive blur from 0 to `radius`, over a plain ground.
fn progressive_card_doc(radius: f64) -> Document {
    let mut d = Document::new("gpu progressive card");
    let p = d.first_page().unwrap();
    let ground = d.create(p, NodeKind::Rectangle, "ground", 40.0, 30.0, 520.0, 640.0).unwrap();
    d.get_mut(ground).unwrap().fills = vec![Paint::solid(Color::from_hex("E8E8E8").unwrap_or(Color::WHITE))];
    let card = d.create(p, NodeKind::Rectangle, "ramp", 150.0, 100.0, 200.0, 300.0).unwrap();
    {
        let n = d.get_mut(card).unwrap();
        n.corners.radii = [12.0; 4];
        n.fills = vec![Paint::solid(Color::from_hex("2040E0").unwrap_or(Color::BLACK))];
        n.effects = vec![Effect {
            kind: EffectKind::LayerBlur(figcraft_doc::Blur {
                radius,
                blur_type: figcraft_doc::BlurType::Progressive,
                start_radius: 0.0,
                start: (0.5, 0.0),
                end: (0.5, 1.0),
            }),
            visible: true,
        }];
    }
    d
}

/// The card with the 0 → `radius` progressive blur dragged, for a radius the box passes cost the
/// same at as any other.
#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn progressive_drag_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    for radius in [300.0, 600.0] {
        let d = progressive_card_doc(radius);
        let p = d.first_page().unwrap();
        let card = d.node(p).and_then(|p| p.children.first().copied());
        for (w, h) in [(904u32, 860u32), (2400, 1600)] {
            let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
            let background = scene::background(&d, p);
            let moved = |i: u32| {
                let mut e = d.clone();
                if let Some(id) = card {
                    let by = figcraft_geom::Affine::translate((f64::from(i), 0.0));
                    let was = e.get(id).map(|n| n.transform).unwrap_or(figcraft_geom::Affine::IDENTITY);
                    if let Ok(n) = e.get_mut(id) {
                        n.transform = by * was;
                    }
                }
                e
            };
            let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
            for _ in 0..2 {
                assert!(gpu_frame(&mut canvas, &device, &queue, &d, &view, background));
            }
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let mut drag = Vec::new();
            for i in 0..10 {
                let t = std::time::Instant::now();
                assert!(gpu_frame(&mut canvas, &device, &queue, &moved(i), &view, background));
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                drag.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            let cpu = frame_ms(|| render_view(&moved(1), p, &view, &Options::default()).is_some(), 3);
            println!(
                "progressive card 0 → {radius:.0} {w} × {h}: CPU full {cpu:5.2} ms | GPU drag {:5.2} ms (median), {:5.2} ms worst",
                median(&mut drag),
                drag.iter().copied().fold(0.0, f64::max)
            );
        }
    }
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn canvas_frame_timings() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    for (w, h) in [(2320u32, 1440u32), (1160, 720)] {
        for (name, cards) in [("one card", 1u32), ("360 cards", 360)] {
            let d = heavy_doc(cards);
            let p = d.first_page().unwrap();
            let view = View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h };
            let bg = scene::background(&d, p);
            let _ = render_view(&d, p, &view, &Options::default());
            let cpu = frame_ms(|| render_view(&d, p, &view, &Options::default()).is_some(), 5);
            let mut canvas = CanvasGpu::new(&device, w, h).unwrap();
            let built = scene::build(&d, p, &view).unwrap();
            // Warm-up: vello compiles its pipelines on the first render.
            let _ = canvas.render(&device, &queue, &built, bg);
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let build = frame_ms(
                || {
                    let _ = scene::build(&d, p, &view);
                    true
                },
                8,
            );
            // A frame as the canvas draws it: build, draw, and wait for the result (what the
            // next frame's overlay and any screenshot see).
            let frame = frame_ms(
                || {
                    let s = scene::build(&d, p, &view).unwrap();
                    let ok = canvas.render(&device, &queue, &s, bg).is_some();
                    let _ = device.poll(wgpu::PollType::wait_indefinitely());
                    ok
                },
                8,
            );
            // Pipelined, as the app draws: submit frame after frame, wait only for the last.
            let t = std::time::Instant::now();
            for _ in 0..8 {
                let s = scene::build(&d, p, &view).unwrap();
                let _ = canvas.render(&device, &queue, &s, bg);
            }
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let pipelined = t.elapsed().as_secs_f64() * 1000.0 / 8.0;
            let read = frame_ms(|| canvas.read(&device, &queue).is_some(), 3);
            println!(
                "{w} × {h}, {name:>9}: CPU {cpu:5.2} ms | GPU frame {frame:5.2} ms (pipelined {pipelined:5.2}, \
                 scene {build:5.2}), read-back {read:5.2}"
            );
        }
    }
}
