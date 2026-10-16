//! The catalog checks: every key the code looks up is in the English catalog, no catalog has keys
//! the code doesn't use, placeholders match English, and complete languages translate everything.

use std::collections::BTreeSet;

use super::*;

/// Languages that must translate every UI string and command label.
const COMPLETE: &[&str] = &["es"];

/// Keys looked up from data rather than written as `tr("…")` in the code: the main menu's
/// submenu names (the engine's menu catalog) and labels of engine and document enums.
fn dynamic_keys() -> BTreeSet<String> {
    let mut keys: BTreeSet<String> = figcraft_engine::catalog::MENU.iter().flat_map(|m| m.menu.split('/')).map(str::to_string).collect();
    keys.extend(dynamic_labels().into_iter().map(str::to_string));
    keys
}

/// Labels from engine and document data the UI shows through `tr(x.label())` and isn't written as
/// a literal anywhere in the UI: node kinds (and the Design panel header's plurals), blend modes,
/// stroke caps and width profiles, effects, brushes, image filters, frame preset categories, Dev Mode
/// properties, comment filters and sorts, change kinds, the live status.
// shortcut: a hand-kept list; enumerate each enum's `ALL` here when a label goes missing in review.
const DATA_LABELS: &[&str] = &[
    "Added",
    "Align",
    "All comments",
    "Alpha",
    "Background blur",
    "Black & white",
    "Blend",
    "Bristle",
    "Circle",
    "Color burn",
    "Color dodge",
    "Component",
    "Component set",
    "Component sets",
    "Confetti",
    "Content",
    "Cool",
    "Darken",
    "Desktop",
    "Difference",
    "Direction",
    "Dots",
    "Drop shadow",
    "Duo",
    "Ellipses",
    "Exclude",
    "Excludes",
    "Exclusion",
    "Eye",
    "Fade",
    "Fills",
    "Frames",
    "Glass",
    "Group",
    "Groups",
    "Hard light",
    "Hue",
    "Ink",
    "Inner shadow",
    "Instances",
    "Intersect",
    "Intersects",
    "Joining…",
    "Layer blur",
    "Leaves",
    "Lighten",
    "Line arrow",
    "Lines",
    "Luminance",
    "Luminosity",
    "Mono",
    "Multi",
    "Multiply",
    "Muted",
    "No status",
    "Noise",
    "Normal",
    "Only your threads",
    "Padding",
    "Paper",
    "Pass through",
    "Phone",
    "Pinch",
    "Plus darker",
    "Plus lighter",
    "Polygons",
    "Presentation",
    "Progressive",
    "Rectangles",
    "Removed",
    "Reversed triangle",
    "Ribbon",
    "Rounded",
    "Screen",
    "Sections",
    "Slices",
    "Social media",
    "Soft light",
    "Sort by date",
    "Sort by page",
    "Square",
    "Stars",
    "Strokes",
    "Style",
    "Subtract",
    "Subtracts",
    "Tablet",
    "Taper",
    "Taper both ends",
    "Text align",
    "Texts",
    "Texture",
    "Triangle arrow",
    "Uniform",
    "Union",
    "Unions",
    "Unread",
    "Vectors",
    "Vivid",
    "Warm",
    "Watch",
    "Wedge",
    "Weight",
];

/// Labels of engine enums the UI shows through `tr(x.label())`, read from the engine, plus [`DATA_LABELS`].
fn dynamic_labels() -> Vec<&'static str> {
    use figcraft_engine::keymap::{GESTURES, Group};
    let mut v: Vec<&'static str> = figcraft_engine::ToolId::ALL.iter().map(|t| t.label()).collect();
    v.extend(figcraft_engine::vectoredit::VTool::ALL.iter().map(|t| t.label()));
    v.extend(Group::ALL.iter().map(|g| g.label()));
    v.extend(GESTURES.iter().flat_map(|(_, label, mac, other)| [*label, *mac, *other]));
    v.extend(DATA_LABELS);
    // The starter templates' names, descriptions and kinds.
    v.extend(figcraft_engine::templates::TEMPLATES.iter().flat_map(|t| [t.name, t.description, t.kind.label()]));
    v
}

