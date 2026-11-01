//! Where lines may break and where carets may stop: break opportunities (a small subset of UAX #14
//! with the CJK line-start and line-end rules, plus the word boundaries of Thai, Lao, Khmer and
//! Burmese from ICU4X's line segmenter) and grapheme clusters (UAX #29).

use icu_segmenter::options::LineBreakOptions;
use icu_segmenter::{LineSegmenter, LineSegmenterBorrowed};
use unicode_segmentation::UnicodeSegmentation;

/// The break class of a char.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    /// Breakable space: lines break after it (it hangs at the line's end).
    Space,
    /// Ideographs, kana, Hangul and full-width forms: lines may break before and after each.
    Ideo,
    /// Opening brackets and quotes: never end a line.
    Open,
    /// Closing brackets, quotes and punctuation, small kana, the prolonged sound mark and
    /// iteration marks: never start a line.
    Close,
    /// Hyphens, dashes and the slash: lines may break after them.
    Hyphen,
    /// Everything else (letters, digits, other symbols): no break inside a run of them.
    Other,
    /// The figure space: glues what's on either side, ideographs included.
    Glue,
}

/// Opening brackets and quotes (line-end prohibited, JIS X 4051's 行末禁則).
const OPEN: &str = "([{‘“〈《「『【〔〖〘〚〝（［｛｢｟";
/// Closing brackets, quotes and punctuation, iteration marks, small kana, the prolonged sound mark
/// and the (semi-)voiced sound marks (line-start prohibited, JIS X 4051's 行頭禁則). Two blocks
/// are ranges in [`class`] instead: the small katakana extensions U+31F0..U+31FF and the halfwidth
/// small kana U+FF67..U+FF6F plus the halfwidth voicing marks U+FF9E/U+FF9F.
const CLOSE: &str = ")]},.:;!?’”‥…‼⁇⁈⁉、。々〉》」』】〕〗〙〛〜〞〟〻ぁぃぅぇぉっゃゅょゎゕゖゝゞ゠ァィゥェォッャュョヮヵヶ・ーヽヾ！），．：；？］｝｡｣､･ｰ｠゛゜〱〲〳〴〵";

fn class(c: char) -> Class {
    if c.is_ascii_alphanumeric() {
        return Class::Other;
    }
    if OPEN.contains(c) {
        return Class::Open;
    }
    if CLOSE.contains(c) || ('\u{31F0}'..='\u{31FF}').contains(&c) || ('\u{FF67}'..='\u{FF6F}').contains(&c) || matches!(c, '\u{FF9E}' | '\u{FF9F}') {
        return Class::Close;
    }
    match c {
        ' ' | '\t' | '\u{3000}' | '\u{2002}'..='\u{2006}' | '\u{2008}'..='\u{200B}' => Class::Space,
        '\u{2007}' => Class::Glue,
        '-' | '/' | '\u{2010}' | '\u{2013}' | '\u{2014}' => Class::Hyphen,
        '\u{2E80}'..='\u{2FFF}'
        | '\u{3003}'..='\u{33FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{A960}'..='\u{A97F}'
        | '\u{AC00}'..='\u{D7AF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF00}'..='\u{FFEF}'
        | '\u{20000}'..='\u{3FFFF}' => Class::Ideo,
        _ => Class::Other,
    }
}

/// Whether a line may break between `a` and the `b` after it (the break goes before `b`).
pub(crate) fn break_between(a: char, b: char) -> bool {
    use Class::*;
    match (class(a), class(b)) {
        // Spaces hang at the end of the line before them; kinsoku: no closing mark starts a line,
        // no opening mark ends one; glue holds on both sides.
        (_, Space | Close | Glue) | (Open | Glue, _) => false,
        (Space | Hyphen | Ideo, _) | (_, Ideo) => true,
        _ => false,
    }
}

/// Whether char `c` is a breakable space (it hangs at a line's end and doesn't count for its width).
pub(crate) fn is_space(c: char) -> bool {
    class(c) == Class::Space
}

/// Whether `c` is in a script written without spaces between words that the line segmenter has
/// word data for (UAX #14 class SA): Thai, Lao, Burmese (with the Myanmar extensions) and Khmer.
fn writes_no_spaces(c: char) -> bool {
    matches!(c, '\u{0E00}'..='\u{0EFF}' | '\u{1000}'..='\u{109F}' | '\u{1780}'..='\u{17FF}' | '\u{19E0}'..='\u{19FF}' | '\u{A9E0}'..='\u{A9FF}' | '\u{AA60}'..='\u{AA7F}')
}

