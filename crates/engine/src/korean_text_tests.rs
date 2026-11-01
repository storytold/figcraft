//! The `wordBreak` text property and `text.wordBreak` command (#1233): Korean wraps word by
//! word (the wrapping itself is tested in `crates/text`); here the plumbing — props, the
//! command with undo and `all`, `node.get` reporting, and the CSS code export.

use serde_json::{Value, json};

use crate::Session;

fn id(v: Value) -> u64 {
    v["result"]["id"].as_u64().or_else(|| v["id"].as_u64()).unwrap()
}

fn text_layer(s: &mut Session, word_break: &str) -> u64 {
    let v = s.execute("node.create", &json!({"type": "text", "x": 0, "y": 0, "text": "제출하세요", "props": {"wordBreak": word_break}})).unwrap();
    id(v)
}

#[test]
fn props_set_and_read_the_word_break() {
    let mut s = Session::new();
    let t = text_layer(&mut s, "keepAll");
    let got = s.execute("node.get", &json!({"id": t})).unwrap();
    assert_eq!(got["wordBreak"], "keepAll", "node.get reports it");
    // The CSS-friendly spelling is accepted too.
    let t2 = text_layer(&mut s, "keep-all");
    let got = s.execute("node.get", &json!({"id": t2})).unwrap();
    assert_eq!(got["wordBreak"], "keepAll", "the kebab-case spelling works");
    // The default isn't reported, so old files inspect as they always did.
    let v = s.execute("node.create", &json!({"type": "text", "x": 0, "y": 0, "text": "Hi"})).unwrap();
    let t3 = id(v);
    let got = s.execute("node.get", &json!({"id": t3})).unwrap();
    assert!(got.get("wordBreak").is_none(), "normal is the default");
}