/// Ids of main-menu items whose label isn't the command's (besides a closing "…").
fn menu_worded_differently() -> BTreeSet<&'static str> {
    let bare = |s: &'static str| s.trim_end_matches('…');
    figcraft_engine::catalog::MENU
        .iter()
        .filter(|m| figcraft_engine::find_command(m.id).is_some_and(|c| bare(c.label) != bare(m.label)))
        .map(|m| m.id)
        .collect()
}

/// The string literals passed to `tr`, `trf`, `trn` and `tr_noop` in this crate's sources.
fn used_keys() -> BTreeSet<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut keys = BTreeSet::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "i18n.rs" {
            continue;
        }
        keys.extend(literals(&std::fs::read_to_string(&path).unwrap()));
    }
    keys
}

/// The first-argument string literals of the lookup functions in `src`.
fn literals(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for f in ["tr(", "trf(", "trn(", "tr_noop("] {
        let mut from = 0;
        while let Some(i) = src.get(from..).and_then(|s| s.find(f)) {
            let at = from + i;
            from = at + f.len();
            let prev = src.get(..at).and_then(|s| s.chars().next_back());
            if prev.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let rest = src.get(from..).unwrap().trim_start();
            if let Some(lit) = rest.strip_prefix('"').and_then(string_literal) {
                out.push(lit);
            }
        }
    }
    out
}

/// A Rust string literal's value (after its opening quote), up to the closing quote.
fn string_literal(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '\n' => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                }
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
    None
}

fn placeholders(s: &str) -> BTreeSet<&str> {
    s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(name, _)| name)).collect()
}

fn forms(t: &Text) -> Vec<&'static str> {
    match t {
        Text::Plain(s) => vec![s],
        Text::Plural { one, few, many, other } => one.iter().chain(few).chain(many).copied().chain([*other]).collect(),
    }
}

fn english() -> &'static Lang {
    LANGS.first().filter(|l| l.code == "en").expect("locales/en.toml is baked in first")
}

#[test]
fn every_key_in_the_code_is_in_the_english_catalog_and_no_other() {
    let used: BTreeSet<String> = used_keys().into_iter().chain(dynamic_keys()).collect();
    let en: BTreeSet<String> = english().ui.iter().map(|(k, _)| k.to_string()).collect();
    let missing: Vec<_> = used.difference(&en).collect();
    let unused: Vec<_> = en.difference(&used).collect();
    assert!(
        missing.is_empty(),
        "keys missing from locales/en.toml:\n{}",
        missing.iter().map(|k| format!("{k:?} = {k:?}")).collect::<Vec<_>>().join("\n")
    );
    assert!(unused.is_empty(), "keys in locales/en.toml the code doesn't use: {unused:?}");
}

#[test]
fn english_text_is_its_key_except_plurals() {
    for (k, t) in english().ui {
        if let Text::Plain(s) = t {
            // "Fill|sizing": the same English as "Fill", translated apart (a context after `|`).
            assert_eq!(k.split('|').next(), Some(*s), "an English entry differs from its key (the key is the English text)");
        }
    }
    assert!(english().commands.is_empty() && english().menu.is_empty(), "English command labels come from the engine");
}

