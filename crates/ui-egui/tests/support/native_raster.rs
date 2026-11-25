//! Software rendering of native egui Home-screen meshes and texture uploads.
//! Adapted from SolidCraft tests/support/raster.rs at upstream 57d789c0aaf1bcf4691647f9a1cb7a547eb45fdf.
//! This test helper has no FigCraft UI or storage implementation.
/*
MIT License

Copyright (c) 2026 SolidCraft contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/
use egui::{ColorImage, TextureId, pos2};
use std::collections::HashMap;

pub fn remember(textures: &mut HashMap<TextureId, ColorImage>, out: &mut egui::FullOutput) {
    for (id, deltas) in &out.textures_delta.set {
        for delta in deltas {
            let egui::ImageData::Color(image) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let dest = textures.get_mut(id).unwrap();
                let dest_width = dest.width();
                for row in 0..image.height() {
                    for col in 0..image.width() {
                        dest.pixels[(y + row) * dest_width + x + col] = image.pixels[row * image.width() + col];
                    }
                }
            } else {
                textures.insert(*id, (**image).clone());
            }
        }
    }
    out.textures_delta.clear();
}

pub fn raster(ctx: &egui::Context, textures: &HashMap<TextureId, ColorImage>, out: egui::FullOutput) -> image::RgbaImage {
    let ppp = out.pixels_per_point;
    let size = ctx.viewport_rect().size();
    let (width, height) = ((size.x * ppp) as u32, (size.y * ppp) as u32);
    let mut pixels = vec![255u8; (width * height * 4) as usize];
    for clipped in ctx.tessellate(out.shapes, ppp) {
        let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive else { continue };
        let Some(texture) = textures.get(&mesh.texture_id) else { continue };
        for indices in mesh.indices.as_chunks::<3>().0 {
            let vertices = indices.map(|i| {
                let mut v = mesh.vertices[i as usize];
                v.pos = pos2(v.pos.x * ppp, v.pos.y * ppp);
                v
            });
            triangle(&mut pixels, [width, height], texture, clipped.clip_rect * ppp, vertices);
        }
    }
    image::RgbaImage::from_raw(width, height, pixels).unwrap()
}

fn triangle(pixels: &mut [u8], [width, height]: [u32; 2], texture: &ColorImage, clip: egui::Rect, [a, b, c]: [egui::epaint::Vertex; 3]) {
    let cross = |u: egui::Vec2, v: egui::Vec2| u.x * v.y - u.y * v.x;
    let area = cross(b.pos - a.pos, c.pos - a.pos);
    if area.abs() < 1e-6 {
        return;
    }
    let left = a.pos.x.min(b.pos.x).min(c.pos.x).max(clip.left()).max(0.0).floor() as u32;
    let right = a.pos.x.max(b.pos.x).max(c.pos.x).min(clip.right()).min(width as f32).ceil() as u32;
    let top = a.pos.y.min(b.pos.y).min(c.pos.y).max(clip.top()).max(0.0).floor() as u32;
    let bottom = a.pos.y.max(b.pos.y).max(c.pos.y).min(clip.bottom()).min(height as f32).ceil() as u32;
    for y in top..bottom {
        for x in left..right {
            let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
            let w = [cross(b.pos - p, c.pos - p) / area, cross(c.pos - p, a.pos - p) / area, cross(a.pos - p, b.pos - p) / area];
            if w.iter().any(|v| *v < -1e-5) {
                continue;
            }
            let uv = a.uv.to_vec2() * w[0] + b.uv.to_vec2() * w[1] + c.uv.to_vec2() * w[2];
            let tx = (uv.x * texture.width() as f32 - 0.5).clamp(0.0, texture.width() as f32 - 1.0);
            let ty = (uv.y * texture.height() as f32 - 0.5).clamp(0.0, texture.height() as f32 - 1.0);
            let (ix, iy) = (tx.floor() as usize, ty.floor() as usize);
            let get = |x: usize, y: usize| texture.pixels[y * texture.width() + x].to_array().map(|v| v as f32 / 255.0);
            let samples = [
                get(ix, iy),
                get((ix + 1).min(texture.width() - 1), iy),
                get(ix, (iy + 1).min(texture.height() - 1)),
                get((ix + 1).min(texture.width() - 1), (iy + 1).min(texture.height() - 1)),
            ];
            let (fx, fy) = (tx.fract(), ty.fract());
            let col = [a.color.to_array(), b.color.to_array(), c.color.to_array()];
            let mut src = [0.0; 4];
            for i in 0..4 {
                let tex = samples[0][i] * (1.0 - fx) * (1.0 - fy)
                    + samples[1][i] * fx * (1.0 - fy)
                    + samples[2][i] * (1.0 - fx) * fy
                    + samples[3][i] * fx * fy;
                src[i] = tex * (col[0][i] as f32 * w[0] + col[1][i] as f32 * w[1] + col[2][i] as f32 * w[2]) / 255.0;
            }
            let base = ((y * width + x) * 4) as usize;
            for i in 0..3 {
                pixels[base + i] = ((src[i] + pixels[base + i] as f32 / 255.0 * (1.0 - src[3])) * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
    }
}
