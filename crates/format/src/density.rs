const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const POINTS_PER_INCH: f64 = 72.0;
const METERS_PER_INCH: f64 = 0.0254;
const UNIT_METER: u8 = 1;
const MAX_DENSITY: f64 = 4.0;
const DENSITY_TOLERANCE: f64 = 0.005;

pub fn pixel_density(bytes: &[u8]) -> f64 {
    png_pixels_per_meter(bytes).map_or(1.0, density_from_pixels_per_meter)
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

fn png_pixels_per_meter(bytes: &[u8]) -> Option<u32> {
    let mut rest = bytes.strip_prefix(PNG_SIGNATURE)?;
    while rest.len() >= 12 {
        let length = usize::try_from(u32::from_be_bytes(rest[..4].try_into().ok()?)).ok()?;
        let kind = &rest[4..8];
        let data = rest.get(8..8usize.checked_add(length)?)?;
        match kind {
            b"pHYs" if data.len() == 9 => {
                let x = u32::from_be_bytes(data[..4].try_into().ok()?);
                let y = u32::from_be_bytes(data[4..8].try_into().ok()?);
                return (data[8] == UNIT_METER && x == y).then_some(x);
            }
            b"IDAT" | b"IEND" => return None,
            _ => rest = rest.get(12usize.checked_add(length)?..)?,
        }
    }
    None
}
