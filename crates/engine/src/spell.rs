//! Spell check: the word list and which words it doesn't know. The checker draws squiggles in
//! the UI while text is being edited (Text ▸ Spell check turns it on); it never blocks typing.

use std::collections::HashSet;
use std::sync::OnceLock;

/// The word list the checker knows: the system's `/usr/share/dict/words`, lowercased, ASCII
/// only, bounded in size, entries and word length. Absent (as on some Linux installs and in the
/// browser) the checker is off and nothing is underlined.
#[cfg(not(target_arch = "wasm32"))]
pub fn dictionary() -> Option<&'static HashSet<String>> {
    static DICT: OnceLock<Option<HashSet<String>>> = OnceLock::new();
    DICT.get_or_init(|| {
        let bytes = crate::file_access::read_capped("/usr/share/dict/words", 16 << 20).ok()?;
        let raw = String::from_utf8_lossy(&bytes);
        let mut words = HashSet::new();
        for line in raw.lines().take(2_000_000) {
            if line.is_empty() || line.len() > 64 || !line.is_ascii() {
                continue;
            }
            words.insert(line.to_lowercase());
        }
        if words.is_empty() { None } else { Some(words) }
    })
    .as_ref()
}

/// No word list in the browser: the checker is off.
#[cfg(target_arch = "wasm32")]
pub fn dictionary() -> Option<&'static HashSet<String>> {
    None
}

/// Whether `word` is misspelled: the checker is on, the word list is present, and the word
/// (letters only, lowercased) isn't in it. Words with digits, marks or other scripts are
/// skipped — this checker only judges simple ASCII letter words.
pub fn misspelled(on: bool, word: &str) -> bool {
    if !on {
        return false;
    }
    let Some(dict) = dictionary() else { return false };
    let lower = word.to_lowercase();
    if lower.is_empty() || !lower.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    !dict.contains(&lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn misspelled_needs_the_checker_on_and_a_dictionary() {
        // Whatever the dictionary's presence, the off checker flags nothing and never panics.
        assert!(!misspelled(false, "teh"));
    }
}
