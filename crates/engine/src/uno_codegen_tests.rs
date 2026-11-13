//! Uno C# Markup code: `inspect.code {language: "uno"}` writes a fluent C# expression tree of
//! WinUI elements for a layer and its subtree.

use crate::Session;
use crate::codegen::{CodeLang, asset_name, cs_ident, cs_literal, cs_unique};
use crate::components::tests::{ex, idv};
use figcraft_doc::*;
use serde_json::json;
use std::collections::BTreeSet;

fn uno(s: &mut Session, id: NodeId) -> String {
    ex(s, "inspect.code", json!({"id": id, "language": "uno"}))["code"].as_str().unwrap_or_default().to_string()
}

#[test]
fn uno_is_a_code_language() {
    assert!(CodeLang::ALL.contains(&CodeLang::Uno));
    assert_eq!(CodeLang::Uno.id(), "uno");
    assert_eq!(CodeLang::Uno.label(), "Uno C# Markup");
    for alias in ["uno", "UNO", "csharp-markup", "cs-markup", "Uno C# Markup"] {
        assert_eq!(CodeLang::from_id(alias), Some(CodeLang::Uno), "{alias}");
    }
    assert!(!CodeLang::Uno.is_web() && !CodeLang::Uno.escapes_dollar());
    // Not connectable to code until components are mapped.
    assert!(crate::code_connect::language("uno").is_none());
}

#[test]
fn inspect_code_answers_in_uno() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 50, "name": "Box"})));
    let r = ex(&mut s, "inspect.code", json!({"id": f, "language": "uno"}));
    assert_eq!(r["language"], "uno");
    assert!(!uno(&mut s, f).is_empty());
}

fn frame(s: &mut Session, name: &str, w: f64, h: f64) -> NodeId {
    idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": w, "height": h, "name": name})))
}

fn set(s: &mut Session, id: NodeId, props: serde_json::Value) {
    ex(s, "node.set", json!({"id": id, "props": props}));
}

/// The statements of the generated constructor, as written (eight spaces of indent): what sits
/// between `public XView()` `{` and its closing brace.
fn tree(code: &str) -> String {
    let start = code.find("    {\n").map_or(0, |i| i + "    {\n".len());
    let end = code.rfind("    }\n}").unwrap_or(code.len());
    code.get(start..end).unwrap_or_default().to_string()
}

/// [`tree`] with the indent taken off.
fn body(code: &str) -> String {
    tree(code).lines().map(|l| format!("{}\n", l.get(8..).unwrap_or(l))).collect::<String>().trim_end().to_string()
}

/// Whether `block` (lines, indent and a closing `;` ignored) appears in `code` as consecutive lines.
fn has_block(code: &str, block: &str) -> bool {
    let norm = |t: &str| t.lines().map(|l| l.trim().trim_end_matches(';').to_string()).collect::<Vec<_>>();
    let (c, b) = (norm(code), norm(block));
    !b.is_empty() && c.windows(b.len()).any(|w| w == b.as_slice())
}

/// Whether one line of `code` is exactly `call` (a closing `;` ignored).
fn has_call(code: &str, call: &str) -> bool {
    has_block(code, call)
}

#[test]
fn an_empty_frame_is_a_view_class_with_a_sized_border() {
    let mut s = Session::new();
    let f = frame(&mut s, "Empty", 100.0, 50.0);
    let expected = "using Windows.UI;

// Empty
namespace MyApp;

internal sealed partial class EmptyView : UserControl
{
    public EmptyView()
    {
        var empty = new Border()
            .Width(100)
            .Height(50)
            .Background(Color.FromArgb(0xFF, 0xFF, 0xFF, 0xFF));
        Content = empty;
    }
}
";
    assert_eq!(uno(&mut s, f), expected);
    // Deterministic: the same document gives the same code.
    assert_eq!(uno(&mut s, f), uno(&mut s, f));
}