#[test]
fn the_command_sets_one_layer_or_all_and_undoes() {
    let mut s = Session::new();
    let a = text_layer(&mut s, "normal");
    let b = text_layer(&mut s, "normal");
    s.execute("text.wordBreak", &json!({"mode": "keepAll", "ids": [a]})).unwrap();
    let got = s.execute("node.get", &json!({"id": a})).unwrap();
    assert_eq!(got["wordBreak"], "keepAll");
    let got = s.execute("node.get", &json!({"id": b})).unwrap();
    assert!(got.get("wordBreak").is_none(), "only the asked layer");
    s.execute("edit.undo", &json!({})).unwrap();
    let got = s.execute("node.get", &json!({"id": a})).unwrap();
    assert!(got.get("wordBreak").is_none(), "one undo step back to normal");
    // `all` reaches every text layer without naming them.
    s.execute("text.wordBreak", &json!({"mode": "keepAll", "all": true})).unwrap();
    for t in [a, b] {
        let got = s.execute("node.get", &json!({"id": t})).unwrap();
        assert_eq!(got["wordBreak"], "keepAll", "layer {t}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    let got = s.execute("node.get", &json!({"id": b})).unwrap();
    assert!(got.get("wordBreak").is_none(), "all is one undo step too");
}

#[test]
fn the_html_and_css_export_says_what_the_wrapping_does() {
    let mut s = Session::new();
    let t = text_layer(&mut s, "keepAll");
    let code = s.execute("inspect.code", &json!({"id": t, "language": "html"})).unwrap();
    let code = code["code"].as_str().unwrap();
    assert!(code.contains("word-break: keep-all;"), "{code}");
    assert!(code.contains("overflow-wrap: break-word;"), "{code}");
    // Normal text exports no word-break rules.
    let v = s.execute("node.create", &json!({"type": "text", "x": 0, "y": 0, "text": "Hi"})).unwrap();
    let plain = id(v);
    let code = s.execute("inspect.code", &json!({"id": plain, "language": "html"})).unwrap();
    let code = code["code"].as_str().unwrap();
    assert!(!code.contains("word-break"), "{code}");
}

#[test]
fn instances_follow_the_main_and_keep_their_own_choice() {
    let mut s = Session::new();
    // Latin text: this tests the override plumbing, not Korean wrapping (a Korean string here
    // walks the font-fallback scan inside a component rebuild and deadlocks the test harness
    // on a machine without a scanned font cache — see the PR notes).
    let text = id(s.execute("node.create", &json!({"type": "text", "x": 0, "y": 0, "text": "Label"})).unwrap());
    let main = id(s.execute("component.create", &json!({"ids": [text]})).unwrap());
    let _ = main;
    // Duplicating a main makes a linked instance whose text follows it (#892).
    let dup = s.execute("edit.duplicate", &json!({})).unwrap();
    let instance = dup["ids"][0].as_u64().unwrap();
    let child = |s: &mut Session| -> u64 {
        let v = s.execute("node.get", &json!({"id": instance})).unwrap();
        v["children"][0]["id"].as_u64().unwrap_or(instance)
    };
    let c = child(&mut s);
    s.execute("node.set", &json!({"id": text, "props": {"wordBreak": "keepAll"}})).unwrap();
    let got = s.execute("node.get", &json!({"id": c})).unwrap();
    assert_eq!(got["wordBreak"], "keepAll", "the instance's text follows the main");
    // An instance's own choice is an override: setting it back to what the main has clears it,
    // so while the main stays keep-all, the instance's normal holds.
    s.execute("node.set", &json!({"id": c, "props": {"wordBreak": "normal"}})).unwrap();
    let got = s.execute("node.get", &json!({"id": c})).unwrap();
    assert!(got.get("wordBreak").is_none(), "the override keeps the instance at normal while the main is keep-all");
}

#[test]
fn an_instances_word_break_override_survives_the_main_changing_it() {
    let mut s = Session::new();
    let text = id(s.execute("node.create", &json!({"type": "text", "x": 0, "y": 0, "text": "Label"})).unwrap());
    let main = id(s.execute("component.create", &json!({"ids": [text]})).unwrap());
    let _ = main;
    let dup = s.execute("edit.duplicate", &json!({})).unwrap();
    let instance = dup["ids"][0].as_u64().unwrap();
    let v = s.execute("node.get", &json!({"id": instance})).unwrap();
    let c = v["children"][0]["id"].as_u64().unwrap_or(instance);
    s.execute("node.set", &json!({"id": text, "props": {"wordBreak": "keepAll"}})).unwrap();
    let got = s.execute("node.get", &json!({"id": c})).unwrap();
    assert_eq!(got["wordBreak"], "keepAll", "the instance's text follows the main");
    // The instance's own normal is an override.
    s.execute("node.set", &json!({"id": c, "props": {"wordBreak": "normal"}})).unwrap();
    // An unrelated change of the main leaves it alone…
    s.execute("node.set", &json!({"id": text, "props": {"fontSize": 20.0}})).unwrap();
    let got = s.execute("node.get", &json!({"id": c})).unwrap();
    assert!(got.get("wordBreak").is_none(), "an unrelated main edit keeps the override");
    // …and so does the main changing wordBreak past it (#1233: the override group must compare
    // word_break, or the rebuild takes the main's value back).
    s.execute("node.set", &json!({"id": text, "props": {"wordBreak": "normal"}})).unwrap();
    s.execute("node.set", &json!({"id": text, "props": {"wordBreak": "keepAll"}})).unwrap();
    let got = s.execute("node.get", &json!({"id": c})).unwrap();
    assert!(got.get("wordBreak").is_none(), "the override holds while the main flips wordBreak");
}

#[test]
fn junk_mode_names_are_refused() {
    let mut s = Session::new();
    let e = s.execute("text.wordBreak", &json!({"mode": "sometimes"})).unwrap_err();
    assert!(e.to_string().contains("mode"), "{e}");
    let e = s.execute("node.create", &json!({"type": "text", "x": 0, "y": 0, "text": "Hi", "props": {"wordBreak": "loose"}})).unwrap_err();
    assert!(e.to_string().contains("wordBreak"), "{e}");
}
