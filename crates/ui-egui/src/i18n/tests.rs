//! The catalog checks: every key the code looks up is in the English catalog, no catalog has keys
//! the code doesn't use, and placeholders match English. A missing translation reads English and
//! fails nothing: [`missing_translations_report`] lists them (`cargo xtask i18n-report`).

use std::collections::BTreeSet;

use super::{Forms, LANGS, Lang, Plural, Text, chain, counted, fill, find, lookup, plural, plural_one, resolve};

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
    // Board shapes and stamps (the board tools' flyouts and the contextual bar).
    v.extend(figcraft_doc::BoardShape::ALL.iter().map(|b| b.label()));
    v.extend(figcraft_doc::Stamp::ALL.iter().map(|b| b.label()));
    v.extend(figcraft_engine::board_templates::TEMPLATES.iter().map(|t| t.name));
    v.extend(Group::ALL.iter().map(|g| g.label()));
    v.extend(GESTURES.iter().flat_map(|(_, label, mac, other)| [*label, *mac, *other]));
    v.extend(DATA_LABELS);
    v.push(figcraft_engine::live::TOO_LARGE_TO_JOIN);
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
    for path in sources(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with("_tests.rs") || name == "i18n.rs" {
            continue;
        }
        keys.extend(literals(&std::fs::read_to_string(&path).unwrap(), LOOKUPS));
    }
    keys
}

/// Every `.rs` file under `dir`, in subdirectories too, leaving out test code: `tests.rs` and
/// anything under a `tests` directory.
fn sources(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "tests" {
                out.extend(sources(&path));
            }
        } else if name.ends_with(".rs") && name != "tests.rs" {
            out.push(path);
        }
    }
    out
}

/// The importers' fixed notes: the string literals passed to `warn` in the `.fig` and REST import
/// sources. The import report shows a note translated when the catalog has it, so each one is a key
/// `en.toml` (and every complete language) must have, and an entry for a note the importers no
/// longer give shows up as unused.
fn importer_notes() -> BTreeSet<String> {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../format/src");
    let mut keys = BTreeSet::new();
    for dir in ["fig", "rest"] {
        // The module root sits beside its directory (`fig.rs` + `fig/`) or inside it (`fig/mod.rs`).
        let root = src.join(format!("{dir}.rs"));
        for path in sources(&src.join(dir)).into_iter().chain(root.exists().then_some(root)) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // export.rs and export/ warn about what a `.fig` export leaves out, not about an import.
            if name.contains("tests") || name == "export.rs" || path.starts_with(src.join("fig/export")) {
                continue;
            }
            keys.extend(literals(&std::fs::read_to_string(&path).unwrap(), &["warn("]));
        }
    }
    keys
}

/// The lookup functions whose first argument is a catalog key.
const LOOKUPS: &[&str] = &["tr(", "trf(", "trn(", "tr_noop("];

/// The first-argument string literals of the functions `fns` (each with its `(`) in `src`.
fn literals(src: &str, fns: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for f in fns {
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
        Text::Plural(Forms { one, few, many, other }) => one.iter().chain(few).chain(many).copied().chain([*other]).collect(),
    }
}

/// The browser build bakes in every catalog: a row is a key and a two-word entry, not an entry
/// with room for four strings.
#[test]
fn a_catalog_row_is_four_words() {
    assert_eq!(std::mem::size_of::<Text>(), 2 * std::mem::size_of::<usize>());
    assert_eq!(std::mem::size_of::<(&str, Text)>(), 4 * std::mem::size_of::<usize>());
}

fn english() -> &'static Lang {
    LANGS.first().filter(|l| l.code == "en").expect("locales/en.toml is baked in first")
}

#[test]
fn every_key_in_the_code_is_in_the_english_catalog_and_no_other() {
    let used: BTreeSet<String> = used_keys().into_iter().chain(dynamic_keys()).chain(importer_notes()).collect();
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

/// The ids `[commands]` entries translate: every command and every main-menu item.
fn command_ids() -> BTreeSet<&'static str> {
    figcraft_engine::all_commands()
        .iter()
        .map(|c| c.id)
        .chain(figcraft_engine::catalog::MENU.iter().map(|m| m.id).filter(|id| !id.is_empty()))
        .collect()
}