#[test]
fn a_frame_with_a_stroke_and_children_keeps_the_children_out_of_the_border() {
    let mut s = Session::new();
    let f = frame(&mut s, "Login Screen", 200.0, 100.0);
    set(&mut s, f, json!({"fill": "#112233", "stroke": "#FF0000", "strokeWeight": 3, "opacity": 0.5, "cornerRadius": 8}));
    ex(&mut s, "shape.rectangle", json!({"x": 5, "y": 5, "width": 20, "height": 20, "parent": f, "props": {"name": "Box"}}));
    let expected = "// warning: 1 frame has no auto layout: its children are positioned absolutely and do not adapt to size.
var loginScreen = new Grid()
    .Width(200)
    .Height(100)
    .Opacity(0.5);
var loginScreenBorder = new Border()
    .Background(Color.FromArgb(0xFF, 0x11, 0x22, 0x33))
    .BorderBrush(Color.FromArgb(0xFF, 0xFF, 0x00, 0x00))
    .BorderThickness(3)
    .CornerRadius(8);
loginScreen.Children.Add(loginScreenBorder);
var loginScreenCanvas = new Canvas();
var box = new Border()
    .Canvas(left: 5, top: 5)
    .Width(20)
    .Height(20)
    .Background(Color.FromArgb(0xFF, 0xD9, 0xD9, 0xD9));
loginScreenCanvas.Children.Add(box);
loginScreen.Children.Add(loginScreenCanvas);
Content = loginScreen;";
    let code = uno(&mut s, f);
    assert!(code.contains("internal sealed partial class LoginScreenView : UserControl"), "{code}");
    assert_eq!(body(&code), expected, "{code}");
}

#[test]
fn a_box_maps_its_stroke_sides_corners_opacity_rotation_and_what_it_cannot() {
    let mut s = Session::new();
    let f = frame(&mut s, "Frame", 120.0, 80.0);
    let r = idv(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 30, "parent": f, "props": {"name": "Box"}})));
    set(&mut s, r, json!({"fill": "#336699", "topLeftRadius": 4, "topRightRadius": 8, "bottomRightRadius": 12, "bottomLeftRadius": 16}));
    let code = uno(&mut s, f);
    assert!(code.contains(".CornerRadius(new CornerRadius(4, 8, 12, 16))"), "{code}");
    // Per-side stroke weights are top, right, bottom, left; a Thickness takes left first.
    set(&mut s, r, json!({"stroke": "#000000", "strokeWeight": 1}));
    set(&mut s, r, json!({"strokeTopWeight": 1, "strokeRightWeight": 2, "strokeBottomWeight": 3, "strokeLeftWeight": 4}));
    let code = uno(&mut s, f);
    assert!(code.contains(".BorderThickness(new Thickness(4, 1, 2, 3))"), "{code}");
    set(&mut s, r, json!({"opacity": 0.25, "rotation": 90, "strokeAlign": "outside"}));
    let code = uno(&mut s, f);
    assert!(code.contains(".Opacity(0.25)"), "{code}");
    assert!(code.contains(".RenderTransform(new RotateTransform().Angle(-90))"), "{code}");
    assert!(code.contains("// not mapped: stroke outside or centred"), "{code}");
}

#[test]
fn a_shadow_is_left_as_a_not_mapped_comment() {
    let mut s = Session::new();
    let f = frame(&mut s, "Fx", 100.0, 100.0);
    set(&mut s, f, json!({"effects": [{"type": "dropShadow", "color": "#00000040", "offset": [0, 4], "radius": 8}]}));
    let code = uno(&mut s, f);
    assert!(code.contains("// not mapped: drop shadow (needs Uno.Toolkit ShadowContainer)"), "{code}");
}

#[test]
fn an_ellipse_is_an_ellipse_with_a_centred_stroke() {
    let mut s = Session::new();
    let f = frame(&mut s, "Dots", 100.0, 100.0);
    ex(
        &mut s,
        "shape.ellipse",
        json!({"x": 10, "y": 20, "width": 40, "height": 40, "parent": f, "props": {"name": "Dot", "fill": "#FF0000", "stroke": "#000000", "strokeWeight": 2, "strokeAlign": "center"}}),
    );
    let code = uno(&mut s, f);
    let expected = "var dot = new Ellipse()
    .Canvas(left: 10, top: 20)
    .Width(40)
    .Height(40)
    .Fill(Color.FromArgb(0xFF, 0xFF, 0x00, 0x00))
    .Stroke(Color.FromArgb(0xFF, 0x00, 0x00, 0x00))
    .StrokeThickness(2);";
    assert!(has_block(&code, expected), "{code}");
}

