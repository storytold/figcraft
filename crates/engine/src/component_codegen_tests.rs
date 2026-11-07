//! Component code: components become React / Leptos / SwiftUI / Compose components whose
//! properties are parameters, and instances become calls passing the values they set.

use crate::Session;
use crate::codegen::{CodeLang, code_for};
use crate::component_codegen::{ParamKind, describe, main_of};
use crate::components::tests::{ex, idv};
use figcraft_doc::*;
use serde_json::{Value, json};

fn code(s: &mut Session, id: NodeId, lang: &str) -> String {
    ex(s, "inspect.code", json!({"id": id, "language": lang}))["code"].as_str().unwrap_or_default().to_string()
}

fn component_code(s: &mut Session, id: NodeId, lang: &str) -> Value {
    ex(s, "inspect.componentCode", json!({"id": id, "language": lang}))
}

/// A "Card" component with a "Title" text (text property), a "Dot" ellipse (boolean property
/// "Show dot") and a "Bg" rectangle. Returns (card, title, dot).
fn card(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 100, "parent": f, "props": {"name": "Bg", "fill": "#eeeeee"}}));
    let title = idv(&ex(s, "text.create", json!({"text": "Hello", "x": 10, "y": 10, "parent": f, "props": {"name": "Title"}})));
    let dot = idv(&ex(s, "shape.ellipse", json!({"x": 170, "y": 10, "width": 20, "height": 20, "parent": f, "props": {"name": "Dot"}})));
    let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
    ex(s, "component.addProperty", json!({"id": dot, "name": "Show dot", "type": "boolean", "default": true}));
    ex(s, "component.addProperty", json!({"id": c, "name": "Title", "type": "text", "layer": title}));
    (c, title, dot)
}

