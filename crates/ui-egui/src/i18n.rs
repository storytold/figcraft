//! The UI language: lookups over the string catalogs in `crates/ui-egui/locales/` (one TOML file
//! per language, baked in by build.rs; see docs/translating.md).
//!
//! Keys are the English text as written in the code: `tr("Fill")`,
//! `trf("Delete {name}", &[("name", &name)])`, `trn("{n} layers", count)`. A key a catalog lacks
//! shows its English text, so the UI never shows a raw key. Command labels are looked up by command
//! id ([`command`]): ids, the CLI, MCP and the control channel stay English.

use std::borrow::Cow;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};

/// One catalog entry.
pub enum Text {
    Plain(&'static str),
    /// Optional language-specific plural forms, with `other` as the fallback.
    Plural {
        one: Option<&'static str>,
        few: Option<&'static str>,
        many: Option<&'static str>,
        other: &'static str,
    },
}

/// A language's catalog (generated from `locales/<code>.toml`).
pub struct Lang {
    pub code: &'static str,
    /// The language's own name for itself (the language picker shows it untranslated).
    pub name: &'static str,
    /// Machine-drafted, waiting for a native speaker's review (`[meta] status`).
    pub draft: bool,
    /// Index into [`LANGS`] of the catalog a regional one overlays ("es" under "es-419": it lists
    /// only what differs), else 0 (English).
    pub parent: usize,
    /// UI strings by English text, sorted.
    pub ui: &'static [(&'static str, Text)],
    /// Command labels by command id, sorted.
    pub commands: &'static [(&'static str, Text)],
    /// Main-menu labels by command id where the menu words it differently ("Fill" in Select
    /// matching for "Select same fill"), sorted.
    pub menu: &'static [(&'static str, Text)],
}

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Index into [`LANGS`] of the language the UI shows (0: English).
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The languages the UI can show, English first.
pub fn languages() -> &'static [Lang] {
    LANGS
}

/// The language the UI shows.
pub fn current() -> Option<&'static Lang> {
    LANGS.get(CURRENT.load(Ordering::Relaxed))
}

/// Show the language of `pref` (a language code, or "system" for `system`, the OS or browser
/// languages, most preferred first and comma-separated). Unknown languages show English.
pub fn set(pref: &str, system: Option<&str>) {
    CURRENT.store(resolve(pref, system), Ordering::Relaxed);
}

/// The [`LANGS`] index for a preference: the first language in it the UI has, else English.
fn resolve(pref: &str, system: Option<&str>) -> usize {
    let want = if pref.is_empty() || pref == "system" { system.unwrap_or("en") } else { pref };
    want.split(',').find_map(resolve_one).unwrap_or(0)
}

/// Regions whose Spanish reads the Latin American catalog (es-419): Latin America and the US.
const LATIN_AMERICA: &[&str] =
    &["419", "AR", "BO", "CL", "CO", "CR", "CU", "DO", "EC", "GT", "HN", "MX", "NI", "PA", "PE", "PR", "PY", "SV", "US", "UY", "VE"];

/// The [`LANGS`] index for one tag: the exact tag ("pt-BR"), else its base language ("es" for
/// "es_ES.UTF-8"), else a regional catalog of that language ("pt-BR" for "pt-PT"); Latin American
/// Spanish ("es-MX") reads es-419, and Traditional Chinese (zh-Hant, zh-TW, zh-HK, zh-MO) none: it
/// has no catalog and Simplified isn't a stand-in. A language the UI can't draw (CJK on the web
/// without CJK faces built in) doesn't count.
fn resolve_one(want: &str) -> Option<usize> {
    let tag = want.trim().split(['.', '@']).next().unwrap_or("").replace('_', "-");
    let base = tag.split('-').next().unwrap_or("");
    let has = |subtags: &[&str]| tag.split('-').skip(1).any(|t| subtags.iter().any(|s| s.eq_ignore_ascii_case(t)));
    if base.eq_ignore_ascii_case("zh") && (has(&["Hant"]) || (!has(&["Hans"]) && has(&["TW", "HK", "MO"]))) {
        return None;
    }
    let find = |t: &str| LANGS.iter().position(|l| l.code.eq_ignore_ascii_case(t));
    let alias = (base.eq_ignore_ascii_case("es") && has(LATIN_AMERICA)).then(|| find("es-419")).flatten();
    let regional = || LANGS.iter().position(|l| !base.is_empty() && l.code.split('-').next().is_some_and(|b| b.eq_ignore_ascii_case(base)));
    alias.or_else(|| find(&tag)).or_else(|| find(base)).or_else(regional).filter(|i| LANGS.get(*i).is_some_and(|l| crate::cjk::can_show(l.code)))
}

fn find(table: &'static [(&'static str, Text)], key: &str) -> Option<&'static Text> {
    table.binary_search_by(|(k, _)| (*k).cmp(key)).ok().and_then(|i| table.get(i)).map(|(_, t)| t)
}

/// Language `lang`, the catalog it overlays ("es" under "es-419"), then English.
fn chain(lang: usize) -> impl Iterator<Item = &'static Lang> {
    let parent = LANGS.get(lang).map_or(0, |l| l.parent);
    [lang, parent, 0].into_iter().filter_map(|i| LANGS.get(i))
}

