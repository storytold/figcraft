//! Code blocks (#2237): the languages, the saved item, the height rule and the colours.

use serde_json::json;

use crate::BoardItem;
use crate::board_code::{CODE_MIN_H, CODE_PAD, CODE_PANEL, CODE_TEXT, CodeLanguage, CodeToken, code_height, code_tokens};

#[test]
fn sixteen_languages_by_name_with_typescript_the_default() {
    assert_eq!(CodeLanguage::ALL.len(), 16);
    let ids: std::collections::BTreeSet<&str> = CodeLanguage::ALL.iter().map(|l| l.id()).collect();
    assert_eq!(ids.len(), 16, "ids are unique");
    let labels: Vec<String> = CodeLanguage::ALL.iter().map(|l| l.label().to_lowercase()).collect();
    assert!(labels.windows(2).all(|w| w[0] < w[1]), "labels ascend: {labels:?}");
    for l in CodeLanguage::ALL {
        assert_eq!(CodeLanguage::from_id(l.id()), Some(l));
        assert_eq!(CodeLanguage::from_id(l.label()), Some(l));
        assert_eq!(serde_json::to_value(l).unwrap(), json!(l.id()));
    }
    assert_eq!(CodeLanguage::from_id("CPP"), Some(CodeLanguage::Cpp));
    assert_eq!(CodeLanguage::from_id("c++"), Some(CodeLanguage::Cpp));
    assert_eq!(CodeLanguage::from_id(" plain text "), Some(CodeLanguage::PlainText));
    assert_eq!(CodeLanguage::from_id("markdown"), None);
    assert_eq!(CodeLanguage::from_id(""), None);
    assert_eq!(CodeLanguage::default(), CodeLanguage::Typescript);
}

#[test]
fn a_code_block_item_saves_its_language() {
    let item = BoardItem::CodeBlock { language: CodeLanguage::Rust };
    let v = json!({"type": "codeBlock", "language": "rust"});
    assert_eq!(serde_json::to_value(&item).unwrap(), v);
    assert_eq!(serde_json::from_value::<BoardItem>(v).unwrap(), item);
    let plain: BoardItem = serde_json::from_value(json!({"type": "codeBlock"})).unwrap();
    assert_eq!(plain, BoardItem::CodeBlock { language: CodeLanguage::Typescript });
    assert!(serde_json::from_value::<BoardItem>(json!({"type": "codeBlock", "language": "klingon"})).is_err());
    assert_eq!(item.kind_id(), "codeBlock");
    assert!(item.is_atom());
    assert!(item.has_text());
}

#[test]
fn the_height_is_32_and_24_a_line_from_four_lines_up() {
    assert_eq!(code_height(0), 128.0);
    assert_eq!(code_height(1), 128.0);
    assert_eq!(code_height(4), 128.0);
    assert_eq!(code_height(5), 152.0);
    assert_eq!(code_height(9), 248.0);
    assert!(code_height(usize::MAX).is_finite());
    assert_eq!(CODE_MIN_H, code_height(0));
    assert_eq!(CODE_PAD[0] + CODE_PAD[2], 32.0);
    assert_eq!(CODE_PAD[1] + CODE_PAD[3], 80.0);
}

#[test]
fn plain_code_reads_on_the_panel() {
    assert!(CODE_TEXT.contrast(CODE_PANEL) >= 4.5, "{}", CODE_TEXT.contrast(CODE_PANEL));
}

/// The tokens of `text` in `language`, as the text they cover.
fn tokens(language: CodeLanguage, text: &str) -> Vec<(String, CodeToken)> {
    code_tokens(language, text).into_iter().map(|(a, b, t)| (text.chars().skip(a).take(b - a).collect(), t)).collect()
}

fn want(v: &[(&str, CodeToken)]) -> Vec<(String, CodeToken)> {
    v.iter().map(|(s, t)| ((*s).to_string(), *t)).collect()
}

