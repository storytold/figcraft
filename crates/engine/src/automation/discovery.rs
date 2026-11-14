//! How a client finds the running desktop app: while its control server listens, the app keeps
//! `control.json` in its settings folder, holding `{port, pid}`. `figcraft-cli mcp --attach auto`
//! reads it instead of being told a port. The file is written beside its final name and renamed
//! into place, read with a size cap, and removed by the process that wrote it. A file left by an
//! app that was killed is not detected: connecting to its port is what fails.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The state file's name in the settings folder.
pub const STATE_FILE: &str = "control.json";
/// The settings folder's name in the platform folder.
pub const APP_DIR: &str = "FigCraft";
/// Names another settings folder (set but empty: none).
pub const CONFIG_DIR_VAR: &str = "FIGCRAFT_CONFIG_DIR";
/// Most bytes read from a state file: a real one is a few dozen.
const MAX_STATE: u64 = 4096;

/// The platform's config folder, read from the environment: `%APPDATA%` on Windows,
/// `~/Library/Application Support` on macOS, else `$XDG_CONFIG_HOME` or `~/.config`.
pub fn platform_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    #[cfg(not(any(unix, windows)))]
    let base = None;
    base
}

/// `~/.config` whatever `$XDG_CONFIG_HOME` says, where there is such a folder.
fn home_config_dir() -> Option<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos")))]
    let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"));
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let home = None;
    home
}

/// The first place a client looks for the state file without being told ([`search_dirs`]).
pub fn default_dir() -> Option<PathBuf> {
    search_dirs().into_iter().next()
}

/// Where a client looks for the state file without being told, in order: the folder
/// [`CONFIG_DIR_VAR`] names and no other (set but empty: none); else the app's folder in the
/// platform folder, then in `~/.config`. The second one is for a client run in a sandbox (Flatpak,
/// Snap), whose `$XDG_CONFIG_HOME` is its own while the app outside keeps its settings in
/// `~/.config`. A portable copy of the app keeps its settings beside itself: a client names that
/// folder with the variable.
pub fn search_dirs() -> Vec<PathBuf> {
    dirs_from(std::env::var_os(CONFIG_DIR_VAR), platform_config_dir(), home_config_dir())
}

/// [`search_dirs`] from what the environment says.
fn dirs_from(named: Option<std::ffi::OsString>, platform: Option<PathBuf>, home: Option<PathBuf>) -> Vec<PathBuf> {
    match named {
        Some(dir) if dir.is_empty() => Vec::new(),
        Some(dir) => vec![PathBuf::from(dir)],
        None => {
            let mut dirs: Vec<PathBuf> = platform.into_iter().chain(home).map(|d| d.join(APP_DIR)).collect();
            dirs.dedup();
            dirs
        }
    }
}

/// Write the state file of this process in `dir`: its control server listens on `port`.
// The settings folder is the app's own (named by the app, never by a command or a control
// client), so automation roots don't apply to it: they confine what agents ask for.
#[allow(clippy::disallowed_methods)]
pub fn write(dir: &Path, port: u16) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let (path, beside) = (dir.join(STATE_FILE), dir.join(format!("{STATE_FILE}.{}", std::process::id())));
    std::fs::write(&beside, json!({"port": port, "pid": std::process::id()}).to_string())?;
    std::fs::rename(&beside, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&beside);
    })?;
    Ok(path)
}

/// The state file in `dir`, when it reads as one: `(port, pid)`.
// The app's own settings folder, as in [`write`]; the read is capped.
#[allow(clippy::disallowed_methods)]
pub fn read(dir: &Path) -> Option<(u16, u32)> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(dir.join(STATE_FILE)).ok()?.take(MAX_STATE).read_to_string(&mut text).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let port = v.get("port")?.as_u64().and_then(|p| u16::try_from(p).ok()).filter(|p| *p != 0)?;
    let pid = v.get("pid")?.as_u64().and_then(|p| u32::try_from(p).ok())?;
    Some((port, pid))
}

/// Remove the state file in `dir` if this process wrote it (another app's is left alone).
// The app's own settings folder, as in [`write`].
#[allow(clippy::disallowed_methods)]
pub fn remove(dir: &Path) {
    if read(dir).is_some_and(|(_, pid)| pid == std::process::id()) {
        let _ = std::fs::remove_file(dir.join(STATE_FILE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("figcraft-discovery-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_state_file_names_the_port_and_its_writer_removes_it() {
        let d = dir("round-trip");
        assert_eq!(read(&d), None, "no folder yet");
        let path = write(&d, 7979).unwrap();
        assert_eq!(path, d.join(STATE_FILE));
        assert_eq!(read(&d), Some((7979, std::process::id())));
        remove(&d);
        assert_eq!(read(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_client_looks_in_the_named_folder_only_else_in_the_platform_folder_then_in_the_home_one() {
        let (platform, home) = (PathBuf::from("/sandbox/config"), PathBuf::from("/home/me/.config"));
        let both = || (Some(platform.clone()), Some(home.clone()));
        assert_eq!(dirs_from(Some("/portable".into()), both().0, both().1), [PathBuf::from("/portable")]);
        assert!(dirs_from(Some("".into()), both().0, both().1).is_empty(), "set but empty: no folder");
        // A sandboxed client: its own config folder first, then the one the app outside writes in.
        assert_eq!(dirs_from(None, both().0, both().1), [platform.join(APP_DIR), home.join(APP_DIR)]);
        assert_eq!(dirs_from(None, Some(home.clone()), Some(home.clone())), [home.join(APP_DIR)], "the same folder once");
        assert!(dirs_from(None, None, None).is_empty());
    }

    #[test]
    fn a_bad_file_is_no_state_and_another_apps_file_stays() {
        let d = dir("bad");
        std::fs::create_dir_all(&d).unwrap();
        for bad in ["", "nope", "{\"port\": 0, \"pid\": 1}", "{\"port\": 70000, \"pid\": 1}", "{\"port\": 7979}"] {
            std::fs::write(d.join(STATE_FILE), bad).unwrap();
            assert_eq!(read(&d), None, "{bad}");
        }
        let other = std::process::id().wrapping_add(1);
        std::fs::write(d.join(STATE_FILE), json!({"port": 7979, "pid": other}).to_string()).unwrap();
        remove(&d);
        assert_eq!(read(&d), Some((7979, other)), "written by another process: left alone");
        let _ = std::fs::remove_dir_all(&d);
    }
}
