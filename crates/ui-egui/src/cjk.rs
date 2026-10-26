//! Chinese, Japanese, Korean, Bengali and Thai text in the UI. egui's own fonts and Inter have no glyphs
//! for them, so the UI adds a face per script it needs: the UI language's, the scripts in the open file's names
//! and comments, the language picker's and typed text. Desktop finds them among the installed
//! fonts (the system font scan), only once one is needed, so English users load none; the web
//! build has only the CJK faces built in from craft-fonts (`FIGCRAFT_EMBED_CJK`), else none (no
//! Bengali or Thai face is built in, so the web leaves them out).

use std::sync::Arc;

use figcraft_text::Face;

pub const KANA: u8 = 1;
pub const HANGUL: u8 = 2;
/// Han characters (and CJK punctuation and full-width forms).
pub const HAN: u8 = 4;
/// Bengali script.
pub const BENGALI: u8 = 8;
/// Thai script.
pub const THAI: u8 = 16;
/// Every script bit: nothing more to look for once all have a face.
pub const ALL: u8 = KANA | HANGUL | HAN | BENGALI | THAI;

/// Bengali faces: cross-platform, Windows, macOS, then Linux names.
const BENGALI_FONTS: &[&str] = &["Noto Sans Bengali", "Nirmala UI", "Bangla Sangam MN", "Kalpurush", "SolaimanLipi"];

/// Thai faces: cross-platform, Windows, macOS, then Linux names.
const THAI_FONTS: &[&str] =
    &["Noto Sans Thai", "Leelawadee UI", "Tahoma", "Sukhumvit Set", "Thonburi", "Ayuthaya", "Sarabun", "Loma", "Garuda", "Waree"];

/// Japanese faces, the built-in one first: Windows, macOS, then Linux names.
const JAPANESE: &[&str] = &[
    "BIZ UDPGothic",
    "Yu Gothic UI",
    "Yu Gothic",
    "Meiryo UI",
    "Meiryo",
    "MS UI Gothic",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Noto Sans CJK JP",
    "Noto Sans JP",
    "Source Han Sans JP",
    "IPAexGothic",
    "IPAPGothic",
    "TakaoPGothic",
];
const KOREAN: &[&str] =
    &["Malgun Gothic", "Apple SD Gothic Neo", "Noto Sans CJK KR", "Noto Sans KR", "Source Han Sans KR", "NanumGothic", "NanumBarunGothic", "UnDotum"];
const CHINESE: &[&str] = &[
    "Noto Sans CJK SC",
    "Microsoft YaHei UI",
    "Microsoft YaHei",
    "PingFang SC",
    "Hiragino Sans GB",
    "Heiti SC",
    "Noto Sans SC",
    "Source Han Sans SC",
    "WenQuanYi Micro Hei",
    "WenQuanYi Zen Hei",
    "Droid Sans Fallback",
];

/// Prefer Taiwan glyph shapes for the Taiwan Traditional Chinese interface.
const TRADITIONAL_CHINESE: &[&str] = &[
    "Noto Sans CJK TC",
    "Microsoft JhengHei UI",
    "Microsoft JhengHei",
    "PingFang TC",
    "Heiti TC",
    "Noto Sans TC",
    "Source Han Sans TC",
    "Noto Sans CJK HK",
];

fn language_samples(need: u8, code: &str) -> Vec<(char, Vec<&'static str>)> {
    let mut choices = samples(need, language(code));
    if code.eq_ignore_ascii_case("zh-TW") || code.eq_ignore_ascii_case("zh-Hant") {
        for (character, families) in &mut choices {
            if *character == '们' {
                *character = '體';
                *families = [TRADITIONAL_CHINESE, CHINESE, JAPANESE, KOREAN].concat();
            }
        }
    }
    choices
}

/// The CJK, Bengali and Thai scripts in `s`.
pub fn scripts(s: &str) -> u8 {
    s.chars().fold(0, |m, c| {
        m | match u32::from(c) {
            0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => KANA,
            0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xFFA0..=0xFFDC => HANGUL,
            0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF01..=0xFF65 | 0x20000..=0x3FFFF => HAN,
            0x0980..=0x09FF => BENGALI,
            0x0E00..=0x0E7F => THAI,
            _ => 0,
        }
    })
}

