//! FigCraft's [`DocumentConverter`]: the printed sheets as PWG raster, for a printer that lists
//! raster but no PDF.
//!
//! The registry decides *whether* to convert (the printer does not take the document's format, a
//! print-ready PDF) and *at what resolution* (the job's own, else the printer's listed one nearest
//! 300 dpi); this is the *how*. FigCraft has no PDF reader: the PDF was drawn from the page's frames,
//! so the converter takes the same frames ([`PrintSheet`]: which frame, how big its sheet is, where
//! the frame lands on it) and renders them with FigCraft's own renderer, onto white sheets, with
//! craft-print's neutral [`pwg`] encoder. It makes nothing but PWG raster, from nothing but PDF.

use std::sync::Arc;

use figcraft_doc::Document as Doc;
use figcraft_engine::cmd::file::PrintSheet;

use super::{ConvertContext, Document, DocumentConverter, DocumentFormat, Duplex, ErrorKind, OwnedDocument, PrintError, pwg};

/// The most pixels all the sheets of one job may hold (3 bytes each in colour): a job larger than
/// this at the chosen resolution is refused rather than exhausting memory.
const MAX_PIXELS: u64 = 400_000_000;

fn failed(why: impl Into<String>) -> PrintError {
    PrintError::new(ErrorKind::Failed, why)
}

/// The frames of one print, as raster. Made per print, from the document the PDF was made from.
pub struct FramesToRaster {
    doc: Arc<Doc>,
    sheets: Vec<PrintSheet>,
}

impl FramesToRaster {
    /// `doc` must be in sRGB (the PDF's colours are), `sheets` the print's sheets in order.
    pub fn new(doc: Arc<Doc>, sheets: Vec<PrintSheet>) -> Self {
        Self { doc, sheets }
    }
}

impl DocumentConverter for FramesToRaster {
    fn targets(&self, from: DocumentFormat) -> Vec<DocumentFormat> {
        if from == DocumentFormat::Pdf { vec![DocumentFormat::PwgRaster] } else { Vec::new() }
    }

    fn convert(&self, document: Document<'_>, target: DocumentFormat, context: &ConvertContext) -> Result<OwnedDocument, PrintError> {
        if document.format != DocumentFormat::Pdf || target != DocumentFormat::PwgRaster {
            return Err(PrintError::new(
                ErrorKind::UnsupportedFormat,
                format!("this converter makes PWG raster from the print-ready PDF, not {} from {}", target.mime(), document.format.mime()),
            ));
        }
        if self.sheets.is_empty() {
            return Err(failed("the print has no sheets"));
        }
        let (dx, dy) = (context.resolution.x_dpi.max(1), context.resolution.y_dpi.max(1));
        let top = dx.max(dy);
        let gray = !context.colour;
        let bpp = if gray { 1 } else { 3 };
        let total: u64 = self.sheets.iter().map(|s| pixels(s.sheet, top)).sum();
        if total > MAX_PIXELS {
            return Err(failed(format!("the sheets are too large to render at {top} dpi: lower the resolution")));
        }
        let mut pages = Vec::with_capacity(self.sheets.len());
        for (index, sheet) in self.sheets.iter().enumerate() {
            if context.cancel.is_cancelled() {
                return Err(PrintError::new(ErrorKind::Cancelled, "the print job was cancelled while the document was rendered"));
            }
            let (pw, ph, pixels) =
                render_sheet(&self.doc, sheet, top, gray).ok_or_else(|| failed(format!("sheet {} could not be rendered", index + 1)))?;
            let target = |points: f64, dpi: u32| ((points * f64::from(dpi) / 72.0).ceil() as u32).max(1);
            pages.push(if dx == dy {
                pwg::RasterPage { width: pw, height: ph, gray, pixels }
            } else {
                let (tw, th) = (target(sheet.sheet.0, dx), target(sheet.sheet.1, dy));
                pwg::RasterPage { width: tw, height: th, gray, pixels: resample(&pixels, bpp, (pw, ph), (tw, th)) }
            });
        }
        let duplex = match context.duplex {
            None | Some(Duplex::Off) => pwg::Duplex::Off,
            Some(Duplex::LongEdge) => pwg::Duplex::LongEdge,
            Some(Duplex::ShortEdge) => pwg::Duplex::ShortEdge,
        };
        let raster = pwg::encode(&pages, &pwg::PwgOptions { dpi: (dx, dy), duplex }).map_err(|e| failed(e.to_string()))?;
        Ok(OwnedDocument::new(DocumentFormat::PwgRaster, raster))
    }
}

