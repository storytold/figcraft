//! Semantic web code: prototype clicks become links and buttons, Accessibility annotations set
//! elements and labels, image fills get their text, and unsafe URLs never reach the code.

use crate::Session;
use crate::components::tests::{ex, idv};
use crate::semantics::{anchor, safe_url};
use figcraft_doc::*;
use serde_json::json;

fn code(s: &mut Session, id: NodeId, lang: &str) -> String {
    ex(s, "inspect.code", json!({"id": id, "language": lang}))["code"].as_str().unwrap_or_default().to_string()
}

/// A "Page" frame with a "Title" text, a "Buy" frame holding a "Label" text, and a "Pricing"
/// frame. Returns (page, title, buy, pricing).
fn page(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let p = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 800, "height": 600, "name": "Page"})));
    let title = idv(&ex(s, "text.create", json!({"text": "Seu site em 7 dias", "x": 0, "y": 0, "parent": p, "props": {"name": "Title"}})));
    let buy = idv(&ex(s, "frame.create", json!({"x": 0, "y": 100, "width": 200, "height": 48, "parent": p, "name": "Buy"})));
    ex(s, "text.create", json!({"text": "Agendar", "x": 0, "y": 0, "parent": buy, "props": {"name": "Label"}}));
    let pricing = idv(&ex(s, "frame.create", json!({"x": 0, "y": 300, "width": 800, "height": 200, "parent": p, "name": "Pricing section"})));
    (p, title, buy, pricing)
}

#[test]
fn prototype_clicks_become_links_and_buttons() {
    let mut s = Session::new();
    let (p, _, buy, pricing) = page(&mut s);
    ex(&mut s, "proto.interaction.add", json!({"id": buy, "action": {"type": "openUrl", "url": "https://wa.me/5511999999999"}}));
    let html = code(&mut s, p, "html");
    assert!(html.contains("<a class=\"buy\" href=\"https://wa.me/5511999999999\">"), "{html}");
    assert!(html.contains("</a>"), "{html}");
    // Navigating to a frame links to its anchor.
    ex(&mut s, "proto.interaction.remove", json!({"id": buy, "index": 0}));
    ex(&mut s, "proto.interaction.add", json!({"id": buy, "action": {"type": "scrollTo", "dest": pricing}}));
    let html = code(&mut s, p, "html");
    assert!(html.contains("href=\"#pricing-section\""), "{html}");
    assert!(html.contains("<div class=\"pricing-section\" id=\"pricing-section\">"), "the target carries the anchor: {html}");
    // Any other click is a button, and its text is phrasing content.
    ex(&mut s, "proto.interaction.remove", json!({"id": buy, "index": 0}));
    ex(&mut s, "proto.interaction.add", json!({"id": buy, "action": {"type": "back"}}));
    let react = code(&mut s, p, "react");
    assert!(react.contains("<button className=\"buy\" type=\"button\">"), "{react}");
    assert!(react.contains("<span className=\"label\">Agendar</span>") && react.contains("</button>"), "{react}");
}

#[test]
fn accessibility_annotations_set_elements_and_labels() {
    let mut s = Session::new();
    let (p, title, buy, pricing) = page(&mut s);
    ex(&mut s, "annotation.add", json!({"id": title, "label": "heading level 1", "category": "accessibility"}));
    ex(&mut s, "annotation.add", json!({"id": pricing, "label": "section\nlabel: Investimento", "category": "accessibility"}));
    ex(&mut s, "annotation.add", json!({"id": buy, "label": "href: /agendar", "category": "accessibility"}));
    // Annotations in other categories don't change the code.
    ex(&mut s, "annotation.add", json!({"id": p, "label": "nav", "category": "development"}));
    let html = code(&mut s, p, "html");
    assert!(html.contains("<h1 class=\"title\">Seu site em 7 dias</h1>"), "{html}");
    assert!(html.contains("<section class=\"pricing-section\" aria-label=\"Investimento\">"), "{html}");
    assert!(html.contains("<a class=\"buy\" href=\"/agendar\">"), "{html}");
    assert!(html.contains("<div class=\"page\""), "{html}");
    let leptos = code(&mut s, p, "leptos");
    assert!(leptos.contains("<h1 class=\"title\">") && leptos.contains("href=\"/agendar\""), "{leptos}");
}