#[test]
fn text_maps_its_style_and_box() {
    let mut s = Session::new();
    let f = frame(&mut s, "Texts", 300.0, 200.0);
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Say \"hi\"\n\\", "x": 10, "y": 12, "parent": f, "props": {"name": "T"}})));
    set(
        &mut s,
        t,
        json!({"fontFamily": "Inter", "fontSize": 16, "fontWeight": 600, "fontStyle": "Semi Bold Italic", "letterSpacing": 2, "lineHeight": "150%", "textDecoration": "underline", "textAlign": "center", "width": 150, "textAutoResize": "height", "fill": "#112233"}),
    );
    let code = uno(&mut s, f);
    assert!(code.contains("using Microsoft.UI.Text;"), "{code}");
    for line in [
        ".Canvas(left: 10, top: 12)",
        ".Text(\"Say \\\"hi\\\"\\n\\\\\")",
        ".FontFamily(\"Inter\")",
        ".FontSize(16)",
        ".FontWeight(FontWeights.SemiBold)",
        ".FontStyle(FontStyle.Italic)",
        ".Foreground(Color.FromArgb(0xFF, 0x11, 0x22, 0x33))",
        ".LineHeight(24)",
        ".CharacterSpacing(125)",
        ".TextDecorations(Windows.UI.Text.TextDecorations.Underline)",
        ".TextAlignment(TextAlignment.Center)",
        ".Width(150)",
        ".TextWrapping(TextWrapping.Wrap)",
    ] {
        assert!(has_call(&code, line) || code.contains(&format!("{line})")), "{line} in {code}");
    }
    // Auto height has a width but no height of its own.
    assert!(!code.contains(".Height(") || code.matches(".Height(").count() == 1, "{code}");
    // A fixed box aligned to the middle sits in a grid of its height.
    set(&mut s, t, json!({"textAutoResize": "none", "height": 60, "textAlignVertical": "middle"}));
    let code = uno(&mut s, f);
    assert!(code.contains("new Grid()") && code.contains(".VerticalAlignment(VerticalAlignment.Center)"), "{code}");
    // Truncation ends in an ellipsis.
    set(&mut s, t, json!({"textAutoResize": "truncate", "textAlignVertical": "top"}));
    assert!(uno(&mut s, f).contains(".TextTrimming(TextTrimming.CharacterEllipsis)"));
}

#[test]
fn text_case_is_applied_when_the_code_runs() {
    let mut s = Session::new();
    let f = frame(&mut s, "Cases", 300.0, 100.0);
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 0, "y": 0, "parent": f, "props": {"name": "T"}})));
    for (case, expr) in [("upper", ".Text(\"Hello\".ToUpperInvariant())"), ("lower", ".Text(\"Hello\".ToLowerInvariant())")] {
        set(&mut s, t, json!({"textCase": case}));
        let code = uno(&mut s, f);
        assert!(code.contains(expr), "{code}");
    }
    set(&mut s, t, json!({"textCase": "title"}));
    assert!(uno(&mut s, f).contains("TextInfo.ToTitleCase(\"Hello\")"));
    set(&mut s, t, json!({"textCase": "smallCaps"}));
    assert!(uno(&mut s, f).contains("// not mapped: small caps"));
}

#[test]
fn mixed_styles_become_runs() {
    let mut s = Session::new();
    let f = frame(&mut s, "Runs", 300.0, 100.0);
    let t = idv(&ex(
        &mut s,
        "text.create",
        json!({"text": "Plain bold end", "x": 0, "y": 0, "parent": f, "props": {"name": "T", "fontFamily": "Inter", "fontSize": 18}}),
    ));
    set(&mut s, t, json!({"fontWeight": 700, "start": 6, "end": 10}));
    let code = uno(&mut s, f);
    let expected = ".Inlines(
    new Run()
        .Text(\"Plain \"),
    new Run()
        .Text(\"bold\")
        .FontWeight(FontWeights.Bold),
    new Run()
        .Text(\" end\"))";
    assert!(has_block(&code, expected), "{code}");
    assert!(!code.contains(".Text(\"Plain bold end\")"), "{code}");
}

#[test]
fn vectors_are_svg_assets_with_unique_lowercase_names() {
    let mut s = Session::new();
    let f = frame(&mut s, "Icons", 200.0, 100.0);
    for x in [0, 50, 100] {
        ex(&mut s, "node.create", json!({"type": "star", "parent": f, "name": "Star Icon", "x": x, "y": 0, "width": 40, "height": 40}));
    }
    let code = uno(&mut s, f);
    for n in ["star_icon", "star_icon_2", "star_icon_3"] {
        assert!(code.contains(&format!(".Source(\"ms-appx:///Assets/{n}.svg\")")), "{n} in {code}");
        assert!(code.contains(&format!("// Star \"Star Icon\": export it as an SVG to Assets/{n}.svg")), "{code}");
    }
    assert!(code.contains(".Stretch(Stretch.Uniform)"), "{code}");
}

#[test]
fn an_image_fill_is_an_image_brush_on_a_png_asset() {
    use base64::Engine as _;
    let mut s = Session::new();
    let image = image::RgbaImage::from_fn(8, 4, |x, y| image::Rgba([x as u8, y as u8, 120, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
    let img = idv(&ex(&mut s, "file.import", json!({"data": data, "name": "Foto Größe.png", "x": 0, "y": 0})));
    set(&mut s, img, json!({"name": "Foto Größe"}));
    let code = uno(&mut s, img);
    assert!(
        code.contains(".Background(new ImageBrush().ImageSource(\"ms-appx:///Assets/foto_grosse.png\").Stretch(Stretch.UniformToFill))"),
        "{code}"
    );
}

#[test]
fn hidden_layers_and_deep_trees_are_left_out_or_summarised() {
    let mut s = Session::new();
    let f = frame(&mut s, "Tree", 100.0, 100.0);
    let hidden = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10, "parent": f, "props": {"name": "Hidden"}})));
    set(&mut s, hidden, json!({"visible": false}));
    assert!(!uno(&mut s, f).contains("Hidden"));
    let mut parent = f;
    for i in 0..20 {
        parent = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50, "parent": parent, "name": format!("Level {i}")})));
    }
    let code = uno(&mut s, f);
    assert!(code.contains("// Level ") && code.contains(" …\n"), "{code}");
    assert!(code.lines().all(|l| l.len() < 400), "{code}");
}

