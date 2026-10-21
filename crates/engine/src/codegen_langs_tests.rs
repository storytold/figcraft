//! The code the Inspect panel shows in the languages besides CSS, HTML, React and Leptos:
//! Tailwind, SCSS, Vue, Angular, Svelte, styled-components, UIKit, Android XML and Flutter.

use crate::Session;
use crate::codegen::{CodeLang, code_for};
use crate::params::id_of;
use serde_json::json;

/// A frame with a text and an ellipse inside, on auto layout.
fn card(s: &mut Session) -> figcraft_doc::NodeId {
    let f = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})).unwrap()["id"]).unwrap();
    s.execute("node.set", &json!({"id": f, "props": {"fills": ["#FFFFFF"], "cornerRadius": 12}})).unwrap();
    let t = id_of(&s.execute("text.create", &json!({"text": "Say \"hi\"", "x": 0, "y": 0, "parent": f})).unwrap()["id"]).unwrap();
    s.execute("node.set", &json!({"id": t, "props": {"fontSize": 16, "fontWeight": 600, "fills": ["#111111"]}})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 24, "height": 24, "parent": f})).unwrap();
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    f
}

#[test]
fn tailwind_turns_the_declarations_into_utility_classes() {
    let mut s = Session::new();
    let f = card(&mut s);
    let doc = s.doc().unwrap();
    let code = code_for(doc, f, CodeLang::Tailwind);
    assert!(code.contains("className=\"relative flex flex-row"), "{code}");
    assert!(code.contains("rounded-[12px]"), "{code}");
    assert!(code.contains("bg-[#FFFFFF]"), "{code}");
    assert!(code.contains("text-[#111111]"), "{code}");
    assert!(code.contains("font-semibold"), "{code}");
    assert!(code.contains("rounded-full"), "an ellipse is round: {code}");
    assert!(!code.contains("<style"), "Tailwind has no stylesheet: {code}");
    // A zero gap is no class.
    assert!(!code.contains("gap-[0px]"), "{code}");
}

#[test]
fn scss_nests_the_rules_as_the_layers_are() {
    let mut s = Session::new();
    let f = card(&mut s);
    let doc = s.doc().unwrap();
    let code = code_for(doc, f, CodeLang::Scss);
    assert!(code.contains(".card {"), "{code}");
    assert!(code.contains("  .say-hi {"), "the child's rule is nested: {code}");
    assert!(code.contains("    font-size: 16px;"), "{code}");
}

#[test]
fn vue_angular_and_svelte_are_single_file_components() {
    let mut s = Session::new();
    let f = card(&mut s);
    let doc = s.doc().unwrap();
    let vue = code_for(doc, f, CodeLang::Vue);
    assert!(vue.starts_with("<template>"), "{vue}");
    assert!(vue.contains("<style scoped>"), "{vue}");
    assert!(vue.contains("class=\"card\""), "{vue}");
    let angular = code_for(doc, f, CodeLang::Angular);
    assert!(angular.contains("import { Component } from \"@angular/core\";"), "{angular}");
    assert!(angular.contains("selector: \"app-card\""), "{angular}");
    assert!(angular.contains("export class CardComponent {}"), "{angular}");
    let svelte = code_for(doc, f, CodeLang::Svelte);
    assert!(svelte.contains("<script lang=\"ts\">"), "{svelte}");
    assert!(svelte.contains("<style>"), "{svelte}");
}

#[test]
fn styled_components_name_every_layer() {
    let mut s = Session::new();
    let f = card(&mut s);
    let doc = s.doc().unwrap();
    let code = code_for(doc, f, CodeLang::StyledComponents);
    assert!(code.contains("import styled from \"styled-components\";"), "{code}");
    assert!(code.contains("export const Card = styled.div`"), "{code}");
    assert!(code.contains("const SayHi = styled.p`"), "{code}");
    assert!(code.contains("<Card>"), "{code}");
}

#[test]
fn uikit_android_xml_and_flutter_have_native_code() {
    let mut s = Session::new();
    let f = card(&mut s);
    let doc = s.doc().unwrap();
    let uikit = code_for(doc, f, CodeLang::SwiftUiKit);
    assert!(uikit.starts_with("import UIKit"), "{uikit}");
    assert!(uikit.contains("UIView(frame: CGRect("), "{uikit}");
    assert!(uikit.contains("card.layer.cornerRadius = 12"), "{uikit}");
    assert!(uikit.contains("card.addSubview("), "{uikit}");
    let xml = code_for(doc, f, CodeLang::AndroidXml);
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"), "{xml}");
    assert!(xml.contains("<LinearLayout"), "{xml}");
    assert!(xml.contains("android:textSize=\"16sp\""), "{xml}");
    assert_eq!(xml.matches("xmlns:android").count(), 1, "the namespace is only on the root: {xml}");
    let flutter = code_for(doc, f, CodeLang::Flutter);
    assert!(flutter.contains("Container("), "{flutter}");
    assert!(flutter.contains("BorderRadius.circular(12)"), "{flutter}");
    assert!(flutter.contains("MainAxisAlignment.start"), "{flutter}");
}

#[test]
fn every_language_has_a_name_a_command_takes_and_its_own_output() {
    let mut s = Session::new();
    let f = card(&mut s);
    let doc = s.doc().unwrap().clone();
    for lang in CodeLang::ALL {
        assert!(!lang.label().is_empty());
        assert_eq!(CodeLang::from_id(lang.id()), Some(lang), "{}", lang.id());
        let code = code_for(&doc, f, lang);
        assert!(!code.is_empty(), "{} has no code", lang.id());
        // The command takes the same names.
        let v = s.execute("inspect.code", &json!({"id": f, "language": lang.id()})).unwrap();
        assert_eq!(v["code"].as_str().unwrap(), code, "{}", lang.id());
        assert_eq!(v["language"].as_str().unwrap(), lang.id());
    }
}

#[test]
fn the_aliases_the_developers_type_are_accepted() {
    assert_eq!(CodeLang::from_id("tsx"), Some(CodeLang::React));
    assert_eq!(CodeLang::from_id("Tailwind CSS"), Some(CodeLang::Tailwind));
    assert_eq!(CodeLang::from_id("tw"), Some(CodeLang::Tailwind));
    assert_eq!(CodeLang::from_id("dart"), Some(CodeLang::Flutter));
    assert_eq!(CodeLang::from_id("uikit"), Some(CodeLang::SwiftUiKit));
    assert_eq!(CodeLang::from_id("xml"), Some(CodeLang::AndroidXml));
    assert_eq!(CodeLang::from_id("styled-components"), Some(CodeLang::StyledComponents));
    assert_eq!(CodeLang::from_id("cobol"), None);
}

#[test]
fn the_web_languages_can_be_shown_in_rem() {
    let mut s = Session::new();
    let f = card(&mut s);
    for lang in [CodeLang::Css, CodeLang::Tailwind, CodeLang::Scss, CodeLang::Html, CodeLang::Vue] {
        assert!(lang.is_web(), "{}", lang.id());
    }
    assert!(!CodeLang::SwiftUi.is_web());
    let v = s.execute("inspect.code", &json!({"id": f, "language": "css", "unit": "rem", "rootFontSize": 16})).unwrap();
    assert!(v["code"].as_str().unwrap().contains("rem"), "{}", v["code"]);
}
