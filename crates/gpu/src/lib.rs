//! GPU work for the canvas: the raster effects that dominate a render (blurs) run on the GPU where
//! there is one, and a GPU rasterizer draws a page's shapes into a texture egui can sample
//! ([`canvas`], [`scene`]; plan and numbers in `docs/gpu-rasterizer.md`). The renderer keeps its
//! CPU paths for exports, tests, the browser and machines without an adapter — the canvas installs
//! this only when its window has a wgpu device.
//!
//! The blur is the CPU blur's: three box passes each way, with the same radii (see
//! [`figcraft_render::fx::box_radii`]), so a canvas drawn with the GPU and an export drawn without
//! it look the same.

mod blur;
pub mod canvas;
mod compose;
mod glass;
mod mix;
mod plus;
pub mod scene;
mod shadow;

use std::sync::Arc;

use figcraft_render::{Accelerator, GlassPaint, Pixmap};
use wgpu::{Device, Queue};

/// The GPU effects a window can use, built from its own device.
pub struct Gpu {
    device: Arc<Device>,
    queue: Arc<Queue>,
    blur: blur::Blur,
    glass: glass::Glass,
}

impl Gpu {
    /// The effects for `device`/`queue`, or `None` when the pipelines can't be built there.
    pub fn new(device: Arc<Device>, queue: Arc<Queue>) -> Option<Arc<Self>> {
        let blur = blur::Blur::new(&device)?;
        let glass = glass::Glass::new(&device)?;
        Some(Arc::new(Self { device, queue, blur, glass }))
    }
}

/// Give the renderer and the canvas the window's GPU (once; later calls do nothing). Returns
/// whether the effects run on the GPU afterwards.
pub fn install(device: Arc<Device>, queue: Arc<Queue>) -> bool {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| {
        // The canvas and the effects are set up apart: either can run without the other.
        canvas::install(device.clone(), queue.clone());
        let Some(gpu) = Gpu::new(device, queue) else {
            log::info!("effects stay on the CPU: the window's GPU can't run them");
            return false;
        };
        figcraft_render::set_accelerator(Some(gpu));
        log::info!("effects run on the GPU");
        true
    })
}

impl Accelerator for Gpu {
    fn blur(&self, p: &mut Pixmap, sigma: f64) -> bool {
        // A read-back costs about as much as a small blur on the CPU: leave those to it.
        if p.data().len() < blur::GPU_MIN_BYTES {
            return false;
        }
        blur::blur(&self.device, &self.queue, &self.blur, p, sigma)
    }

    fn glass(&self, paint: &GlassPaint<'_>) -> Option<Pixmap> {
        self.glass.paint(&self.device, &self.queue, paint)
    }
}