/// A "Button" set with State=Default|Hover variants (the hover background blue). Returns (set,
/// default variant, hover variant).
fn button_set(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let mut variants = Vec::new();
    for (k, (state, color)) in [("Default", "#ff0000"), ("Hover", "#0000ff")].iter().enumerate() {
        let f = idv(&ex(s, "frame.create", json!({"x": k as f64 * 150.0, "y": 300, "width": 100, "height": 40, "name": format!("State={state}")})));
        ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": f, "props": {"name": "Bg", "fill": color}}));
        variants.push(idv(&ex(s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(s, "component.combineVariants", json!({"ids": variants.clone()})));
    ex(s, "node.set", json!({"id": set, "props": {"name": "Button"}}));
    (set, variants[0], variants[1])
}

#[test]
fn strict_mode_reserved_words_are_not_parameter_names() {
    let mut s = Session::new();
    let (c, title, _) = card(&mut s);
    ex(&mut s, "component.addProperty", json!({"id": c, "name": "Protected", "type": "text", "layer": title}));
    let react = code(&mut s, c, "react");
    assert!(react.contains("protectedValue"), "React renames the property:\n{react}");
    for bad in ["  protected?", "  protected =", "{protected}"] {
        assert!(!react.contains(bad), "reserved word used as a binding ({bad:?}):\n{react}");
    }
}

#[test]
fn components_take_their_properties_as_parameters() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    let react = code(&mut s, c, "react");
    for want in [
        "export interface CardProps {",
        "  showDot?: boolean;",
        "  title?: string;",
        "export function Card({",
        "  showDot = true,",
        "  title = \"Hello\",",
        "}: CardProps) {",
        "{showDot && (",
        ">{title}</p>",
        "/* card.css */",
        "border-radius: 50%;",
    ] {
        assert!(react.contains(want), "React has {want:?}:\n{react}");
    }
    let swift = code(&mut s, c, "swiftui");
    for want in ["struct Card: View {", "    var showDot: Bool = true", "    var title: String = \"Hello\"", "if showDot {", "Text(title)"] {
        assert!(swift.contains(want), "SwiftUI has {want:?}:\n{swift}");
    }
    let kt = code(&mut s, c, "compose");
    for want in ["@Composable\nfun Card(", "    showDot: Boolean = true,", "    title: String = \"Hello\",", "if (showDot) {", "text = title,"] {
        assert!(kt.contains(want), "Compose has {want:?}:\n{kt}");
    }
    let ark = code(&mut s, c, "arkui");
    for want in [
        "@Component\nexport struct Card {",
        "    @Prop showDot: boolean = true",
        "    @Prop title: string = \"Hello\"",
        "    build() {",
        "        if (this.showDot) {",
        "Text(this.title)",
    ] {
        assert!(ark.contains(want), "ArkUI has {want:?}:\n{ark}");
    }
    let leptos = code(&mut s, c, "leptos");
    for want in [
        "use leptos::prelude::*;\n\n#[component]\npub fn Card(\n",
        "    #[prop(default = true)] show_dot: bool,\n",
        "    #[prop(into, default = \"Hello\".to_string())] title: String,\n",
        ") -> impl IntoView {\n    view! {\n",
        "{show_dot.then(|| view! {",
        ">{title.clone()}</p>",
        "/* card.css */",
    ] {
        assert!(leptos.contains(want), "Leptos has {want:?}:\n{leptos}");
    }
    let dart = code(&mut s, c, "flutter");
    for want in [
        "import 'package:flutter/material.dart';\n\nclass Card extends StatelessWidget {\n",
        "  const Card({\n    super.key,\n    this.showDot = true,\n    this.title = \"Hello\",\n  });\n",
        "  final bool showDot;\n  final String title;\n",
        "  Widget build(BuildContext context) {\n    return Container(",
        "if (showDot)\n",
        "Text(\n              title,",
    ] {
        assert!(dart.contains(want), "Flutter has {want:?}:\n{dart}");
    }
    // The command describes the properties.
    let v = component_code(&mut s, c, "react");
    assert_eq!(v["name"], "Card");
    assert_eq!(
        v["properties"],
        json!([
            {"name": "Show dot", "param": "showDot", "type": "boolean", "default": true},
            {"name": "Title", "param": "title", "type": "text", "default": "Hello"},
        ])
    );
    assert_eq!(v["definition"].as_str(), Some(react.as_str()));
    assert!(v.get("usage").is_none(), "a main has no call");
    // Deterministic.
    assert_eq!(code(&mut s, c, "react"), react);
}

#[test]
fn instances_are_calls_passing_the_values_they_set() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    let v = component_code(&mut s, i, "react");
    assert_eq!(v["usage"], "<Card />\n", "defaults aren't passed");
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Show dot": false, "Title": "Buy"}}));
    let v = component_code(&mut s, i, "react");
    assert_eq!(v["usage"], "<Card showDot={false} title=\"Buy\" />\n");
    assert_eq!(v["properties"][0]["value"], false);
    assert_eq!(v["properties"][1]["value"], "Buy");
    // inspect.code on an instance: the call, then the definition.
    let react = code(&mut s, i, "react");
    assert!(react.starts_with("<Card showDot={false} title=\"Buy\" />\n"), "{react}");
    assert!(react.contains("export function Card({"));
    assert_eq!(component_code(&mut s, i, "swiftui")["usage"], "Card(showDot: false, title: \"Buy\")\n");
    assert_eq!(component_code(&mut s, i, "compose")["usage"], "Card(showDot = false, title = \"Buy\")\n");
    assert_eq!(component_code(&mut s, i, "flutter")["usage"], "Card(showDot: false, title: \"Buy\")\n");
    assert_eq!(component_code(&mut s, i, "arkui")["usage"], "Card({ showDot: false, title: \"Buy\" })\n");
    assert_eq!(component_code(&mut s, i, "leptos")["usage"], "<Card show_dot={false} title=\"Buy\" />\n");
    // Text that needs escaping is an expression in JSX and escaped per language.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Say \"hi\" $5\nnow"}}));
    assert_eq!(component_code(&mut s, i, "react")["usage"], "<Card showDot={false} title={\"Say \\\"hi\\\" $5\\nnow\"} />\n");
    assert_eq!(component_code(&mut s, i, "swiftui")["usage"], "Card(showDot: false, title: \"Say \\\"hi\\\" $5\\nnow\")\n");
    assert_eq!(component_code(&mut s, i, "compose")["usage"], "Card(showDot = false, title = \"Say \\\"hi\\\" \\$5\\nnow\")\n");
    assert_eq!(component_code(&mut s, i, "flutter")["usage"], "Card(showDot: false, title: \"Say \\\"hi\\\" \\$5\\nnow\")\n");
    assert_eq!(component_code(&mut s, i, "arkui")["usage"], "Card({ showDot: false, title: \"Say \\\"hi\\\" $5\\nnow\" })\n");
    assert_eq!(component_code(&mut s, i, "leptos")["usage"], "<Card show_dot={false} title={\"Say \\\"hi\\\" $5\\nnow\"} />\n");
    // A quoted title becomes a binding whose attribute value is escaped: the expression itself
    // keeps its JS quotes, and the attribute's own quotes are entities.
    assert_eq!(component_code(&mut s, i, "vue")["usage"], "<Card :showDot=\"false\" :title=\"&quot;Say \\&quot;hi\\&quot; $5\\nnow&quot;\" />\n");
    let vue = code(&mut s, i, "vue");
    assert!(vue.starts_with("<Card :showDot=\"false\" :title=\"&quot;Say \\&quot;hi\\&quot; $5\\nnow&quot;\" />\n"), "{vue}");
    // A plain attribute keeps HTML entities intact: a literal ampersand is written as `&amp;`,
    // and a literal `&amp;` as `&amp;amp;`.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Fish & Chips"}}));
    assert_eq!(component_code(&mut s, i, "vue")["usage"], "<Card :showDot=\"false\" title=\"Fish &amp; Chips\" />\n");
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Fish &amp; Chips"}}));
    assert_eq!(component_code(&mut s, i, "vue")["usage"], "<Card :showDot=\"false\" title=\"Fish &amp;amp; Chips\" />\n");
}

#[test]
fn nested_instances_in_a_layer_tree_are_calls() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    let screen = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 500, "width": 400, "height": 300, "name": "Home screen"})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 20, "y": 30, "parent": screen})));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Inbox"}}));
    let react = code(&mut s, screen, "react");
    assert!(react.contains("export function HomeScreen() {"), "{react}");
    assert!(react.contains("<Card title=\"Inbox\" />"), "{react}");
    assert!(!react.contains("Hello"), "the instance's layers aren't spelled out: {react}");
    assert!(code(&mut s, screen, "swiftui").contains("Card(title: \"Inbox\")"));
    assert!(code(&mut s, screen, "compose").contains("Card(title = \"Inbox\")"));
    let dart = code(&mut s, screen, "flutter");
    assert!(dart.contains("Card(title: \"Inbox\")") && !dart.contains("Hello"), "{dart}");
    let leptos = code(&mut s, screen, "leptos");
    assert!(leptos.contains("pub fn HomeScreen() -> impl IntoView {") && leptos.contains("<Card title=\"Inbox\" />"), "{leptos}");
    assert!(!leptos.contains("Hello"), "{leptos}");
    // HTML and CSS stay plain markup.
    assert!(code(&mut s, screen, "html").contains("Inbox"));
}