#[test]
fn literals_are_escaped_never_interpolated() {
    assert_eq!(cs_literal("a\"b\\c"), "\"a\\\"b\\\\c\"");
    assert_eq!(cs_literal("line\r\n\ttab"), "\"line\\r\\n\\ttab\"");
    assert_eq!(cs_literal("{x} $y\u{0}\u{2028}\u{85}"), "\"{x} $y\\u0000\\u2028\\u0085\"");
    assert_eq!(cs_literal("é日本"), "\"é日本\"");
    // Cut, not crashed, at 10,000 characters.
    assert_eq!(cs_literal(&"x".repeat(20_000)).chars().count(), 10_002);
}

#[test]
fn identifiers_are_c_sharp_names() {
    assert_eq!(cs_ident("Login Screen", true, "Frame"), "LoginScreen");
    assert_eq!(cs_ident("main card", false, "Frame"), "mainCard");
    assert_eq!(cs_ident("mainCard / title-1", true, "Frame"), "MainCardTitle1");
    assert_eq!(cs_ident("кнопка", true, "Frame"), "Knopka");
    assert_eq!(cs_ident("3 cards", true, "Frame"), "_3Cards");
    assert_eq!(cs_ident("class", false, "Frame"), "class_");
    assert_eq!(cs_ident("???", true, "frame"), "Frame");
    assert_eq!(cs_ident("", false, "Text"), "text");
    assert!(cs_ident(&"word ".repeat(40), true, "Frame").chars().count() <= 40);
    let mut used = BTreeSet::new();
    assert_eq!(cs_unique("Card", &mut used), "Card");
    assert_eq!(cs_unique("Card", &mut used), "Card2");
    assert_eq!(cs_unique("Card", &mut used), "Card3");
    assert_eq!(cs_unique("card", &mut used), "card");
}

