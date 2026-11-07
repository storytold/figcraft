//! Making the desktop app the opener of `figcraft://` invite links (the engine's
//! [`LinkRegistrar`]). On Windows that's a per-user URL scheme under
//! `HKEY_CURRENT_USER\Software\Classes\figcraft` (no administrator rights; uninstalling the user's
//! registration removes nothing else); the shell then starts `figcraft.exe "<link>"`, and the app
//! joins the session it names. macOS and Linux declare the scheme in their packages
//! (`CFBundleURLTypes`, the desktop entry's `x-scheme-handler/figcraft`), so there is nothing to
//! register at run time.
//!
//! The registry sits behind [`Registry`] so tests use a fake: nothing here touches the real
//! registry unless the app installs [`system`]'s registrar and the user (or an installed build's
//! default) asks for it.

use std::path::{Path, PathBuf};

use figcraft_engine::invite_links::LinkRegistrar;

/// The scheme's key under the current user's hive.
pub const KEY: &str = r"Software\Classes\figcraft";

/// String values in a registry hive (the current user's).
pub trait Registry: Send {
    /// The value `name` of `key` (`""`: the key's default value), if there is one.
    fn get(&self, key: &str, name: &str) -> Option<String>;
    /// Set the string value `name` of `key`, creating the key.
    fn set(&mut self, key: &str, name: &str, value: &str) -> Result<(), String>;
    /// Delete `key` and everything under it.
    fn remove_tree(&mut self, key: &str) -> Result<(), String>;
}

/// The app's registration as the opener of `figcraft://` links.
pub struct UrlScheme<R> {
    reg: R,
    exe: PathBuf,
    installed: bool,
}

impl<R: Registry> UrlScheme<R> {
    /// For the app at `exe`; `installed` sets the preference's default (see [`installed_build`]).
    pub fn new(reg: R, exe: PathBuf, installed: bool) -> Self {
        Self { reg, exe, installed }
    }

    /// The command the shell runs for a link.
    pub fn command(&self) -> String {
        format!("\"{}\" \"%1\"", self.exe.display())
    }

    fn values(&self) -> [(String, &'static str, String); 4] {
        [
            (KEY.into(), "", "URL:ClawDesign invite link".into()),
            (KEY.into(), "URL Protocol", String::new()),
            (format!(r"{KEY}\DefaultIcon"), "", format!("\"{}\",0", self.exe.display())),
            (format!(r"{KEY}\shell\open\command"), "", self.command()),
        ]
    }

    /// The registry, for tests.
    pub fn registry(&self) -> &R {
        &self.reg
    }
}

impl<R: Registry> LinkRegistrar for UrlScheme<R> {
    fn registered(&self) -> bool {
        self.reg.get(&format!(r"{KEY}\shell\open\command"), "").is_some_and(|c| c.eq_ignore_ascii_case(&self.command()))
    }

    fn installed(&self) -> bool {
        self.installed
    }

    fn set(&mut self, on: bool) -> Result<(), String> {
        if on {
            for (key, name, value) in self.values() {
                self.reg.set(&key, name, &value)?;
            }
            Ok(())
        } else if self.registered() {
            self.reg.remove_tree(KEY)
        } else {
            // Another copy's registration (or none): not ours to remove.
            Ok(())
        }
    }
}

/// `exe` is an installed copy of the app: a release build that isn't inside a Cargo build
/// directory (which holds a `CACHEDIR.TAG`), so `cargo run` and test builds never register.
pub fn installed_build(exe: &Path) -> bool {
    !cfg!(debug_assertions) && !exe.ancestors().skip(1).any(|d| d.join("CACHEDIR.TAG").is_file())
}

/// This machine's registrar for the running app, where the app registers the scheme itself
/// (Windows); `None` elsewhere.
pub fn system() -> Option<Box<dyn LinkRegistrar>> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().ok()?;
        let installed = installed_build(&exe);
        Some(Box::new(UrlScheme::new(hkcu::CurrentUser, exe, installed)))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
mod hkcu {
    use windows_registry::CURRENT_USER;

    /// `HKEY_CURRENT_USER`.
    pub struct CurrentUser;

    impl super::Registry for CurrentUser {
        fn get(&self, key: &str, name: &str) -> Option<String> {
            CURRENT_USER.open(key).ok()?.get_string(name).ok()
        }
        fn set(&mut self, key: &str, name: &str, value: &str) -> Result<(), String> {
            CURRENT_USER.create(key).and_then(|k| k.set_string(name, value)).map_err(|e| e.message())
        }
        fn remove_tree(&mut self, key: &str) -> Result<(), String> {
            CURRENT_USER.remove_tree(key).map_err(|e| e.message())
        }
    }
}
