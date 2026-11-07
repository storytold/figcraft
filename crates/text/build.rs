//! Embeds the optional craft-fonts build input (`CRAFT_FONTS_DIR`): both `fonts/manifest.txt` and the
//! opt-in Latin presentation fonts in `fonts/latin-manifest.txt` (Inter, Roboto, …). Unset or missing,
//! `CRAFT_FONTS` is empty and native apps use system fonts. Required web releases enforce Inter
//! Regular/SemiBold and Noto Sans Georgian Regular after filtering. Recipe: craft-fonts `docs/integration.md`.
//! Noto Sans CJK only with `FIGCRAFT_EMBED_CJK` set (the web build's only way to show CJK UI text).
//! `FIGCRAFT_WEB_FONTS` (comma-separated families) adds manifest families to the web build.
use std::fmt::Write as _;
use std::path::PathBuf;

/// Font build script: emit `craft_fonts.rs` from `input_fonts`, passing the
/// `CRAFT_FONTS_DIR`, `CRAFT_FONTS_REQUIRED`, wasm target, `FIGCRAFT_EMBED_CJK` and `FIGCRAFT_WEB_FONTS` inputs;
/// report generation and write failures with `cargo::error`.
fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    println!("cargo::rerun-if-env-changed=FIGCRAFT_EMBED_CJK");
    println!("cargo::rerun-if-env-changed=FIGCRAFT_WEB_FONTS");
    let mut src = String::from("pub static CRAFT_FONTS: &[CraftFont] = &[\n");
    let dir = std::env::var_os("CRAFT_FONTS_DIR").map(PathBuf::from);
    let required = std::env::var_os("CRAFT_FONTS_REQUIRED").is_some();
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let embed_cjk = std::env::var_os("FIGCRAFT_EMBED_CJK").is_some();
    let web_fonts = web_fonts(&std::env::var("FIGCRAFT_WEB_FONTS").unwrap_or_default());
    match input_fonts(dir.as_deref(), wasm, required, embed_cjk, &web_fonts) {
        Ok(entries) => src.push_str(&entries),
        Err(e) => println!("cargo::error={e}"),
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// The families `FIGCRAFT_WEB_FONTS` names: comma-separated, trimmed, empty names dropped.
fn web_fonts(list: &str) -> Vec<String> {
    list.split(',').map(str::trim).filter(|f| !f.is_empty()).map(str::to_owned).collect()
}

/// The families in `web_fonts` that `faces` has no face of, in the order named.
fn not_embedded<'a>(web_fonts: &'a [String], faces: &[(String, String)]) -> Vec<&'a str> {
    web_fonts.iter().map(String::as_str).filter(|w| !faces.iter().any(|(family, _)| family == w)).collect()
}

/// Collect the effective catalog and enforce the required web release faces after filtering.
/// Families named in `web_fonts` that nothing embedded get a warning (most likely misspelt).
fn input_fonts(dir: Option<&std::path::Path>, wasm: bool, required: bool, embed_cjk: bool, web_fonts: &[String]) -> Result<String, String> {
    let Some(dir) = dir else {
        for family in web_fonts {
            println!("cargo::warning=FIGCRAFT_WEB_FONTS: {family:?} isn't embedded: this build has no craft-fonts");
        }
        return if required { Err("CRAFT_FONTS_REQUIRED is set but CRAFT_FONTS_DIR is missing".into()) } else { Ok(String::new()) };
    };
    let mut out = String::new();
    let mut faces = Vec::new();
    let mut loaded = false;
    for manifest in ["fonts/latin-manifest.txt", "fonts/manifest.txt"] {
        let path = dir.join(manifest);
        if manifest == "fonts/latin-manifest.txt" && !path.exists() {
            watch(&path);
            continue;
        }
        match craft_fonts(dir, manifest, wasm, embed_cjk, web_fonts) {
            Ok(catalog) => {
                loaded = true;
                out.push_str(&catalog.entries);
                faces.extend(catalog.faces);
            }
            Err(e) if required => return Err(format!("CRAFT_FONTS_DIR={}: {e}", dir.display())),
            Err(e) => println!("cargo::warning=craft-fonts {manifest} skipped: CRAFT_FONTS_DIR={}: {e}", dir.display()),
        }
    }
    let why =
        if loaded { "no manifest row was kept for it (misspelt? CJK also needs FIGCRAFT_EMBED_CJK)" } else { "no craft-fonts manifest was read" };
    for family in not_embedded(web_fonts, &faces) {
        println!("cargo::warning=FIGCRAFT_WEB_FONTS: {family:?} isn't embedded: {why}");
    }
    if required && wasm {
        for (family, style) in [("Inter", "Regular"), ("Inter", "SemiBold"), ("Noto Sans Georgian", "Regular")] {
            if !faces.iter().any(|(f, s)| f == family && s == style) {
                return Err(format!("required web font missing: {family} {style}; CRAFT_FONTS_DIR must include both Latin and core release faces"));
            }
        }
    }
    Ok(out)
}

