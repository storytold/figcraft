//! The page's URL: the invite link its fragment names, and whether the page is a device viewer.

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

/// `#view&join=<link>`: the page is a device viewer (`figcraft_ui_egui::viewer`): only the
/// prototype of the file the link opens (the address Share ▸ View on another device gives). Only
/// entries before `join=` count: the link takes the rest of the fragment, and its own text (its
/// secret too) never sets a flag. In the fragment, like the link, so the web host never sees it.
pub fn view(fragment: &str) -> bool {
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
    let flags = if fragment.starts_with("join=") { "" } else { fragment.find("&join=").and_then(|i| fragment.get(..i)).unwrap_or(fragment) };
    flags.split('&').any(|f| f == "view")
}
