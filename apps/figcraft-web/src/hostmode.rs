//! Pure host-mode rules, shared by the bridge ([`crate::host`]) and testable without a browser
//! (like [`crate::query`]): the name the bridge fills in when an open request has none, and whether
//! a handed-over file becomes the document's file. Which pages host the app is `craft-web-host`'s.

/// The file name used when an open request carries no usable one.
pub const DEFAULT_OPEN_NAME: &str = "Untitled.figcraft";

/// The extensions of files that can be a document's own file: the formats `file.save` and
/// `file.saveAs` write (`file.open` opens both as native documents). A `.fig` file is an export
/// copy instead, and so is everything else the app hands over (rendered images, SVG, PDF,
/// variables).
const DOC_EXTS: &[&str] = &["figcraft", "json"];

/// The suffix `variable.export`'s downloads carry: a `.json` that is a variables sidecar, not a
/// document, so its extension alone can't tell the `saveAs` decision.
const VARIABLES_SIDECAR: &str = ".variables.json";

/// The name `craft-web-host` gives an open request that carries no usable one (it cleans a host's
/// name and falls back to this).
const NAMELESS: &str = "document";

/// A usable file name for an open request: the cleaned name the crate hands over, or
/// [`DEFAULT_OPEN_NAME`] when the host sent none.
pub fn open_name(name: &str) -> String {
    if name == NAMELESS { DEFAULT_OPEN_NAME.to_string() } else { name.to_string() }
}

/// Whether the file `name` becomes the document's own file when handed over (`saveAs: true`),
/// given the file the host opened (`None`: the host never sent one). The rule:
///
/// - A document file (`.figcraft` or `.json`, [`DOC_EXTS`]'s formats) under a name the host didn't
///   open — Save As, or a Save of a document the host never sent — is the document's new file:
///   `true`. Names compare without case.
/// - A Save of the host's own file under its name is not: `false`.
/// - Everything else is a copy or export that never becomes the document — rendered images, SVG,
///   PDF, `.fig` copies, variables — and the variables sidecar counts as one whatever its `.json`
///   extension says: `false`.
pub fn save_as(name: &str, host_file: Option<&str>) -> bool {
    if name.to_ascii_lowercase().ends_with(VARIABLES_SIDECAR) {
        return false;
    }
    let ext = std::path::Path::new(name).extension().and_then(std::ffi::OsStr::to_str);
    if !ext.is_some_and(|e| DOC_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e))) {
        return false;
    }
    match host_file {
        Some(host) => !host.eq_ignore_ascii_case(name),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_names_fall_back_to_the_default_when_the_host_sent_none() {
        assert_eq!(open_name("Poster.figcraft"), "Poster.figcraft");
        assert_eq!(open_name("Poster 2.json"), "Poster 2.json");
        assert_eq!(open_name("document"), DEFAULT_OPEN_NAME, "what the crate says for a missing or blank name");
    }

    #[test]
    fn a_save_of_the_host_s_file_is_not_save_as() {
        assert!(!save_as("Poster.figcraft", Some("Poster.figcraft")));
        assert!(!save_as("poster.FIGCRAFT", Some("Poster.figcraft")), "names compare without case");
        assert!(!save_as("design.json", Some("design.json")));
    }

    #[test]
    fn a_new_document_file_is_save_as() {
        assert!(save_as("Poster 2.figcraft", Some("Poster.figcraft")));
        assert!(save_as("Poster.json", Some("Poster.figcraft")), "the other format a document's file takes");
        assert!(save_as("Untitled.figcraft", None), "the host never sent a file");
        assert!(save_as("Poster.figcraft", Some("design.json")), "the opened .json saved as .figcraft is a new file");
    }

    #[test]
    fn copies_and_exports_never_become_the_document_s_file() {
        for name in ["Poster.png", "Poster@2x.jpg", "Poster.svg", "Poster.pdf", "Poster.fig", "Poster.csv", "Poster.variables.css"] {
            assert!(!save_as(name, Some("Poster.figcraft")), "{name} is a copy");
            assert!(!save_as(name, None), "{name} is a copy even without a host file");
        }
    }

    #[test]
    fn the_variables_sidecar_is_a_copy_whatever_its_extension() {
        assert!(!save_as("Poster.variables.json", Some("Poster.figcraft")));
        assert!(!save_as("Poster.variables.json", None));
        assert!(!save_as("design.variables.json", Some("design.variables.json")));
    }
}