#[test]
fn asset_names_are_lowercase_ascii_starting_with_a_letter_and_unique() {
    let mut used = BTreeSet::new();
    assert_eq!(asset_name("Hero Image", "Rectangle", &mut used), "hero_image");
    assert_eq!(asset_name("hero image", "Rectangle", &mut used), "hero_image_2");
    assert_eq!(asset_name("HERO-IMAGE", "Rectangle", &mut used), "hero_image_3");
    assert_eq!(asset_name("Größe über Äpfel", "Vector", &mut used), "grosse_uber_apfel");
    assert_eq!(asset_name("로그인 화면", "Vector", &mut used), "rogeuin_hwamyeon");
    assert_eq!(asset_name("2x icon", "Vector", &mut used), "img_2x_icon");
    assert_eq!(asset_name("***", "Star", &mut used), "star");
    assert_eq!(asset_name("", "", &mut used), "image");
    let long = asset_name(&"segment ".repeat(20), "Vector", &mut used);
    assert!(long.len() <= 40 && !long.ends_with('_') && long.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{long}");
}

#[test]
fn instances_and_components_are_walked_as_frames() {
    let mut s = Session::new();
    let f = frame(&mut s, "Card", 100.0, 50.0);
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let code = uno(&mut s, c);
    assert!(code.contains("internal sealed partial class CardView : UserControl"), "{code}");
    assert!(!code.contains("export default") && !code.contains("function "), "{code}");
}

fn rect(s: &mut Session, parent: NodeId, w: f64, h: f64, name: &str) -> NodeId {
    idv(&ex(s, "shape.rectangle", json!({"parent": parent, "x": 0, "y": 0, "width": w, "height": h, "props": {"name": name, "fill": "#FF0000"}})))
}

/// A frame with auto layout and one rectangle child.
fn row(s: &mut Session, props: serde_json::Value) -> NodeId {
    let f = frame(s, "Row", 200.0, 100.0);
    let mut all = json!({"layout": "horizontal", "padding": 0, "gap": 0, "fills": [], "sizingH": "fixed", "sizingV": "fixed"});
    if let (Some(all), serde_json::Value::Object(extra)) = (all.as_object_mut(), props) {
        all.extend(extra);
    }
    set(s, f, all);
    rect(s, f, 20.0, 20.0, "A");
    f
}

#[test]
fn a_row_is_an_auto_layout_with_its_box_gap_padding_and_alignment() {
    let mut s = Session::new();
    let f = row(
        &mut s,
        json!({"fills": ["#112233"], "gap": 12, "padding": [7, 15, 7, 15], "mainAlign": "center", "crossAlign": "center", "cornerRadius": 8}),
    );
    rect(&mut s, f, 20.0, 20.0, "B");
    let code = uno(&mut s, f);
    assert!(code.starts_with("using Windows.UI;\nusing Uno.Toolkit.UI;\n") || code.contains("using Uno.Toolkit.UI;"), "{code}");
    let expected = "var row = new AutoLayout()
    .Width(200)
    .Height(100)
    .Background(Color.FromArgb(0xFF, 0x11, 0x22, 0x33))
    .CornerRadius(8)
    .Orientation(Orientation.Horizontal)
    .Spacing(12)
    .Padding(15, 7, 15, 7)
    .PrimaryAxisAlignment(AutoLayoutAlignment.Center)
    .CounterAxisAlignment(AutoLayoutAlignment.Center);
var a = new Border()
    .Width(20)
    .Height(20)
    .Background(Color.FromArgb(0xFF, 0xFF, 0x00, 0x00));
row.Children.Add(a);
var b = new Border()
    .Width(20)
    .Height(20)
    .Background(Color.FromArgb(0xFF, 0xFF, 0x00, 0x00));
row.Children.Add(b);
Content = row;";
    assert_eq!(body(&code), expected, "{code}");
    // A frame with auto layout is not a frame without one: no warning.
    assert!(!code.contains("// warning"), "{code}");
}

#[test]
fn a_column_has_a_vertical_orientation_and_a_uniform_padding_is_one_number() {
    let mut s = Session::new();
    let f = row(&mut s, json!({"layout": "vertical", "padding": 8, "gap": 0}));
    let code = uno(&mut s, f);
    assert!(has_call(&code, ".Orientation(Orientation.Vertical)"), "{code}");
    assert!(has_call(&code, ".Padding(8)"), "{code}");
    assert!(!code.contains(".Spacing("), "no gap, no spacing: {code}");
    set(&mut s, f, json!({"padding": 0}));
    assert!(!uno(&mut s, f).contains(".Padding("));
}

#[test]
fn main_and_cross_alignment_map_with_notes_for_what_the_toolkit_lacks() {
    let mut s = Session::new();
    let f = row(&mut s, json!({"gap": 5}));
    for (main, line) in [("max", ".PrimaryAxisAlignment(AutoLayoutAlignment.End)"), ("spaceBetween", ".Justify(AutoLayoutJustify.SpaceBetween)")] {
        set(&mut s, f, json!({"mainAlign": main}));
        let code = uno(&mut s, f);
        assert!(code.contains(line), "{main}: {code}");
        assert!(!code.contains("// not mapped"), "{main}: {code}");
    }
    // The distributions make the gaps: the gap value is not written.
    assert!(!uno(&mut s, f).contains(".Spacing("));
    for main in ["spaceAround", "spaceEvenly"] {
        set(&mut s, f, json!({"mainAlign": main}));
        let code = uno(&mut s, f);
        assert!(
            code.contains(".Justify(AutoLayoutJustify.SpaceBetween)")
                && code.contains("// not mapped: space around/evenly exported as space between"),
            "{code}"
        );
    }
    set(&mut s, f, json!({"mainAlign": "min", "crossAlign": "max"}));
    assert!(uno(&mut s, f).contains(".CounterAxisAlignment(AutoLayoutAlignment.End)"));
    set(&mut s, f, json!({"crossAlign": "baseline"}));
    let code = uno(&mut s, f);
    assert!(!code.contains("CounterAxisAlignment") && code.contains("// not mapped: baseline alignment"), "{code}");
}

#[test]
fn the_stroke_weight_comes_off_the_padding_unless_strokes_are_included() {
    let mut s = Session::new();
    let f = row(&mut s, json!({"padding": 10, "stroke": "#0000FF", "strokeWeight": 4, "strokeAlign": "inside", "strokesIncluded": false}));
    let code = uno(&mut s, f);
    assert!(code.contains(".BorderThickness(4)") && has_call(&code, ".Padding(6)"), "{code}");
    assert!(!code.contains("new Grid()"), "{code}");
    set(&mut s, f, json!({"strokesIncluded": true}));
    assert!(has_call(&uno(&mut s, f), ".Padding(10)"));
    // A stroke thicker than the padding on a side: the border sits behind the layout.
    set(&mut s, f, json!({"strokesIncluded": false, "padding": 2}));
    let code = uno(&mut s, f);
    assert!(code.contains("new Grid()") && code.contains("new Border()") && has_call(&code, ".Padding(2)"), "{code}");
    let (grid, layout) = (code.find("new Grid()"), code.find("new AutoLayout()"));
    assert!(grid < layout, "{code}");
    assert!(!code[layout.unwrap_or(0)..].contains(".BorderThickness("), "{code}");
}

#[test]
fn frames_without_auto_layout_get_one_warning_per_view_with_their_count() {
    let mut s = Session::new();
    let outer = frame(&mut s, "Free", 200.0, 200.0);
    for name in ["One", "Two"] {
        let inner = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50, "parent": outer, "name": name})));
        rect(&mut s, inner, 10.0, 10.0, "R");
    }
    let code = uno(&mut s, outer);
    assert_eq!(code.matches("// warning:").count(), 1, "{code}");
    assert!(code.contains("// warning: 3 frames have no auto layout: children are positioned absolutely and do not adapt to size."), "{code}");
    // An empty frame has no children to place: nothing to warn about.
    let empty = frame(&mut s, "Empty2", 10.0, 10.0);
    assert!(!uno(&mut s, empty).contains("// warning"));
}

