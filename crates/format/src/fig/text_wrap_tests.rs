//! The line breaks a `.fig` file saved with its text (`derivedTextData`) come across as the text's
//! saved breaks, on messages built in code.

use figcraft_doc::{Document, TextData};

use super::kiwi::Kv;

type Fields = Vec<(&'static str, Kv<'static>)>;

fn object(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn guid(id: u64) -> Kv<'static> {
    object(vec![("sessionID", Kv::Uint(9)), ("localID", Kv::Uint(id))])
}

fn node(id: u64, ty: &'static str, name: &str, parent: u64, mut more: Fields) -> Kv<'static> {
    let mut fields = vec![
        ("guid", guid(id)),
        ("type", Kv::Enum(ty)),
        ("name", Kv::Str(name.into())),
        ("parentIndex", object(vec![("guid", guid(parent)), ("position", Kv::Str("a".into()))])),
    ];
    fields.append(&mut more);
    object(fields)
}

/// The saved layout: one baseline per line, starting at these UTF-16 offsets.
fn derived(firsts: &[u64]) -> Kv<'static> {
    let lines = firsts.iter().map(|f| object(vec![("firstCharacter", Kv::Uint(*f))])).collect();
    object(vec![("baselines", Kv::Array(lines))])
}

/// A 100 × 40 text layer named `name` with `chars`, sizing `resize` and the saved layout `firsts`.
fn label(id: u64, name: &str, parent: u64, chars: &str, resize: &'static str, firsts: &[u64]) -> Kv<'static> {
    node(
        id,
        "TEXT",
        name,
        parent,
        vec![
            ("size", object(vec![("x", Kv::Float(100.0)), ("y", Kv::Float(40.0))])),
            ("fontSize", Kv::Float(14.0)),
            ("textAutoResize", Kv::Enum(resize)),
            ("textData", object(vec![("characters", Kv::Str(chars.into()))])),
            ("derivedTextData", derived(firsts)),
        ],
    )
}

fn import(nodes: Vec<Kv<'static>>) -> Document {
    let page = vec![
        object(vec![("guid", guid(1)), ("type", Kv::Enum("DOCUMENT")), ("name", Kv::Str("Document".into()))]),
        node(2, "CANVAS", "Page", 1, vec![]),
    ];
    let message = object(vec![("nodeChanges", Kv::Array(page.into_iter().chain(nodes).collect()))]);
    let value = super::map::to_rest(&message, "Synthetic wrap", crate::rest::MAX_NODES).0;
    crate::rest::import_fig(&value, &|_| None).unwrap().0
}

fn texts<'d>(doc: &'d Document, name: &str) -> Vec<&'d TextData> {
    doc.nodes().filter(|n| n.name == name).filter_map(|n| n.kind.text()).collect()
}

#[test]
fn saved_line_starts_come_across_as_chars_without_hard_breaks() {
    // UTF-16: the emoji takes two units, so "c" is unit 6 but char 5; "e" follows a hard break.
    let doc = import(vec![
        label(10, "Wrapped", 2, "😀 ab cd\nef", "HEIGHT", &[0, 6, 9]),
        label(11, "Unwrapped", 2, "Followers", "NONE", &[0]),
        label(12, "Auto width", 2, "ab cd", "WIDTH_AND_HEIGHT", &[0, 3]),
        label(13, "Inside a pair", 2, "😀 ab", "HEIGHT", &[0, 1]),
        label(14, "Out of order", 2, "ab cd ef", "HEIGHT", &[0, 6, 3]),
        label(15, "Not from the start", 2, "ab cd", "HEIGHT", &[3]),
    ]);
    let wrapped = texts(&doc, "Wrapped")[0];
    assert_eq!(wrapped.saved_breaks_for(100.0), Some(&[5][..]));
    assert_eq!(texts(&doc, "Unwrapped")[0].saved_breaks_for(100.0), Some(&[][..]), "one line: no wrapped start");
    for name in ["Auto width", "Inside a pair", "Out of order", "Not from the start"] {
        assert_eq!(texts(&doc, name)[0].saved_breaks, None, "{name}");
    }
}

#[test]
fn a_saved_layout_for_other_text_is_left_out() {
    let row = object(vec![
        ("guidPath", object(vec![("guids", Kv::Array(vec![guid(21)]))])),
        ("textData", object(vec![("characters", Kv::Str("Something much longer".into()))])),
    ]);
    let doc = import(vec![
        node(20, "SYMBOL", "Main", 2, vec![]),
        label(21, "Label", 20, "Hi there", "HEIGHT", &[0, 3]),
        node(30, "INSTANCE", "Inst", 2, vec![("symbolData", object(vec![("symbolID", guid(20)), ("symbolOverrides", Kv::Array(vec![row]))]))]),
    ]);
    let labels = texts(&doc, "Label");
    let main = labels.iter().find(|t| t.characters == "Hi there").unwrap();
    assert_eq!(main.saved_breaks_for(100.0), Some(&[3][..]));
    // The instance's text comes from its override, the saved layout only from the main.
    let inst = labels.iter().find(|t| t.characters == "Something much longer").unwrap();
    assert_eq!(inst.saved_breaks, None);
}
