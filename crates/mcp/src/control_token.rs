//! Where the desktop app keeps the token that guards its control channel, so `mcp --attach` can
//! find it. The app writes the file on each `--control` start; only the user can read it.

use std::path::PathBuf;

/// The FigCraft config folder: the one the app keeps its preferences in.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join("FigCraft"))
}

/// The token file for the control server on `port`.
pub fn token_path(port: u16) -> Option<PathBuf> {
    config_dir().map(|d| d.join(format!("control-{port}.token")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_file_sits_in_the_config_folder_per_port() {
        let path = token_path(7979).unwrap();
        assert!(path.ends_with("FigCraft/control-7979.token"), "{}", path.display());
    }
}