#[test]
fn catalogs_have_known_keys_and_matching_placeholders() {
    let en = english();
    let ids = command_ids();
    for lang in LANGS {
        assert!(!lang.name.is_empty(), "{}: [meta] name is missing", lang.code);
        for (k, t) in lang.ui {
            let src = find(en.ui, k).unwrap_or_else(|| panic!("{}: unknown key {k:?}", lang.code));
            let want = placeholders(forms(src).last().unwrap());
            for (i, form) in forms(t).iter().enumerate() {
                let got = placeholders(form);
                // A "one" form may spell the count out ("una capa").
                let ok = if matches!(t, Text::Plural(Forms { one: Some(_), .. })) && i == 0 { got.is_subset(&want) } else { got == want };
                assert!(ok, "{}: {k:?} has placeholders {got:?}, English has {want:?}", lang.code);
            }
            assert_eq!(matches!(t, Text::Plural(_)), matches!(src, Text::Plural(_)), "{}: {k:?} plural or not as in English", lang.code);
        }
        for (id, t) in lang.commands.iter().chain(lang.menu) {
            assert!(ids.contains(id), "{}: unknown command id {id:?}", lang.code);
            assert!(matches!(t, Text::Plain(s) if !s.trim().is_empty()), "{}: {id:?} is empty or plural", lang.code);
        }
        for (id, _) in lang.menu {
            assert!(menu_worded_differently().contains(id), "{}: [menu] {id:?} reads like the command; [commands] covers it", lang.code);
        }
    }
}

/// What each language lacks: UI strings, command labels and menu wordings that read as in English
/// because neither its catalog nor the one it overlays has them. It prints them and always passes.
#[test]
fn missing_translations_report() {
    let ids = command_ids();
    let worded = menu_worded_differently();
    for (i, lang) in LANGS.iter().enumerate().filter(|(_, l)| l.code != "en") {
        let own = || chain(i).filter(|l| l.code != "en");
        let ui: Vec<_> = english().ui.iter().map(|(k, _)| *k).filter(|k| own().all(|l| find(l.ui, k).is_none())).collect();
        let commands: Vec<_> = ids.iter().filter(|id| own().all(|l| find(l.commands, id).is_none())).collect();
        let menu: Vec<_> = worded.iter().filter(|id| own().all(|l| find(l.menu, id).is_none())).collect();
        println!("{}: {} UI strings, {} command labels, {} menu wordings missing", lang.code, ui.len(), commands.len(), menu.len());
        for k in ui {
            println!("  {k:?}");
        }
        for id in commands {
            println!("  [commands] {id}");
        }
        for id in menu {
            println!("  [menu] {id}");
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
fn a_live_failure_the_ui_knows_is_read_back_for_translation() {
    use figcraft_engine::live::TOO_LARGE_TO_JOIN;
    let reason = TOO_LARGE_TO_JOIN.replace("{mb}", "1024");
    assert!(matches!(crate::live::failure_text(&reason), std::borrow::Cow::Owned(t) if t == reason));
    assert!(matches!(crate::live::failure_text("Couldn't reach the server"), std::borrow::Cow::Borrowed(_)));
}

#[test]
fn system_languages_pick_the_first_one_with_a_catalog() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    assert_eq!(code("system", Some("fr-CA")), "fr");
    assert_eq!(code("system", Some("de_CH.UTF-8")), "de");
    assert_eq!(code("system", Some("cs_CZ.UTF-8")), "cs");
    assert_eq!(code("system", Some("cs-CZ,en-US")), "cs");
    assert_eq!(code("cs", None), "cs");
    assert_eq!(code("system", Some("pt-BR")), "pt-BR");
    assert_eq!(code("system", Some("tr_TR.UTF-8")), "tr");
    assert_eq!(code("system", Some("id_ID.UTF-8")), "id");
    assert_eq!(code("system", Some("vi_VN.UTF-8")), "vi");
    // Portugal's Portuguese and a bare "pt" read the Brazilian catalog.
    assert_eq!(code("system", Some("pt-PT")), "pt-BR");
    assert_eq!(code("pt", None), "pt-BR");
    // An OS list: the first language there is a catalog for (English counts). Hindi has no
    // catalog (Dutch gained one; `hi` keeps exercising the no-catalog path).
    assert_eq!(code("system", Some("hi-IN,de-DE,en-US")), "de");
    assert_eq!(code("system", Some("hi-IN, en-US,fr-FR")), "en");
    assert_eq!(code("system", Some("hi,da")), "en");
    assert_eq!(code("system", Some("hi,sv")), "sv");
    assert_eq!(code("fr", Some("de-DE")), "fr", "a chosen language beats the system's");
}

#[test]
fn italian_is_picked_for_italian_locales() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    for tag in ["it", "it-IT", "it-CH", "it_IT.UTF-8"] {
        assert_eq!(code("system", Some(tag)), "it", "{tag}");
    }
    let it = LANGS.iter().position(|l| l.code == "it").unwrap();
    assert_eq!(LANGS[it].name, "Italiano");
    assert!(LANGS[it].draft);
    assert!(!plural_one(it, 0) && plural_one(it, 1) && !plural_one(it, 2));
}

#[test]
fn swedish_is_picked_for_swedish_locales() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    for tag in ["sv", "sv-SE", "sv-FI", "sv_SE.UTF-8"] {
        assert_eq!(code("system", Some(tag)), "sv", "{tag}");
    }
    let sv = LANGS.iter().position(|l| l.code == "sv").unwrap();
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
    for (k, _) in LANGS[es].ui.iter().filter(|(k, _)| find(LANGS[la].ui, k).is_none()) {
        assert!(lookup(la, k).is_some_and(|t| std::ptr::eq(t, find(LANGS[es].ui, k).unwrap())), "{k:?}");
    }
    let undo = |i: usize| chain(i).find_map(|l| find(l.commands, "edit.undo")).map(forms);
    assert_eq!(undo(la), undo(es), "command labels fall back too");
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
    for code in ["en", "es", "de", "cs", "pl"] {
        for n in [0, 1, 2, 3, 4, 5, 21] {
            assert_eq!(one(code, n), n == 1, "{code}: {n}");
        }
    }
}

