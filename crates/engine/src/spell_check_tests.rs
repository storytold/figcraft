//! Spell check: the word ranges the checker judges, the preference behind Text ▸ Spell check,
//! and what a missing word list means (the checker is silently off).

use serde_json::json;

use crate::Session;
use crate::canvas::textedit::word_ranges;
use crate::spell;

fn ranges(text: &str) -> Vec<(usize, usize)> {
    word_ranges(&text.chars().collect::<Vec<_>>())
}

#[test]
fn word_ranges_split_on_spaces_and_punctuation() {
    assert_eq!(ranges("two words"), vec![(0, 3), (4, 9)]);
    assert_eq!(ranges("hello, world!"), vec![(0, 5), (7, 12)]);
    // Digits belong to the word beside them (Latin letters and digits share a class).
    assert_eq!(ranges("v2 goes"), vec![(0, 2), (3, 7)]);
}

#[test]
fn word_ranges_respect_char_boundaries() {
    // Emoji, combining marks and CJK count as chars, never split mid-cluster.
    let text = "a👍 b́ cd 漢字";
    let chars: Vec<char> = text.chars().collect();
    for (start, end) in ranges(text) {
        assert!(start < end && end <= chars.len(), "{start}..{end} out of the text");
        assert!(chars.get(start).is_some() && chars.get(end - 1).is_some());
    }
    assert_eq!(ranges("漢字 text"), vec![(0, 2), (3, 7)], "a CJK run is one word of its own");
}

#[test]
fn spell_check_is_a_persisted_preference() {
    let mut s = Session::new();
    let r = s.execute("text.spellCheck", &json!({})).unwrap();
    assert_eq!(r["on"], true, "off by default, so the first toggle turns it on");
    assert_eq!(crate::cmd::view::checked(&s, "text.spellCheck"), Some(true));
    let r = s.execute("text.spellCheck", &json!({"on": false})).unwrap();
    assert_eq!(r["on"], false);
    // The app writes its preferences as JSON and reads them back at the next launch.
    let saved = serde_json::to_value(&s.prefs).unwrap();
    let mut next = Session::empty();
    next.prefs = serde_json::from_value(saved).unwrap();
    assert!(!next.prefs.spell_check);
    next.prefs.spell_check = true;
    assert!(next.prefs.spell_check);
}

#[test]
fn a_missing_word_list_means_off() {
    // Whatever this system has, the off checker flags nothing, and an unknown word is only
    // flagged when a word list is present — never an error, never a panic.
    assert!(!spell::misspelled(false, "teh"));
    let with_dict = spell::dictionary().is_some();
    assert_eq!(spell::misspelled(true, "teh"), with_dict, "a dictionary turns the judgement on");
    // Non-letters are never judged, dictionary or not.
    assert!(!spell::misspelled(true, "123"));
    assert!(!spell::misspelled(true, ""));
    assert!(!spell::misspelled(true, "漢字"));
}