#[test]
fn keywords_strings_numbers_and_comments_per_language() {
    use CodeLanguage::*;
    use CodeToken::*;
    // A keyword inside a string or a comment is part of it.
    assert_eq!(
        tokens(
            Rust,
            "fn main() { let s = \"fn\"; // fn here
  x = 4.2; }"
        ),
        want(&[("fn", Keyword), ("let", Keyword), ("\"fn\"", String), ("// fn here", Comment), ("4.2", Number)])
    );
    assert_eq!(tokens(Python, "def f(): # note"), want(&[("def", Keyword), ("# note", Comment)]));
    assert_eq!(tokens(Sql, "SELECT name FROM t -- all"), want(&[("SELECT", Keyword), ("FROM", Keyword), ("-- all", Comment)]));
    // A quoted string ends with its line; a block comment runs over lines.
    assert_eq!(
        tokens(
            Typescript,
            "const a = 'open
let b /* x
y */"
        ),
        want(&[
            ("const", Keyword),
            ("'open", String),
            ("let", Keyword),
            (
                "/* x
y */",
                Comment
            )
        ])
    );
    // Markup: tag names, quotes only inside a tag (an apostrophe in the text is none).
    assert_eq!(
        tokens(Html, "<a href=\"x\">don't</a> <!-- c -->"),
        want(&[("a", Keyword), ("\"x\"", String), ("a", Keyword), ("<!-- c -->", Comment)])
    );
    // `#` starts a comment only at the start of a word.
    assert_eq!(tokens(Bash, "echo $# a#b # c"), want(&[("# c", Comment)]));
    assert!(tokens(PlainText, "fn \"x\" 1 // y").is_empty());
    // Python's `#` needs no space before it; an escaped line end still ends a quoted string.
    assert_eq!(tokens(Python, "x=1# return \"x\""), want(&[("1", Number), ("# return \"x\"", Comment)]));
    assert_eq!(tokens(Json, "\"open\\\ntrue"), want(&[("\"open\\", String), ("true", Keyword)]));
    // An escaped backtick doesn't end a template string; a `#` after a Bash operator starts a comment.
    assert_eq!(
        tokens(Javascript, "const s = `a\\`b`; const n = 1;"),
        want(&[("const", Keyword), ("`a\\`b`", String), ("const", Keyword), ("1", Number)])
    );
    assert_eq!(tokens(Bash, "echo ok;# if true"), want(&[("# if true", Comment)]));
    // Rust: a character literal is a string, and a lifetime's quote opens nothing.
    assert_eq!(
        tokens(Rust, "let q = '\"'; let n = '\\n'; fn f<'a>(x: &'a str) {}"),
        want(&[("let", Keyword), ("'\"'", String), ("let", Keyword), ("'\\n'", String), ("fn", Keyword)])
    );
    // In every language the stretches are in order, apart and inside the text, even unclosed.
    let text = "if (x) { y = \"a\\\"b\" + 'c' + `d` /* e */ // f
# g
-- h
<i j=\"k\"> 1.5 0x1F \"\"\" l";
    let n = text.chars().count();
    for l in CodeLanguage::ALL {
        let v = code_tokens(l, text);
        assert!(v.iter().all(|(a, b, _)| a < b && *b <= n), "{l:?}: {v:?}");
        assert!(v.windows(2).all(|w| w[0].1 <= w[1].0), "{l:?}: {v:?}");
    }
}

#[test]
fn highlight_colours_read_on_the_panel_and_differ() {
    let all = [CodeToken::Keyword, CodeToken::String, CodeToken::Number, CodeToken::Comment];
    for t in all {
        assert!(t.color().contrast(CODE_PANEL) >= 4.5, "{t:?}: {}", t.color().contrast(CODE_PANEL));
        assert_ne!(t.color(), CODE_TEXT, "{t:?}");
    }
    let distinct: std::collections::BTreeSet<String> = all.iter().map(|t| t.color().to_hex()).collect();
    assert_eq!(distinct.len(), all.len());
}