#[test]
fn japanese_korean_chinese_and_vietnamese_have_no_plural_forms() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    assert_eq!(code("system", Some("ja-JP")), "ja");
    assert_eq!(code("system", Some("ko_KR.UTF-8")), "ko");
    assert_eq!(code("system", Some("vi-VN")), "vi");
    assert_eq!(code("system", Some("ja,en-US")), "ja");
    assert_eq!(LANGS[resolve("vi", None)].name, "Tiếng Việt");
    assert!(LANGS[resolve("vi", None)].draft);
    for code in ["ja", "ko", "th", "zh-Hans", "vi"] {
        let lang = LANGS.iter().position(|l| l.code == code).unwrap();
        assert!(!plural_one(lang, 0) && !plural_one(lang, 1) && !plural_one(lang, 2), "{code}");
        let ones: Vec<_> = LANGS[lang].ui.iter().filter(|(_, t)| matches!(t, Text::Plural(Forms { one: Some(_), .. }))).map(|(k, _)| k).collect();
        assert!(ones.is_empty(), "{code}: plural entries give only `other`: {ones:?}");
    }
}

#[test]
fn georgian_is_picked_for_georgian_locales() {
    let code = |pref: &str, system: Option<&str>| LANGS[resolve(pref, system)].code;
    for tag in ["ka", "ka-GE", "ka_GE.UTF-8"] {
        assert_eq!(code("system", Some(tag)), "ka", "{tag}");
    }
}

#[test]
fn arabic_tags_and_count_neutral_forms() {
    let ar = LANGS.iter().position(|l| l.code == "ar").unwrap();
    assert_eq!(LANGS[ar].name, "العربية");
    for tag in ["ar", "ar-SA", "ar_EG.UTF-8", "ar_AE@arabic"] {
        assert_eq!(resolve("system", Some(tag)), ar, "{tag}");
        assert_eq!(resolve(tag, None), ar, "{tag}");
    }
    let counts = [0, 1, 2, 3, 10, 11, 99, 100, 102, 103, 111, 200, 1000];
    for (key, text) in LANGS[ar].ui {
        if let Text::Plural(Forms { one, few, many, other }) = text {
            assert!(one.is_none() && few.is_none() && many.is_none(), "{key}: count-neutral wording uses only other");
            for n in counts {
                assert_eq!(counted(ar, key, n), *other, "{key}, {n}");
                assert!(fill(other, &[("n", &n)]).contains(&n.to_string()));
            }
        }
    }
}

#[test]
fn the_built_in_arabic_face_draws_the_arabic_catalog() {
    if !figcraft_text::CRAFT_FONTS.iter().any(|f| f.family == "Noto Sans Arabic" && f.style == "Regular") {
        eprintln!("built without craft-fonts' Noto Sans Arabic; skipping");
        return;
    }
    let Some(face) = figcraft_text::face_with_char('ع', &["Noto Sans Arabic"]) else {
        panic!("the supplied Noto Sans Arabic face must draw Arabic");
    };
    let ar = &LANGS[LANGS.iter().position(|l| l.code == "ar").unwrap()];
    let letters: BTreeSet<char> = ar
        .ui
        .iter()
        .chain(ar.commands)
        .chain(ar.menu)
        .flat_map(|(_, t)| forms(t))
        .flat_map(str::chars)
        .filter(|c| matches!(u32::from(*c), 0x0600..=0x06FF))
        .collect();
    assert!(letters.len() > 28);
    let missing: String = letters.iter().filter(|c| !face.has_char(**c)).collect();
    assert!(missing.is_empty(), "Arabic catalog glyphs missing from the embedded face: {missing:?}");
}