#[test]
fn catalogs_have_known_keys_and_matching_placeholders() {
    let en = english();
    let ids: BTreeSet<&str> = figcraft_engine::all_commands()
        .iter()
        .map(|c| c.id)
        .chain(figcraft_engine::catalog::MENU.iter().map(|m| m.id).filter(|id| !id.is_empty()))
        .collect();
    for lang in LANGS {
        assert!(!lang.name.is_empty(), "{}: [meta] name is missing", lang.code);
        for (k, t) in lang.ui {
            let src = find(en.ui, k).unwrap_or_else(|| panic!("{}: unknown key {k:?}", lang.code));
            let want = placeholders(forms(src).last().unwrap());
            for (i, form) in forms(t).iter().enumerate() {
                let got = placeholders(form);
                // A "one" form may spell the count out ("una capa").
                let ok = if matches!(t, Text::Plural { one: Some(_), .. }) && i == 0 { got.is_subset(&want) } else { got == want };
                assert!(ok, "{}: {k:?} has placeholders {got:?}, English has {want:?}", lang.code);
            }
            assert_eq!(matches!(t, Text::Plural { .. }), matches!(src, Text::Plural { .. }), "{}: {k:?} plural or not as in English", lang.code);
        }
        for (id, t) in lang.commands.iter().chain(lang.menu) {
            assert!(ids.contains(id), "{}: unknown command id {id:?}", lang.code);
            assert!(matches!(t, Text::Plain(s) if !s.trim().is_empty()), "{}: {id:?} is empty or plural", lang.code);
        }
        for (id, _) in lang.menu {
            assert!(menu_worded_differently().contains(id), "{}: [menu] {id:?} reads like the command; [commands] covers it", lang.code);
        }
        if COMPLETE.contains(&lang.code) {
            let missing: Vec<_> = en.ui.iter().filter(|(k, _)| find(lang.ui, k).is_none()).map(|(k, _)| k).collect();
            assert!(missing.is_empty(), "{}: untranslated: {missing:?}", lang.code);
            let missing: Vec<_> = ids.iter().filter(|id| find(lang.commands, id).is_none()).collect();
            assert!(missing.is_empty(), "{}: untranslated commands: {missing:?}", lang.code);
            let missing: Vec<_> = menu_worded_differently().into_iter().filter(|id| find(lang.menu, id).is_none()).collect();
            assert!(missing.is_empty(), "{}: menu items worded unlike their command, missing from [menu]: {missing:?}", lang.code);
        }
    }
}

#[test]
fn lookups_fall_back_to_english_and_fill_placeholders() {
    assert_eq!(fill("Delete {name}?", &[("name", &"Frame {1}")]), "Delete Frame {1}?");
    assert_eq!(fill("{a} and {b", &[("a", &1)]), "1 and {b");
    assert!(lookup(0, "no such key").is_none());
    assert_eq!(resolve("system", Some("es_ES.UTF-8")), resolve("es", None));
    assert_eq!(resolve("system", None), 0);
    assert_eq!(resolve("xx-YY", None), 0);
    let es = resolve("es-419", None);
    assert_ne!(es, 0, "Spanish is baked in");
    // A key Spanish lacks reads English; Spanish entries read Spanish.
    let (k, t) = LANGS[es].ui.first().unwrap();
    assert!(matches!(lookup(es, k), Some(x) if std::ptr::eq(x, t)));
}

#[test]
fn system_languages_pick_the_first_one_with_a_catalog() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    assert_eq!(code("system", Some("fr-CA")), "fr");
    assert_eq!(code("system", Some("de_CH.UTF-8")), "de");
    assert_eq!(code("system", Some("cs_CZ.UTF-8")), "cs");
    assert_eq!(code("system", Some("cs-CZ,en-US")), "cs");
    assert_eq!(code("cs", None), "cs");
    assert_eq!(LANGS[resolve("cs", None)].name, "Čeština");
    assert!(LANGS[resolve("cs", None)].draft);
    assert_eq!(code("system", Some("pt-BR")), "pt-BR");
    assert_eq!(code("system", Some("tr_TR.UTF-8")), "tr");
    assert_eq!(code("system", Some("id_ID.UTF-8")), "id");
    // Portugal's Portuguese and a bare "pt" read the Brazilian catalog.
    assert_eq!(code("system", Some("pt-PT")), "pt-BR");
    assert_eq!(code("pt", None), "pt-BR");
    // An OS list: the first language there is a catalog for (English counts).
    assert_eq!(code("system", Some("nl-NL,de-DE,en-US")), "de");
    assert_eq!(code("system", Some("nl-NL, en-US,fr-FR")), "en");
    assert_eq!(code("system", Some("nl,da")), "en");
    assert_eq!(code("system", Some("nl,sv")), "sv");
    assert_eq!(code("fr", Some("de-DE")), "fr", "a chosen language beats the system's");
}

#[test]
fn swedish_is_picked_for_swedish_locales() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    for tag in ["sv", "sv-SE", "sv-FI", "sv_SE.UTF-8"] {
        assert_eq!(code("system", Some(tag)), "sv", "{tag}");
    }
    let sv = LANGS.iter().position(|l| l.code == "sv").unwrap();
    assert_eq!(LANGS[sv].name, "Svenska");
    assert!(!LANGS[sv].draft);
    assert!(!plural_one(sv, 0) && plural_one(sv, 1) && !plural_one(sv, 2));
}

