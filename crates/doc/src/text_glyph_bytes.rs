//! Lossless byte-plane columns for portable glyph numbers. Separating equally significant
//! bytes makes repeated layouts compress without decimal float formatting or JSON number parsing.
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::text_appearance::{CompactGlyph, Placement};

const MAX_GLYPHS: usize = 100_000;
// Four u32 + four f64 columns, two flag bytes, and at most six affine f64s per record.
const MAX_RAW: usize = 4 + MAX_GLYPHS * 98;
const MAX_PACKED: usize = 4 + MAX_RAW + MAX_RAW.div_ceil(128);
pub(crate) const MAX_ENCODED: usize = MAX_PACKED.div_ceil(3) * 4;

// Version three's bounded zero-run stream. A four-byte expanded size precedes blocks:
// high-bit tags append 1..128 zeros; low-bit tags copy 1..128 following literal bytes.
// One layer cannot ask a tiny record to allocate beyond the normal glyph byte budget.
fn pack(bytes: &[u8]) -> Vec<u8> {
    let mut out = (bytes.len() as u32).to_le_bytes().to_vec();
    let mut at = 0;
    while at < bytes.len() {
        // Short zero runs stay literal; otherwise alternating bytes could expand by 50%.
        let zero = bytes.get(at..at + 3) == Some(&[0, 0, 0]);
        let mut end = at + 1;
        while end < bytes.len() && end - at < 128 {
            if if zero { bytes.get(end) != Some(&0) } else { bytes.get(end..end + 3) == Some(&[0, 0, 0]) } {
                break;
            }
            end += 1;
        }
        out.push((end - at - 1) as u8 | if zero { 128 } else { 0 });
        if !zero {
            out.extend_from_slice(bytes.get(at..end).unwrap_or_default());
        }
        at = end;
    }
    out
}

fn unpack(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let header: [u8; 4] = bytes.get(..4).ok_or("truncated portable run header")?.try_into().map_err(|_| "invalid portable run header")?;
    let size = u32::from_le_bytes(header) as usize;
    if size > MAX_RAW {
        return Err("expanded portable bytes exceed limit");
    }
    let mut out = Vec::with_capacity(size);
    let mut reader = Reader { bytes, offset: 4 };
    while reader.offset < bytes.len() {
        let tag = *reader.take(1)?.first().ok_or("missing portable run tag")?;
        let count = usize::from(tag & 127) + 1;
        if count > size.saturating_sub(out.len()) {
            return Err("portable run exceeds expanded size");
        }
        if tag & 128 != 0 {
            out.resize(out.len() + count, 0);
        } else {
            out.extend_from_slice(reader.take(count)?);
        }
    }
    if out.len() != size {
        return Err("truncated expanded portable bytes");
    }
    Ok(out)
}

fn put_column<const N: usize>(out: &mut Vec<u8>, values: impl Iterator<Item = [u8; N]>) {
    let values: Vec<_> = values.collect();
    for plane in 0..N {
        for value in &values {
            out.push(value[plane]);
        }
    }
}

fn finite_bytes(value: f64) -> [u8; 8] {
    // Native JSON's Finite adapter cannot inspect an encoded string. Apply its same rule here.
    (if value.is_finite() { value } else { 0.0 }).to_le_bytes()
}

pub(crate) fn encode(glyphs: &[CompactGlyph]) -> Result<String, &'static str> {
    encode_version(glyphs, true)
}

