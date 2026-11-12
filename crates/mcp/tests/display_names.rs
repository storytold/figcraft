//! Guard the production display surfaces without banning compatibility identifiers or notices.
use std::path::{Path, PathBuf};

const BANNED: &[&str] = &["Figma-class", "Accessibility at Figma"];
const ALLOWED: &[&str] = &[];
const ROOTS: &[&str] = &["src", "../engine/src/cmd"];

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn banned_display(text: &str) -> bool {
    if ALLOWED.contains(&text) {
        return false;
    }
    BANNED.iter().any(|banned| {
        text.match_indices(banned)
            .any(|(at, _)| !text[..at].chars().next_back().is_some_and(is_word) && !text[at + banned.len()..].chars().next().is_some_and(is_word))
    })
}

// Read ordinary and raw Rust string literals, ignoring comments. No compiler dependency is
// needed, so this guard can also run by itself when a full application build is unavailable.
#[allow(clippy::cognitive_complexity)] // This small test-only lexer keeps raw-string/comment state together.
fn literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    let mut test_module = false;
    let mut test_depth = 0_usize;
    while at < chars.len() {
        if chars.get(at..at + 12) == Some(&['#', '[', 'c', 'f', 'g', '(', 't', 'e', 's', 't', ')', ']']) {
            at += 12;
            let rest: String = chars[at..].iter().collect();
            if rest.trim_start().starts_with("mod ") {
                test_module = true;
            }
        } else if chars.get(at..at + 2) == Some(&['/', '/']) {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
        } else if chars.get(at..at + 2) == Some(&['/', '*']) {
            at += 2;
            let mut depth = 1;
            while at < chars.len() && depth > 0 {
                if chars.get(at..at + 2) == Some(&['/', '*']) {
                    depth += 1;
                    at += 2;
                } else if chars.get(at..at + 2) == Some(&['*', '/']) {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
        } else if chars[at] == 'r' && matches!(chars.get(at + 1), Some('"' | '#')) {
            let start = at;
            at += 1;
            while chars.get(at) == Some(&'#') {
                at += 1;
            }
            let hashes = at - start - 1;
            if chars.get(at) != Some(&'"') {
                continue;
            }
            at += 1;
            let begin = at;
            while at < chars.len() {
                if chars[at] == '"' && (1..=hashes).all(|n| chars.get(at + n) == Some(&'#')) {
                    break;
                }
                at += 1;
            }
            if !test_module {
                out.push(chars[begin..at].iter().collect());
            }
            at += 1 + hashes;
        } else if chars[at] == '"' {
            at += 1;
            let mut text = String::new();
            while at < chars.len() {
                let c = chars[at];
                at += 1;
                if c == '"' {
                    break;
                }
                if c == '\\' && at < chars.len() {
                    text.push(chars[at]);
                    at += 1;
                } else {
                    text.push(c);
                }
            }
            if !test_module {
                out.push(text);
            }
        } else if chars[at] == '\'' && chars.get(at + 2) == Some(&'\'') {
            at += 3; // A character literal such as '"', not the start of a string.
        } else {
            if test_module {
                match chars[at] {
                    '{' => test_depth += 1,
                    '}' => {
                        test_depth = test_depth.saturating_sub(1);
                        if test_depth == 0 {
                            test_module = false;
                        }
                    }
                    ';' if test_depth == 0 => test_module = false,
                    _ => {}
                }
            }
            at += 1;
        }
    }
    out
}

fn check_tree(path: &Path, failures: &mut Vec<String>) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            check_tree(&path, failures)?;
        }
    } else if path.extension().is_some_and(|ext| ext == "rs") {
        let name = path.file_name().ok_or_else(|| std::io::Error::other("source filename missing"))?.to_string_lossy();
        if name == "tests.rs" || name.ends_with("_tests.rs") {
            return Ok(());
        }
        let source = std::fs::read_to_string(path)?;
        // Skip only inline test modules, not later production following test-only imports.
        for text in literals(&source) {
            if banned_display(&text) {
                failures.push(format!("{}: {text:?}", path.display()));
            }
        }
    }
    Ok(())
}

#[test]
fn production_display_names_stay_neutral() -> std::io::Result<()> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut failures = Vec::new();
    for root in ROOTS {
        let root = base.join(root);
        assert!(root.exists(), "missing display source: {}", root.display());
        check_tree(&root, &mut failures)?;
    }
    assert!(failures.is_empty(), "Banned display names:\n{}", failures.join("\n"));
    Ok(())
}

#[test]
fn guard_distinguishes_labels_from_identifiers_and_comments() {
    for name in BANNED {
        assert!(banned_display(name), "{name}");
    }
    for name in ALLOWED {
        assert!(!banned_display(name), "{name}");
    }
    assert!(!banned_display("filter.cameraRaw"));
    assert!(!banned_display("tool.guidedUpright"));
    assert!(!banned_display("insert.smartArt"));
    assert_eq!(literals("// \"ignored\"\n/* \"ignored\" */ \"label\""), ["label"]);
    assert_eq!(literals("r#\"raw label\"#"), ["raw label"]);
}

#[test]
fn guard_scans_production_after_test_only_items() {
    let source = r###"#[cfg(test)] mod tests { let _ = "ignored"; }
        #[cfg(test)] use fixtures::Helper;
        fn production() { let _ = r#"live label"#; }
    "###;
    assert_eq!(literals(source), ["live label"]);
    assert_eq!(literals("#[cfg(test)] mod tests; fn live() { let _ = \"live label\"; }"), ["live label"]);
}

#[test]
fn source_read_failures_are_reported() {
    // A file cannot be traversed as a directory. A failed scan must fail, not silently pass.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/display_names.rs/not-a-directory.rs");
    assert!(check_tree(&path, &mut Vec::new()).is_err());
}