#[test]
fn latin_american_spanish_overlays_spanish() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    for tag in ["es-419", "es-MX", "es-AR", "es_CO.UTF-8", "es-US", "es-Latn-MX"] {
        assert_eq!(code("system", Some(tag)), "es-419", "{tag}");
    }
    for tag in ["es", "es-ES", "es_ES.UTF-8", "es-GQ"] {
        assert_eq!(code("system", Some(tag)), "es", "{tag}");
    }
    let es = LANGS.iter().position(|l| l.code == "es").unwrap();
    let la = LANGS.iter().position(|l| l.code == "es-419").unwrap();
    assert_eq!(LANGS[la].parent, es);
    assert!(es < la, "the picker lists Español first");
    // Its own wording, else Spanish's, else English's.
    assert!(matches!(lookup(la, "Add fill"), Some(Text::Plain("Agregar relleno"))));
    assert!(matches!(lookup(la, "Fill"), Some(Text::Plain("Relleno"))));
    let undo = |i: usize| chain(i).find_map(|l| find(l.commands, "edit.undo")).map(forms);
    assert!(undo(es).is_some() && undo(la) == undo(es), "command labels fall back too");
    // An overlay lists only what differs from the catalog under it.
    for lang in LANGS.iter().filter(|l| l.parent != 0) {
        let under = &LANGS[lang.parent];
        for (table, base) in [(lang.ui, under.ui), (lang.commands, under.commands), (lang.menu, under.menu)] {
            for (k, t) in table {
                let same = find(base, k).is_some_and(|b| forms(b) == forms(t));
                assert!(!same, "{}: {k:?} reads as in {}", lang.code, under.code);
            }
        }
    }
}

#[test]
fn french_and_portuguese_count_zero_as_singular() {
    let one = |code: &str, n: usize| plural_one(LANGS.iter().position(|l| l.code == code).unwrap(), n);
    for code in ["fr", "pt-BR"] {
        assert!(one(code, 0) && one(code, 1) && !one(code, 2), "{code}");
    }
    for code in ["en", "es", "de", "cs"] {
        for n in [0, 1, 2, 3, 4, 5, 21] {
            assert_eq!(one(code, n), n == 1, "{code}: {n}");
        }
    }
}

#[test]
fn japanese_korean_and_chinese_have_no_plural_forms() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    assert_eq!(code("system", Some("ja-JP")), "ja");
    assert_eq!(code("system", Some("ko_KR.UTF-8")), "ko");
    assert_eq!(code("system", Some("ja,en-US")), "ja");
    for code in ["ja", "ko", "zh-Hans"] {
        let lang = LANGS.iter().position(|l| l.code == code).unwrap();
        assert!(!plural_one(lang, 0) && !plural_one(lang, 1) && !plural_one(lang, 2), "{code}");
        let ones: Vec<_> = LANGS[lang].ui.iter().filter(|(_, t)| matches!(t, Text::Plural { one: Some(_), .. })).map(|(k, _)| k).collect();
        assert!(ones.is_empty(), "{code}: plural entries give only `other`: {ones:?}");
    }
}

#[test]
fn chinese_reads_simplified_and_traditional_has_no_catalog() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    for tag in ["zh-CN", "zh_CN.UTF-8", "zh-SG", "zh-Hans", "zh-Hans-CN", "zh-Hans-HK", "zh"] {
        assert_eq!(code("system", Some(tag)), "zh-Hans", "{tag}");
    }
    // No Traditional Chinese catalog: English, never Simplified; else the next OS language.
    for tag in ["zh-TW", "zh-HK", "zh-MO", "zh-Hant", "zh-Hant-TW", "zh_TW.UTF-8"] {
        assert_eq!(code("system", Some(tag)), "en", "{tag}");
        assert_eq!(code(tag, None), "en", "{tag}");
    }
    assert_eq!(code("system", Some("zh-TW,ja-JP")), "ja");
    assert_eq!(LANGS[resolve("zh-Hans", None)].name, "简体中文");
}

#[test]
fn the_key_scanner_reads_rust_literals() {
    let src = r#"x(tr("A \"b\"")); trf(
        "C {n}", &[]); attr("no"); tr_noop("D\
          E"); tr(label);"#;
    assert_eq!(literals(src), vec!["A \"b\"", "C {n}", "DE"]);
}

