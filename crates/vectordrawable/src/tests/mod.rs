//! Unit tests: documents built in code, the written XML parsed back (every test checks it's
//! well-formed) and compared by structure and numbers.

mod compat;
mod gradients;
mod shapes;
mod strokes;
mod xml_out;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeId, NodeKind};

use crate::{Compat, VdExport, VdOptions, try_export};

pub(super) fn hex(s: &str) -> Color {
    Color::from_hex(s).unwrap()
}

pub(super) fn doc() -> (Document, NodeId) {
    let d = Document::new("vd");
    let p = d.first_page().unwrap();
    (d, p)
}

pub(super) fn rect(d: &mut Document, parent: NodeId, name: &str, x: f64, y: f64, w: f64, h: f64, fill: &str) -> NodeId {
    let r = d.create(parent, NodeKind::Rectangle, name, x, y, w, h).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(hex(fill))];
    r
}

pub(super) fn ellipse() -> NodeKind {
    NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }
}

pub(super) fn export(d: &Document, ids: &[NodeId], compat: Compat) -> VdExport {
    export_with(d, ids, VdOptions { compat, ..Default::default() })
}

pub(super) fn export_with(d: &Document, ids: &[NodeId], opts: VdOptions) -> VdExport {
    let out = try_export(d, ids, &opts).unwrap();
    // Always well-formed, never a non-finite number.
    if let Err(e) = roxmltree::Document::parse(&out.xml) {
        panic!("not well-formed: {e}\n{}", out.xml);
    }
    for bad in ["NaN", "inf", "=\"-0\"", "e+", "e-"] {
        assert!(!out.xml.contains(bad), "{bad} in\n{}", out.xml);
    }
    out
}

/// An element as (tag name, attributes without namespace prefixes).
#[derive(Debug, Clone)]
pub(super) struct El {
    pub tag: String,
    pub attrs: std::collections::BTreeMap<String, String>,
}

impl El {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.attrs.get(k).map(String::as_str)
    }
}

/// Every element of `xml`, document order.
pub(super) fn elements(xml: &str) -> Vec<El> {
    let doc = roxmltree::Document::parse(xml).unwrap();
    doc.descendants()
        .filter(|n| n.is_element())
        .map(|n| El { tag: n.tag_name().name().to_string(), attrs: n.attributes().map(|a| (a.name().to_string(), a.value().to_string())).collect() })
        .collect()
}

pub(super) fn tagged<'a>(els: &'a [El], tag: &str) -> Vec<&'a El> {
    els.iter().filter(|e| e.tag == tag).collect()
}

/// The numbers of a path data string, in order.
pub(super) fn numbers(data: &str) -> Vec<f64> {
    data.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect()
}

/// The (x, y) bounding box of a path data string's points.
pub(super) fn bounds(data: &str) -> (f64, f64, f64, f64) {
    let n = numbers(data);
    let xs = n.iter().step_by(2);
    let ys = n.iter().skip(1).step_by(2);
    let (x0, x1) = xs.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let (y0, y1) = ys.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    (x0, y0, x1, y1)
}

pub(super) fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// The `<path>` elements of `xml`.
pub(super) fn paths_of(xml: &str) -> Vec<El> {
    elements(xml).into_iter().filter(|e| e.tag == "path").collect()
}
