//! A URL stored in a document reaches the browser or the system opener, so only the schemes a
//! browser follows on a plain link are allowed ([`crate::doc_urls`]).

use crate::doc_urls::openable;

#[test]
fn web_links_are_opened() {
    for url in [
        "http://example.com",
        "https://example.com",
        "https://example.com/a/b?c=d#e",
        "HTTPS://EXAMPLE.COM",
        "HtTp://example.com",
        "mailto:a@b.c",
        "mailto:a@b.c?subject=hi",
        "  https://example.com  ",
        "https://user:pw@example.com:8443/p",
    ] {
        assert!(openable(url), "{url:?} should open");
    }
}

#[test]
fn script_and_handler_schemes_are_refused() {
    for url in [
        // Runs in the app's own origin on the web build: a saved document would be stored XSS.
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "jAvAsCrIpT:alert(document.cookie)",
        "vbscript:msgbox(1)",
        "data:text/html,<script>alert(1)</script>",
        "blob:https://example.com/x",
        // Reach the file system; a UNC host also sends Windows credentials.
        "file:///etc/passwd",
        "file://attacker.example/share/x",
        "smb://attacker.example/share/x",
        "webdav://attacker.example/x",
        // Windows application handlers.
        "ms-msdt:/id PCWDiagnostic",
        "ms-officecmd:x",
        "search-ms:query=x",
        "shell:startup",
        // Anything else a platform may have registered.
        "tel:+1234",
        "sms:+1234",
        "ssh://host",
        "ftp://host/x",
        "figcraft://join?host=127.0.0.1",
        "zoommtg://zoom.us/join?x",
        "view-source:https://example.com",
        "jar:https://example.com/x!/y",
    ] {
        assert!(!openable(url), "{url:?} must be refused");
    }
}

#[test]
fn a_scheme_is_required_and_control_characters_are_refused() {
    for url in [
        // No scheme: the web opener would resolve these against the app's own page.
        "",
        "   ",
        "example.com",
        "/etc/passwd",
        "//example.com",
        "\\\\host\\share\\x",
        "about",
        // Browsers ignore some control characters inside a scheme, so a smuggled one must not
        // leave `javascript` looking like an unknown scheme that we then refuse by luck.
        "java\tscript:alert(1)",
        "java\nscript:alert(1)",
        "java\rscript:alert(1)",
        "javascript\u{0}:alert(1)",
        "https://example.com/\u{0}",
        // A colon in ordinary text is not a scheme.
        "see page 2: the diagram",
        "1https://example.com",
        // The allowed schemes still need their own part.
        "mailto:",
        "http:",
        "http:example.com",
        "http:///p",
        "https://",
    ] {
        assert!(!openable(url), "{url:?} must be refused");
    }
}
