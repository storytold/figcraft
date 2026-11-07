//! The page's URL: the invite link its fragment names, and the flags its query sets; and the server
//! the page names for itself.

/// The invite link in `#join=<link>` (`fragment` is `location.hash`, with or without its `#`), still
/// escaped. The fragment, unlike the query, never leaves the browser, so the link's secret stays out
/// of the web host's logs and `Referer` headers. `join` takes the rest of the fragment, so a pasted
/// `figcraft://join?host=…&file=…&token=…` keeps its own `&`s. Anything longer than an escaped invite
/// link can be is ignored; what is left is checked by `collab.join` like a link the desktop app is
/// opened with.
pub fn join_link(fragment: &str) -> Option<&str> {
    // An invite link is at most 2 KiB; every byte of it escaped is three.
    const MAX: usize = 3 * 2048;
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
    let start = if fragment.starts_with("join=") { 0 } else { fragment.find("&join=")? + 1 };
    fragment.get(start + "join=".len()..).filter(|link| !link.is_empty() && link.len() <= MAX)
}

/// The server a page names for itself in `<meta name="figcraft-server" content="wss://…/">` (a
/// host serving the browser build beside its own `figcraft-server`): the page's default for online
/// sharing, join codes and the team list. Only a `ws://` or `wss://` address without a query (no
/// key can ride in it), at most 512 characters; anything else is ignored.
pub fn page_server(content: &str) -> Option<String> {
    let c = content.trim();
    let scheme = c.starts_with("wss://") || c.starts_with("ws://");
    (scheme && c.len() <= 512 && !c.contains(|ch: char| ch.is_whitespace() || ch.is_control() || ch == '?' || ch == '#')).then(|| c.to_string())
}

/// Whether `search` (`location.search`, with or without its `?`) has a `control` entry (`?control`,
/// or `?control=…` with any value): the page's control channel, `window.figcraft`, is installed only
/// when it is asked for.
pub fn control(search: &str) -> bool {
    search.strip_prefix('?').unwrap_or(search).split('&').any(|entry| entry.split('=').next() == Some("control"))
}