/// `key`'s entry in language `lang`, else in the catalog it overlays, else in English.
fn lookup(lang: usize, key: &str) -> Option<&'static Text> {
    chain(lang).find_map(|l| find(l.ui, key))
}

/// The UI string for `key` (its English text) in the current language. One English text that needs
/// two translations gets a context after `|` ("Fill|sizing"); the English catalog maps it back.
pub fn tr(key: &str) -> &str {
    match lookup(CURRENT.load(Ordering::Relaxed), key) {
        Some(Text::Plain(s) | Text::Plural { other: s, .. }) => s,
        None => key,
    }
}

/// Mark a key for the catalog where the lookup happens later (`tr(label)` at display time), e.g. in
/// a `const` table.
pub const fn tr_noop(key: &str) -> &str {
    key
}

/// [`tr`] with `{name}` placeholders filled from `args`.
pub fn trf(key: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(tr(key), args)
}

/// The plural form of `key` (its English "other" form) for count `n`, with `{n}` and `args` filled.
pub fn trn(key: &str, n: usize, args: &[(&str, &dyn Display)]) -> String {
    let lang = CURRENT.load(Ordering::Relaxed);
    let text = counted(lang, key, n);
    let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    fill(text, &all)
}

/// Pick a plural using the catalog that supplies the text: an English fallback keeps English's
/// rule, even when the selected language is Russian.
fn counted(lang: usize, key: &str, n: usize) -> &str {
    chain(lang)
        .find_map(|l| {
            find(l.ui, key).map(|text| match text {
                Text::Plain(s) => *s,
                Text::Plural { one, few, many, other } => match plural(l.code, n) {
                    Plural::One => one.unwrap_or(other),
                    Plural::Few => few.unwrap_or(other),
                    Plural::Many => many.unwrap_or(other),
                    Plural::Other => other,
                },
            })
        })
        .unwrap_or(key)
}

#[derive(Debug, PartialEq, Eq)]
enum Plural {
    One,
    Few,
    Many,
    Other,
}

/// CLDR integer rules. Existing one/other catalogs may omit the extra forms.
fn plural(code: &str, n: usize) -> Plural {
    match code {
        "cs" if n == 1 => Plural::One,
        "cs" if (2..=4).contains(&n) => Plural::Few,
        "cs" => Plural::Other,
        "ru" if n % 10 == 1 && n % 100 != 11 => Plural::One,
        "ru" if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) => Plural::Few,
        "ru" => Plural::Many,
        "ja" | "ko" | "zh-Hans" => Plural::Other,
        "fr" | "pt-BR" if n <= 1 => Plural::One,
        _ if n == 1 => Plural::One,
        _ => Plural::Other,
    }
}

// CLDR's million-count "many" in French, Spanish and Portuguese still reads their existing
// "other" wording. Czech supplies separate few forms; Russian supplies few/many forms.

/// Whether count `n` takes the "one" form: 0 and 1 in French and Brazilian Portuguese, never in
/// Japanese, Korean and Chinese (no plural forms: always "other"), 1 elsewhere (CLDR's integer rules).
#[cfg(test)]
fn plural_one(lang: usize, n: usize) -> bool {
    plural(LANGS.get(lang).map_or("en", |l| l.code), n) == Plural::One
}

/// Replace each `{name}` in `text` with its argument, in one pass (an argument's own braces stay).
fn fill(text: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let (before, tail) = rest.split_at(open);
        out.push_str(before);
        let arg = tail.find('}').and_then(|close| {
            let name = tail.get(1..close)?;
            args.iter().find(|(k, _)| *k == name).map(|(_, v)| (close, v))
        });
        match arg {
            Some((close, v)) => {
                out.push_str(&v.to_string());
                rest = tail.get(close + 1..).unwrap_or("");
            }
            None => {
                out.push('{');
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// A command's label in the current language: the catalog's by command id, else `english` (the
/// engine's label).
pub fn command<'a>(id: &str, english: &'a str) -> Cow<'a, str> {
    label(false, id, english)
}

/// A main-menu item's label: the catalog's menu wording for the id, else the command's. An English
/// label ending in "…" (opens a dialog) keeps it.
pub fn menu_item<'a>(id: &str, english: &'a str) -> Cow<'a, str> {
    label(true, id, english)
}

fn label<'a>(menu: bool, id: &str, english: &'a str) -> Cow<'a, str> {
    let text = chain(CURRENT.load(Ordering::Relaxed)).find_map(|l| menu.then(|| find(l.menu, id)).flatten().or_else(|| find(l.commands, id)));
    match text {
        Some(Text::Plain(s) | Text::Plural { other: s, .. }) if english.ends_with('…') && !s.ends_with('…') => Cow::Owned(format!("{s}…")),
        Some(Text::Plain(s) | Text::Plural { other: s, .. }) => Cow::Borrowed(s),
        None => Cow::Borrowed(english),
    }
}

#[cfg(test)]
#[path = "i18n_tests.rs"]
mod tests;