/// The pixel count of a `sheet` (points) at `dpi`.
fn pixels(sheet: (f64, f64), dpi: u32) -> u64 {
    let side = |points: f64| (points * f64::from(dpi) / 72.0).ceil().clamp(1.0, 1.0e6) as u64;
    side(sheet.0) * side(sheet.1)
}

/// One sheet at `dpi` on both axes: its frame rendered at the sheet's scale and laid on white, as
/// 8-bit grey or RGB. `None` when the renderer refuses the size.
fn render_sheet(doc: &Doc, sheet: &PrintSheet, dpi: u32, gray: bool) -> Option<(u32, u32, Vec<u8>)> {
    let per_point = f64::from(dpi) / 72.0;
    let side = |points: f64| (points * per_point).ceil().clamp(1.0, 16_384.0) as usize;
    let (w, h) = (side(sheet.sheet.0), side(sheet.sheet.1));
    let bpp = if gray { 1 } else { 3 };
    let mut canvas = vec![255u8; w.checked_mul(h)?.checked_mul(bpp)?];
    let pix = figcraft_render::render_area(doc, &[sheet.frame], sheet.bounds, sheet.scale * per_point)?;
    let (px, py) = ((sheet.offset.0 * per_point).round() as i64, (sheet.offset.1 * per_point).round() as i64);
    let (pw, ph) = (pix.width() as usize, pix.height() as usize);
    let data = pix.data();
    for y in 0..ph {
        let cy = y as i64 + py;
        if cy < 0 || cy >= h as i64 {
            continue;
        }
        for x in 0..pw {
            let cx = x as i64 + px;
            if cx < 0 || cx >= w as i64 {
                continue;
            }
            let Some(src) = data.get((y * pw + x) * 4..(y * pw + x) * 4 + 4) else { continue };
            let a = u16::from(src[3]);
            // Premultiplied colour over white.
            let over = |c: u8| (u16::from(c) + (255 - a)).min(255) as u8;
            let (r, g, b) = (over(src[0]), over(src[1]), over(src[2]));
            let at = (cy as usize * w + cx as usize) * bpp;
            let Some(dst) = canvas.get_mut(at..at + bpp) else { continue };
            if gray {
                dst[0] = figcraft_pdf::print::gray_of(r, g, b);
            } else {
                dst.copy_from_slice(&[r, g, b]);
            }
        }
    }
    Some((w as u32, h as u32, canvas))
}