fn encode_version(glyphs: &[CompactGlyph], predicted: bool) -> Result<String, &'static str> {
    if glyphs.len() > MAX_GLYPHS {
        return Err("too many portable glyphs");
    }
    let index = |v: usize| u32::try_from(v).map(u32::to_le_bytes).map_err(|_| "portable glyph index exceeds u32");
    let mut previous_end = 0u32;
    let mut previous_style = 0u32;
    let integers = glyphs
        .iter()
        .map(|g| {
            let start = u32::try_from(g.5).map_err(|_| "portable glyph index exceeds u32")?;
            let end = u32::try_from(g.6).map_err(|_| "portable glyph index exceeds u32")?;
            let style = u32::try_from(g.7).map_err(|_| "portable glyph index exceeds u32")?;
            let row = [
                index(g.0)?,
                (start ^ if predicted { previous_end } else { 0 }).to_le_bytes(),
                (end ^ if predicted { start.wrapping_add(1) } else { 0 }).to_le_bytes(),
                (style ^ if predicted { previous_style } else { 0 }).to_le_bytes(),
            ];
            previous_end = end;
            previous_style = style;
            Ok(row)
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    let count = u32::try_from(glyphs.len()).map_err(|_| "too many portable glyphs")?;
    let mut bytes = count.to_le_bytes().to_vec();
    for column in 0..4 {
        put_column(&mut bytes, integers.iter().filter_map(|v| v.get(column)).copied());
    }
    for column in 0..4 {
        let mut previous = 0.0;
        put_column(
            &mut bytes,
            glyphs.iter().map(|g| {
                let current = match column {
                    0 => g.1,
                    1 => g.2,
                    2 => g.3,
                    _ => g.4,
                };
                let bits = u64::from_le_bytes(finite_bytes(current));
                // XOR bits, never subtract or quantize coordinates. Ordinary adjacent glyphs
                // start at the previous x + advance; line breaks and bidi remain exact residuals.
                let residual = bits ^ if predicted { u64::from_le_bytes(finite_bytes(previous)) } else { 0 };
                previous = if column == 0 { f64::from_bits(bits) + f64::from_le_bytes(finite_bytes(g.4)) } else { f64::from_bits(bits) };
                residual.to_le_bytes()
            }),
        );
    }
    bytes.extend(glyphs.iter().map(|g| u8::from(g.8)));
    bytes.extend(glyphs.iter().map(|g| match g.9 {
        None => 0,
        Some(Placement::Scale(_)) => 1,
        Some(Placement::Matrix(_)) => 2,
    }));
    let mut previous_placement = [0u64; 6];
    let mut put_number = |bytes: &mut Vec<u8>, col: usize, v: f64| {
        let bits = u64::from_le_bytes(finite_bytes(v));
        bytes.extend((bits ^ if predicted { previous_placement[col] } else { 0 }).to_le_bytes());
        previous_placement[col] = bits;
    };
    for g in glyphs {
        match g.9 {
            None => {}
            Some(Placement::Scale(s)) => put_number(&mut bytes, 0, s),
            Some(Placement::Matrix(m)) => {
                for (col, value) in m.into_iter().enumerate() {
                    put_number(&mut bytes, col, value);
                }
            }
        }
    }
    Ok(STANDARD.encode(if predicted { pack(&bytes) } else { bytes }))
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], &'static str> {
        let end = self.offset.checked_add(size).ok_or("portable glyph offset overflow")?;
        let result = self.bytes.get(self.offset..end).ok_or("truncated portable glyph bytes")?;
        self.offset = end;
        Ok(result)
    }
    fn column<const N: usize>(&mut self, count: usize) -> Result<Column<'a, N>, &'static str> {
        let bytes = self.take(count.checked_mul(N).ok_or("portable glyph column overflow")?)?;
        Ok(Column { bytes, count })
    }
}

fn number(bytes: [u8; 8]) -> Result<f64, &'static str> {
    let value = f64::from_le_bytes(bytes);
    if value.is_finite() { Ok(value) } else { Err("non-finite portable number") }
}