#[test]
fn nested_auto_layouts_carry_no_canvas_offsets() {
    let mut s = Session::new();
    let outer = row(&mut s, json!({"layout": "vertical"}));
    let inner = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50, "parent": outer, "name": "Inner"})));
    set(&mut s, inner, json!({"layout": "horizontal", "padding": 3, "gap": 2}));
    rect(&mut s, inner, 5.0, 5.0, "C");
    let code = uno(&mut s, outer);
    assert_eq!(code.matches("new AutoLayout()").count(), 2, "{code}");
    assert!(!code.contains(".Canvas("), "{code}");
}

#[test]
fn children_are_added_never_chained() {
    // Fluent `.Children(…)` / `.Child(…)` doubles the load time with every level of nesting, so
    // containers are locals and their children are added one by one.
    let mut s = Session::new();
    let outer = frame(&mut s, "Outer", 300.0, 300.0);
    set(&mut s, outer, json!({"stroke": "#000000", "strokeWeight": 2}));
    let inner = idv(&ex(&mut s, "frame.create", json!({"x": 5, "y": 5, "width": 100, "height": 100, "parent": outer, "name": "Inner"})));
    set(&mut s, inner, json!({"layout": "vertical", "padding": 4, "gap": 3, "stroke": "#FF0000", "strokeWeight": 9, "strokesIncluded": false}));
    rect(&mut s, inner, 10.0, 10.0, "R");
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Hi", "x": 0, "y": 0, "parent": outer, "props": {"name": "T"}})));
    set(&mut s, t, json!({"textAutoResize": "none", "width": 50, "height": 40, "textAlignVertical": "middle"}));
    let mut parent = outer;
    for i in 0..6 {
        parent = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50, "parent": parent, "name": format!("Level {i}")})));
    }
    let code = uno(&mut s, outer);
    assert!(!code.contains(".Children(") && !code.contains(".Child("), "{code}");
    assert!(code.contains(".Children.Add(") && code.contains(".Child = "), "{code}");
    // A single child of a border is assigned.
    let lone = frame(&mut s, "Lone", 20.0, 20.0);
    rect(&mut s, lone, 5.0, 5.0, "X");
    let code = uno(&mut s, lone);
    assert!(!code.contains(".Child(") && !code.contains(".Children("), "{code}");
    // Names of locals are unique C# identifiers, the parent's before its children's.
    let twins = frame(&mut s, "Card", 50.0, 50.0);
    rect(&mut s, twins, 5.0, 5.0, "Card");
    rect(&mut s, twins, 5.0, 5.0, "class");
    let code = uno(&mut s, twins);
    for decl in ["var card = ", "var card2 = ", "var class_ = "] {
        assert_eq!(code.matches(decl).count(), 1, "{decl} in {code}");
    }
}

