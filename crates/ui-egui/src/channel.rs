//! Which build channel this binary is: the stable app, or the Launcher-managed Nightly.
//!
//! The desktop app installs the Nightly identity once, before the first window, from its build
//! flavor ([`build_flavor`](crate) in the app crate) and the version it was built with. Everything
//! that names the app to the user (the window title, About) reads it here, so the two channels are
//! never mixed up in a caption or a taskbar. The web build and the tests keep the stable identity.

use std::sync::OnceLock;

struct Channel {
    nightly: bool,
    /// The version bundled into this build: the exact prepared Nightly version, else the Cargo one.
    version: String,
    /// The commit the Nightly was built from; `None` for the stable build.
    commit: Option<String>,
}

static CHANNEL: OnceLock<Channel> = OnceLock::new();

fn channel() -> &'static Channel {
    CHANNEL.get_or_init(|| Channel { nightly: false, version: crate::dialogs::VERSION.to_string(), commit: None })
}

/// Install the Nightly identity, once, with the version and commit the app was built with. Later
/// calls keep the first: a window cannot promote itself to another channel at run time.
pub fn install_nightly(version: impl Into<String>, commit: Option<impl Into<String>>) {
    let _ = CHANNEL.set(Channel { nightly: true, version: version.into(), commit: commit.map(Into::into).filter(|c| !c.is_empty()) });
}

pub fn is_nightly() -> bool {
    channel().nightly
}

/// The app's name: the window title, About and the launcher's dialog say "FigCraft Nightly" for
/// the Nightly build, so a window is never mistaken for the stable app's.
pub fn name() -> &'static str {
    if is_nightly() { "FigCraft Nightly" } else { "FigCraft" }
}

/// The commit this Nightly was built from, for its About window and a bug report.
pub fn commit() -> Option<&'static str> {
    channel().commit.as_deref()
}

/// A one-line identity for About and a bug report: the channel, and the commit a Nightly was
/// built from.
pub fn describe() -> String {
    match (is_nightly(), commit()) {
        (true, Some(commit)) => format!("{} · commit {commit}", name()),
        _ => name().to_string(),
    }
}

/// The bundled version as shown, tagged with the channel when it is the Nightly's.
pub fn version() -> String {
    let version = &channel().version;
    if is_nightly() { format!("{version} (nightly-v1)") } else { version.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_an_installed_identity_the_app_is_the_stable_one() {
        // The other tests and the web build never install an identity.
        assert!(!is_nightly());
        assert_eq!(name(), "FigCraft");
        assert_eq!(version(), crate::dialogs::VERSION);
    }
}
