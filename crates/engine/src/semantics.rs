//! What a layer is on a web page, for the HTML, React and Leptos code: its element and the
//! attributes assistive technology and links need, read from what the designer already set:
//!
//! - **Prototype interactions.** A click that opens a URL is a link (`<a href>`); one that
//!   navigates or scrolls to a frame is a link to that frame's anchor (`#frame-name`); any other
//!   click action is a `<button type="button">`.
//! - **Accessibility annotations** (Dev Mode annotations in the Accessibility category), one
//!   instruction per line: an element name (`h1`…`h6`, `nav`, `header`, `footer`, `main`,
//!   `section`, `article`, `aside`, `ul`, `ol`, `li`, `button`, `a`, `label`, `span`, `p`), or
//!   `heading 2` / `heading level 2`, `alt: …` (an image's text), `label: …` (`aria-label`),
//!   `href: …`, `decorative` (hidden from assistive technology).
//! - **Image fills.** A layer whose fill is an image is `role="img"` with its alt text (the
//!   annotation's, else the layer name), unless marked decorative.
//!
//! URLs other than http(s), mailto, tel, relative paths and anchors are dropped, so a document
//! can't put `javascript:` into generated code.

use figcraft_color::PaintKind;
use figcraft_doc::*;

/// The id of the built-in Accessibility annotation category.
const A11Y: &str = "accessibility";

const TAGS: [&str; 20] = [
    "h1", "h2", "h3", "h4", "h5", "h6", "nav", "header", "footer", "main", "section", "article", "aside", "ul", "ol", "li", "button", "a", "label",
    "span",
];

/// A layer's element and attributes on a web page (all `None`/false: a plain `div` or `p`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Semantics {
    /// The element, when it isn't the default `div` (frames) or `p` (text).
    pub tag: Option<&'static str>,
    pub href: Option<String>,
    /// An image's text (`role="img"` + `aria-label`).
    pub alt: Option<String>,
    /// `aria-label`.
    pub label: Option<String>,
    /// Hidden from assistive technology (`aria-hidden="true"`).
    pub decorative: bool,
}

impl Semantics {
    /// The element for a layer that is text (`text`) or not.
    pub fn element(&self, text: bool) -> &'static str {
        self.tag.unwrap_or(if text { "p" } else { "div" })
    }

    /// Whether the element only takes phrasing content (its text children become `span`).
    pub fn phrasing(&self) -> bool {
        matches!(self.tag, Some("button" | "label" | "span" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"))
    }
}

/// `url` when it is safe to put in an `href`: http(s), mailto, tel, a relative path or an anchor.
pub fn safe_url(url: &str) -> Option<String> {
    let u = url.trim();
    if u.is_empty() || u.len() > 2_000 || u.chars().any(|c| c.is_control()) {
        return None;
    }
    let lower = u.to_ascii_lowercase();
    let scheme = lower.split_once(':').map(|(s, _)| s).filter(|s| !s.contains(['/', '?', '#']));
    match scheme {
        None => Some(u.to_string()),
        Some("http" | "https" | "mailto" | "tel") => Some(u.to_string()),
        Some(_) => None,
    }
}

/// An anchor id from a frame name ("Pricing section" → `pricing-section`).
pub fn anchor(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase).take(60) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "section".into() } else { out }
}

/// One annotation line applied to `s`.
fn apply_line(s: &mut Semantics, line: &str) {
    let line = line.trim();
    let lower = line.to_ascii_lowercase();
    if let Some((key, value)) = line.split_once(':') {
        let value = value.trim().to_string();
        match key.trim().to_ascii_lowercase().as_str() {
            "alt" => s.alt = Some(value),
            "label" | "aria-label" => s.label = Some(value).filter(|v| !v.is_empty()),
            "href" | "link" => s.href = safe_url(&value),
            "tag" | "element" | "role" => {
                let v = value.to_ascii_lowercase();
                s.tag = TAGS.iter().find(|t| **t == v).copied().or(s.tag);
            }
            _ => {}
        }
        return;
    }
    if lower == "decorative" {
        s.decorative = true;
        return;
    }
    // "heading 2", "heading level 2", "h2"
    let level = lower.strip_prefix("heading").map(|r| r.trim().trim_start_matches("level").trim()).and_then(|d| d.parse::<u8>().ok());
    if let Some(l @ 1..=6) = level {
        s.tag = TAGS.get(usize::from(l) - 1).copied();
        return;
    }
    if let Some(t) = TAGS.iter().find(|t| **t == lower) {
        s.tag = Some(t);
    }
}