/// ICU4X's line segmenter with its dictionaries for Thai, Lao, Khmer and Burmese words.
#[cfg(not(target_arch = "wasm32"))]
fn segmenter() -> LineSegmenterBorrowed<'static> {
    LineSegmenter::new_dictionary(LineBreakOptions::default())
}

/// ICU4X's line segmenter with its LSTM models for Thai, Lao, Khmer and Burmese words: a sixth of
/// the dictionaries' size, which the browser's download budget can't take (crates/text/Cargo.toml),
/// though they split a few compounds the dictionaries keep whole.
#[cfg(target_arch = "wasm32")]
fn segmenter() -> LineSegmenterBorrowed<'static> {
    LineSegmenter::new_lstm(LineBreakOptions::default())
}

/// For each char of `chars`, whether a line may break before it because a Thai, Lao, Khmer or
/// Burmese word starts there (#1219). These scripts write no spaces between words, so the words come
/// from the segmenter's word data. Only breaks inside a run of these scripts are marked: where the
/// run meets other text, [`break_between`] decides. Empty when the text has none of them.
pub(crate) fn word_breaks(chars: &[char]) -> Vec<bool> {
    if !chars.iter().copied().any(writes_no_spaces) {
        return Vec::new();
    }
    let segmenter = segmenter();
    let mut breaks = vec![false; chars.len()];
    let mut at = 0;
    for run in chars.chunk_by(|a, b| writes_no_spaces(*a) == writes_no_spaces(*b)) {
        if run.first().copied().is_some_and(writes_no_spaces) {
            let text: String = run.iter().collect();
            let offsets: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
            for byte in segmenter.segment_str(&text) {
                // The run's start (char 0) and its end (no char) are not inside it. Marks with no
                // letter before them can make ICU4X break before a mark: never split those.
                if let Ok(k) = offsets.binary_search(&byte)
                    && let (Some(&a), Some(&b)) = (k.checked_sub(1).and_then(|j| run.get(j)), run.get(k))
                    && safe_emergency_break(a, b)
                    && let Some(brk) = breaks.get_mut(at + k)
                {
                    *brk = true;
                }
            }
        }
        at += run.len();
    }
    breaks
}

/// Whether a line may break between `a` and the `b` after it when the text offers no break
/// opportunity and has to break inside a word (Thai, Lao, Khmer and Burmese write no spaces):
/// never before a letter's trailing mark, which belongs to the letter before it, and never after a
/// leading vowel, which is written before the consonant it is read with (#1219).
pub(crate) fn safe_emergency_break(a: char, b: char) -> bool {
    !is_trailing_mark(b) && !matches!(a, '\u{0E40}'..='\u{0E44}' | '\u{0EC0}'..='\u{0EC4}')
}

/// For each char of `text`, whether it starts a grapheme cluster (a user-perceived character:
/// an emoji sequence, a letter with its combining marks).
pub(crate) fn grapheme_starts(text: &str) -> Vec<bool> {
    if text.is_ascii() && !text.contains('\r') {
        return vec![true; text.len()];
    }
    let mut starts = Vec::with_capacity(text.len());
    for g in text.graphemes(true) {
        starts.extend(g.chars().enumerate().map(|(k, _)| k == 0));
    }
    starts
}

/// The char index of the grapheme cluster boundary after (`forward`) or before char `index` in
/// `text`: where an arrow key or Delete moves the caret to.
pub fn grapheme_step(text: &str, index: usize, forward: bool) -> usize {
    let mut at = 0;
    for g in text.graphemes(true) {
        let next = at + g.chars().count();
        if (forward && index < next) || (!forward && index <= next) {
            return if forward { next } else { at };
        }
        at = next;
    }
    at.min(index)
}

/// The start of the text deleted by Backspace at char `index`.
///
/// A letter's trailing mark (Grapheme_Cluster_Break Extend or SpacingMark, such as Thai MAI THO or
/// SARA AM) is deleted on its own; other grapheme clusters remain atomic, so emoji sequences, flags,
/// keycaps and Hangul syllables written as conjoining jamo stay intact.
pub fn backspace_step(text: &str, index: usize) -> usize {
    let end = text.chars().count().min(index);
    let start = grapheme_step(text, end, false);
    if start == end {
        return end;
    }

    let cluster = text.chars().skip(start).take(end - start).collect::<Vec<_>>();
    let letter_cluster = cluster.len() > 1 && cluster.first().is_some_and(|c| c.is_alphabetic());
    if letter_cluster && cluster.last().is_some_and(|&c| is_trailing_mark(c)) { end - 1 } else { start }
}

