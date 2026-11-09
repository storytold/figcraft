//! The transparency grid behind the page; rendered pixels already contain the page colour.

use egui::{Color32, CornerRadius, Id, Painter, Rect, TextureHandle, pos2};

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
    use egui::vec2;

    fn shapes(alpha: u8, rendered: Option<Rect>) -> egui::FullOutput {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            paint(
                ui.painter(),
                Rect::from_min_size(egui::Pos2::ZERO, vec2(100.0, 100.0)),
                Color32::from_rgba_unmultiplied(255, 0, 0, alpha),
                rendered,
            );
        });
        out.textures_delta.clear();
        out
    }

    #[test]
    fn the_canvas_draws_the_transparency_grid_for_a_transparent_page() {
        let mut ui = crate::layers_drag_tests::Ui::new(figcraft_engine::Session::new());
        ui.app.run("page.setBackground", serde_json::json!({"color": "#FF000000"}));
        let out = ui.frame_output(vec![]);
        let (tile, _) =
            ui.ctx.data(|d| d.get_temp::<(TextureHandle, [Color32; 2])>(Id::unique("canvas_transparency_grid"))).expect("the canvas checkerboard");
        assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id == tile.id())));
    }

    #[test]
    fn the_grid_updates_its_colours_when_the_theme_changes() {
        let ctx = egui::Context::default();
        let mut previous_id = None;
        for tokens in [crate::theme::Tokens::light(), crate::theme::Tokens::dark(), crate::theme::Tokens::light()] {
            crate::theme::apply(&ctx, tokens);
            let mut tile = None;
            let mut out = ctx.run_ui(Default::default(), |_ui| tile = Some(grid_texture(&ctx)));
            let tile = tile.unwrap();
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

    #[test]
    fn translucent_pages_show_the_grid_without_applying_page_alpha_twice() {
        let rendered = Rect::from_min_max(pos2(20.0, 20.0), pos2(80.0, 80.0));
        for alpha in [0, 128, 255] {
            let out = shapes(alpha, Some(rendered));
            let fills: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) => Some(r.rect),
                    _ => None,
                })
                .collect();
            assert_eq!(fills.len(), 4);
            assert!(fills.iter().all(|r| !r.intersect(rendered).is_positive()), "the render's alpha has no page colour beneath it");
            for point in [pos2(10.0, 10.0), pos2(90.0, 90.0), pos2(10.0, 50.0), pos2(90.0, 50.0)] {
                assert_eq!(fills.iter().filter(|r| r.contains(point)).count(), 1, "uncovered pixels get the page colour once");
            }
            let images = out.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Mesh(_))).count();
            assert_eq!(images, usize::from(alpha < 255), "only transparent pages need the checkerboard");
        }
    }

    #[test]
    fn absent_or_offscreen_renders_fill_the_page_once() {
        for rendered in [None, Some(Rect::from_min_size(pos2(200.0, 200.0), vec2(10.0, 10.0)))] {
            let out = shapes(128, rendered);
            let fills: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) => Some(r),
                    _ => None,
                })
                .collect();
            assert_eq!(fills.len(), 1);
            assert_eq!(fills[0].rect, Rect::from_min_size(egui::Pos2::ZERO, vec2(100.0, 100.0)));
            assert_eq!(fills[0].fill.a(), 128);
        }
    }
}