#[test]
fn variant_sets_share_one_definition() {
    let mut s = Session::new();
    let (set, default, hover) = button_set(&mut s);
    let doc = s.doc().unwrap();
    assert_eq!(main_of(doc, set), Some(default));
    assert_eq!(main_of(doc, hover), Some(default), "every variant is defined by the set's first");
    let c = describe(doc, default).unwrap();
    assert_eq!(c.type_name, "Button");
    assert_eq!(c.params.len(), 1);
    assert_eq!(c.params[0].ident, "state");
    assert_eq!(c.params[0].kind, ParamKind::Variant(vec!["Default".into(), "Hover".into()]));
    let react = code(&mut s, set, "react");
    assert!(react.contains("  state?: \"Default\" | \"Hover\";"), "{react}");
    assert!(react.contains("  state = \"Default\","), "{react}");
    assert!(react.contains("data-state={state}"), "{react}");
    // The hover variant's difference is a rule on its data attribute.
    assert!(react.contains(".button[data-state=\"Hover\"] .bg {\n  background: #0000FF;"), "{react}");
    let swift = code(&mut s, set, "swiftui");
    assert!(
        swift.contains("    enum State: String, CaseIterable {\n        case `default` = \"Default\"\n        case hover = \"Hover\""),
        "{swift}"
    );
    assert!(swift.contains("    var state: State = .`default`"), "{swift}");
    let kt = code(&mut s, set, "compose");
    assert!(kt.contains("enum class ButtonState { Default, Hover }"), "{kt}");
    assert!(kt.contains("    state: ButtonState = ButtonState.Default,"), "{kt}");
    let ark = code(&mut s, set, "arkui");
    assert!(ark.contains("export enum ButtonState { Default, Hover }"), "{ark}");
    assert!(ark.contains("    @Prop state: ButtonState = ButtonState.Default"), "{ark}");
    let leptos = code(&mut s, set, "leptos");
    for want in [
        "#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]\npub enum ButtonState {\n    #[default]\n    Default,\n    Hover,\n}",
        "            Self::Hover => \"Hover\",",
        "    #[prop(optional)] state: ButtonState,",
        "data-state={state.as_str()}",
        ".button[data-state=\"Hover\"] .bg {\n  background: #0000FF;",
    ] {
        assert!(leptos.contains(want), "Leptos has {want:?}:\n{leptos}");
    }
    // An instance showing the hover variant passes it.
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 600})));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Hover"}}));
    assert_eq!(component_code(&mut s, i, "react")["usage"], "<Button state=\"Hover\" />\n");
    assert_eq!(component_code(&mut s, i, "swiftui")["usage"], "Button(state: .hover)\n");
    assert_eq!(component_code(&mut s, i, "compose")["usage"], "Button(state = ButtonState.Hover)\n");
    assert_eq!(component_code(&mut s, i, "arkui")["usage"], "Button({ state: ButtonState.Hover })\n");
    assert_eq!(component_code(&mut s, i, "leptos")["usage"], "<Button state=ButtonState::Hover />\n");
}