/// Whether `c` is Extend or SpacingMark: it joins a plain letter's cluster. ZWJ also joins one but
/// is a joiner, not a mark.
fn is_trailing_mark(c: char) -> bool {
    c != '\u{200D}' && format!("a{c}").graphemes(true).count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figure_space_is_non_breaking() {
        assert!(!break_between('\u{2007}', '0'));
        assert!(!break_between('1', '\u{2007}'));
        assert!(!break_between('日', '\u{2007}') && !break_between('\u{2007}', '日'), "nor beside ideographs");
        assert!(!is_space('\u{2007}'));
    }

    #[test]
    fn emergency_breaks_keep_marks_and_leading_vowels_with_their_letter() {
        // #1219: Thai and Lao write no spaces, so these are where a line has to be forced to break.
        assert!(!safe_emergency_break('\u{0E01}', '\u{0E49}'), "never before a tone mark");
        assert!(!safe_emergency_break('\u{0E01}', '\u{0E33}'), "nor before SARA AM");
        assert!(!safe_emergency_break('\u{0E40}', '\u{0E2B}'), "never after a leading vowel");
        assert!(!safe_emergency_break('\u{0EC0}', '\u{0EAB}'), "nor in Lao");
        assert!(safe_emergency_break('\u{0E01}', '\u{0E02}'), "between two letters is fine");
        assert!(safe_emergency_break('\u{0E2B}', '\u{0E40}'), "before a leading vowel is fine");
    }

    /// The text split where [`word_breaks`] marks a word start.
    fn split_words(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let breaks = word_breaks(&chars);
        let mut words = vec![String::new()];
        for (k, c) in chars.iter().enumerate() {
            if breaks.get(k).copied().unwrap_or(false) {
                words.push(String::new());
            }
            words.last_mut().unwrap().push(*c);
        }
        words
    }

    /// Thai words come from the dictionary, compounds and leading vowels included.
    #[test]
    fn thai_lines_break_between_words() {
        // #1219's sentence ("the system helps software teams produce structured documents in one
        // place"): ซอฟต์แวร์ (software) and โครงสร้าง (structure) stay whole, and no word is split
        // after a leading vowel or before a mark. Pinned to ICU4X 2.3's Thai dictionary.
        let words = ["ระบบ", "ช่วย", "ให้", "ทีม", "พัฒนา", "ซอฟต์แวร์", "จัด", "ทำ", "เอกสาร", "ที่", "มี", "โครงสร้าง", "ได้", "ใน", "ที่", "เดียว"];
        assert_eq!(split_words(&words.concat()), words);
    }

    /// Lao, Khmer and Burmese words come from their dictionaries too.
    #[test]
    fn lao_khmer_and_burmese_lines_break_between_words() {
        // "new world, the Lao language"; "Khmer is the official language"; "the Burmese language".
        for words in [&["ໂລກ", "ໃໝ່", "ພາສາ", "ລາວ"][..], &["ភាសាខ្មែរ", "គឺជា", "ភាសាផ្លូវការ"], &["မြန်မာဘာသာ", "စကား", "သည်"]]
        {
            assert_eq!(split_words(&words.concat()), words);
        }
    }

    /// Text without these scripts is left to the UAX #14 rules, and so is the edge of a run.
    #[test]
    fn word_breaks_leave_other_text_alone() {
        assert!(word_breaks(&"Latin, 日本語 and Ελληνικά".chars().collect::<Vec<_>>()).is_empty(), "no work without these scripts");
        // Where a Thai run meets Latin or a space, break_between decides: no break inside "ไทยabc".
        assert_eq!(split_words("ภาษาไทยabcภาษาไทย ไทย"), ["ภาษา", "ไทยabcภาษา", "ไทย ไทย"]);
    }

    /// Malformed runs neither panic nor start a word on a mark.
    #[test]
    fn word_breaks_survive_junk() {
        // Stray marks, a lone leading vowel, joiners and a long run: no panic, and never a word
        // start on a mark, which belongs to the letter before it.
        let long = "ก\u{0E49}".repeat(5_000);
        for text in ["\u{0E49}\u{0E49}\u{0E33}", "เ", "ก\u{200D}ข\u{200C}ค", "\u{1000}\u{103A}\u{200D}\u{1004}", &long] {
            let chars: Vec<char> = text.chars().collect();
            let breaks = word_breaks(&chars);
            assert_eq!(breaks.len(), chars.len(), "{text:?}");
            for (k, c) in chars.iter().enumerate() {
                assert!(
                    !(breaks.get(k).copied().unwrap_or(false) && is_trailing_mark(*c)),
                    "{text:?}: a word starts on mark U+{:04X} at {k}",
                    *c as u32
                );
            }
        }
    }
}
