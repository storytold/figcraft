//! Code blocks (#2237): the languages, the saved item, the height rule and the colours.

use serde_json::json;

use crate::BoardItem;
use crate::board_code::{CODE_MIN_H, CODE_PAD, CODE_PANEL, CODE_TEXT, CodeLanguage, code_height};

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