/// Rebuild when `manifest` changes or, while it is missing, when it is added to its folder.
///
/// Cargo takes a watched path that doesn't exist as changed on every build, and then rebuilds this
/// crate and every crate that depends on it each time. So a missing manifest is never watched
/// itself: its folder is, when there is one (a craft-fonts checkout without the Latin manifest).
/// With no craft-fonts at all there is nothing to watch, so the build can't notice it arriving:
/// rebuild this crate after adding a previously absent checkout.
fn watch(manifest: &std::path::Path) {
    let watched = if manifest.exists() { Some(manifest) } else { manifest.parent().filter(|folder| folder.is_dir()) };
    if let Some(path) = watched {
        println!("cargo::rerun-if-changed={}", path.display());
    }
}

struct Catalog {
    entries: String,
    faces: Vec<(String, String)>,
}

/// Parse a font manifest of `family | style | file | scripts` rows into `CraftFont`
/// entries: skip Adobe-authored families, skip CJK unless requested, keep the web catalog
/// to its budgeted families and the ones `web_fonts` adds, canonicalize and watch each face, and
/// emit `include_bytes!` items.
fn craft_fonts(dir: &std::path::Path, manifest_rel: &str, wasm: bool, embed_cjk: bool, web_fonts: &[String]) -> Result<Catalog, String> {
    let manifest = dir.join(manifest_rel);
    watch(&manifest);
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut out = String::new();
    let mut faces = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        // FigCraft's asset policy excludes Adobe-authored visual assets on every target.
        if matches!(*family, "Source Sans 3" | "Source Serif 4") {
            continue;
        }
        // CJK fonts are large; the desktop app finds system CJK fonts instead unless asked.
        let cjk = scripts.contains("Hans");
        if cjk && !embed_cjk {
            continue;
        }
        // Web builds have a size budget: Inter, Noto Sans Georgian (small, for the Georgian UI), only the
        // SemiBold wordmark, optionally Noto Sans CJK (every CJK script, for CJK UI text), and the
        // families a distribution names in `FIGCRAFT_WEB_FONTS` (a Thai face for Thai text, say).
        let wordmark = *family == "Archivo" && *style == "SemiBold";
        let added = web_fonts.iter().any(|f| f == family);
        if wasm && !matches!(*family, "Inter" | "Noto Sans Georgian") && !wordmark && !cjk && !added {
            continue;
        }
        let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
        faces.push(((*family).to_owned(), (*style).to_owned()));
        let _ = writeln!(out, "    CraftFont {{ family: {family:?}, style: {style:?}, bytes: include_bytes!({:?}) }},", path.display().to_string());
    }
    Ok(Catalog { entries: out, faces })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        /// Test fixture: create a unique temporary directory, write a synthetic face file and
        /// install the default Georgian manifest.
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("fig-font-input-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(dir.join("fonts")).unwrap();
            // The build script validates manifest/file presence; font bytes are checked centrally.
            std::fs::write(dir.join("fonts/face.ttf"), b"synthetic build-input fixture").unwrap();
            let fixture = Self(dir);
            fixture.manifest("manifest.txt", &[("Noto Sans Georgian", "Regular")]);
            fixture
        }

        /// Test fixture: write manifest rows that all point at the fixture's `fonts/face.ttf` face.
        fn manifest(&self, name: &str, faces: &[(&str, &str)]) {
            let text = faces.iter().map(|(family, style)| format!("{family} | {style} | fonts/face.ttf | Latn\n")).collect::<String>();
            std::fs::write(self.0.join("fonts").join(name), text).unwrap();
        }
    }

    impl Drop for Fixture {
        /// Remove the fixture's temporary directory, ignoring errors.
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Asserts the native Latin catalog keeps its 43 permitted presentation faces, the web
    /// catalog keeps only the four Inter faces, Adobe-authored families are absent, and
    /// permitted native faces remain.
    #[test]
    fn restored_latin_catalog_preserves_permitted_native_faces_and_web_budget() {
        let f = Fixture::new();
        std::fs::write(f.0.join("fonts/latin-manifest.txt"), include_str!("tests/fixtures/latin-families.txt")).unwrap();
        let native = craft_fonts(&f.0, "fonts/latin-manifest.txt", false, false, &[]).unwrap();
        assert_eq!(native.faces.len(), 43, "all prior permitted presentation faces remain available");
        let web = craft_fonts(&f.0, "fonts/latin-manifest.txt", true, false, &[]).unwrap();
        assert_eq!(web.faces.len(), 4, "the web catalog keeps only the four Inter faces");
        for catalog in [&native, &web] {
            assert!(!catalog.entries.contains("Source Sans 3"));
            assert!(!catalog.entries.contains("Source Serif 4"));
        }
        assert!(native.faces.iter().any(|(family, _)| family == "Carlito"));
        assert!(native.faces.iter().any(|(family, _)| family == "Liberation Serif"));
        assert!(input_fonts(Some(&f.0), true, true, false, &[]).is_ok());
    }

    /// Asserts required web input rejects a catalog missing Inter Regular or Inter SemiBold.
    #[test]
    fn required_web_rejects_core_only_and_partial_latin_catalogs() {
        let f = Fixture::new();
        assert!(input_fonts(Some(&f.0), true, true, false, &[]).unwrap_err().contains("Inter Regular"));
        f.manifest("latin-manifest.txt", &[("Inter", "Regular"), ("Unrelated", "SemiBold")]);
        assert!(input_fonts(Some(&f.0), true, true, false, &[]).unwrap_err().contains("Inter SemiBold"));
    }

    /// Asserts required web input embeds the Inter and Georgian catalogs (three entries) and
    /// rejects a manifest without Noto Sans Georgian Regular.
    #[test]
    fn required_web_embeds_both_catalogs_and_requires_georgian() {
        let f = Fixture::new();
        f.manifest("latin-manifest.txt", &[("Inter", "Regular"), ("Inter", "SemiBold")]);
        let entries = input_fonts(Some(&f.0), true, true, false, &[]).unwrap();
        assert_eq!(entries.matches("CraftFont {").count(), 3);
        assert!(entries.contains("family: \"Inter\""));
        assert!(entries.contains("family: \"Noto Sans Georgian\""));
        f.manifest("manifest.txt", &[("Other", "Regular")]);
        assert!(input_fonts(Some(&f.0), true, true, false, &[]).unwrap_err().contains("Noto Sans Georgian Regular"));
    }

    /// Asserts absent font input is empty when optional and an error when required, while present
    /// input includes Noto Sans Georgian.
    #[test]
    fn optional_and_native_inputs_keep_their_existing_policy() {
        let f = Fixture::new();
        assert!(input_fonts(None, true, false, false, &[]).unwrap().is_empty());
        assert!(input_fonts(None, true, true, false, &[]).unwrap_err().contains("CRAFT_FONTS_DIR is missing"));
        assert!(input_fonts(Some(&f.0), true, false, false, &[]).unwrap().contains("Noto Sans Georgian"));
        assert!(input_fonts(Some(&f.0), false, true, false, &[]).unwrap().contains("Noto Sans Georgian"));
    }

    /// Asserts required web input rejects a manifest whose face file is missing, even when every
    /// face row is listed.
    #[test]
    fn required_web_rejects_missing_asset_even_with_all_face_rows() {
        let f = Fixture::new();
        f.manifest("latin-manifest.txt", &[("Inter", "Regular"), ("Inter", "SemiBold")]);
        std::fs::remove_file(f.0.join("fonts/face.ttf")).unwrap();
        assert!(input_fonts(Some(&f.0), true, true, false, &[]).unwrap_err().contains("fonts/face.ttf"));
    }

    /// Asserts `FIGCRAFT_WEB_FONTS` (split on commas, trimmed) adds the families it names to the web
    /// catalog only, leaves native and unset builds as they were, and reports a family nothing
    /// embedded.
    #[test]
    fn web_fonts_add_the_families_they_name_to_the_web_catalog() {
        let f = Fixture::new();
        f.manifest("manifest.txt", &[("Noto Sans Georgian", "Regular"), ("Noto Sans Thai", "Regular"), ("Noto Sans Hebrew", "Regular")]);
        let named = web_fonts(" Noto Sans Thai, ,Noto Sans Thia ");
        assert_eq!(named, ["Noto Sans Thai", "Noto Sans Thia"]);
        let families = |c: &Catalog| c.faces.iter().map(|(family, _)| family.clone()).collect::<Vec<_>>();
        let web = craft_fonts(&f.0, "fonts/manifest.txt", true, false, &named).unwrap();
        assert_eq!(families(&web), ["Noto Sans Georgian", "Noto Sans Thai"], "the named family joins; the others stay out");
        assert_eq!(families(&craft_fonts(&f.0, "fonts/manifest.txt", true, false, &[]).unwrap()), ["Noto Sans Georgian"], "unset: as before");
        assert_eq!(craft_fonts(&f.0, "fonts/manifest.txt", false, false, &[]).unwrap().faces.len(), 3, "native builds embed every family");
        assert_eq!(not_embedded(&named, &web.faces), ["Noto Sans Thia"], "a misspelt family is reported");
        assert!(input_fonts(Some(&f.0), true, false, false, &named).unwrap().contains("family: \"Noto Sans Thai\""));
    }
}
