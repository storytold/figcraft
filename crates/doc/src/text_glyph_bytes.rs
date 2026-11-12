//! Lossless byte-plane columns for portable glyph numbers. Separating equally significant
//! bytes makes repeated layouts compress without decimal float formatting or JSON number parsing.
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::text_appearance::{CompactGlyph, Placement};

const MAX_GLYPHS: usize = 100_000;
// Four u32 + four f64 columns, two flag bytes, and at most six affine f64s per record.
const MAX_RAW: usize = 4 + MAX_GLYPHS * 98;
pub(crate) const MAX_ENCODED: usize = MAX_RAW.div_ceil(3) * 4;

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
    if glyphs.len() > MAX_GLYPHS {
        return Err("too many portable glyphs");
    }
    let index = |v: usize| u32::try_from(v).map(u32::to_le_bytes).map_err(|_| "portable glyph index exceeds u32");
    let integers = glyphs.iter().map(|g| Ok([index(g.0)?, index(g.5)?, index(g.6)?, index(g.7)?])).collect::<Result<Vec<_>, &'static str>>()?;
    let count = u32::try_from(glyphs.len()).map_err(|_| "too many portable glyphs")?;
    let mut bytes = count.to_le_bytes().to_vec();
    for column in 0..4 {
        put_column(&mut bytes, integers.iter().filter_map(|v| v.get(column)).copied());
    }
    for column in 0..4 {
        put_column(
            &mut bytes,
            glyphs.iter().map(|g| {
                finite_bytes(match column {
                    0 => g.1,
                    1 => g.2,
                    2 => g.3,
                    _ => g.4,
                })
            }),
        );
    }
    bytes.extend(glyphs.iter().map(|g| u8::from(g.8)));
    bytes.extend(glyphs.iter().map(|g| match g.9 {
        None => 0,
        Some(Placement::Scale(_)) => 1,
        Some(Placement::Matrix(_)) => 2,
    }));
    for g in glyphs {
        match g.9 {
            None => {}
            Some(Placement::Scale(s)) => bytes.extend(finite_bytes(s)),
            Some(Placement::Matrix(m)) => {
                for value in m {
                    bytes.extend(finite_bytes(value));
                }
            }
        }
    }
    Ok(STANDARD.encode(bytes))
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
    fn column<const N: usize>(&mut self, count: usize) -> Result<Vec<[u8; N]>, &'static str> {
        let data = self.take(count.checked_mul(N).ok_or("portable glyph column overflow")?)?;
        let mut values = vec![[0; N]; count];
        if count == 0 {
            return Ok(values);
        }
        for (plane, column) in data.chunks_exact(count).enumerate() {
            for (row, value) in values.iter_mut().zip(column) {
                if let Some(slot) = row.get_mut(plane) {
                    *slot = *value;
                }
            }
        }
        Ok(values)
    }
    fn number(&mut self) -> Result<f64, &'static str> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| "invalid portable number")?;
        number(bytes)
    }
}

fn number(bytes: [u8; 8]) -> Result<f64, &'static str> {
    let value = f64::from_le_bytes(bytes);
    if value.is_finite() { Ok(value) } else { Err("non-finite portable number") }
}

fn value<const N: usize>(column: &[[u8; N]], row: usize) -> Result<[u8; N], &'static str> {
    column.get(row).copied().ok_or("missing portable column value")
}

pub(crate) fn decode(text: &str) -> Result<Vec<CompactGlyph>, &'static str> {
    if text.len() > MAX_ENCODED {
        return Err("portable glyph bytes exceed limit");
    }
    let bytes = STANDARD.decode(text).map_err(|_| "invalid base64 portable glyphs")?;
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
    for row in 0..count {
        let direction = match rtl.get(row) {
            Some(0) => false,
            Some(1) => true,
            _ => return Err("invalid portable direction flag"),
        };
        let placement = match tags.get(row) {
            Some(0) => None,
            Some(1) => Some(Placement::Scale(reader.number()?)),
            Some(2) => {
                Some(Placement::Matrix([reader.number()?, reader.number()?, reader.number()?, reader.number()?, reader.number()?, reader.number()?]))
            }
            _ => return Err("invalid portable placement flag"),
        };
        glyphs.push((
            u32::from_le_bytes(value(&indices, row)?) as usize,
            number(value(&x, row)?)?,
            number(value(&y, row)?)?,
            number(value(&size, row)?)?,
            number(value(&advance, row)?)?,
            u32::from_le_bytes(value(&starts, row)?) as usize,
            u32::from_le_bytes(value(&ends, row)?) as usize,
            u32::from_le_bytes(value(&styles, row)?) as usize,
            direction,
            placement,
        ));
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
    fn exact_columns_cover_zero_records_flags_affines_and_truncation() {
        assert!(decode(&encode(&[]).unwrap()).unwrap().is_empty());
        let input = vec![(0, 3.123456789012345, -0.0, 12.0, 4.5, 0, 1, 0, true, Some(Placement::Matrix([0.5, 0.2, -0.2, 0.5, 2.0, 3.0])))];
        let encoded = encode(&input).unwrap();
        let back = decode(&encoded).unwrap();
        assert_eq!(back[0].1.to_bits(), input[0].1.to_bits());
        assert_eq!(back[0].2.to_bits(), input[0].2.to_bits());
        assert!(back[0].8);
        assert!(matches!(back[0].9, Some(Placement::Matrix(m)) if m == [0.5, 0.2, -0.2, 0.5, 2.0, 3.0]));
        let raw = STANDARD.decode(encoded).unwrap();
        for end in 0..raw.len() {
            assert!(decode(&STANDARD.encode(&raw[..end])).is_err());
        }
        let mut corrupt = raw.clone();
        corrupt.push(0);
        assert!(decode(&STANDARD.encode(corrupt)).is_err());
        let mut corrupt = raw.clone();
        corrupt[4 + 16 + 32] = 2;
        assert!(decode(&STANDARD.encode(corrupt)).is_err());
        let mut nonfinite = raw.clone();
        nonfinite[20..28].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(decode(&STANDARD.encode(nonfinite)).is_err());
        let mut corrupt = raw;
        corrupt[4 + 16 + 32 + 1] = 3;
        assert!(decode(&STANDARD.encode(corrupt)).is_err());
        assert!(decode(&STANDARD.encode(u32::MAX.to_le_bytes())).is_err());
        assert!(decode("not base64").is_err());
    }
}
