//! The handoff mode is named "Developer Mode" in every label a person reads. Figma holds a US
//! registration for DEV MODE (US 7574348) and sent a cease-and-desist over it in 2025, so neither
//! our labels nor a translation may call it that again.
//!
//! Identifiers keep their names (`view.devMode`, `devMode.*`, `dev_mode`, the module and file
//! names), and the Actions palette keeps the old name as a search alias so people who know it still
//! find the command (`palette_tests::the_old_name_still_finds_the_mode`).
//!
//! The labels are read where they are published: the engine's command labels and parameter docs,
//! the menu catalog, and the baked-in catalogs. A `tr("Dev Mode")` added to the code would need an
//! English entry, which this rejects.

use figcraft_engine::catalog::MENU;

use crate::i18n::{LANGS, Text};

/// The retired name, as a label would read it.
const RETIRED: &str = "Dev Mode";

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `text` reads the retired name: whole words, so "Developer Mode" and "devMode" don't.
fn retired(text: &str) -> bool {
    text.match_indices(RETIRED)
        .any(|(at, _)| !text[..at].chars().next_back().is_some_and(is_word) && !text[at + RETIRED.len()..].chars().next().is_some_and(is_word))
}

/// The strings one catalog entry shows.
fn entry_texts(text: &Text) -> Vec<&str> {
    match text {
        Text::Plain(value) => vec![*value],
        Text::Plural(forms) => [forms.one, forms.few, forms.many].into_iter().flatten().chain([forms.other]).collect(),
    }
}

/// Every label the app publishes, with where it comes from.
fn labels() -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> =
        figcraft_engine::all_commands().iter().flat_map(|c| [(format!("{} label", c.id), c.label), (format!("{} params", c.id), c.params)]).collect();
    out.extend(MENU.iter().map(|m| (format!("menu {}", m.id), m.label)));
    out
}

#[test]
fn command_labels_and_parameter_docs_read_developer_mode() {
    let bad: Vec<String> = labels().into_iter().filter(|(_, text)| retired(text)).map(|(what, text)| format!("{what}: {text:?}")).collect();
    assert!(bad.is_empty(), "a label still names the retired mode:\n{}", bad.join("\n"));
    let id = figcraft_engine::find_command("view.devMode").expect("the mode's command exists");
    assert_eq!(id.label, "Developer Mode", "the mode's command label");
}

#[test]
fn the_catalogs_name_developer_mode() {
    let mut bad = Vec::new();
    for lang in LANGS {
        for (key, text) in lang.ui {
            if retired(key) {
                bad.push(format!("{}: key {key:?}", lang.code));
            }
            for value in entry_texts(text).into_iter().filter(|value| retired(value)) {
                bad.push(format!("{}: {key:?} = {value:?}", lang.code));
            }
        }
        for (id, text) in lang.commands {
            for value in entry_texts(text).into_iter().filter(|value| retired(value)) {
                bad.push(format!("{}: {id} = {value:?}", lang.code));
            }
        }
        for (id, text) in lang.menu {
            for value in entry_texts(text).into_iter().filter(|value| retired(value)) {
                bad.push(format!("{}: {id} = {value:?}", lang.code));
            }
        }
    }
    assert!(bad.is_empty(), "a catalog still names the retired mode:\n{}", bad.join("\n"));
}

#[test]
fn the_inspect_tab_and_its_refusal_read_developer_mode() {
    assert_eq!(crate::app::right_tab_label(crate::RightTab::Inspect), "Developer Mode", "the panel's name");
    // The engine's refusal on a board, and the toast the UI shows where the mode refuses an edit.
    let board = figcraft_engine::board::NOT_ON_BOARDS;
    assert!(!retired(board), "{board:?}");
    assert_eq!(crate::i18n::LANGS.first().map(|l| l.code), Some("en"), "English is baked in first");
    assert!(
        LANGS.first().is_some_and(|l| l.ui.iter().any(|(k, _)| *k == "Developer Mode only inspects: switch to Design (Shift+D) to edit")),
        "the refusal toast is in the catalog"
    );
}

#[test]
fn the_guard_reads_words_not_identifiers() {
    for label in ["Dev Mode", "Dev Mode on/off", "the Dev Mode panel"] {
        assert!(retired(label), "{label}");
    }
    for label in ["Developer Mode", "Developer Mode on/off", "view.devMode", "devMode.addMeasurement", "Development", "Ready for dev"] {
        assert!(!retired(label), "{label}");
    }
}