/// Nearest-neighbour sampling of an interleaved 8-bit picture.
fn resample(pixels: &[u8], bpp: usize, (sw, sh): (u32, u32), (tw, th): (u32, u32)) -> Vec<u8> {
    let mut out = Vec::with_capacity(tw as usize * th as usize * bpp);
    for y in 0..th {
        let sy = (u64::from(y) * u64::from(sh) / u64::from(th)).min(u64::from(sh) - 1) as usize;
        for x in 0..tw {
            let sx = (u64::from(x) * u64::from(sw) / u64::from(tw)).min(u64::from(sw) - 1) as usize;
            let at = (sy * sw as usize + sx) * bpp;
            out.extend_from_slice(pixels.get(at..at + bpp).unwrap_or(&[]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_color::Color;
    use figcraft_doc::{NodeId, NodeKind};
    use figcraft_engine::cmd::file::{print_plan, printable_frames};
    use figcraft_pdf::{Fit, FitOrientation, FitScale};

    fn red_frame() -> (Doc, NodeId) {
        let mut d = Doc::new("raster");
        let p = d.first_page().unwrap();
        let f = d.create(p, NodeKind::Frame, "Poster", 0.0, 0.0, 200.0, 100.0).unwrap();
        d.get_mut(f).unwrap().fills = vec![figcraft_color::Paint::solid(Color::from_hex("FF0000").unwrap())];
        (d, f)
    }

    #[test]
    fn resampling_keeps_rows_and_columns_in_place() {
        // 2x2 grey picture to 4x1: the columns double, the rows are sampled down.
        assert_eq!(resample(&[1, 2, 3, 4], 1, (2, 2), (4, 1)), [1, 1, 2, 2]);
        assert_eq!(resample(&[1, 2, 3, 4], 1, (2, 2), (1, 2)), [1, 3]);
    }

    #[test]
    fn it_converts_the_print_pdf_and_nothing_else() {
        let (d, f) = red_frame();
        let page = d.first_page().unwrap();
        let fit = Fit { paper: (595.28, 841.89), orientation: FitOrientation::Portrait, scale: FitScale::Fill, margin: 36.0 };
        let plan = print_plan(&d, &printable_frames(&d, page, None), Some(fit), false).unwrap();
        assert_eq!(plan.sheets[0].frame, f);
        let converter = FramesToRaster::new(Arc::new(d), plan.sheets.clone());
        assert_eq!(converter.targets(DocumentFormat::Pdf), [DocumentFormat::PwgRaster]);
        assert!(converter.targets(DocumentFormat::Jpeg).is_empty());
        let ctx = ConvertContext::new(super::super::Resolution::square(72), true);
        let e = converter.convert(Document::new(DocumentFormat::Jpeg, &[1]), DocumentFormat::PwgRaster, &ctx).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::UnsupportedFormat);
        let out = converter.convert(Document::new(DocumentFormat::Pdf, &plan.pdf), DocumentFormat::PwgRaster, &ctx).unwrap();
        assert_eq!(out.format, DocumentFormat::PwgRaster);
        assert!(out.bytes.starts_with(b"RaS2"), "a PWG raster stream");
    }

    #[test]
    fn a_sheet_is_white_with_the_frame_where_the_plan_puts_it() {
        let (d, f) = red_frame();
        let page = d.first_page().unwrap();
        let fit = Fit { paper: (200.0, 300.0), orientation: FitOrientation::Portrait, scale: FitScale::Shrink, margin: 0.0 };
        let plan = print_plan(&d, &printable_frames(&d, page, None), Some(fit), false).unwrap();
        let sheet = &plan.sheets[0];
        assert_eq!(sheet.frame, f);
        // 200 x 100 on 200 x 300: centred vertically at 100 pt from the top.
        assert_eq!(sheet.offset, (0.0, 100.0));
        let (w, h, px) = render_sheet(&d, sheet, 72, false).unwrap();
        assert_eq!((w, h), (200, 300));
        let at = |x: usize, y: usize| px[(y * w as usize + x) * 3..][..3].to_vec();
        assert_eq!(at(10, 10), [255, 255, 255], "above the frame: paper");
        assert_eq!(at(100, 150), [255, 0, 0], "in the frame");
        assert_eq!(at(100, 250), [255, 255, 255], "below the frame: paper");
        let (_, _, grey) = render_sheet(&d, sheet, 72, true).unwrap();
        assert_eq!(grey.len(), 200 * 300);
        assert_eq!(grey[150 * 200 + 100], 53, "red as Rec. 709 luma");
    }

    #[test]
    fn a_job_too_large_to_render_is_refused() {
        let (d, _) = red_frame();
        let sheet = PrintSheet {
            frame: NodeId(1),
            bounds: figcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0),
            sheet: (5000.0, 5000.0),
            scale: 1.0,
            offset: (0.0, 0.0),
        };
        let converter = FramesToRaster::new(Arc::new(d), vec![sheet]);
        let ctx = ConvertContext::new(super::super::Resolution::square(1200), true);
        let e = converter.convert(Document::new(DocumentFormat::Pdf, b"%PDF"), DocumentFormat::PwgRaster, &ctx).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Failed);
    }
}
