//! Agent control as Preferences ask for it, with no restart: while "Allow AI agents to control
//! FigCraft" is on, the control server listens on a free local port and `control.json` in the
//! settings folder says which ([`figcraft_engine::discovery`]); turned off, the server stops and
//! its connections end. A launch that names its own port (`--control`, `FIGCRAFT_CONTROL_PORT`)
//! keeps that server until exit: Preferences do not switch it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use figcraft_engine::discovery;
use figcraft_engine::events::EventLog;
use figcraft_engine::file_access::Roots;
use figcraft_ui_egui::{ControlRequest, FigApp};

use crate::control_server::{self, Stop};

/// What the app's own folder for agents is called in the user's documents.
const DOCUMENTS_FOLDER: &str = "FigCraft";

/// What the control server needs from the app.
pub struct Wiring {
    pub queue: Arc<Mutex<Vec<ControlRequest>>>,
    pub ctx: Arc<Mutex<Option<egui::Context>>>,
    pub events: Arc<EventLog>,
    /// The settings folder, where the state file goes (none: no state file).
    pub config_dir: Option<PathBuf>,
}

/// The control server and what started it.
pub struct AgentControl {
    wiring: Wiring,
    /// The launch named the port: Preferences do not switch this server.
    fixed: bool,
    /// The launch named automation roots: the preference's folder is not used.
    launch_roots: bool,
    server: Option<Stop>,
    /// The preference values the running state answers: (allowed, folder).
    applied: Option<(bool, String)>,
}

impl AgentControl {
    /// The server of a launch that named `port`, if any; else one that follows Preferences.
    pub fn new(wiring: Wiring, port: Option<u16>, busy: std::time::Duration, launch_roots: bool) -> Self {
        let mut control = Self { wiring, fixed: port.is_some(), launch_roots, server: None, applied: None };
        if let Some(port) = port {
            control.listen(port, busy);
        }
        control
    }

    /// Start the server on `port` (0: a free one) and write the state file. The error is for the
    /// user: the app goes on without the server.
    fn listen(&mut self, port: u16, busy: std::time::Duration) -> Option<String> {
        let w = &self.wiring;
        match control_server::start(port, busy, w.queue.clone(), w.ctx.clone(), w.events.clone()) {
            Ok(stop) => {
                if let Some(dir) = &w.config_dir
                    && let Err(e) = discovery::write(dir, stop.port())
                {
                    log::error!("control server: can't write {}: {e}", dir.join(discovery::STATE_FILE).display());
                }
                self.server = Some(stop);
                None
            }
            Err(e) => Some(format!("Agent control could not start: {e}")),
        }
    }

    /// Stop the server, end its connections, drop the requests they still wait on and remove the
    /// state file.
    pub fn stop(&mut self) {
        let Some(server) = self.server.take() else { return };
        server.stop();
        self.wiring.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        if let Some(dir) = &self.wiring.config_dir {
            discovery::remove(dir);
        }
    }

    /// Make the server what Preferences ask, when they changed (cheap otherwise: every tick calls it).
    pub fn follow(&mut self, fig: &mut FigApp) {
        if self.fixed {
            return;
        }
        let prefs = &fig.session.prefs;
        if self.applied.as_ref().is_some_and(|(on, root)| *on == prefs.agent_control && *root == prefs.automation_root) {
            return;
        }
        let want = (prefs.agent_control, prefs.automation_root.clone());
        self.stop();
        if !self.launch_roots {
            fig.automation_roots = None;
            fig.automation_roots_for_agents = false;
        }
        if want.0
            && let Some(error) = self.allow(fig, &want.1)
        {
            fig.show_error(error);
        }
        self.applied = Some(want);
    }

    /// Confine file access to the preference's folder, then listen. Without the folder there is
    /// no server: agents must never work unconfined because a folder was missing.
    fn allow(&mut self, fig: &mut FigApp, folder: &str) -> Option<String> {
        if !self.launch_roots {
            let dir = if folder.is_empty() { documents_dir().map(|d| d.join(DOCUMENTS_FOLDER)) } else { Some(PathBuf::from(folder)) };
            let Some(dir) = dir else {
                return Some("Agent control could not start: no documents folder was found; choose a folder for agents".into());
            };
            let name = dir.to_string_lossy().into_owned();
            // Only the app's own default folder is made; a folder the user named must exist.
            if folder.is_empty()
                && let Err(e) = std::fs::create_dir_all(&dir)
            {
                return Some(format!("Agent control could not start: {name}: {e}"));
            }
            match Roots::new(std::slice::from_ref(&name), std::slice::from_ref(&name)) {
                Ok(roots) => {
                    fig.automation_roots = Some(Arc::new(roots));
                    fig.automation_roots_for_agents = true;
                }
                Err(e) => return Some(format!("Agent control could not start: the folder for agents: {e}")),
            }
        }
        self.listen(0, std::time::Duration::ZERO)
    }
}

/// The user's documents folder: `XDG_DOCUMENTS_DIR` from `user-dirs.dirs` on Linux and the BSDs
/// when it names one, else `Documents` in the home folder.
fn documents_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)?;
    let named = if cfg!(all(unix, not(target_os = "macos"))) {
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|text| xdg_documents(&text, &home))
    } else {
        None
    };
    Some(named.unwrap_or_else(|| home.join("Documents")))
}

/// `XDG_DOCUMENTS_DIR="$HOME/…"` or an absolute path, in the text of `user-dirs.dirs`.
fn xdg_documents(text: &str, home: &std::path::Path) -> Option<PathBuf> {
    let value = text.lines().filter_map(|l| l.trim().strip_prefix("XDG_DOCUMENTS_DIR=")).next_back()?.trim().trim_matches('"');
    match value.strip_prefix("$HOME/") {
        Some(rest) if !rest.is_empty() => Some(home.join(rest)),
        _ => std::path::Path::new(value).is_absolute().then(|| PathBuf::from(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::xdg_documents;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_documents_folder_follows_user_dirs() {
        let home = Path::new("/home/ada");
        assert_eq!(
            xdg_documents("XDG_DESKTOP_DIR=\"$HOME/Bureau\"\nXDG_DOCUMENTS_DIR=\"$HOME/Mes documents\"\n", home),
            Some(PathBuf::from("/home/ada/Mes documents"))
        );
        assert_eq!(xdg_documents("XDG_DOCUMENTS_DIR=\"/data/docs\"", home), Some(PathBuf::from("/data/docs")));
        // The home folder itself (xdg-user-dirs writes it for a folder that is gone) and a
        // relative path name no documents folder.
        assert_eq!(xdg_documents("XDG_DOCUMENTS_DIR=\"$HOME/\"", home), None);
        assert_eq!(xdg_documents("XDG_DOCUMENTS_DIR=\"docs\"", home), None);
        assert_eq!(xdg_documents("# nothing", home), None);
    }
}
