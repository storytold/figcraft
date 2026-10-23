//! The colours the code in the Inspect panel is shown with: every language's code comes back
//! whole, and the pieces that matter are coloured.

use figcraft_engine::codegen::CodeLang;

use crate::code_highlight::{self, Kind};

/// The code of every language, as the panel generates it.
fn sample(lang: CodeLang) -> String {
    match lang {
        CodeLang::Css => ".card {\n  display: flex;\n  /* a note */\n  color: #FFFFFF;\n}\n".into(),
        CodeLang::Scss => ".card {\n  color: red;\n  .title {\n    font-size: 16px;\n  }\n}\n".into(),
        CodeLang::Html => "<div class=\"card\">\n  <p class=\"title\">Hello</p>\n</div>\n".into(),
        CodeLang::Tailwind => "<div className=\"flex flex-col gap-[8px]\">\n  <p className=\"text-[16px]\">Hello</p>\n</div>\n".into(),
        CodeLang::React => "export function Card() {\n  return <div className=\"card\">Hi</div>;\n}\n".into(),
        CodeLang::StyledComponents => "const Card = styled.div`\n  color: #FFFFFF;\n`;\n".into(),
        CodeLang::Vue => "<template>\n  <div class=\"card\">Hi</div>\n</template>\n\n<style scoped>\n.card {\n  color: red;\n}\n</style>\n".into(),
        CodeLang::Angular => "import { Component } from \"@angular/core\";\n\n@Component({\n  selector: \"app-card\",\n})\nexport class CardComponent {}\n".into(),
        CodeLang::Svelte => "<script lang=\"ts\">\n  let name = \"x\";\n</script>\n\n<div class=\"card\">Hi</div>\n".into(),
        CodeLang::Leptos => "use leptos::prelude::*;\n\n#[component]\npub fn Card() -> impl IntoView {\n    view! { <div class=\"card\">Hi</div> }\n}\n".into(),
        CodeLang::SwiftUi => "Text(\"Hello\")\n    .font(.system(size: 16))\n    .foregroundColor(Color(red: 1, green: 1, blue: 1, opacity: 1))\n".into(),
        CodeLang::SwiftUiKit => "let card = UIView(frame: CGRect(x: 0, y: 0, width: 10, height: 10))\ncard.backgroundColor = UIColor(red: 1, green: 1, blue: 1, alpha: 1)\n".into(),
        CodeLang::Compose => "@Composable\nfun Card() {\n    Text(text = \"Hello\", fontSize = 16.sp)\n}\n".into(),
        CodeLang::AndroidXml => "<LinearLayout\n    android:layout_width=\"wrap_content\">\n    <TextView android:text=\"Hi\" />\n</LinearLayout>\n".into(),
        CodeLang::Flutter => "Container(\n  width: 100,\n  child: Text('Hello'),\n)\n".into(),
    }
}

#[test]
fn every_language_comes_back_whole() {
    for lang in CodeLang::ALL {
        let code = sample(lang);
        let pieces = code_highlight::pieces(&code, lang);
        let whole: String = pieces.iter().map(|(_, t)| *t).collect();
        assert_eq!(whole, code, "{} lost a piece of the code", lang.id());
        assert!(!pieces.is_empty(), "{} has no pieces", lang.id());
    }
}

#[test]
fn the_keywords_the_strings_and_the_comments_are_coloured() {
    let code = "export function Card() {\n  // a note\n  return \"hello\";\n}\n";
    let pieces = code_highlight::pieces(code, CodeLang::React);
    let kind_of = |text: &str| pieces.iter().find(|(_, t)| *t == text).map(|(k, _)| *k);
    assert_eq!(kind_of("export"), Some(Kind::Keyword));
    assert_eq!(kind_of("function"), Some(Kind::Keyword));
    assert_eq!(kind_of("return"), Some(Kind::Keyword));
    assert_eq!(kind_of("// a note"), Some(Kind::Comment));
    assert_eq!(kind_of("\"hello\""), Some(Kind::String));
    assert_eq!(kind_of("Card"), Some(Kind::Function));
}

#[test]
fn the_stylesheets_colour_their_properties_and_their_values() {
    let code = ".card {\n  display: flex;\n  /* a note */\n  color: #FFFFFF;\n}\n";
    let pieces = code_highlight::pieces(code, CodeLang::Css);
    let kind_of = |text: &str| pieces.iter().find(|(_, t)| *t == text).map(|(k, _)| *k);
    assert_eq!(kind_of("display"), Some(Kind::Property));
    assert_eq!(kind_of("color"), Some(Kind::Property));
    assert_eq!(kind_of("#FFFFFF"), Some(Kind::Number));
    assert_eq!(kind_of("/* a note */"), Some(Kind::Comment));
}

#[test]
fn the_markup_colours_its_tags_and_its_attributes() {
    let code = "<div class=\"card\">\n  <p class=\"title\">Hello</p>\n</div>\n";
    let pieces = code_highlight::pieces(code, CodeLang::Html);
    let kind_of = |text: &str| pieces.iter().find(|(_, t)| *t == text).map(|(k, _)| *k);
    assert_eq!(kind_of("div"), Some(Kind::Tag));
    assert_eq!(kind_of("class"), Some(Kind::Attribute));
    assert_eq!(kind_of("\"card\""), Some(Kind::String));
    assert_eq!(kind_of("Hello"), Some(Kind::Plain));
}

#[test]
fn the_curly_brace_languages_know_their_own_keywords() {
    let swift = code_highlight::pieces("func makeCard() -> UIView {\n  return card\n}\n", CodeLang::SwiftUiKit);
    assert!(swift.iter().any(|(k, t)| *k == Kind::Keyword && *t == "func"));
    let kotlin = code_highlight::pieces("@Composable\nfun Card() {}\n", CodeLang::Compose);
    assert!(kotlin.iter().any(|(k, t)| *k == Kind::Keyword && *t == "fun"));
    assert!(kotlin.iter().any(|(k, t)| *k == Kind::Type && *t == "Composable"));
    let rust = code_highlight::pieces("pub fn card() -> impl IntoView {\n    view! {}\n}\n", CodeLang::Leptos);
    assert!(rust.iter().any(|(k, t)| *k == Kind::Keyword && *t == "pub"));
    assert!(rust.iter().any(|(k, t)| *k == Kind::Keyword && *t == "fn"));
    let dart = code_highlight::pieces("Container(width: 100)\n", CodeLang::Flutter);
    assert!(dart.iter().any(|(k, t)| *k == Kind::Function && *t == "Container"));
}

#[test]
fn the_palette_has_a_colour_for_every_kind() {
    for dark in [true, false] {
        let palette = if dark { code_highlight::Palette::dark() } else { code_highlight::Palette::light() };
        for kind in [
            Kind::Plain,
            Kind::Keyword,
            Kind::String,
            Kind::Number,
            Kind::Comment,
            Kind::Function,
            Kind::Type,
            Kind::Tag,
            Kind::Attribute,
            Kind::Property,
            Kind::Variable,
            Kind::Punctuation,
        ] {
            let c = kind.color(&palette);
            assert!(c.a() > 0, "{kind:?} has no colour");
        }
    }
}