#[cfg(test)]
mod canvas_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine's adapter (none in a headless CI container: the tests then skip).
    fn gpu() -> Option<Arc<Gpu>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Gpu::new(Arc::new(device), Arc::new(queue))
    }

    /// A pixmap with a bright square on a dark ground (enough detail for a blur to show).
    fn scene(w: u32, h: u32) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let inside = (w / 4..w / 2).contains(&x) && (h / 4..h / 2).contains(&y);
                let v = if inside { 255 } else { 40 };
                if let Some(px) = p.data_mut().get_mut(i..i + 4) {
                    px.copy_from_slice(&[v, v / 2, v / 3, 255]);
                }
            }
        }
        p
    }

    #[test]
    fn the_gpu_blur_matches_the_cpu_one() {
        let Some(gpu) = gpu() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        for (w, h, sigma) in [(320u32, 240u32, 3.0), (640, 480, 12.0)] {
            let mut cpu = scene(w, h);
            let mut on_gpu = cpu.clone();
            figcraft_render::fx::blur(&mut cpu, sigma, false);
            assert!(gpu.blur(&mut on_gpu, sigma), "{w}×{h}: the GPU drew it");
            let worst = cpu.data().iter().zip(on_gpu.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(worst <= 1, "{w}×{h} sigma {sigma}: a channel differs by {worst}");
        }
    }

    #[test]
    fn the_gpu_glass_matches_the_cpu_one() {
        use figcraft_render::{Options, View, render_view};

        let Some(gpu) = gpu() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let (d, p, _) = canvas_scene();
        let view = View { origin: (200.0, 120.0), scale: 1.0, width: 800, height: 600 };
        figcraft_render::set_accelerator(None);
        let cpu = render_view(&d, p, &view, &Options::default()).unwrap();
        figcraft_render::set_accelerator(Some(gpu));
        let on_gpu = render_view(&d, p, &view, &Options::default()).unwrap();
        figcraft_render::set_accelerator(None);
        assert_eq!((cpu.width(), cpu.height()), (on_gpu.width(), on_gpu.height()));
        let worst = cpu.data().iter().zip(on_gpu.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        let differing = cpu.data().iter().zip(on_gpu.data()).filter(|(a, b)| a != b).count();
        assert!(worst <= 4, "a channel differs by {worst} ({differing} bytes differ)");
    }

    #[test]
    #[ignore = "timing: run in release with --ignored --nocapture"]
    fn blur_timings() {
        let Some(gpu) = gpu() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        // A band of the canvas benchmark's glass pane: the blur it runs every frame.
        let (w, h) = (1380u32, 458u32);
        for sigma in [4.0, 12.0] {
            let mut p = scene(w, h);
            let t = std::time::Instant::now();
            for _ in 0..8 {
                figcraft_render::fx::blur(&mut p, sigma, false);
            }
            let cpu = t.elapsed().as_secs_f64() * 1000.0 / 8.0;
            let t = std::time::Instant::now();
            for _ in 0..8 {
                assert!(gpu.blur(&mut p, sigma));
            }
            let on_gpu = t.elapsed().as_secs_f64() * 1000.0 / 8.0;
            println!("{w}×{h} sigma {sigma}: CPU {cpu:.1} ms, GPU {on_gpu:.1} ms (upload and read-back included)");
        }
    }

    /// A glass pane over stripes, card and all, at 2400 × 1600 device pixels (the canvas
    /// benchmark's scene): (document, page, pane).
    fn canvas_scene() -> (figcraft_doc::Document, figcraft_doc::NodeId, figcraft_doc::NodeId) {
        use figcraft_color::{Color, Paint};
        use figcraft_doc::{Document, Effect, EffectKind, Glass, NodeKind};
        use figcraft_geom::Affine;

        let mut d = Document::new("gpu bench");
        let p = d.first_page().unwrap();
        let colours = ["E03030", "2040E0", "20C060", "F0C830"].map(|h| Color::from_hex(h).unwrap());
        for i in 0..24u32 {
            let s = d.create(p, NodeKind::Rectangle, "Stripe", f64::from(i) * 50.0, 0.0, 50.0, 800.0).unwrap();
            d.get_mut(s).unwrap().fills = vec![Paint::solid(colours[i as usize % 4])];
        }
        let card = d.create(p, NodeKind::Rectangle, "Card", 420.0, 300.0, 160.0, 120.0).unwrap();
        d.get_mut(card).unwrap().fills = vec![Paint::solid(Color::WHITE)];
        let pane = d.create(p, NodeKind::Rectangle, "Glass", 0.0, 0.0, 600.0, 360.0).unwrap();
        {
            let n = d.get_mut(pane).unwrap();
            n.transform = Affine::translate((300.0, 220.0));
            n.corners.radii = [40.0; 4];
            n.fills = vec![Paint::solid(Color { a: 0.1, ..Color::WHITE })];
            n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
        }
        (d, p, pane)
    }

    /// Mean milliseconds of eight canvas frames dragging the glass pane, with the accelerator as
    /// it is now.
    fn drag_frames(frost: f64) -> f64 {
        use figcraft_doc::{Effect, EffectKind, Glass};
        use figcraft_geom::Affine;
        use figcraft_render::{Options, Retained, View};

        let (d, p, pane) = canvas_scene();
        let mut d = d;
        if frost < 0.0 {
            // No glass effect at all: what the shapes and the canvas cost on their own.
            if let Ok(n) = d.get_mut(pane) {
                n.effects.clear();
            }
        } else if frost != Glass::default().frost
            && let Ok(n) = d.get_mut(pane)
        {
            n.effects = vec![Effect { kind: EffectKind::Glass(Glass { frost, ..Glass::default() }), visible: true }];
        }
        let view = View { origin: (0.0, 0.0), scale: 2.0, width: 2400, height: 1600 };
        let mut retained = Retained::default();
        let mut doc = Arc::new(d);
        let _ = retained.render(&doc, p, &view, &Options::default());
        let t = std::time::Instant::now();
        for i in 1..=8u32 {
            let mut next = (*doc).clone();
            next.get_mut(pane).unwrap().transform = Affine::translate((300.0 + 3.0 * f64::from(i), 220.0));
            doc = Arc::new(next);
            let _ = retained.render(&doc, p, &view, &Options::default());
        }
        t.elapsed().as_secs_f64() * 1000.0 / 8.0
    }

    /// The canvas the owner drags in: thirty shadowed cards, three glass panes, a background-blurred
    /// card and a layer-blurred one, at a 904 × 860 canvas (100% zoom, 1 device pixel per point).
    fn heavy_scene() -> (figcraft_doc::Document, figcraft_doc::NodeId, figcraft_doc::NodeId) {
        use figcraft_color::{Color, Paint};
        use figcraft_doc::{Document, Effect, EffectKind, Glass, NodeKind};

        let mut d = Document::new("heavy");
        let p = d.first_page().unwrap();
        let colours = ["E03030", "2040E0", "20C060", "F0C830", "A040E0", "20B0D0"].map(|h| Color::from_hex(h).unwrap());
        for i in 0..30u32 {
            let (x, y) = (f64::from(i % 6) * 150.0 + 10.0, f64::from(i / 6) * 150.0 + 10.0);
            let s = d.create(p, NodeKind::Rectangle, "Card", x, y, 130.0, 130.0).unwrap();
            let n = d.get_mut(s).unwrap();
            n.fills = vec![Paint::solid(colours[i as usize % 6])];
            n.effects = vec![Effect::drop_shadow()];
        }
        let mut panes = Vec::new();
        for (x, y, w, h) in [(80.0, 80.0, 300.0, 220.0), (360.0, 260.0, 300.0, 220.0), (600.0, 480.0, 280.0, 200.0)] {
            let id = d.create(p, NodeKind::Rectangle, "Glass Pane", x, y, w, h).unwrap();
            let n = d.get_mut(id).unwrap();
            n.corners.radii = [36.0; 4];
            n.fills = vec![Paint::solid(Color { a: 0.08, ..Color::WHITE })];
            n.effects = vec![Effect { kind: EffectKind::Glass(Glass::default()), visible: true }];
            panes.push(id);
        }
        let card = d.create(p, NodeKind::Rectangle, "Blurred card", 200.0, 560.0, 260.0, 160.0).unwrap();
        d.get_mut(card).unwrap().effects = vec![Effect::background_blur()];
        let card2 = d.create(p, NodeKind::Rectangle, "Layer blur", 520.0, 640.0, 220.0, 140.0).unwrap();
        d.get_mut(card2).unwrap().effects = vec![Effect::layer_blur(), Effect::drop_shadow()];
        (d, p, panes[2])
    }

    /// Mean milliseconds of eight frames dragging `pane` by three points, from `d`.
    fn drag_ms(d: &figcraft_doc::Document, p: figcraft_doc::NodeId, pane: figcraft_doc::NodeId, doc_id: u64) -> f64 {
        use figcraft_geom::Affine;
        use figcraft_render::{Options, Retained, View};

        let view = View { origin: (0.0, 0.0), scale: 1.0, width: 904, height: 860 };
        let mut retained = Retained::default();
        let mut doc = Arc::new(d.clone());
        let opts = Options { doc_id, ..Options::default() };
        let _ = retained.render(&doc, p, &view, &opts);
        let t = std::time::Instant::now();
        for i in 1..=8u32 {
            let mut next = (*doc).clone();
            next.get_mut(pane).unwrap().transform = Affine::translate((600.0 + 5.0 * f64::from(i), 480.0 + 3.0 * f64::from(i)));
            doc = Arc::new(next);
            let _ = retained.render(&doc, p, &view, &opts);
        }
        t.elapsed().as_secs_f64() * 1000.0 / 8.0
    }

    #[test]
    #[ignore = "timing: run in release with --ignored --nocapture"]
    fn shadow_cost_timings() {
        use figcraft_color::{Color, Paint};
        use figcraft_doc::{Document, Effect, NodeKind};
        use figcraft_render::{Options, View, render_view};

        let make = |cards: u32, shadow: Option<f64>| {
            let mut d = Document::new("shadows");
            let p = d.first_page().unwrap();
            for i in 0..cards {
                let (x, y) = (f64::from(i % 4) * 140.0, f64::from(i / 4) * 140.0);
                let s = d.create(p, NodeKind::Rectangle, "Card", x, y, 130.0, 130.0).unwrap();
                let n = d.get_mut(s).unwrap();
                n.fills = vec![Paint::solid(Color::WHITE)];
                if let Some(r) = shadow {
                    n.effects = vec![Effect::drop_shadow()];
                    if let figcraft_doc::EffectKind::DropShadow(sh) = &mut n.effects[0].kind {
                        sh.radius = r;
                    }
                }
            }
            d
        };
        let view = View { origin: (0.0, 0.0), scale: 1.0, width: 600, height: 600 };
        // The pieces of a shadow on a 130 × 130 layer, on their own.
        let mut p = figcraft_render::Pixmap::new(130, 130).unwrap();
        for px in p.data_mut().as_chunks_mut::<4>().0 {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
        let t = std::time::Instant::now();
        for _ in 0..8 {
            figcraft_render::fx::blur(&mut p, 6.0, false);
        }
        println!("  blur 130 × 130 sigma 6: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 8.0);
        println!("one render of 600 × 600 (ms):");
        for (name, cards, shadow) in [
            ("1 card, no effect", 1, None),
            ("1 card, shadow r=12", 1, Some(12.0)),
            ("1 card, shadow r=0", 1, Some(0.0)),
            ("16 cards, no effect", 16, None),
            ("16 cards, shadow r=12", 16, Some(12.0)),
            ("16 cards, shadow r=0", 16, Some(0.0)),
        ] {
            let d = make(cards, shadow);
            let p = d.first_page().unwrap();
            let _ = render_view(&d, p, &view, &Options::default());
            let t = std::time::Instant::now();
            for _ in 0..8 {
                let _ = render_view(&d, p, &view, &Options::default());
            }
            println!("  {name:24} {:.2}", t.elapsed().as_secs_f64() * 1000.0 / 8.0);
        }
    }

    #[test]
    #[ignore = "timing: run in release with --ignored --nocapture"]
    fn heavy_scene_timings() {
        let Some(gpu) = gpu() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let (d, p, pane) = heavy_scene();
        // One effect group at a time, from a plain scene.
        let mut rows: Vec<(&str, f64, f64)> = Vec::new();
        #[derive(Clone, Copy)]
        enum Group {
            Nothing,
            Shadows,
            Glass,
            Back,
            Layer,
            GlassShadows,
            All,
        }
        let keeps = |g: Group, k: &figcraft_doc::EffectKind| {
            use figcraft_doc::EffectKind as K;
            match (g, k) {
                (Group::Nothing, _) => false,
                (Group::All, _) => true,
                (Group::Shadows, K::DropShadow(_)) => true,
                (Group::Glass, K::Glass(_)) => true,
                (Group::Back, K::BackgroundBlur(_)) => true,
                (Group::Layer, K::LayerBlur(_)) => true,
                (Group::GlassShadows, K::Glass(_) | K::DropShadow(_)) => true,
                _ => false,
            }
        };
        let only = |g: Group| {
            let mut x = d.clone();
            for id in x.nodes().map(|n| n.id).collect::<Vec<_>>() {
                if let Ok(n) = x.get_mut(id) {
                    n.effects.retain(|e| keeps(g, &e.kind));
                }
            }
            x
        };
        let ids = std::sync::atomic::AtomicU64::new(1);
        let all = |d: &figcraft_doc::Document| {
            figcraft_render::set_accelerator(None);
            let cpu = drag_ms(d, p, pane, ids.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            figcraft_render::set_accelerator(Some(gpu.clone()));
            let gpu_ms = drag_ms(d, p, pane, ids.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            (cpu, gpu_ms)
        };
        for (name, g) in [
            ("nothing", Group::Nothing),
            ("30 drop shadows", Group::Shadows),
            ("3 glass panes", Group::Glass),
            ("background blur", Group::Back),
            ("layer blur", Group::Layer),
            ("glass + shadows", Group::GlassShadows),
            ("everything", Group::All),
        ] {
            let (cpu, gpu_ms) = all(&only(g));
            rows.push((name, cpu, gpu_ms));
        }
        figcraft_render::set_accelerator(None);
        println!("heavy scene, drag a glass pane (904 × 860):");
        for (name, cpu, gpu_ms) in rows {
            println!("  {name:16} CPU {cpu:6.1} ms, GPU {gpu_ms:6.1} ms");
        }
    }

    #[test]
    #[ignore = "timing: run in release with --ignored --nocapture"]
    fn canvas_drag_timings() {
        use figcraft_doc::Glass;

        let Some(gpu) = gpu() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let frost = Glass::default().frost;
        let cpu = drag_frames(frost);
        figcraft_render::set_accelerator(Some(gpu));
        let on_gpu = drag_frames(frost);
        let dry_cpu = drag_frames(0.0);
        let dry_gpu = drag_frames(0.0);
        let plain = drag_frames(-1.0);
        figcraft_render::set_accelerator(None);
        println!("canvas: drag the pane (2400 × 1600): CPU {cpu:.1} ms, GPU {on_gpu:.1} ms a frame");
        println!("canvas: the same with no frost: CPU {dry_cpu:.1} ms, GPU {dry_gpu:.1} ms a frame");
        println!("canvas: the same with no glass at all: {plain:.1} ms a frame");
    }
}
