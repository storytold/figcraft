//! Keeping the font database in step with the active document's icon fonts.
//!
//! A document carries its icon fonts as outlines ([`figcraft_doc::IconFont`]). To use one in a text
//! layer it has to exist as a real font family in the font database, so the active document's set
//! is compiled and registered whenever it changes. Compiling is the expensive part, so the built
//! bytes are cached against a fingerprint of the outlines: an unchanged font is only hashed, and a
//! font whose outlines changed is rebuilt.
//!
//! [`sync_icon_fonts`] is cheap enough to call after every command, which is what keeps the picker
//! honest across edits, undo, redo and switching documents.

use std::sync::Mutex;

use figcraft_doc::{Document, IconFont};
use figcraft_geom::BezPath;
use figcraft_text::{fontdb, iconfont};

/// A font already compiled from the document's outlines: its family name, the fingerprint of the
/// outlines it was built from, and the bytes (`None`: those outlines make nothing drawable, so don't
/// retry them).
type BuiltFont = (String, u64, Option<Vec<u8>>);

/// Fonts built from the current outlines, by family name.
static BUILT: Mutex<Vec<BuiltFont>> = Mutex::new(Vec::new());

/// A hash of everything about a font that changes its glyphs.
fn fingerprint(font: &IconFont) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    font.name.hash(&mut h);
    for g in &font.glyphs {
        g.name.hash(&mut h);
        g.codepoint.hash(&mut h);
        g.path.hash(&mut h);
    }
    h.finish()
}

/// Compile one icon font, or `None` when its outlines make nothing drawable (logged).
fn compile(font: &IconFont) -> Option<Vec<u8>> {
    let mut glyphs: Vec<(u32, BezPath)> = Vec::with_capacity(font.glyphs.len());
    for g in &font.glyphs {
        let Ok(path) = BezPath::from_svg(&g.path) else {
            log::warn!("icon {:?} of {:?} has an unreadable outline; it is left out", g.name, font.name);
            continue;
        };
        glyphs.push((g.codepoint, path));
    }
    match iconfont::build_ttf(&font.name, &glyphs) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            log::warn!("icon font {:?} was not built: {e}", font.name);
            None
        }
    }
}

/// Register the active document's icon fonts, dropping any family this document doesn't have.
///
/// Returns how many fonts were compiled (the rest were already built from the same outlines).
pub fn sync_icon_fonts(doc: &Document) -> usize {
    let mut built = BUILT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut keep: Vec<BuiltFont> = Vec::with_capacity(doc.icon_fonts().len());
    let mut wanted: Vec<(String, Vec<u8>)> = Vec::with_capacity(doc.icon_fonts().len());
    let mut rebuilt = 0;
    for font in doc.icon_fonts() {
        let print = fingerprint(font);
        let bytes = match built.iter().find(|(name, p, _)| name == &font.name && *p == print) {
            Some((_, _, bytes)) => bytes.clone(),
            None => {
                rebuilt += 1;
                compile(font)
            }
        };
        keep.push((font.name.clone(), print, bytes.clone()));
        if let Some(bytes) = bytes {
            wanted.push((font.name.clone(), bytes));
        }
    }
    *built = keep;
    drop(built);
    fontdb::sync_families(&wanted);
    rebuilt
}

/// Forget the fonts built from the current outlines, so the next [`sync_icon_fonts`] compiles them
/// again (`family`: only that one; `None`: all of them).
///
/// [`crate::cmd::icon`]'s `icon.generateFont` uses this to rebuild on demand. Nothing else needs it:
/// the fingerprint already rebuilds a font whose outlines changed.
pub fn forget(family: Option<&str>) {
    let mut built = BUILT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match family {
        Some(name) => built.retain(|(n, _, _)| n != name),
        None => built.clear(),
    }
}

/// Serialises the tests that register icon fonts.
///
/// They share the one process-wide font database, and a sync makes it hold one document's icon
/// fonts *instead of* what it holds, so two of them running at once would drop each other's
/// families. Only tests that actually have icon fonts sync, so nothing else has to take this.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Hold this for the length of a test that registers icon fonts.
#[cfg(test)]
pub(crate) fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[path = "iconfont_tests.rs"]
mod tests;