/// The scripts a UI language (a catalog code) is written in.
pub fn language(code: &str) -> u8 {
    match code.split('-').next() {
        Some("ja") => KANA | HAN,
        Some("ko") => HANGUL,
        Some("zh") => HAN,
        Some("bn") => BENGALI,
        Some("th") => THAI,
        _ => 0,
    }
}

/// Whether the UI can draw language `code`: always on desktop (a system face is found when it's
/// needed; tofu at worst), on the web only with built-in faces for its scripts (Georgian: craft-fonts'
/// Noto Sans Georgian). The language picker and the system language skip a language it can't.
pub fn can_show(code: &str) -> bool {
    shows(code, cfg!(target_arch = "wasm32"))
}

/// [`can_show`] in a desktop or (`web`) a web build.
fn shows(code: &str, web: bool) -> bool {
    if !web {
        return true;
    }
    if code.split('-').next() == Some("ka") {
        return figcraft_text::face_with_char('ა', &["Noto Sans Georgian"]).is_some();
    }
    let need = language(code);
    need == 0 || samples(need, 0).iter().all(|(c, _)| figcraft_text::face_with_char(*c, &["BIZ UDPGothic", "Noto Sans CJK SC"]).is_some())
}

/// A character standing for each script in `need`, with the families to look for it in, the UI
/// language's script (`ui`, see [`language`]) first. Han reads as Chinese (a simplified character,
/// Chinese faces first) in a Chinese UI, and without kana in a UI that isn't Japanese or Korean;
/// else a Japanese or Korean face draws it.
fn samples(need: u8, ui: u8) -> Vec<(char, Vec<&'static str>)> {
    let chinese = need & HAN != 0 && (ui == HAN || (ui == 0 && need & KANA == 0));
    let han: (char, Vec<&str>) = match (chinese, ui) {
        (true, _) => ('们', [CHINESE, JAPANESE, KOREAN].concat()),
        (false, HANGUL) => ('漢', [KOREAN, JAPANESE, CHINESE].concat()),
        (false, _) => ('漢', [JAPANESE, KOREAN, CHINESE].concat()),
    };
    let mut v = vec![
        (KANA, ('あ', JAPANESE.to_vec())),
        (HAN, han),
        (HANGUL, ('한', KOREAN.to_vec())),
        (BENGALI, ('অ', BENGALI_FONTS.to_vec())),
        (THAI, ('ก', THAI_FONTS.to_vec())),
    ];
    match ui {
        HANGUL => v.swap(0, 2),
        HAN => v.swap(0, 1),
        BENGALI | THAI => {
            if let Some(pos) = v.iter().position(|(bit, _)| *bit == ui) {
                let item = v.remove(pos);
                v.insert(0, item);
            }
        }
        _ => {}
    }
    v.into_iter().filter(|(bit, _)| need & bit != 0).map(|(_, s)| s).collect()
}

