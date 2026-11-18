//! Opening invite links from the system: a `figcraft://join?…` link clicked in a browser, a chat
//! or an email opens FigCraft, which joins the session (milestone M9).
//!
//! The app makes itself the opener of the `figcraft` URL scheme. Where the app registers it at run
//! time (Windows: per user, no administrator rights), the desktop app installs a
//! [`LinkRegistrar`] (like the [`LiveLink`](crate::live::LiveLink)); the engine keeps the
//! preference ([`Prefs::invite_links`](crate::Prefs::invite_links)) and the
//! `collab.registerLinks` command. The preference defaults to on for installed builds and off for
//! development builds, so running from a source checkout never takes the scheme over. (macOS and
//! Linux packages declare the scheme in the bundle and the desktop entry instead.)

use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::boolean;
use crate::{EngineError, Result, Session, spec};

/// Registers this copy of the app as the opener of `figcraft://` links (the desktop app installs
/// one: `session.live.links`).
pub trait LinkRegistrar: Send {
    /// This copy of the app is the registered opener.
    fn registered(&self) -> bool;
    /// This is an installed build (the preference defaults to on), not a development build.
    fn installed(&self) -> bool;
    /// Register this copy (`on`), or remove its registration (`off`: another copy's stays).
    fn set(&mut self, on: bool) -> std::result::Result<(), String>;
}

impl Session {
    /// Whether invite links should open this app: the preference, or on for installed builds.
    /// `None` when this app can't register them.
    pub fn invite_links_on(&self) -> Option<bool> {
        let r = self.live.links.as_ref()?;
        Some(self.prefs.invite_links.unwrap_or_else(|| r.installed()))
    }

    /// At startup: register as the opener when the preference says so and it isn't already this
    /// copy; remove this copy's registration only when the preference was turned off.
    pub fn invite_links_startup(&mut self) {
        let pref = self.prefs.invite_links;
        let Some(r) = self.live.links.as_mut() else { return };
        let want = match pref {
            Some(on) => on,
            None if r.installed() => true,
            // A development build leaves whatever is registered alone.
            None => return,
        };
        if want != r.registered()
            && let Err(e) = r.set(want)
        {
            log::warn!("invite links: {e}");
        }
    }
}

fn register(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.registerLinks";
    let Some(r) = s.live.links.as_mut() else {
        return Err(EngineError::Disabled(
            CMD.into(),
            "this app doesn't register link openers itself (the Windows desktop app does; the macOS and Linux packages declare figcraft:// when installed)"
                .into(),
        ));
    };
    let on = boolean(p, "on").unwrap_or_else(|| !s.prefs.invite_links.unwrap_or_else(|| r.installed()));
    r.set(on).map_err(|e| EngineError::Other(format!("couldn't {} invite links: {e}", if on { "register" } else { "unregister" })))?;
    let registered = r.registered();
    s.prefs.invite_links = Some(on);
    Ok(json!({"on": on, "registered": registered}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "collab.registerLinks",
        "Open invite links in FigCraft",
        "{on? (default: switch)} → {on, registered}: make this app the opener of figcraft:// invite links (or stop); also the Preferences checkbox",
        false,
        register
    )]
}
