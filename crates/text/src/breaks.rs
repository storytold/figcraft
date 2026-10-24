//! Where lines may break and where carets may stop: break opportunities (a small subset of UAX #14
//! with the CJK line-start and line-end rules) and grapheme clusters (UAX #29).

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
        ' ' | '\t' | '\u{3000}' | '\u{2002}'..='\u{200B}' => Class::Space,
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
        // no opening mark ends one.
        (_, Space | Close) | (Open, _) => false,
        (Space | Hyphen | Ideo, _) | (_, Ideo) => true,
        _ => false,
    }
}

/// Whether char `c` is a breakable space (it hangs at a line's end and doesn't count for its width).
pub(crate) fn is_space(c: char) -> bool {
    class(c) == Class::Space
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
/// `text`: where an arrow key, Delete or Backspace moves the caret to.
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