#[test]
fn flutter_variant_sets_share_one_enum() {
    let mut s = Session::new();
    let (set, _, _) = button_set(&mut s);
    let dart = code(&mut s, set, "flutter");
    assert!(dart.contains("enum ButtonState { defaultValue, hover }\n"), "{dart}");
    assert!(dart.contains("    this.state = ButtonState.defaultValue,\n"), "{dart}");
    assert!(dart.contains("  final ButtonState state;\n"), "{dart}");
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 600})));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Hover"}}));
    assert_eq!(component_code(&mut s, i, "flutter")["usage"], "Button(state: ButtonState.hover)\n");
}

#[test]
fn instance_swap_properties_take_views() {
    let mut s = Session::new();
    let icon = |s: &mut Session, name: &str, x: f64| {
        let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 400, "width": 24, "height": 24, "name": name})));
        ex(s, "shape.star", json!({"x": 0, "y": 0, "width": 24, "height": 24, "parent": f}));
        idv(&ex(s, "component.create", json!({"ids": [f]})))
    };
    let (heart, star) = (icon(&mut s, "Heart icon", 0.0), icon(&mut s, "Star icon", 50.0));
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 120, "height": 40, "name": "Chip"})));
    let nested = idv(&ex(&mut s, "instance.create", json!({"component": heart, "x": 8, "y": 8, "parent": f})));
    let chip = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.addProperty", json!({"id": chip, "name": "Icon", "type": "instanceSwap", "default": heart, "layer": nested}));
    let react = code(&mut s, chip, "react");
    assert!(react.contains("  icon?: React.ReactNode;") && react.contains("  icon = <HeartIcon />,"), "{react}");
    assert!(react.contains("{icon}"), "the nested instance is the parameter: {react}");
    let swift = code(&mut s, chip, "swiftui");
    assert!(swift.contains("    var icon: AnyView = AnyView(HeartIcon())"), "{swift}");
    let kt = code(&mut s, chip, "compose");
    assert!(kt.contains("    icon: @Composable () -> Unit = { HeartIcon() },") && kt.contains("icon()"), "{kt}");
    let dart = code(&mut s, chip, "flutter");
    assert!(dart.contains("  final Widget icon;\n") && dart.contains("    this.icon = const HeartIcon(),\n"), "{dart}");
    assert!(dart.contains("child: icon\n"), "the nested instance is the parameter: {dart}");
    assert!(!dart.contains("child: Container("), "the swap's widget tree isn't spelled out: {dart}");
    let ark = code(&mut s, chip, "arkui");
    for want in [
        "    @BuilderParam icon: () => void = this.defaultIconBuilder",
        "    @Builder\n    defaultIconBuilder() {\n        HeartIcon()\n    }",
        "            this.icon()",
    ] {
        assert!(ark.contains(want), "ArkUI has {want:?}:\n{ark}");
    }
    let leptos = code(&mut s, chip, "leptos");
    assert!(leptos.contains("    #[prop(into, default = (|| view! { <HeartIcon /> }).into())] icon: ViewFn,"), "{leptos}");
    assert!(leptos.contains("{icon.run()}"), "{leptos}");
    let vue = code(&mut s, chip, "vue");
    assert!(vue.contains("    icon?: unknown;"), "{vue}");
    assert!(vue.contains("    icon: HeartIcon,"), "the swap defaults to its component: {vue}");
    assert!(vue.contains("<component :is=\"icon\" />"), "a swap is a component, not an element: {vue}");
    let i = idv(&ex(&mut s, "instance.create", json!({"component": chip, "x": 0, "y": 100})));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Icon": star}}));
    assert_eq!(component_code(&mut s, i, "react")["usage"], "<Chip icon={<StarIcon />} />\n");
    assert_eq!(component_code(&mut s, i, "swiftui")["usage"], "Chip(icon: AnyView(StarIcon()))\n");
    assert_eq!(component_code(&mut s, i, "compose")["usage"], "Chip(icon = { StarIcon() })\n");
    assert_eq!(component_code(&mut s, i, "flutter")["usage"], "Chip(icon: StarIcon())\n");
    assert_eq!(component_code(&mut s, i, "arkui")["usage"], "Chip({ icon: () => { StarIcon() } })\n");
    assert_eq!(component_code(&mut s, i, "leptos")["usage"], "<Chip icon={|| view! { <StarIcon /> }} />\n");
    assert_eq!(component_code(&mut s, i, "vue")["usage"], "<Chip :icon=\"StarIcon\" />\n");
}