/// The calls of the local `var`, in order, without indent or the closing `;`.
fn chain(code: &str, var: &str) -> Vec<String> {
    let head = format!("var {var} = ");
    let mut out = Vec::new();
    let mut lines = code.lines().skip_while(|l| !l.trim_start().starts_with(&head));
    let Some(first) = lines.next() else { return out };
    let mut done = first.trim_end().ends_with(';');
    for l in lines {
        // A `// not mapped` comment may follow the call that ends the statement.
        if done && !l.trim_start().starts_with("//") {
            break;
        }
        done = done || l.trim_end().ends_with(';');
        out.push(l.trim().trim_end_matches(';').to_string());
    }
    out
}

fn first_child(s: &Session, f: NodeId) -> NodeId {
    s.doc().ok().and_then(|d| d.node(f)).and_then(|n| n.children.first().copied()).unwrap_or(f)
}

#[test]
fn fill_stretches_on_the_axis_it_fills_and_fixed_keeps_its_size() {
    let mut s = Session::new();
    let f = row(&mut s, json!({}));
    let a = first_child(&s, f);
    let has = |c: &[String], p: &str| c.iter().any(|l| l.starts_with(p));
    // Horizontal parent: width is the primary axis, height the counter axis.
    set(&mut s, a, json!({"sizingH": "fill"}));
    let code = uno(&mut s, f);
    let kid = chain(&code, "a");
    assert_eq!(kid[0], ".AutoLayout(primaryAlignment: AutoLayoutPrimaryAlignment.Stretch)", "{code}");
    assert!(!has(&kid, ".Width(") && has(&kid, ".Height("), "{code}");
    set(&mut s, a, json!({"sizingV": "fill"}));
    let kid = chain(&uno(&mut s, f), "a");
    assert_eq!(kid[0], ".AutoLayout(primaryAlignment: AutoLayoutPrimaryAlignment.Stretch, counterAlignment: AutoLayoutAlignment.Stretch)");
    assert!(!has(&kid, ".Width(") && !has(&kid, ".Height("));
    // In a column the axes swap.
    set(&mut s, f, json!({"layout": "vertical"}));
    set(&mut s, a, json!({"sizingH": "fixed"}));
    let kid = chain(&uno(&mut s, f), "a");
    assert_eq!(kid[0], ".AutoLayout(primaryAlignment: AutoLayoutPrimaryAlignment.Stretch)");
    assert!(has(&kid, ".Width("));
    set(&mut s, a, json!({"sizingH": "fill", "sizingV": "fixed"}));
    assert_eq!(chain(&uno(&mut s, f), "a")[0], ".AutoLayout(counterAlignment: AutoLayoutAlignment.Stretch)");
}

#[test]
fn a_hugging_frame_has_no_size_of_its_own_and_the_root_keeps_its_size() {
    let mut s = Session::new();
    let outer = row(&mut s, json!({"layout": "vertical", "sizingH": "hug", "sizingV": "hug"}));
    let inner = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50, "parent": outer, "name": "Inner"})));
    set(&mut s, inner, json!({"layout": "horizontal", "padding": 3, "gap": 2, "sizingH": "hug", "sizingV": "hug"}));
    rect(&mut s, inner, 5.0, 5.0, "C");
    let code = uno(&mut s, outer);
    let has = |c: &[String], p: &str| c.iter().any(|l| l.starts_with(p));
    let root = chain(&code, "row");
    assert!(has(&root, ".Width(") && has(&root, ".Height("), "the root keeps its size: {code}");
    let nested = chain(&code, "inner");
    assert!(!has(&nested, ".Width(") && !has(&nested, ".Height("), "{code}");
}

#[test]
fn min_and_max_sizes_go_on_the_child() {
    let mut s = Session::new();
    let f = row(&mut s, json!({}));
    let a = first_child(&s, f);
    set(&mut s, a, json!({"sizingH": "fill", "minWidth": 40, "maxWidth": 120, "minHeight": 10, "maxHeight": 30}));
    let code = uno(&mut s, f);
    for line in [".MinWidth(40)", ".MaxWidth(120)", ".MinHeight(10)", ".MaxHeight(30)"] {
        assert!(chain(&code, "a").contains(&line.to_string()), "{line} in {code}");
    }
}

