//! Pure host-mode rules, shared by the bridge ([`crate::host`]) and testable without a browser
//! (like [`crate::query`]): what opts a page into hosting FigCraft in its own document, the name
//! the bridge fills in when an open request has none, and whether a handed-over file becomes the
//! document's file.

/// The one `data-host` value that opts a page in: the exact string `"page"`.
pub const PAGE_HOST: &str = "page";

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

/// Does this `data-host` value ask for page host mode? Only the exact value [`PAGE_HOST`] counts;
/// anything else is ignored, so the page host mode stays opt-in. `?host=parent`, the parent
/// bridge's URL opt-in, is answered before this is asked.
pub fn page_host_requested(data_host: Option<&str>) -> bool {
    data_host == Some(PAGE_HOST)
}

/// A usable file name for an open request: the host's name when it isn't blank, else the
/// [`DEFAULT_OPEN_NAME`].
pub fn open_name(name: Option<&str>) -> String {
    name.filter(|n| !n.trim().is_empty()).unwrap_or(DEFAULT_OPEN_NAME).to_string()
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
    fn only_the_exact_page_value_opts_in() {
        assert!(page_host_requested(Some("page")));
        for off in [None, Some(""), Some("Page"), Some("PAGE"), Some(" page"), Some("page "), Some("parent")] {
            assert!(!page_host_requested(off), "{off:?} must keep the page host mode off");
        }
    }

    #[test]
    fn open_names_fall_back_to_the_default_when_blank() {
        assert_eq!(open_name(Some("Poster.figcraft")), "Poster.figcraft");
        assert_eq!(open_name(Some(" Poster.figcraft ")), " Poster.figcraft ", "a name with its spacing is kept");
        assert_eq!(open_name(Some("")), DEFAULT_OPEN_NAME);
        assert_eq!(open_name(Some("   ")), DEFAULT_OPEN_NAME);
        assert_eq!(open_name(None), DEFAULT_OPEN_NAME);
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
