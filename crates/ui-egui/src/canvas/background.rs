//! The transparency grid behind the page; rendered pixels already contain the page colour.

use egui::{Color32, CornerRadius, Id, Painter, Rect, TextureHandle, pos2};

/// Reuse the checkerboard texture, updating its pixels when the active theme changes.
fn grid_texture(ctx: &egui::Context) -> TextureHandle {
    let id = Id::unique("canvas_transparency_grid");
    let tokens = crate::theme::Tokens::get(ctx);
    let colors = [tokens.panel, tokens.canvas];
    let cached = ctx.data(|data| data.get_temp::<(TextureHandle, [Color32; 2])>(id));
    if let Some((texture, old_colors)) = &cached
        && *old_colors == colors
    {
        return texture.clone();
    }
    let image = egui::ColorImage::new([2, 2], vec![colors[0], colors[1], colors[1], colors[0]]);
    let texture = match cached {
        Some((mut texture, _)) => {
            texture.set(image, egui::TextureOptions::NEAREST_REPEAT);
            texture
        }
        None => ctx.load_texture("canvas-transparency", image, egui::TextureOptions::NEAREST_REPEAT),
    };
    ctx.data_mut(|data| data.insert_temp(id, (texture.clone(), colors)));
    texture
}

/// Draw the transparency grid and apply page colour only outside the rendered image.
pub(super) fn paint(painter: &Painter, rect: Rect, background: Color32, rendered: Option<Rect>) {
    if background.a() < 255 {
        // Repeat four pixels instead of allocating one shape per square on a large canvas.
        let texture = grid_texture(painter.ctx());
        let uv = Rect::from_min_size(egui::Pos2::ZERO, rect.size() / 16.0);
        painter.image(texture.id(), rect, uv, Color32::WHITE);
    }
    // The render contains the page colour and its alpha. Painting that colour beneath it
    // applies alpha twice (50% becomes 75%). Only uncovered areas need the fallback colour.
    let covered = rendered.map(|r| r.intersect(rect)).filter(|r| r.is_positive());
    let areas = match covered {
        Some(r) => [
            Rect::from_min_max(rect.min, pos2(rect.right(), r.top())),
            Rect::from_min_max(pos2(rect.left(), r.bottom()), rect.max),
            Rect::from_min_max(pos2(rect.left(), r.top()), r.left_bottom()),
            Rect::from_min_max(r.right_top(), pos2(rect.right(), r.bottom())),
        ],
        None => [rect, Rect::NOTHING, Rect::NOTHING, Rect::NOTHING],
    };
    for area in areas.into_iter().filter(|r| r.is_positive()) {
        painter.rect_filled(area, CornerRadius::ZERO, background);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Mesh, TextureId, vec2};

    /// Paint a small canvas and consume its texture uploads as the UI harness does.
    fn shapes(alpha: u8, rendered: Option<Rect>) -> egui::FullOutput {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            paint(ui.painter(), canvas(), Color32::from_rgba_unmultiplied(255, 0, 0, alpha), rendered);
        });
        out.textures_delta.clear();
        out
    }

    /// Fixed canvas bounds make clipping and uncovered-area checks independent of UI layout.
    fn canvas() -> Rect {
        Rect::from_min_size(egui::Pos2::ZERO, vec2(100.0, 100.0))
    }

    /// Check the actual checkerboard quad, including clipping and its screen-sized UV mapping.
    fn assert_grid(out: &egui::FullOutput, tile: TextureId, rect: Rect) -> usize {
        let meshes: Vec<_> = out
            .shapes
            .iter()
            .enumerate()
            .filter_map(|(index, shape)| match &shape.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == tile => Some((index, shape.clip_rect, mesh)),
                _ => None,
            })
            .collect();
        assert_eq!(meshes.len(), 1, "one checkerboard covers the canvas");
        let (index, clip, mesh) = meshes[0];
        assert_eq!(mesh.calc_bounds(), rect);
        assert!(clip.contains_rect(rect), "the whole checkerboard is visible");
        assert_eq!(mesh.vertices.len(), 4);
        for vertex in &mesh.vertices {
            let expected = (vertex.pos - rect.min) / 16.0;
            assert!((vertex.uv.to_vec2() - expected).length() < 0.001, "UVs keep the squares at a fixed screen size");
        }
        index
    }

    /// Assert strips have the correct colour and partition exactly the area outside the render.
    fn assert_partition(alpha: u8, rendered: Option<Rect>) {
        let out = shapes(alpha, rendered);
        let fills: Vec<_> = out
            .shapes
            .iter()
            .enumerate()
            .filter_map(|(index, s)| match &s.shape {
                egui::Shape::Rect(r) => Some((index, r)),
                _ => None,
            })
            .collect();
        let covered = rendered.map(|r| r.intersect(canvas())).filter(|r| r.is_positive());
        let background = Color32::from_rgba_unmultiplied(255, 0, 0, alpha);
        for (i, (_, strip)) in fills.iter().enumerate() {
            assert_eq!(strip.fill.a(), alpha, "each fallback strip applies alpha exactly once");
            assert_eq!(strip.fill, background, "the fallback keeps the page colour too");
            assert!(strip.rect.is_positive() && canvas().contains_rect(strip.rect));
            if let Some(covered) = covered {
                assert!(!strip.rect.intersect(covered).is_positive(), "no fallback beneath rendered pixels");
            }
            for (_, other) in fills.iter().skip(i + 1) {
                assert!(!strip.rect.intersect(other.rect).is_positive(), "fallback strips do not overlap");
            }
        }
        let area: f32 = fills.iter().map(|(_, strip)| strip.rect.area()).sum();
        assert_eq!(area, canvas().area() - covered.map_or(0.0, |r| r.area()), "no uncovered gaps");
        if covered == Some(canvas()) {
            assert!(fills.is_empty(), "full coverage needs no fallback fills");
        }
        let meshes: Vec<&Mesh> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.as_ref()),
                _ => None,
            })
            .collect();
        if alpha < 255 {
            assert_eq!(meshes.len(), 1);
            let index = assert_grid(&out, meshes[0].texture_id, canvas());
            assert!(fills.iter().all(|(fill_index, _)| *fill_index > index), "the grid is behind every fallback strip");
        } else {
            assert!(meshes.is_empty(), "opaque pages need no checkerboard");
        }
    }

    /// Exercise the real canvas with a render present, checking the grid covers it and draws first.
    #[test]
    fn the_canvas_draws_the_transparency_grid_for_a_transparent_page() {
        let mut ui = crate::layers_drag_tests::Ui::new(figcraft_engine::Session::new());
        ui.app.run("page.setBackground", serde_json::json!({"color": "#FF000000"}));
        let ds = ui.app.session.ds().unwrap();
        let shown = (ds.uid, ds.page);
        // Seed an image so this test does not depend on when the asynchronous worker finishes.
        ui.app.canvas.texture =
            Some(ui.ctx.load_texture("test-page-render", egui::ColorImage::filled([16, 16], Color32::TRANSPARENT), egui::TextureOptions::NEAREST));
        ui.app.canvas.tex_view = Some(figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 16, height: 16 });
        ui.app.canvas.tex_of = Some(shown);
        let out = ui.frame_output(vec![]);
        let (tile, _) =
            ui.ctx.data(|d| d.get_temp::<(TextureHandle, [Color32; 2])>(Id::unique("canvas_transparency_grid"))).expect("the canvas checkerboard");
        let grid_index = assert_grid(&out, tile.id(), ui.app.canvas.rect);
        let rendered = ui.app.canvas.texture.as_ref().unwrap().id();
        let render_index =
            out.shapes.iter().position(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id == rendered)).expect("the rendered page");
        assert!(grid_index < render_index, "the checkerboard is behind the rendered page");
    }

    /// Theme changes must reach drawing, update the pixels used by its mesh and reuse the texture.
    #[test]
    fn the_grid_updates_its_colours_when_the_theme_changes() {
        let ctx = egui::Context::default();
        let mut previous_id = None;
        for tokens in [crate::theme::Tokens::light(), crate::theme::Tokens::dark(), crate::theme::Tokens::light()] {
            crate::theme::apply(&ctx, tokens);
            let mut out = ctx.run_ui(Default::default(), |ui| paint(ui.painter(), canvas(), Color32::TRANSPARENT, None));
            let (tile, _) =
                ctx.data(|d| d.get_temp::<(TextureHandle, [Color32; 2])>(Id::unique("canvas_transparency_grid"))).expect("drawn checkerboard");
            let grid_index = assert_grid(&out, tile.id(), canvas());
            assert!(out.shapes.iter().enumerate().all(|(index, s)| !matches!(&s.shape, egui::Shape::Rect(_)) || index > grid_index));
            if let Some(previous_id) = previous_id {
                assert_eq!(tile.id(), previous_id, "changing theme updates the same texture");
            }
            let delta = out.textures_delta.set.get(&tile.id()).and_then(|d| d.first()).expect("updated checkerboard pixels");
            let egui::ImageData::Color(image) = &delta.image;
            assert_eq!(image.pixels, [tokens.panel, tokens.canvas, tokens.canvas, tokens.panel]);
            previous_id = Some(tile.id());
            out.textures_delta.clear();
        }
    }

    /// Interior, edge-straddling and fully covering renders must all apply the page alpha once.
    #[test]
    fn translucent_pages_show_the_grid_without_applying_page_alpha_twice() {
        let renders = [
            Rect::from_min_max(pos2(20.0, 20.0), pos2(80.0, 80.0)),
            Rect::from_min_max(pos2(-20.0, 20.0), pos2(80.0, 80.0)),
            Rect::from_min_max(pos2(20.0, -20.0), pos2(80.0, 80.0)),
            Rect::from_min_max(pos2(20.0, 20.0), pos2(120.0, 80.0)),
            Rect::from_min_max(pos2(20.0, 20.0), pos2(80.0, 120.0)),
            Rect::from_min_max(pos2(-20.0, -20.0), pos2(80.0, 80.0)),
            canvas(),
            Rect::from_min_max(pos2(-20.0, -20.0), pos2(120.0, 120.0)),
        ];
        for alpha in [0, 128, 255] {
            for rendered in renders {
                assert_partition(alpha, Some(rendered));
            }
        }
    }

    /// Missing, offscreen and merely edge-touching images leave exactly one fallback page fill.
    #[test]
    fn absent_or_offscreen_renders_fill_the_page_once() {
        for rendered in
            [None, Some(Rect::from_min_size(pos2(200.0, 200.0), vec2(10.0, 10.0))), Some(Rect::from_min_max(pos2(-20.0, 20.0), pos2(0.0, 80.0)))]
        {
            for alpha in [0, 128, 255] {
                assert_partition(alpha, rendered);
                let out = shapes(alpha, rendered);
                let fills: Vec<_> = out
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Rect(r) => Some(r),
                        _ => None,
                    })
                    .collect();
                assert_eq!(fills.len(), 1);
                assert_eq!(fills[0].rect, canvas());
            }
        }
    }
}
