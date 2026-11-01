//! Which URLs stored in a document may be handed to the browser or the system opener.
//!
//! A document can carry URLs that the app opens for whoever views it: a prototype's Open link
//! action ([`crate::proto_run`]) and Dev Mode's dev resources ([`crate::dev_resources`]). Both end
//! up at the platform's opener, so a document that someone else made chooses what that opener is
//! asked to do. Only the schemes a browser would follow on a plain link are allowed:
//!
//! - `javascript:` runs script. In the web build the opener is `window.open`, which executes it in
//!   the app's own origin, so a saved document becomes stored XSS.
//! - `file:` and `smb:` reach the file system; on Windows a UNC host also sends credentials.
//! - Windows hands `ms-…:` schemes to application handlers, and macOS and Linux hand unknown
//!   schemes to whichever application registered them.
//!
//! The check is lexical and does no I/O. It is deliberately an allow-list: a deny-list of known
//! dangerous schemes is outgrown by every new handler a platform adds.

/// The schemes a URL stored in a document may use.
const WEB_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// Whether `url`, stored in a document, may be opened.
///
/// Requires a scheme from [`WEB_SCHEMES`], compared ignoring ASCII case. A URL with no scheme is
/// refused: the opener would resolve it against the app's own page on the web. Control characters
/// are refused anywhere, because browsers ignore some of them inside a scheme (`java\tscript:`
/// reaches a `javascript:` handler), and `http`/`https` must name a host.
pub fn openable(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() || url.chars().any(char::is_control) {
        return false;
    }
    let Some((scheme, rest)) = url.split_once(':') else { return false };
    // An RFC 3986 scheme, so a stray colon in text cannot look like one.
    let mut chars = scheme.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return false;
    }
    if !WEB_SCHEMES.iter().any(|s| scheme.eq_ignore_ascii_case(s)) {
        return false;
    }
    if scheme.eq_ignore_ascii_case("mailto") {
        // `mailto:` names an address.
        return !rest.is_empty();
    }
    // `http` and `https` name a host, so the authority is required: `http:x` is not a web link,
    // and `http:///p` has no host.
    let Some(after) = rest.strip_prefix("//") else { return false };
    !after.is_empty() && !after.starts_with('/')
}