#[test]
fn slots_take_optional_content() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Panel"})));
    let content = idv(&ex(&mut s, "frame.create", json!({"x": 10, "y": 50, "width": 280, "height": 140, "parent": f, "name": "Content"})));
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": content, "props": {"name": "Placeholder"}}));
    let panel = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.makeSlot", json!({"id": content}));
    let react = code(&mut s, panel, "react");
    assert!(react.contains("  content?: React.ReactNode;") && react.contains("{content ?? ("), "{react}");
    assert!(code(&mut s, panel, "swiftui").contains("var content: AnyView? = nil"));
    assert!(code(&mut s, panel, "compose").contains("content: (@Composable () -> Unit)? = null,"));
    let dart = code(&mut s, panel, "flutter");
    assert!(dart.contains("  final List<Widget>? content;\n") && dart.contains("    this.content,\n"), "{dart}");
    assert!(dart.contains("...?content,\n") && dart.contains("if (content == null) ...[\n"), "{dart}");
    let ark = code(&mut s, panel, "arkui");
    for want in [
        "    @BuilderParam content: () => void = this.defaultContentBuilder",
        "    @Builder\n    defaultContentBuilder() {\n",
        "            this.content()",
    ] {
        assert!(ark.contains(want), "ArkUI has {want:?}:\n{ark}");
    }
    let leptos = code(&mut s, panel, "leptos");
    assert!(leptos.contains("    #[prop(optional, into)] content: Option<ViewFn>,"), "{leptos}");
    assert!(leptos.contains("{match &content { Some(c) => c.run(), None => view! {\n") && leptos.contains("}.into_any() }}\n"), "{leptos}");
    // An instance filling the slot passes its content.
    let g = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 400, "width": 40, "height": 20, "name": "Badge"})));
    let badge = idv(&ex(&mut s, "component.create", json!({"ids": [g]})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": panel, "x": 400, "y": 0})));
    assert_eq!(component_code(&mut s, i, "react")["usage"], "<Panel />\n", "the main's content isn't passed");
    assert_eq!(component_code(&mut s, i, "flutter")["usage"], "Panel()\n", "the main's content isn't passed");
    ex(&mut s, "instance.slotInsert", json!({"id": i, "slot": "Content", "component": badge, "replace": true}));
    let usage = component_code(&mut s, i, "react")["usage"].as_str().unwrap_or_default().to_string();
    assert!(usage.starts_with("<Panel content={(\n") && usage.contains("<Badge />"), "{usage}");
    let usage = component_code(&mut s, i, "leptos")["usage"].as_str().unwrap_or_default().to_string();
    assert!(usage.starts_with("<Panel content={|| view! {\n") && usage.contains("<Badge />") && usage.ends_with("}} />\n"), "{usage}");
    let usage = component_code(&mut s, i, "flutter")["usage"].as_str().unwrap_or_default().to_string();
    assert!(usage.starts_with("Panel(content: [\n") && usage.contains("Badge()"), "{usage}");
    let usage = component_code(&mut s, i, "arkui")["usage"].as_str().unwrap_or_default().to_string();
    assert!(usage.starts_with("Panel({ content: () => {\n") && usage.contains("Badge()") && usage.ends_with("} })\n"), "{usage}");
}

#[test]
fn identifiers_are_escaped_and_unique() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "Text"})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    for (name, ty, default) in [
        ("default", "boolean", json!(false)),
        ("Show icon?", "boolean", json!(true)),
        ("Label", "text", json!("a")),
        ("label", "text", json!("b")),
        ("2 columns", "boolean", json!(false)),
        ("Color", "variant", json!("Red")),
        ("type", "text", json!("c")),
        ("Props", "variant", json!("Self")),
    ] {
        ex(&mut s, "component.addProperty", json!({"id": c, "name": name, "type": ty, "default": default}));
    }
    let doc = s.doc().unwrap();
    let comp = describe(doc, c).unwrap();
    assert_eq!(comp.type_name, "TextComponent", "never shadows SwiftUI's Text");
    let idents: Vec<(&str, &str)> = comp.params.iter().map(|p| (p.prop.as_str(), p.ident.as_str())).collect();
    for want in [("default", "defaultValue"), ("Show icon?", "showIcon"), ("2 columns", "p2Columns"), ("Color", "color")] {
        assert!(idents.contains(&want), "{want:?} in {idents:?}");
    }
    let labels: Vec<&str> = comp.params.iter().filter(|p| p.prop.eq_ignore_ascii_case("label")).map(|p| p.ident.as_str()).collect();
    assert_eq!(labels.len(), 2);
    assert_ne!(labels[0], labels[1], "unique: {labels:?}");
    let color = comp.params.iter().find(|p| p.prop == "Color").map(|p| p.enum_name.clone());
    assert_eq!(color.as_deref(), Some("ColorVariant"), "never shadows SwiftUI's Color");
    let swift = code_for(doc, c, CodeLang::SwiftUi);
    assert!(swift.contains("struct TextComponent: View {") && swift.contains("enum ColorVariant: String, CaseIterable {"), "{swift}");
    // Rust: snake_case, keywords suffixed, never the props struct's name or `Self`.
    let leptos = code_for(doc, c, CodeLang::Leptos);
    for want in [
        "pub fn TextComponent(",
        "] default_value: bool,",
        "] show_icon: bool,",
        "] p2_columns: bool,",
        "] type_: String,",
        "pub enum TextComponentPropsVariant {",
        "    SelfValue,",
        "Self::SelfValue => \"Self\",",
    ] {
        assert!(leptos.contains(want), "Leptos has {want:?}:\n{leptos}");
    }
    // Dart: lowerCamel members, keywords suffixed, never a type the widget tree names.
    let dart = code_for(doc, c, CodeLang::Flutter);
    for want in [
        "class TextComponent extends StatelessWidget {",
        "enum TextComponentColorVariant { red }\n",
        "    this.defaultValue = false,",
        "    this.p2Columns = false,",
        "  final String type;",
    ] {
        assert!(dart.contains(want), "Flutter has {want:?}:\n{dart}");
    }
    // Errors.
    let r = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    assert!(s.execute("inspect.componentCode", &json!({"id": r})).is_err(), "not a component");
    assert!(s.execute("inspect.componentCode", &json!({"id": c, "language": "html"})).is_err());
    assert!(s.execute("inspect.componentCode", &json!({"id": c, "language": "flutter"})).is_ok());
    assert!(s.execute("inspect.componentCode", &json!({"id": 999_999})).is_err());
}