#[test]
fn czech_integer_plurals() {
    let cs = LANGS.iter().position(|l| l.code == "cs").unwrap();
    assert_eq!(plural("cs", 1), Plural::One);
    assert_eq!(fill(counted(cs, "{n} layers", 1), &[("n", &1)]), "1 vrstva");
    for n in [2, 3, 4] {
        assert_eq!(plural("cs", n), Plural::Few, "{n}");
        assert_eq!(fill(counted(cs, "{n} layers", n), &[("n", &n)]), format!("{n} vrstvy"));
    }
    for n in [0, 5, 11, 12, 21, 22, 100, 1_000_000] {
        assert_eq!(plural("cs", n), Plural::Other, "{n}");
        assert_eq!(fill(counted(cs, "{n} layers", n), &[("n", &n)]), format!("{n} vrstev"));
    }
    for (key, text) in LANGS[cs].ui {
        if let Text::Plural { one, few, .. } = text {
            assert!(one.is_some() && few.is_some(), "{key}: missing Czech plural forms");
        }
    }
}

#[test]
fn russian_language_tags_and_integer_plurals() {
    let ru = LANGS.iter().position(|l| l.code == "ru").unwrap();
    assert_eq!(LANGS[ru].name, "Русский");
    for tag in ["ru", "ru-RU", "ru_RU.UTF-8", "RU", "ru-BY", "ru-Cyrl-RU"] {
        assert_eq!(resolve(tag, None), ru, "{tag}");
        assert_eq!(resolve("system", Some(tag)), ru, "{tag}");
    }
    assert_eq!(resolve("system", Some("nl,ru-RU,en-US")), ru);
    for n in [1, 21, 31, 101, 1_001] {
        assert_eq!(plural("ru", n), Plural::One, "{n}");
        assert_eq!(fill(counted(ru, "{n} layers", n), &[("n", &n)]), format!("{n} слой"));
    }
    for n in [2, 3, 4, 22, 23, 24, 102, 104, 1_002] {
        assert_eq!(plural("ru", n), Plural::Few, "{n}");
        assert_eq!(fill(counted(ru, "{n} layers", n), &[("n", &n)]), format!("{n} слоя"));
    }
    for n in [0, 5, 10, 11, 12, 13, 14, 19, 20, 25, 100, 111, 112, 114, 1_000_000] {
        assert_eq!(plural("ru", n), Plural::Many, "{n}");
        assert_eq!(fill(counted(ru, "{n} layers", n), &[("n", &n)]), format!("{n} слоёв"));
    }
    for (key, text) in LANGS[ru].ui {
        if let Text::Plural { one, few, many, .. } = text {
            assert!(one.is_some() && few.is_some() && many.is_some(), "{key}: missing Russian plural forms");
        }
    }
}

#[test]
fn plural_fallback_uses_the_language_of_its_catalog() {
    let la = LANGS.iter().position(|l| l.code == "es-419").unwrap();
    assert_eq!(counted(la, "{n} layers", 1), "{n} capa");
    assert_eq!(counted(la, "{n} layers", 21), "{n} capas");
    // Unsupported optional categories use other; existing one/other languages are unchanged.
    for code in ["en", "es", "de"] {
        for n in [0, 2, 11, 21, 1_000_000] {
            assert_eq!(plural(code, n), Plural::Other, "{code}: {n}");
        }
        assert_eq!(plural(code, 1), Plural::One, "{code}");
    }
    assert_eq!(counted(0, "{n} layers", 21), "{n} layers");
    assert_eq!(counted(0, "unknown {n}", 21), "unknown {n}");
}

#[test]
fn the_language_preference_takes_language_codes() {
    let mut s = figcraft_engine::Session::new();
    assert_eq!(s.prefs.language, "system");
    s.execute("pref.language", &serde_json::json!({"language": "es-419"})).unwrap();
    assert_eq!(s.prefs.language, "es-419");
    for bad in ["", "../es", "es--ES", "abcdefghi"] {
        assert!(s.execute("pref.language", &serde_json::json!({"language": bad})).is_err(), "{bad:?}");
    }
}
