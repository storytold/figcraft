const POINTS_PER_INCH: f64 = 72.0;
const METERS_PER_INCH: f64 = 0.0254;
const MAX_DENSITY: f64 = 4.0;
const DENSITY_TOLERANCE: f64 = 0.005;

pub fn pixel_density(bytes: &[u8]) -> f64 {
    craft_image_codecs::png_probe::pixels_per_meter(bytes).and_then(|(x, y)| (x == y).then_some(x)).map_or(1.0, density_from_pixels_per_meter)
}

pub fn placed_size(pixel_width: u32, pixel_height: u32, density: f64, max_side: f64) -> (f64, f64) {
    let (w, h) = (f64::from(pixel_width) / density, f64::from(pixel_height) / density);
    let fit = (max_side / w.max(h)).min(1.0);
    (w * fit, h * fit)
}

fn density_from_pixels_per_meter(pixels_per_meter: u32) -> f64 {
    let ratio = f64::from(pixels_per_meter) * METERS_PER_INCH / POINTS_PER_INCH;
    let whole = ratio.round();
    if (2.0..=MAX_DENSITY).contains(&whole) && (ratio - whole).abs() < DENSITY_TOLERANCE { whole } else { 1.0 }
}
