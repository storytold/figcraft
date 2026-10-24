//! Downloading missing fonts (#754) against a scripted font library: nothing is sent without
//! `confirm: true`, the library's licence folders are tried in turn, every file is checked and
//! only then saved and drawn, and a run in the background (one request at a time, the command
//! run again for each answer) installs every family once.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use figcraft_collab::message::ClientMsg;
use serde_json::{Value, json};
use write_fonts::read::FontRef;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::types::NameId;

use super::*;
use crate::live::{Event, HostOptions, LiveLink, NetReply, NetRequest, Presence, SessionInfo};

/// The scripted library: files by URL, the requests it saw, and background requests waiting.
#[derive(Default)]
struct Wire {
    files: BTreeMap<String, Vec<u8>>,
    asked: Vec<String>,
    background: bool,
    out: Vec<(u64, NetRequest)>,
    answers: Vec<Event>,
}

struct Library(Arc<Mutex<Wire>>);

impl LiveLink for Library {
    fn host(&mut self, _: u64, _: HostOptions) -> std::result::Result<SessionInfo, String> {
        Err("no".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> std::result::Result<(), String> {
        Err("no".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, Event)> {
        std::mem::take(&mut self.0.lock().unwrap().answers).into_iter().map(|e| (0, e)).collect()
    }
    fn fetch(&mut self, url: &str, _: usize) -> std::result::Result<Option<Vec<u8>>, String> {
        let mut w = self.0.lock().unwrap();
        w.asked.push(url.to_string());
        Ok(w.files.get(url).cloned())
    }
    fn background(&mut self, job: u64, req: NetRequest) -> bool {
        let mut w = self.0.lock().unwrap();
        if w.background {
            w.out.push((job, req));
        }
        w.background
    }
}

/// A session with the scripted library and a fonts folder of its own.
fn session(wire: &Arc<Mutex<Wire>>, dir: &str) -> (Session, std::path::PathBuf) {
    let mut s = Session::new();
    s.live.install(Box::new(Library(wire.clone())));
    let root = std::env::temp_dir().join(format!("figcraft-font-downloads-{dir}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    s.fonts_dir = Some(root.clone());
    (s, root)
}

/// The bytes of an installed font file (not a collection), if this machine has one.
fn some_font() -> Option<Vec<u8>> {
    figcraft_text::scan_system_fonts();
    figcraft_text::family_names().into_iter().find_map(|f| {
        let face = figcraft_text::find_face(&f, "Regular")?;
        let bytes = face.bytes.data().to_vec();
        (face.index == 0 && FontRef::new(&bytes).is_ok()).then_some(bytes)
    })
}

/// `font` renamed to family `family`, style `style`.
fn renamed(font: &[u8], family: &str, style: &str) -> Vec<u8> {
    let rec = |id: NameId, s: String| NameRecord::new(3, 1, 0x409, id, s.into());
    let name = Name::new(vec![
        rec(NameId::FAMILY_NAME, family.into()),
        rec(NameId::SUBFAMILY_NAME, style.into()),
        rec(NameId::FULL_NAME, format!("{family} {style}")),
        rec(NameId::POSTSCRIPT_NAME, format!("FigCraftTest-{style}")),
    ]);
    write_fonts::FontBuilder::new().add_table(&name).unwrap().copy_missing_tables(FontRef::new(font).unwrap()).build()
}

/// Put `family` in the library's `licence` folder with its font `files` (name, style).
fn publish(wire: &Arc<Mutex<Wire>>, font: &[u8], family: &str, licence: (&str, &str, &str), files: &[(&str, &str)]) {
    let base = format!("{LIBRARY}{}/{}/", licence.0, folder_name(family));
    let listed: String = files.iter().map(|(f, _)| format!("fonts {{\n  name: \"{family}\"\n  filename: \"{f}\"\n}}\n")).collect();
    let meta = format!("name: \"{family}\"\ndesigner: \"Test\"\nlicense: \"{}\"\n{listed}subsets: \"latin\"\n", licence.1);
    let mut w = wire.lock().unwrap();
    w.files.insert(format!("{base}METADATA.pb"), meta.into_bytes());
    for (f, style) in files {
        w.files.insert(format!("{base}{}", url_segment(f)), renamed(font, family, style));
    }
    w.files.insert(format!("{base}{}", licence.2), b"Licence text".to_vec());
}

const OFL: (&str, &str, &str) = ("ofl", "OFL", "OFL.txt");
const UFL: (&str, &str, &str) = ("ufl", "UFL", "UFL.txt");

#[test]
fn metadata_gives_the_family_licence_and_plain_font_files() {
    let text = "name: \"Pinyon Script\"\ndesigner: \"Nicole Fally\"\nlicense: \"OFL\"\ncategory: \"HANDWRITING\"\nfonts {\n  name: \"Pinyon Script\"\n  style: \"normal\"\n  filename: \"PinyonScript-Regular.ttf\"\n}\nsource {\n  files {\n    source_file: \"OFL.txt\"\n  }\n}\n";
    let m = parse_metadata(text).unwrap();
    assert_eq!(m, Metadata { name: "Pinyon Script".into(), license: "OFL".into(), files: vec!["PinyonScript-Regular.ttf".into()] });
    assert!(parse_metadata("fonts {\n  name: \"Nested\"\n}\nlicense: \"OFL\"\n").is_none(), "a nested name isn't the family's");
    assert!(parse_metadata("name: \"A\\\"B\"\nlicense: \"OFL\"\n").is_none(), "escapes are refused");
    assert!(parse_metadata("\u{0}\u{ffff}").is_none());
    let unreadable = "name: \"A\"\nlicense: \"OFL\"\nfonts {\n  filename: \"A-Regular.ttf\"\n  filename: \"A\\\"Bold.ttf\"\n}\n";
    assert!(parse_metadata(unreadable).is_none(), "one unreadable file name refuses the entry, not part of the family");
    assert!(parse_metadata("name: \"A\"\nlicense: \"OFL\"\n  filename: \"A.ttf\" # c\n").is_none());
    for bad in ["../evil.ttf", "a/b.ttf", "a\\b.ttf", ".hidden.ttf", "font.woff2", "C:x.ttf", "", "CON.ttf", "nul.otf", "Com1.ttf", "LPT9.x.ttf"] {
        assert!(!plain_font_file(bad), "{bad}");
    }
    assert!(plain_font_file("Console.ttf") && plain_font_file("COM0.ttf") && plain_font_file("COM10.ttf"));
    assert_eq!(safe_folder("Aux"), "_Aux");
    assert_eq!(safe_folder(&format!("{}x", "a ".repeat(50))), "a ".repeat(50).trim_end(), "no trailing space");
    assert!(plain_font_file("Inter[opsz,wght].ttf") && plain_font_file("Font-Bold.OTF"));
    assert_eq!(folder_name("M PLUS 1p"), "mplus1p");
    assert_eq!(folder_name("游ゴシック"), "");
    assert_eq!(url_segment("Inter[opsz,wght].ttf"), "Inter%5Bopsz%2Cwght%5D.ttf");
}

#[test]
fn nothing_is_sent_without_confirm_or_a_network() {
    let wire = Arc::new(Mutex::new(Wire::default()));
    let (mut s, _) = session(&wire, "confirm");
    let r = s.execute("text.downloadFonts", &json!({"families": ["FigCraft Never Sent"]}));
    assert!(r.unwrap_err().to_string().contains("confirm: true"));
    assert!(s.execute("text.downloadFonts", &json!({"families": ["FigCraft Never Sent"], "confirm": "yes"})).is_err());
    assert!(wire.lock().unwrap().asked.is_empty(), "no request without confirm: true");
    let mut offline = Session::new();
    let r = offline.execute("text.downloadFonts", &json!({"families": ["FigCraft Never Sent"], "confirm": true}));
    assert!(r.unwrap_err().to_string().contains(NO_DOWNLOADS));
}

#[test]
fn installs_from_the_licence_folder_that_has_the_family_and_draws_it() {
    let Some(font) = some_font() else {
        eprintln!("skipped: no fonts on this machine");
        return;
    };
    let wire = Arc::new(Mutex::new(Wire::default()));
    let family = "FigCraft Download Ufl";
    publish(&wire, &font, family, UFL, &[("FigCraftDownloadUfl-Regular.ttf", "Regular"), ("FigCraftDownloadUfl-Bold.ttf", "Bold")]);
    let (mut s, root) = session(&wire, "ufl");
    let t = s.execute("text.create", &json!({"text": "Hamburgefonstiv", "x": 0, "y": 0, "props": {"fontFamily": family}})).unwrap();
    assert_eq!(missing(s.doc().unwrap()), vec![Missing { family: family.into(), layers: 1 }]);
    // Its box as if measured in a fallback font wider than the family.
    let id = s.doc().unwrap().nodes().find(|n| matches!(n.kind, figcraft_doc::NodeKind::Text(_))).unwrap().id;
    let doc = s.doc_mut().unwrap();
    doc.get_mut(id).unwrap().width += 40.0;
    doc.take_dirty();
    let generation = figcraft_text::layout_generation();

    // The open document's missing fonts, by default.
    let r = s.execute("text.downloadFonts", &json!({"confirm": true})).unwrap();
    let folder = root.join(FOLDER).join(family);
    assert_eq!(r["failed"], json!([]));
    assert_eq!(
        r["installed"],
        json!([{"family": family, "license": "UFL", "files": ["FigCraftDownloadUfl-Regular.ttf", "FigCraftDownloadUfl-Bold.ttf"], "path": folder.to_string_lossy()}])
    );
    let asked = wire.lock().unwrap().asked.clone();
    assert!(asked[0].ends_with("/ofl/figcraftdownloadufl/METADATA.pb") && asked[1].ends_with("/apache/figcraftdownloadufl/METADATA.pb"), "{asked:?}");
    assert_eq!(asked.len(), 6, "three folders tried, two fonts and the licence: {asked:?}");
    let mut saved: Vec<String> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    saved.sort();
    assert_eq!(saved, ["FigCraftDownloadUfl-Bold.ttf", "FigCraftDownloadUfl-Regular.ttf", "UFL.txt"]);
    assert!(!root.join(FOLDER).join(format!(".{family}.partial")).exists());

    // Text in the family uses it now.
    assert!(figcraft_text::has_family(family));
    assert_eq!(figcraft_text::find_face(family, "Bold").unwrap().family, family);
    assert!(figcraft_text::layout_generation() > generation, "text is laid out again");
    assert!(missing(s.doc().unwrap()).is_empty());
    // Its auto-sizing box fits the downloaded font.
    assert_eq!(r["refitted"], json!([id]));
    let n = s.doc().unwrap().node(id).unwrap();
    assert!((n.width - figcraft_text::auto_size(n).unwrap().0).abs() < 1e-6);
    let _ = t;
    // Asking again downloads nothing, and says why.
    wire.lock().unwrap().asked.clear();
    let r = s.execute("text.downloadFonts", &json!({"families": [family, family.to_lowercase()], "confirm": true})).unwrap();
    assert_eq!(r, json!({"installed": [], "failed": [{"family": family, "reason": "it's already installed"}], "refitted": []}));
    assert!(wire.lock().unwrap().asked.is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn refusals_keep_nothing() {
    let Some(font) = some_font() else {
        eprintln!("skipped: no fonts on this machine");
        return;
    };
    let wire = Arc::new(Mutex::new(Wire::default()));
    publish(&wire, &font, "FigCraft Refused Licence", ("ofl", "PROPRIETARY", "OFL.txt"), &[("A.ttf", "Regular")]);
    // A file of another family under this family's name.
    publish(&wire, &font, "FigCraft Refused Other", OFL, &[("B.ttf", "Regular")]);
    let other = format!("{LIBRARY}ofl/figcraftrefusedother/B.ttf");
    wire.lock().unwrap().files.insert(other, renamed(&font, "FigCraft Someone Else", "Regular"));
    publish(&wire, &font, "FigCraft Refused Path", OFL, &[("../C.ttf", "Regular")]);
    // A library entry whose font file is gone.
    publish(&wire, &font, "FigCraft Refused Gone", OFL, &[("D.ttf", "Regular")]);
    wire.lock().unwrap().files.retain(|k, _| !k.ends_with("/D.ttf"));
    let (mut s, root) = session(&wire, "refused");
    let families =
        ["FigCraft Refused Licence", "FigCraft Refused Other", "FigCraft Refused Path", "FigCraft Refused Gone", "FigCraft Not In Library"];
    let r = s.execute("text.downloadFonts", &json!({"families": families, "confirm": true})).unwrap();
    assert_eq!(r["installed"], json!([]));
    let reasons: BTreeMap<String, String> =
        r["failed"].as_array().unwrap().iter().map(|f| (f["family"].as_str().unwrap().into(), f["reason"].as_str().unwrap().into())).collect();
    let why = |f: &str| reasons.get(f).cloned().unwrap_or_default();
    assert_eq!(reasons.len(), 5, "{reasons:?}");
    assert!(why("FigCraft Refused Licence").contains("licence (PROPRIETARY)"), "{reasons:?}");
    assert!(why("FigCraft Refused Other").contains("is a font of FigCraft Someone Else"), "{reasons:?}");
    assert!(why("FigCraft Refused Path").contains("no font files"), "{reasons:?}");
    assert!(why("FigCraft Refused Gone").contains("D.ttf is missing"), "{reasons:?}");
    assert!(why("FigCraft Not In Library").contains("isn't in the Google Fonts library"), "{reasons:?}");
    assert!(!root.exists(), "nothing saved");
    assert!(families.iter().all(|f| !figcraft_text::has_family(f)));
}

#[test]
fn past_32_families_the_rest_are_reported_not_refused() {
    let wire = Arc::new(Mutex::new(Wire::default()));
    let (mut s, _) = session(&wire, "many");
    let families: Vec<String> = (0..34).map(|i| format!("FigCraft Not There {i}")).collect();
    let r = s.execute("text.downloadFonts", &json!({"families": families, "confirm": true})).unwrap();
    let failed = r["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 34);
    assert_eq!(wire.lock().unwrap().asked.len(), 32 * 3, "32 families looked up in three licence folders");
    let late: Vec<&str> =
        failed.iter().filter(|f| f["reason"].as_str().unwrap().contains("at most 32")).map(|f| f["family"].as_str().unwrap()).collect();
    assert_eq!(late, ["FigCraft Not There 32", "FigCraft Not There 33"]);
}

#[test]
fn a_half_written_folder_from_an_earlier_download_goes() {
    let Some(font) = some_font() else {
        eprintln!("skipped: no fonts on this machine");
        return;
    };
    let wire = Arc::new(Mutex::new(Wire::default()));
    publish(&wire, &font, "FigCraft Partial Sweep", OFL, &[("P.ttf", "Regular")]);
    let (mut s, root) = session(&wire, "partial");
    let stale = root.join(FOLDER).join(".Someone Else.partial");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("Cut.ttf"), b"half").unwrap();
    // Asked in another spelling, it's kept under the library's.
    s.execute("text.downloadFonts", &json!({"families": ["figcraft PARTIAL sweep"], "confirm": true})).unwrap();
    assert!(!stale.exists());
    assert_eq!(names(&root.join(FOLDER)), ["FigCraft Partial Sweep"]);
    assert!(root.join(FOLDER).join("FigCraft Partial Sweep").join("P.ttf").is_file());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn without_a_fonts_folder_they_last_for_the_session() {
    let Some(font) = some_font() else {
        eprintln!("skipped: no fonts on this machine");
        return;
    };
    let wire = Arc::new(Mutex::new(Wire::default()));
    publish(&wire, &font, "FigCraft Session Only", OFL, &[("S.ttf", "Regular")]);
    let (mut s, _) = session(&wire, "session");
    s.fonts_dir = None;
    let r = s.execute("text.downloadFonts", &json!({"families": ["FigCraft Session Only"], "confirm": true})).unwrap();
    assert_eq!(r["installed"][0]["path"], Value::Null);
    assert!(figcraft_text::has_family("FigCraft Session Only"));
}

/// Answer the background requests waiting, as the network thread would.
fn answer(s: &mut Session, wire: &Arc<Mutex<Wire>>) -> usize {
    let out = std::mem::take(&mut wire.lock().unwrap().out);
    for (job, req) in &out {
        let NetRequest::Fetch { url, .. } = req else { panic!("{req:?}") };
        let mut w = wire.lock().unwrap();
        w.asked.push(url.clone());
        let reply = NetReply::Fetch(Ok(w.files.get(url).cloned().map(Arc::new)));
        w.answers.push(Event::Net { job: *job, reply: Box::new(reply) });
    }
    s.live_poll();
    out.len()
}

#[test]
fn in_the_background_every_family_is_downloaded_before_any_is_kept() {
    let Some(font) = some_font() else {
        eprintln!("skipped: no fonts on this machine");
        return;
    };
    let wire = Arc::new(Mutex::new(Wire { background: true, ..Wire::default() }));
    publish(&wire, &font, "FigCraft Background One", OFL, &[("One-Regular.ttf", "Regular"), ("One-Bold.ttf", "Bold")]);
    publish(&wire, &font, "FigCraft Background Two", OFL, &[("Two-Regular.ttf", "Regular"), ("Two-Italic.ttf", "Italic")]);
    let (mut s, root) = session(&wire, "background");
    let families = ["FigCraft Background One", "FigCraft Background Two"];
    let started = s.start_background("text.downloadFonts", json!({"families": families, "confirm": true})).unwrap();
    assert!(started.is_none(), "it waits for the library");
    let mut answered = 0;
    while s.waiting_for("text.downloadFonts") {
        // Nothing is kept while answers are still coming.
        assert!(!root.exists() && families.iter().all(|f| !figcraft_text::has_family(f)));
        answered += answer(&mut s, &wire);
        assert!(answered < 50, "it ends");
    }
    assert_eq!(answered, 8, "two families: their entry, two fonts and the licence each, one request at a time");
    let (command, result) = s.take_background_done().unwrap();
    assert_eq!(command, "text.downloadFonts");
    let result = result.unwrap();
    let installed: Vec<&str> = result["installed"].as_array().unwrap().iter().map(|f| f["family"].as_str().unwrap()).collect();
    assert_eq!(installed, families);
    for (f, style) in [(families[0], "Bold"), (families[1], "Italic")] {
        assert_eq!(figcraft_text::find_face(f, style).unwrap().style, style, "{f}");
        assert_eq!(std::fs::read_dir(root.join(FOLDER).join(f)).unwrap().count(), 3);
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// A download of `family` with one font file and a licence, each holding `tag`.
fn saved(family: &str, tag: &str) -> Download {
    Download {
        family: family.into(),
        name: family.into(),
        license: "OFL".into(),
        files: vec![("F.ttf".into(), Arc::new(format!("font {tag}").into_bytes()))],
        licence_file: ("OFL.txt".into(), Arc::new(format!("licence {tag}").into_bytes())),
    }
}

fn read(folder: &std::path::Path) -> (String, String) {
    (std::fs::read_to_string(folder.join("F.ttf")).unwrap(), std::fs::read_to_string(folder.join("OFL.txt")).unwrap())
}

fn names(folder: &std::path::Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(folder).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    n.sort();
    n
}

fn rename(a: &std::path::Path, b: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(a, b)
}

#[test]
fn a_failed_replacement_keeps_the_earlier_family() {
    let root = std::env::temp_dir().join(format!("figcraft-font-replace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let folder = std::path::PathBuf::from(save_with(&root, &saved("Kept", "old"), &rename).unwrap());
    // The new family can't be moved into place.
    let refuse = |a: &std::path::Path, b: &std::path::Path| {
        if a.to_string_lossy().ends_with(".partial") { Err(std::io::Error::other("refused")) } else { rename(a, b) }
    };
    assert!(save_with(&root, &saved("Kept", "new"), &refuse).is_err());
    assert_eq!(read(&folder), ("font old".into(), "licence old".into()));
    assert_eq!(names(&root.join(FOLDER)), ["Kept"], "no partial or previous folder left");
    save_with(&root, &saved("Kept", "new"), &rename).unwrap();
    assert_eq!(read(&folder), ("font new".into(), "licence new".into()));
    assert_eq!(names(&root.join(FOLDER)), ["Kept"]);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn tidying_leaves_folders_figcraft_did_not_make() {
    let root = std::env::temp_dir().join(format!("figcraft-font-tidy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let parent = root.join(FOLDER);
    std::fs::create_dir_all(parent.join("Backup.partial")).unwrap();
    std::fs::write(parent.join("Backup.partial").join("mine.ttf"), "mine").unwrap();
    std::fs::create_dir_all(parent.join(".Half Done.partial")).unwrap();
    tidy(&root);
    assert_eq!(names(&parent), ["Backup.partial"]);
    assert_eq!(std::fs::read_to_string(parent.join("Backup.partial").join("mine.ttf")).unwrap(), "mine");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn only_an_https_library_is_used() {
    assert!(usable("https://raw.githubusercontent.com/google/fonts/main/"));
    assert!(usable(" https://fonts.example/ "));
    for library in ["", "  ", "http://fonts.example/", "file:///fonts/", "fonts.example/", "https://"] {
        assert!(!usable(library), "{library:?}");
    }
}

#[test]
fn a_stop_between_the_two_moves_is_put_right_at_the_next_start() {
    let root = std::env::temp_dir().join(format!("figcraft-font-restart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let parent = root.join(FOLDER);
    let stop = || {
        save_with(&root, &saved("Moved Away", "old"), &rename).unwrap();
        save_with(&root, &saved("Moved In", "new"), &rename).unwrap();
        // Stopped after moving the earlier family aside, before the new one went in.
        std::fs::rename(parent.join("Moved Away"), parent.join(".Moved Away.previous")).unwrap();
        std::fs::create_dir_all(parent.join(".Moved Away.partial")).unwrap();
        // Stopped after the new family went in, before the earlier one was deleted.
        std::fs::create_dir_all(parent.join(".Moved In.previous")).unwrap();
    };
    let put_right = |also: &[&str]| {
        assert_eq!(read(&parent.join("Moved Away")), ("font old".into(), "licence old".into()));
        assert_eq!(read(&parent.join("Moved In")), ("font new".into(), "licence new".into()));
        let mut want = vec!["Moved Away", "Moved In"];
        want.extend(also);
        assert_eq!(names(&parent), want);
    };
    // The app tidies the folder before scanning fonts.
    stop();
    tidy(&root);
    put_right(&[]);
    // A save tidies it too.
    std::fs::remove_dir_all(&root).unwrap();
    stop();
    save_with(&root, &saved("Other", "x"), &rename).unwrap();
    put_right(&["Other"]);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_file_larger_than_asked_for_is_refused() {
    let wire = Arc::new(Mutex::new(Wire::default()));
    let url = format!("{LIBRARY}ofl/figcraftoversized/METADATA.pb");
    wire.lock().unwrap().files.insert(url, vec![b' '; MAX_METADATA + 1]);
    let (mut s, root) = session(&wire, "oversized");
    let r = s.execute("text.downloadFonts", &json!({"families": ["FigCraft Oversized"], "confirm": true})).unwrap();
    assert!(r["failed"][0]["reason"].as_str().unwrap().contains("larger than"), "{r}");
    assert!(!root.exists());
}

/// Junk never panics the parsers that read the library's answers, and what they let through stays
/// safe for a path and a URL.
#[test]
fn library_parsers_survive_junk() {
    let pieces = [
        "name: \"",
        "license: \"",
        "  filename: \"",
        "\"",
        "\n",
        "\\",
        "/",
        "..",
        ".ttf",
        ".otf",
        "CON",
        " ",
        "\t",
        "é",
        "\u{0}",
        "{",
        "}",
        "[",
        "%",
    ];
    let mut seed = 0x2545_f491_u32;
    for _ in 0..20_000 {
        let mut text = String::new();
        for _ in 0..(seed % 12) {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            text.push_str(pieces[seed as usize % pieces.len()]);
        }
        if let Some(m) = parse_metadata(&text) {
            assert!(!m.name.contains(['"', '\\']) && m.files.iter().all(|f| !f.contains(['"', '\\'])), "{text:?}");
        }
        if plain_font_file(&text) {
            assert!(!text.contains(['/', '\\', ':', '\0']) && !text.starts_with('.'), "{text:?}");
        }
        assert!(url_segment(&text).bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.~%".contains(&b)), "{text:?}");
        let folder = safe_folder(&text);
        assert!(!folder.contains(['/', '\\', ':', '.', '\0']) && !folder.ends_with(' ') && !windows_device(&folder), "{text:?}");
        let _ = folder_name(&text);
    }
}