#[test]
fn text_in_a_flow_hugs_fills_or_is_fixed() {
    let mut s = Session::new();
    let f = row(&mut s, json!({}));
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 0, "y": 0, "parent": f, "props": {"name": "T"}})));
    let calls = |s: &mut Session| chain(&uno(s, f), "t");
    let has = |c: &[String], p: &str| c.iter().any(|l| l.starts_with(p));
    set(&mut s, t, json!({"textAutoResize": "widthAndHeight", "sizingH": "hug", "sizingV": "hug"}));
    let c = calls(&mut s);
    assert!(c.contains(&".TextWrapping(TextWrapping.NoWrap)".to_string()), "{c:?}");
    assert!(!has(&c, ".Width(") && !has(&c, ".Height("), "{c:?}");
    set(&mut s, t, json!({"textAutoResize": "height", "sizingH": "fill"}));
    let c = calls(&mut s);
    assert_eq!(c.first().map(String::as_str), Some(".AutoLayout(primaryAlignment: AutoLayoutPrimaryAlignment.Stretch)"), "{c:?}");
    assert!(c.contains(&".TextWrapping(TextWrapping.Wrap)".to_string()) && !has(&c, ".Width("), "{c:?}");
    set(&mut s, t, json!({"textAutoResize": "height", "sizingH": "fixed", "width": 90}));
    let c = calls(&mut s);
    assert!(c.contains(&".Width(90)".to_string()) && c.contains(&".TextWrapping(TextWrapping.Wrap)".to_string()), "{c:?}");
}

#[test]
fn an_absolute_child_ignores_the_layout_and_keeps_its_offsets() {
    let mut s = Session::new();
    let f = row(&mut s, json!({"padding": 10}));
    let b = rect(&mut s, f, 12.0, 12.0, "Badge");
    set(&mut s, b, json!({"absolute": true, "x": 30, "y": 40}));
    let code = uno(&mut s, f);
    let expected = [
        ".AutoLayout(isIndependentLayout: true)",
        ".HorizontalAlignment(HorizontalAlignment.Left)",
        ".VerticalAlignment(VerticalAlignment.Top)",
        ".Margin(30, 40, 0, 0)",
        ".Width(12)",
        ".Height(12)",
    ];
    assert_eq!(chain(&code, "badge")[..6], expected, "{code}");
    // A top-left child needs no note; another constraint does, and the offsets stay.
    assert!(!code.contains("// not mapped: constraints"), "{code}");
    set(&mut s, b, json!({"constraints": {"horizontal": "right"}}));
    let code = uno(&mut s, f);
    assert!(chain(&code, "badge").iter().any(|l| l == "// not mapped: constraints (the child stays at its top-left offset)"), "{code}");
    assert!(chain(&code, "badge").contains(&".Margin(30, 40, 0, 0)".to_string()), "{code}");
}

#[test]
fn a_hugging_parent_with_an_absolute_child_is_written_at_its_size() {
    let mut s = Session::new();
    let outer = row(&mut s, json!({"layout": "vertical"}));
    let hug = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 60, "height": 40, "parent": outer, "name": "Hug"})));
    set(&mut s, hug, json!({"layout": "horizontal", "padding": 0, "gap": 0, "sizingH": "hug", "sizingV": "hug"}));
    rect(&mut s, hug, 60.0, 40.0, "Body");
    let code = uno(&mut s, outer);
    assert!(!code.contains("hug size fixed"), "{code}");
    let badge = rect(&mut s, hug, 8.0, 8.0, "Badge");
    set(&mut s, badge, json!({"absolute": true, "x": 50, "y": -4}));
    let code = uno(&mut s, outer);
    let c = chain(&code, "hug");
    assert!(c.contains(&"// not mapped: hug size fixed: an absolute child would enlarge it".to_string()), "{code}");
    assert!(c.contains(&".Width(60)".to_string()) && c.contains(&".Height(40)".to_string()), "{code}");
}

#[test]
fn first_on_top_paints_in_descending_order_without_reordering() {
    let mut s = Session::new();
    let f = row(&mut s, json!({"gap": -10}));
    rect(&mut s, f, 20.0, 20.0, "B");
    rect(&mut s, f, 20.0, 20.0, "C");
    let plain = uno(&mut s, f);
    assert!(!plain.contains("zIndex") && !plain.contains("IsReverseZIndex"), "{plain}");
    set(&mut s, f, json!({"reverseZ": true}));
    let code = uno(&mut s, f);
    for (var, z) in [("a", 3), ("b", 2), ("c", 1)] {
        assert!(chain(&code, var).contains(&format!(".Canvas(zIndex: {z})")), "{var} in {code}");
    }
    // The order of the children is unchanged.
    assert!(code.find("var a = ") < code.find("var b = ") && code.find("var b = ") < code.find("var c = "), "{code}");
    assert!(!code.contains("IsReverseZIndex"), "{code}");
}