/// The layer's semantics from its Accessibility annotations, click interactions and image fill.
pub fn of(doc: &Document, n: &Node) -> Semantics {
    let mut s = Semantics::default();
    for a in n.annotations.iter().filter(|a| a.category.as_deref() == Some(A11Y)) {
        for line in a.label.lines().take(20) {
            apply_line(&mut s, line);
        }
    }
    if s.href.is_none() {
        let clicks = n.interactions.iter().filter(|i| matches!(i.trigger, figcraft_doc::proto::Trigger::OnClick));
        for action in clicks.flat_map(|i| i.actions.iter()) {
            use figcraft_doc::proto::Action;
            let link = match action {
                Action::OpenUrl { url } => safe_url(url),
                Action::Navigate { dest, .. } | Action::ScrollTo { dest, .. } => doc.node(*dest).map(|d| format!("#{}", anchor(&d.name))),
                _ => None,
            };
            match link {
                Some(href) => {
                    s.href = Some(href);
                    break;
                }
                // Another click action: a button, unless the annotation said otherwise.
                None => s.tag = s.tag.or(Some("button")),
            }
        }
    }
    if s.href.is_some() && !matches!(s.tag, Some("a")) {
        s.tag = Some("a");
    }
    let image = n.fills.iter().any(|p| p.visible && matches!(p.kind, PaintKind::Image(_)));
    if image && !s.decorative && s.alt.is_none() {
        s.alt = Some(n.name.trim().to_string());
    }
    s
}

/// The text that is a page's `h1` when nothing marks one: the largest text (at least 24px) in
/// the tree under `root`, first in layer order on ties. `None` when an annotation or the page
/// already names a heading level 1.
pub fn page_h1(doc: &Document, root: NodeId) -> Option<NodeId> {
    let mut best: Option<(f64, NodeId)> = None;
    for id in doc.subtree(root).into_iter().take(20_000) {
        let Some(n) = doc.node(id) else { continue };
        if of(doc, n).tag == Some("h1") {
            return None;
        }
        if let NodeKind::Text(t) = &n.kind
            && n.visible
            && t.style.font_size >= 24.0
            && best.is_none_or(|(size, _)| t.style.font_size > size)
        {
            best = Some((t.style.font_size, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Page code's guesses for a layer nothing marked: the page's `h1`, headings and landmarks by
/// layer name ("Title", "Nav", "Footer"…), and buttons by name ("Button", "CTA").
// ponytail: English and Portuguese names only; annotations cover everything else.
pub fn infer(n: &Node, is_h1: bool, s: &mut Semantics) {
    if s.tag.is_some() || s.href.is_some() {
        return;
    }
    let name = n.name.to_lowercase();
    let word = |w: &str| name.split(|c: char| !c.is_alphanumeric()).any(|p| p == w);
    let text = matches!(n.kind, NodeKind::Text(_));
    s.tag = if is_h1 {
        Some("h1")
    } else if text && ["title", "heading", "headline", "título", "titulo"].iter().any(|w| word(w)) {
        Some("h2")
    } else if text {
        None
    } else if ["nav", "navbar", "menu", "navegação"].iter().any(|w| word(w)) {
        Some("nav")
    } else if word("header") || word("cabeçalho") {
        Some("header")
    } else if word("footer") || word("rodapé") || word("rodape") {
        Some("footer")
    } else if ["button", "btn", "cta", "botão", "botao"].iter().any(|w| word(w)) && !n.children.is_empty() {
        Some("button")
    } else {
        None
    };
}

/// The layers click interactions navigate or scroll to: their code carries `id="<anchor>"`, so
/// the links [`of`] writes (`#frame-name`) land on them.
pub fn link_targets(doc: &Document) -> Vec<NodeId> {
    use figcraft_doc::proto::Action;
    let mut out: Vec<NodeId> = doc
        .nodes()
        .flat_map(|n| n.interactions.iter())
        .filter(|i| matches!(i.trigger, figcraft_doc::proto::Trigger::OnClick))
        .flat_map(|i| i.actions.iter())
        .filter_map(|a| match a {
            Action::Navigate { dest, .. } | Action::ScrollTo { dest, .. } => Some(*dest),
            _ => None,
        })
        .take(10_000)
        .collect();
    out.sort_by_key(|id| id.0);
    out.dedup();
    out
}