/// Faces (built-in or installed) drawing the scripts in `need` that `have` (the UI's other fonts)
/// lacks, in fallback order: one per script, unless an earlier one draws it too. Only the families
/// tried are read from disk.
pub fn faces(need: u8, lang: &str, have: &dyn Fn(char) -> bool) -> Vec<Arc<Face>> {
    let mut out: Vec<Arc<Face>> = Vec::new();
    for (c, families) in language_samples(need, lang) {
        if have(c) || out.iter().any(|f| f.has_char(c)) {
            continue;
        }
        if let Some(face) = figcraft_text::face_with_char(c, &families) {
            out.push(face);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taiwan_system_font_covers_the_catalog_han_characters() {
        let Some(language) = crate::i18n::languages().iter().find(|language| language.code == "zh-TW") else {
            panic!("Taiwan catalog is registered");
        };
        let selected = faces(HAN, "zh-TW", &|_| false);
        let Some(face) = selected.first() else { return }; // Web/CI may have no installed CJK face.
        for (_, text) in language.ui.iter().chain(language.commands).chain(language.menu) {
            let values: Vec<&str> = match text {
                crate::i18n::Text::Plain(text) => vec![text],
                crate::i18n::Text::Plural { one, few, many, other } => {
                    one.iter().chain(few).chain(many).copied().chain(std::iter::once(*other)).collect()
                }
            };
            for character in values.into_iter().flat_map(str::chars).filter(|c| ('\u{3400}'..='\u{9fff}').contains(c)) {
                assert!(face.has_char(character), "{} lacks {character}", face.family);
            }
        }
    }

    #[test]
    fn taiwan_interface_prefers_traditional_glyphs() {
        let choices = language_samples(HAN | KANA, "zh-TW");
        assert_eq!(choices[0].0, '體');
        assert_eq!(choices[0].1[0], "Noto Sans CJK TC");
        assert!(choices[0].1.contains(&"Microsoft JhengHei UI"));
        assert_eq!(language_samples(HAN, "zh-Hans")[0].0, '们');
        assert_eq!(choices[1].0, 'あ');
    }

    /// Each script is found in text and tied to its UI language code.
    #[test]
    fn scripts_and_languages() {
        assert_eq!(scripts("Frame 1"), 0);
        assert_eq!(scripts("ボタン"), KANA);
        assert_eq!(scripts("버튼"), HANGUL);
        assert_eq!(scripts("按钮"), HAN);
        assert_eq!(scripts("বাংলা"), BENGALI);
        assert_eq!(scripts("หน้าแรก"), THAI);
        assert_eq!(language("th"), THAI);
        assert_eq!(scripts("日本語のレイヤー（1）"), KANA | HAN);
        assert_eq!(language("ja"), KANA | HAN);
        assert_eq!(language("ko"), HANGUL);
        assert_eq!(language("zh-Hant"), HAN);
        assert_eq!(language("zh-Hans"), HAN);
        assert_eq!(language("bn"), BENGALI);
        assert_eq!(language("fr"), 0);
        assert!(["en", "es", "es-419", "fr", "de", "pt-BR", "tr", "id", "vi"].iter().all(|c| can_show(c)));
    }

    #[test]
    fn the_web_offers_georgian_only_with_its_face() {
        assert!(shows("ka", false), "desktop always lists Georgian");
        let face = figcraft_text::face_with_char('ა', &["Noto Sans Georgian"]).is_some();
        assert_eq!(shows("ka", true), face);
        assert_eq!(shows("ka-GE", true), face);
        assert!(shows("en", true) && shows("tr", true));
    }

    /// The UI language's script leads the fallback order; the others follow.
    #[test]
    fn the_ui_language_script_comes_first() {
        let order = |need, ui| samples(need, ui).into_iter().map(|(c, _)| c).collect::<String>();
        let first_face = |need, ui| samples(need, ui).first().and_then(|(_, f)| f.first()).copied();
        assert_eq!(order(KANA | HAN | HANGUL, 0), "あ漢한");
        assert_eq!(order(KANA | HAN | HANGUL | BENGALI, language("bn")), "অあ漢한");
        assert_eq!(order(KANA | HAN | HANGUL, KANA | HAN), "あ漢한");
        assert_eq!(order(KANA | HAN | HANGUL, HANGUL), "한漢あ");
        assert_eq!(order(HANGUL, 0), "한");
        assert_eq!(order(THAI, 0), "ก");
        assert_eq!(order(THAI | KANA, language("th")), "กあ");
        // A Korean UI draws Hangul in a Korean face before the Thai or Bengali ones.
        assert_eq!(order(KANA | HAN | HANGUL | BENGALI | THAI, HANGUL), "한漢あঅก");
        // Han without kana outside a Japanese or Korean UI reads as Chinese.
        assert_eq!(order(HAN, 0), "们");
        assert_eq!(first_face(HAN, 0), Some("Noto Sans CJK SC"));
        assert_eq!(first_face(HAN, HANGUL), Some("Malgun Gothic"));
        // A Chinese UI draws Han in a Chinese face first, Japanese names in it too.
        assert_eq!(order(KANA | HAN | HANGUL, HAN), "们あ한");
        assert_eq!(order(KANA | HAN, language("zh-Hans")), "们あ");
        assert!(["Microsoft YaHei UI", "PingFang SC", "Noto Sans CJK SC"].iter().all(|f| samples(HAN, HAN)[0].1[..4].contains(f)));
        assert!(faces(0, "en", &|_| false).is_empty());
        assert!(faces(KANA | HANGUL | HAN | BENGALI, "ja", &|_| true).is_empty(), "nothing is looked up for scripts the UI draws");
    }
}