#[test]
fn image_fills_get_their_text_unless_decorative() {
    let mut s = Session::new();
    let (p, _, _, pricing) = page(&mut s);
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let photo = idv(&ex(&mut s, "image.place", json!({"data": png, "x": 0, "y": 0, "parent": pricing})));
    ex(&mut s, "node.set", json!({"id": photo, "props": {"name": "Fachada da clínica"}}));
    let react = code(&mut s, p, "react");
    assert!(react.contains("role=\"img\" aria-label={\"Fachada da clínica\"}"), "{react}");
    ex(&mut s, "annotation.add", json!({"id": photo, "label": "alt: Recepção \"Aurora\" & sala", "category": "accessibility"}));
    let html = code(&mut s, p, "html");
    assert!(html.contains("aria-label=\"Recepção &quot;Aurora&quot; &amp; sala\""), "escaped: {html}");
    ex(&mut s, "annotation.add", json!({"id": photo, "label": "decorative", "category": "accessibility"}));
    let html = code(&mut s, p, "html");
    assert!(html.contains("aria-hidden=\"true\"") && !html.contains("role=\"img\""), "{html}");
}

#[test]
fn unsafe_urls_never_reach_the_code() {
    for bad in ["javascript:alert(1)", " JavaScript:alert(1)", "data:text/html,x", "vbscript:x", "java\nscript:x", ""] {
        assert_eq!(safe_url(bad), None, "{bad:?}");
    }
    for good in ["https://lookadev.com", "http://x.y/a?b=c#d", "mailto:oi@x.com", "tel:+5511999999999", "/agendar", "#precos", "precos.html", "a/b:c"]
    {
        assert!(safe_url(good).is_some(), "{good:?}");
    }
    let mut s = Session::new();
    let (p, _, buy, _) = page(&mut s);
    ex(&mut s, "proto.interaction.add", json!({"id": buy, "action": {"type": "openUrl", "url": "javascript:alert(document.cookie)"}}));
    let react = code(&mut s, p, "react");
    assert!(!react.contains("javascript"), "{react}");
    assert_eq!(anchor("Preços & planos"), "preços-planos");
    assert_eq!(anchor("!!!"), "section");
}

#[test]
fn stylesheets_reset_browser_defaults_the_design_doesnt_have() {
    let mut s = Session::new();
    let (p, title, buy, _) = page(&mut s);
    ex(&mut s, "annotation.add", json!({"id": title, "label": "h1", "category": "accessibility"}));
    ex(&mut s, "proto.interaction.add", json!({"id": buy, "action": {"type": "openUrl", "url": "/agendar"}}));
    let html = code(&mut s, p, "html");
    let rule = |class: &str| html.split(&format!(".{class} {{\n")).nth(1).and_then(|r| r.split('}').next()).unwrap_or_default().to_string();
    // Boxes measure like the canvas: width includes padding and strokes.
    assert!(rule("page").contains("box-sizing: border-box;"), "{html}");
    assert!(rule("title").contains("margin: 0;"), "headings have no browser margins: {html}");
    assert!(rule("label").contains("margin: 0;"), "nor does other text: {html}");
    let link = rule("buy");
    assert!(link.contains("color: inherit;") && link.contains("text-decoration: none;"), "{html}");
    // The reset comes before the layer's own look, so its declarations win.
    assert!(link.find("text-decoration").unwrap_or(usize::MAX) < link.find("width:").unwrap_or(0), "{link}");
    ex(&mut s, "proto.interaction.remove", json!({"id": buy, "index": 0}));
    ex(&mut s, "proto.interaction.add", json!({"id": buy, "action": {"type": "back"}}));
    let html = code(&mut s, p, "html");
    let button = html.split(".buy {\n").nth(1).and_then(|r| r.split('}').next()).unwrap_or_default();
    assert!(button.contains("border: none;") && button.contains("background: none;") && button.contains("font: inherit;"), "{html}");
}