#[test]
fn bound_variables_become_custom_properties() {
    let mut s = Session::new();
    let (c, _, _) = card(&mut s);
    ex(&mut s, "variable.createCollection", json!({"name": "Tokens"}));
    ex(&mut s, "variable.create", json!({"collection": "Tokens", "name": "Colors/Brand 500", "type": "color", "value": "#3366FF"}));
    ex(&mut s, "variable.bind", json!({"ids": [c], "field": "fill", "variable": "Colors/Brand 500"}));
    let css = code(&mut s, c, "css");
    assert!(css.contains("background: var(--colors-brand-500, #3366FF);"), "{css}");
    let react = code(&mut s, c, "react");
    assert!(react.contains(":root {\n  --colors-brand-500: #3366FF;\n}"), "{react}");
}

#[test]
fn case_colliding_variant_properties_get_distinct_data_attributes() {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, (first, second, fill)) in [("Small", "Big", "#0000FF"), ("Large", "Small", "#FF0000")].iter().enumerate() {
        let name = format!("Size={first}, size={second}");
        let f = idv(&ex(
            &mut s,
            "frame.create",
            json!({"x": k as f64 * 150.0, "y": 0, "width": 100, "height": 40, "name": name, "props": {"fill": fill}}),
        ));
        variants.push(idv(&ex(&mut s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": variants})));
    ex(&mut s, "node.set", json!({"id": set, "props": {"name": "Chip"}}));
    for lang in ["react", "leptos"] {
        let v = component_code(&mut s, set, lang);
        let def = v["definition"].as_str().unwrap_or_default();
        let root = def.lines().find(|l| l.trim_start().starts_with("<div ")).unwrap_or_default();
        let attrs: Vec<&str> =
            root.split_whitespace().filter_map(|w| w.strip_prefix("data-")).map(|w| w.split('=').next().unwrap_or_default()).collect();
        assert_eq!(attrs.len(), 2, "{lang}: {root}");
        assert_ne!(attrs[0], attrs[1], "{lang}: distinct attributes in {root}");
        let sel = format!(".chip[data-{}=\"Large\"][data-{}=\"Small\"]", attrs[0], attrs[1]);
        assert!(def.contains(&sel), "{lang}: {sel} in:\n{def}");
        assert!(def.contains("#FF0000"), "{lang}:\n{def}");
    }
}