/// Release builds embed craft-fonts' Noto Sans Georgian (`theme::install_fonts`' built-in faces),
/// since Inter and egui's fonts have no Georgian: it draws every Georgian letter the catalog uses.
#[test]
fn the_built_in_georgian_face_draws_the_georgian_catalog() {
    let Some(face) = figcraft_text::find_face("Noto Sans Georgian", "Regular").filter(|f| f.family == "Noto Sans Georgian") else {
        eprintln!("built without craft-fonts' Noto Sans Georgian; skipping");
        return;
    };
    let ka = &LANGS[LANGS.iter().position(|l| l.code == "ka").unwrap()];
    let georgian = |c: &char| matches!(u32::from(*c), 0x10A0..=0x10FF | 0x1C90..=0x1CBF | 0x2D00..=0x2D2F);
    let mut letters: BTreeSet<char> = BTreeSet::new();
    for (_, t) in ka.ui.iter().chain(ka.commands).chain(ka.menu) {
        letters.extend(forms(t).into_iter().flat_map(str::chars).filter(georgian));
    }
    assert!(letters.len() > 30, "{letters:?}");
    let missing: String = letters.iter().filter(|c| !face.has_char(**c)).collect();
    assert!(missing.is_empty(), "no glyph for {missing:?}");
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
}

#[test]
fn the_key_scanner_reads_rust_literals() {
    let src = r#"x(tr("A \"b\"")); trf(
        "C {n}", &[]); attr("no"); tr_noop("D\
          E"); tr(label);"#;
    assert_eq!(literals(src, LOOKUPS), vec!["A \"b\"", "C {n}", "DE"]);
    assert_eq!(literals(r#"self.warn("a note"); s.warn(format!("{n}")); forewarn("no")"#, &["warn("]), vec!["a note"]);
}

#[test]
fn czech_integer_plurals() {
    assert_eq!(plural("cs", 1), Plural::One);
    for n in [2, 3, 4] {
        assert_eq!(plural("cs", n), Plural::Few, "{n}");
    }
    for n in [0, 5, 11, 12, 21, 22, 100, 1_000_000] {
        assert_eq!(plural("cs", n), Plural::Other, "{n}");
    }
}

#[test]
fn russian_language_tags_and_integer_plurals() {
    let ru = LANGS.iter().position(|l| l.code == "ru").unwrap();
    for tag in ["ru", "ru-RU", "ru_RU.UTF-8", "RU", "ru-BY", "ru-Cyrl-RU"] {
        assert_eq!(resolve(tag, None), ru, "{tag}");
        assert_eq!(resolve("system", Some(tag)), ru, "{tag}");
    }
    assert_eq!(resolve("system", Some("hi,ru-RU,en-US")), ru);
    for n in [1, 21, 31, 101, 1_001] {
        assert_eq!(plural("ru", n), Plural::One, "{n}");
    }
    for n in [2, 3, 4, 22, 23, 24, 102, 104, 1_002] {
        assert_eq!(plural("ru", n), Plural::Few, "{n}");
    }
    for n in [0, 5, 10, 11, 12, 13, 14, 19, 20, 25, 100, 111, 112, 114, 1_000_000] {
        assert_eq!(plural("ru", n), Plural::Many, "{n}");
    }
}

#[test]
fn polish_integer_plurals() {
    assert_eq!(plural("pl", 1), Plural::One);
    for n in [2, 3, 4, 22, 23, 24, 102, 104, 1_002] {
        assert_eq!(plural("pl", n), Plural::Few, "{n}");
    }
    for n in [0, 5, 10, 11, 12, 13, 14, 19, 20, 21, 25, 31, 100, 101, 111, 112, 114, 1_000_000] {
        assert_eq!(plural("pl", n), Plural::Many, "{n}");
    }
}

#[test]
fn plural_fallback_uses_the_language_of_its_catalog() {
    // A plural es-419 doesn't overlay reads Spanish's entry under Spanish's rule.
    let es = LANGS.iter().position(|l| l.code == "es").unwrap();
    let la = LANGS.iter().position(|l| l.code == "es-419").unwrap();
    for n in [1, 21] {
        assert_eq!(counted(la, "{n} layers", n), counted(es, "{n} layers", n), "{n}");
    }
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