// Borrow byte planes from the decoded buffer instead of allocating/transposing eight more
// vectors per layer. Each returned number is assembled in a fixed-size stack array.
struct Column<'a, const N: usize> {
    bytes: &'a [u8],
    count: usize,
}
fn value<const N: usize>(column: &Column<'_, N>, row: usize) -> Result<[u8; N], &'static str> {
    if row >= column.count {
        return Err("missing portable column value");
    }
    let mut out = [0; N];
    // count*N was checked by Reader::column and row<count above. Constant-size planes
    // need indexed byte loads, not a new runtime chunks/division iterator per number.
    for (plane, slot) in out.iter_mut().enumerate() {
        *slot = *column.bytes.get(plane * column.count + row).ok_or("missing portable column value")?;
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) fn decode(text: &str, predicted: bool) -> Result<Vec<CompactGlyph>, &'static str> {
    decode_map(text, predicted, Ok)
}

/// Validate and construct final records in one preallocated vector. Do not materialize a
/// full temporary tuple/enum vector before building the document's positioned glyphs.
pub(crate) fn decode_map<T>(
    text: &str,
    predicted: bool,
    mut record: impl FnMut(CompactGlyph) -> Result<T, &'static str>,
) -> Result<Vec<T>, &'static str> {
    if text.len() > MAX_ENCODED {
        return Err("portable glyph bytes exceed limit");
    }
    let bytes = STANDARD.decode(text).map_err(|_| "invalid base64 portable glyphs")?;
    if bytes.len() > MAX_PACKED {
        return Err("portable glyph bytes exceed limit");
    }
    let bytes = if predicted { unpack(&bytes)? } else { bytes };
    if bytes.len() > MAX_RAW {
        return Err("portable glyph bytes exceed limit");
    }
    let mut reader = Reader { bytes: &bytes, offset: 0 };
    let header: [u8; 4] = reader.take(4)?.try_into().map_err(|_| "invalid portable glyph count")?;
    let count = u32::from_le_bytes(header) as usize;
    if count > MAX_GLYPHS {
        return Err("too many portable glyphs");
    }
    let indices = reader.column::<4>(count)?;
    let starts = reader.column::<4>(count)?;
    let ends = reader.column::<4>(count)?;
    let styles = reader.column::<4>(count)?;
    let x = reader.column::<8>(count)?;
    let y = reader.column::<8>(count)?;
    let size = reader.column::<8>(count)?;
    let advance = reader.column::<8>(count)?;
    let rtl = reader.take(count)?;
    let tags = reader.take(count)?;
    let mut glyphs = Vec::with_capacity(count);
    let mut previous = [0.0; 4];
    let mut previous_end = 0u32;
    let mut previous_style = 0u32;
    let mut previous_placement = [0u64; 6];
    for row in 0..count {
        let direction = match rtl.get(row) {
            Some(0) => false,
            Some(1) => true,
            _ => return Err("invalid portable direction flag"),
        };
        let mut read_number = |col: usize| -> Result<f64, &'static str> {
            let raw: [u8; 8] = reader.take(8)?.try_into().map_err(|_| "invalid portable number")?;
            let bits = u64::from_le_bytes(raw) ^ if predicted { previous_placement[col] } else { 0 };
            previous_placement[col] = bits;
            number(bits.to_le_bytes())
        };
        let placement = match tags.get(row) {
            Some(0) => None,
            Some(1) => Some(Placement::Scale(read_number(0)?)),
            Some(2) => {
                Some(Placement::Matrix([read_number(0)?, read_number(1)?, read_number(2)?, read_number(3)?, read_number(4)?, read_number(5)?]))
            }
            _ => return Err("invalid portable placement flag"),
        };
        let mut numbers = [0.0; 4];
        for (col, data) in [&x, &y, &size, &advance].into_iter().enumerate() {
            let bits = u64::from_le_bytes(value(data, row)?);
            numbers[col] = number((bits ^ if predicted { u64::from_le_bytes(finite_bytes(previous[col])) } else { 0 }).to_le_bytes())?;
        }
        previous = [numbers[0] + numbers[3], numbers[1], numbers[2], numbers[3]];
        let start = u32::from_le_bytes(value(&starts, row)?) ^ if predicted { previous_end } else { 0 };
        let end = u32::from_le_bytes(value(&ends, row)?) ^ if predicted { start.wrapping_add(1) } else { 0 };
        let style = u32::from_le_bytes(value(&styles, row)?) ^ if predicted { previous_style } else { 0 };
        previous_end = end;
        previous_style = style;
        glyphs.push(record((
            u32::from_le_bytes(value(&indices, row)?) as usize,
            numbers[0],
            numbers[1],
            numbers[2],
            numbers[3],
            start as usize,
            end as usize,
            style as usize,
            direction,
            placement,
        ))?);
    }
    if reader.offset != bytes.len() {
        return Err("trailing portable glyph bytes");
    }
    Ok(glyphs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_runs_bound_expansion_and_refuse_corruption() {
        for bytes in [vec![], vec![0; 1024], vec![1; 1024], (0..=255).collect(), vec![0, 1, 0, 0, 2]] {
            assert_eq!(unpack(&pack(&bytes)).unwrap(), bytes);
        }
        assert!(pack(&[0; 4096]).len() < 64);
        for pattern in [vec![0, 1], vec![0, 0, 1], vec![0, 0, 0, 1], vec![1; 127], (0..=255).collect()] {
            let input = pattern.repeat(1024);
            let packed = pack(&input);
            assert!(packed.len() <= 4 + input.len() + input.len().div_ceil(128));
            assert_eq!(unpack(&packed).unwrap(), input);
        }
        assert!(unpack(&u32::MAX.to_le_bytes()).is_err());
        assert!(unpack(&[0, 0, 0]).is_err());
        assert!(unpack(&[1, 0, 0, 0]).is_err());
        assert!(unpack(&[1, 0, 0, 0, 129]).is_err());
        assert!(unpack(&[1, 0, 0, 0, 0]).is_err());
        assert!(unpack(&[0, 0, 0, 0, 128]).is_err());
    }

    #[test]
    fn prediction_preserves_every_finite_bit_and_old_encoding() {
        // Include negative zero, denormals, overflowed predictions, backwards/bidi positions
        // and unrelated finite bit patterns. Encoding two must remain readable unchanged.
        let mut seed = 0x1234_5678_90ab_cdefu64;
        let mut input = Vec::new();
        for i in 0..4096 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let x = f64::from_bits(seed);
            let x = if x.is_finite() { x } else { -0.0 };
            let placement = match i % 3 {
                0 => None,
                1 => Some(Placement::Scale(x)),
                _ => Some(Placement::Matrix([x, -0.0, 0.2, 0.5, 2.0, 3.0])),
            };
            input.push((i % 7, x, -0.0, f64::MIN_POSITIVE, f64::MAX, i, i + 1, i % 5, i % 2 == 0, placement));
        }
        for predicted in [false, true] {
            let encoded = encode_version(&input, predicted).unwrap();
            let back = decode(&encoded, predicted).unwrap();
            for (a, b) in input.iter().zip(back.iter()) {
                assert_eq!(a.1.to_bits(), b.1.to_bits());
                assert_eq!(a.2.to_bits(), b.2.to_bits());
                assert_eq!(a.3.to_bits(), b.3.to_bits());
                assert_eq!(a.4.to_bits(), b.4.to_bits());
                assert_eq!((a.0, a.5, a.6, a.7, a.8), (b.0, b.5, b.6, b.7, b.8));
                let bits = |p: &Option<Placement>| match p {
                    None => vec![],
                    Some(Placement::Scale(s)) => vec![s.to_bits()],
                    Some(Placement::Matrix(m)) => m.iter().map(|v| v.to_bits()).collect(),
                };
                assert_eq!(bits(&a.9), bits(&b.9));
            }
        }
        let nonfinite = [(0, 1.0, f64::NAN, 12.0, f64::INFINITY, 0, 1, 0, false, None), (0, 2.0, 0.0, 12.0, 3.0, 1, 2, 0, false, None)];
        let back = decode(&encode(&nonfinite).unwrap(), true).unwrap();
        assert_eq!(back[1].1, 2.0);
        assert_eq!(back[0].2, 0.0);
        assert_eq!(back[0].4, 0.0);
    }

    #[test]
    fn adjacent_coordinates_have_zero_prediction_residuals() {
        let glyphs = (0..1000).map(|i| (0, i as f64 * 7.0, 15.0, 12.0, 7.0, i, i + 1, 0, false, None)).collect::<Vec<_>>();
        let old = STANDARD.decode(encode_version(&glyphs, false).unwrap()).unwrap();
        let new = unpack(&STANDARD.decode(encode(&glyphs).unwrap()).unwrap()).unwrap();
        let x_range = 4 + 16 * glyphs.len()..4 + 24 * glyphs.len();
        assert!(old[x_range.clone()].iter().any(|b| *b != 0));
        assert!(new[x_range].iter().all(|b| *b == 0));
        assert_eq!(decode(&encode(&glyphs).unwrap(), true).unwrap()[999].1, 6993.0);
    }

    #[test]
    fn exact_columns_cover_zero_records_flags_affines_and_truncation() {
        assert!(decode(&encode_version(&[], false).unwrap(), false).unwrap().is_empty());
        let input = vec![(0, 3.123456789012345, -0.0, 12.0, 4.5, 0, 1, 0, true, Some(Placement::Matrix([0.5, 0.2, -0.2, 0.5, 2.0, 3.0])))];
        let encoded = encode_version(&input, false).unwrap();
        let back = decode(&encoded, false).unwrap();
        assert_eq!(back[0].1.to_bits(), input[0].1.to_bits());
        assert_eq!(back[0].2.to_bits(), input[0].2.to_bits());
        assert!(back[0].8);
        assert!(matches!(back[0].9, Some(Placement::Matrix(m)) if m == [0.5, 0.2, -0.2, 0.5, 2.0, 3.0]));
        let raw = STANDARD.decode(encoded).unwrap();
        for end in 0..raw.len() {
            assert!(decode(&STANDARD.encode(&raw[..end]), false).is_err());
        }
        let mut corrupt = raw.clone();
        corrupt.push(0);
        assert!(decode(&STANDARD.encode(corrupt), false).is_err());
        let mut corrupt = raw.clone();
        corrupt[4 + 16 + 32] = 2;
        assert!(decode(&STANDARD.encode(corrupt), false).is_err());
        let mut nonfinite = raw.clone();
        nonfinite[20..28].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(decode(&STANDARD.encode(nonfinite), false).is_err());
        let mut corrupt = raw;
        corrupt[4 + 16 + 32 + 1] = 3;
        assert!(decode(&STANDARD.encode(corrupt), false).is_err());
        assert!(decode(&STANDARD.encode(u32::MAX.to_le_bytes()), false).is_err());
        assert!(decode("not base64", false).is_err());
    }
}
