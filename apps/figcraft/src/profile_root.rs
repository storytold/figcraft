//! App-owned nightly profile fences; no stable fallback, migration or process environment changes.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// The portable marker and folder FigCraft already uses (`craft_settings::Portable::DEFAULT`): a
/// `portable` file beside the executable keeps the settings in `config/` beside it.
const PORTABLE_MARKER: &str = "portable";
/// The Nightly's own Flatpak id: its sandbox is its own, so it is not the stable app's instance.
const NIGHTLY_FLATPAK_ID: &str = "ai.storyteller.figcraft.nightly";
/// The Linux wrapper's record of an XDG base it overrode (`FIGCRAFT_STABLE_XDG_CONFIG_HOME`, ...).
const STABLE_XDG_KEY_PREFIX: &str = "FIGCRAFT_STABLE_";
/// What a portable copy holds beside its marker.
const PORTABLE_EXES: [&str; 3] = ["figcraft.exe", "figcraft", "FigCraft.app"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Windows,
    Linux,
    Mac,
}

impl Platform {
    fn current() -> Result<Self, String> {
        match std::env::consts::OS {
            "windows" => Ok(Self::Windows),
            "linux" => Ok(Self::Linux),
            "macos" => Ok(Self::Mac),
            _ => Err("nightly-v1 is not defined for this operating system".into()),
        }
    }

    fn ignores_case(self) -> bool {
        self != Self::Linux
    }
}

fn absolute(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("{label} must be absolute and contain no parent traversal"));
    }
    Ok(path)
}

fn required_base(env: &impl Fn(&str) -> Option<OsString>, key: &str) -> Result<PathBuf, String> {
    let value = env(key).filter(|v| !v.is_empty()).ok_or_else(|| format!("cannot establish the stable {key} boundary"))?;
    absolute(PathBuf::from(value), key)
}

fn xdg_base(env: &impl Fn(&str) -> Option<OsString>, key: &str, fallback: &str) -> Result<PathBuf, String> {
    match env(key).filter(|v| !v.is_empty()) {
        Some(value) => absolute(PathBuf::from(value), key),
        None => Ok(required_base(env, "HOME")?.join(fallback)),
    }
}

/// Resolve existing ancestors before creating anything. A dangling link or inaccessible ancestor
/// is not a missing directory that may be replaced or followed later.
fn canonical_future(path: &Path) -> Result<PathBuf, String> {
    let mut parent = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(parent) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(parent.file_name().ok_or("the profile root has no existing ancestor")?.to_os_string());
                parent = parent.parent().ok_or("the profile root has no existing ancestor")?;
            }
            Err(error) => return Err(format!("cannot inspect the profile boundary: {error}")),
        }
    }
    let mut resolved = std::fs::canonicalize(parent).map_err(|e| format!("cannot resolve the profile boundary: {e}"))?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

fn overlaps(a: &Path, b: &Path, platform: Platform) -> bool {
    let parts = |p: &Path| {
        p.components()
            .map(|c| {
                let value = c.as_os_str().to_string_lossy().into_owned();
                if platform.ignores_case() { value.to_lowercase() } else { value }
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (parts(a), parts(b));
    a.starts_with(&b) || b.starts_with(&a)
}

fn installation_boundary(platform: Platform, exe: &Path) -> Result<PathBuf, String> {
    let parent = exe.parent().ok_or("cannot establish the installation boundary")?;
    if platform == Platform::Mac {
        // A profile in Resources is still inside the versioned install, not beside MacOS.
        if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"))) {
            return Ok(bundle.to_path_buf());
        }
    }
    Ok(parent.to_path_buf())
}

/// Every stable data root of this app (the settings folder `services::config_dir` uses, the
/// installer's folders), so a nightly profile can never read or sweep stable user state.
fn forbidden_roots(platform: Platform, env: &impl Fn(&str) -> Option<OsString>, exe: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = match platform {
        Platform::Windows => {
            let mut roots = ["APPDATA", "LOCALAPPDATA"]
                .into_iter()
                .map(|key| required_base(env, key).map(|p| p.join("FigCraft")))
                .collect::<Result<Vec<_>, _>>()?;
            // The MSI's per-user folder (figcraft.wxs installs under ArtCraft). A custom install
            // folder is not known here; the executable's own folder is fenced below.
            roots.push(required_base(env, "LOCALAPPDATA")?.join("Programs").join("ArtCraft").join("FigCraft"));
            for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(base) = env(key).filter(|v| !v.is_empty()) {
                    roots.push(absolute(PathBuf::from(base), key)?.join("ArtCraft").join("FigCraft"));
                }
            }
            roots
        }
        Platform::Linux => {
            // The settings folder is `<platform config>/FigCraft`; the other XDG folders hold no
            // FigCraft state today, but a copy that starts writing there must not become readable.
            // The Linux wrapper (`packaging/linux/nightly-launch.sh`) points XDG_CONFIG_HOME and
            // XDG_DATA_HOME at per-channel folders and passes the bases in effect before that
            // override as STABLE_XDG_KEY_PREFIX + key, so the stable folders under them stay fenced.
            let mut roots = Vec::new();
            for (key, fallback) in
                [("XDG_CONFIG_HOME", ".config"), ("XDG_DATA_HOME", ".local/share"), ("XDG_CACHE_HOME", ".cache"), ("XDG_STATE_HOME", ".local/state")]
            {
                let mut bases = vec![xdg_base(env, key, fallback)?];
                let stable_key = format!("{STABLE_XDG_KEY_PREFIX}{key}");
                if let Some(value) = env(&stable_key).filter(|v| !v.is_empty()) {
                    bases.push(absolute(PathBuf::from(value), &stable_key)?);
                }
                for base in bases {
                    roots.push(base.join("FigCraft"));
                    roots.push(base.join("figcraft"));
                }
            }
            roots.extend(["/usr/bin/figcraft", "/usr/local/bin/figcraft", "/opt/figcraft"].map(PathBuf::from));
            // The stable Flatpak keeps its data in its own per-app folder.
            roots.push(required_base(env, "HOME")?.join(".var/app/ai.storyteller.figcraft"));
            roots
        }
        Platform::Mac => {
            let home = required_base(env, "HOME")?;
            let library = home.join("Library");
            vec![
                library.join("Application Support/FigCraft"),
                library.join("Caches/FigCraft"),
                library.join("Preferences/FigCraft"),
                library.join("Preferences/ai.storyteller.figcraft.plist"),
                library.join("Saved Application State/ai.storyteller.figcraft.savedState"),
                home.join("Applications/FigCraft.app"),
                PathBuf::from("/Applications/FigCraft.app"),
            ]
        }
    };
    roots.push(installation_boundary(platform, exe)?);
    roots.into_iter().map(|p| canonical_future(&p)).collect()
}

fn private_subdirectory(root: &Path, name: impl AsRef<Path>, label: &str) -> Result<PathBuf, String> {
    let chosen = canonical_future(&absolute(root.to_path_buf(), "the nightly profile root")?)?;
    let child = canonical_future(&chosen.join(name))?;
    if child == chosen || !child.starts_with(&chosen) {
        return Err(format!("the nightly {label} must stay inside its selected profile"));
    }
    Ok(child)
}

/// Every existing entry directly in the profile (Plugins, logs, settings and so on) must resolve
/// inside it, so a link there cannot point Nightly's writes at another app's data.
fn entries_stay_inside(root: &Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect the nightly profile: {error}")),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot inspect the nightly profile: {e}"))?;
        private_subdirectory(root, entry.file_name(), "profile entry")?;
    }
    Ok(())
}

/// A portable FigCraft copy: the marker file beside the executable (or the `.app` on macOS).
fn portable_copy(dir: &Path) -> bool {
    dir.join(PORTABLE_MARKER).is_file() && PORTABLE_EXES.iter().any(|exe| dir.join(exe).exists())
}

/// The portable layout in both the supplied and resolved path: the chosen folder itself or any
/// ancestor is a portable copy. A marker only counts beside a FigCraft executable, so an unrelated
/// `portable` file higher up (in a user folder or at a drive root) does not refuse the profile, and
/// a folder merely named `config` (the XDG configuration folder holds the Launcher's profiles) is
/// not a portable settings folder.
fn is_portable_layout(path: &Path) -> bool {
    path.ancestors().any(portable_copy)
}

/// The app's exclusive write probe: the nightly profile must be writable without touching any
/// stable folder, and a failed probe never falls back to the stable settings folder. Each attempt
/// uses a fresh name, so a probe left by a killed run (or a concurrent one) never blocks a writable
/// profile, and only the entry this attempt created is removed.
pub fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for _ in 0..16 {
        let probe = dir.join(format!("nightly-write-probe-{}-{nanos}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
            Ok(file) => {
                drop(file);
                return std::fs::remove_file(probe);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no fresh name for the nightly write probe"))
}

/// The writable hook is injected so filesystem boundary tests exercise the real policy without
/// starting the app or changing global state.
pub fn validate(
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    writable: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    validate_for(Platform::current()?, root, env, exe, writable)
}

fn validate_for(
    platform: Platform,
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    writable: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    absolute(root.to_path_buf(), "the nightly profile root")?;
    if env("FIGCRAFT_CONFIG_DIR").is_some_and(|v| !v.is_empty()) {
        return Err("nightly-v1 cannot be combined with FIGCRAFT_CONFIG_DIR".into());
    }
    if platform != Platform::Windows {
        // A native tar/app-bundle launch must not inherit another app's Flatpak runtime namespace,
        // which could point this profile at the stable app's socket. The Nightly's own Flatpak
        // bundle (ai.storyteller.figcraft.nightly) is a separate sandbox and is allowed.
        let flatpak = env("FLATPAK_ID").filter(|v| !v.is_empty());
        if let Some(id) = flatpak
            && id != NIGHTLY_FLATPAK_ID
        {
            return Err("nightly-v1 cannot inherit another app's Flatpak instance; use its own package or the native Launcher".into());
        }
    }
    let chosen = canonical_future(root)?;
    if is_portable_layout(root) || is_portable_layout(&chosen) {
        return Err("the nightly root must not use a portable FigCraft installation or its config folder".into());
    }
    let exe = absolute(exe.ok_or("cannot establish the installation boundary")?.to_path_buf(), "the executable")?;
    let forbidden = forbidden_roots(platform, &env, &exe)?;
    if forbidden.iter().any(|stable| overlaps(&chosen, stable, platform)) {
        return Err("the nightly root must not overlap stable data or an installation".into());
    }
    entries_stay_inside(&chosen)?;
    // Recovery can read, adopt, overwrite and sweep entries. Never let its alias leave this
    // private profile, including toward a stable portable folder not in our known roots.
    private_subdirectory(&chosen, "Recovery", "recovery folder")?;
    let mut create = std::fs::DirBuilder::new();
    create.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        create.mode(0o700);
    }
    create.create(&chosen).map_err(|e| format!("cannot create the nightly profile: {e}"))?;
    let confirmed = std::fs::canonicalize(&chosen).map_err(|e| format!("cannot resolve the created nightly profile: {e}"))?;
    if confirmed != chosen || forbidden.iter().any(|stable| overlaps(&confirmed, stable, platform)) {
        return Err("the nightly profile changed while it was being validated".into());
    }
    private_subdirectory(&confirmed, "Recovery", "recovery folder")?;
    entries_stay_inside(&confirmed)?;
    writable(&confirmed).map_err(|e| format!("the nightly profile is not writable: {e}"))?;
    Ok(confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// The portable settings folder beside a portable copy's executable.
    const PORTABLE_DIR: &str = "config";

    struct Fixture {
        root: PathBuf,
        env: BTreeMap<String, OsString>,
        exe: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = loop {
                let candidate =
                    std::env::temp_dir().join(format!("figcraft-profile-policy-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                match std::fs::create_dir(&candidate) {
                    Ok(()) => break std::fs::canonicalize(candidate).unwrap(),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create owned fixture: {error}"),
                }
            };
            let home = root.join("home");
            let exe = root.join("install/figcraft");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"fixture executable, never run").unwrap();
            let env = [("HOME", home), ("APPDATA", root.join("roaming")), ("LOCALAPPDATA", root.join("local"))]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into_os_string()))
                .collect();
            Self { root, env, exe }
        }

        fn get(&self, key: &str) -> Option<OsString> {
            self.env.get(key).cloned()
        }

        fn check(&self, platform: Platform, root: &Path) -> Result<PathBuf, String> {
            validate_for(
                platform,
                root,
                |key| self.get(key),
                Some(&self.exe),
                |dir| {
                    let probe = dir.join("exclusive-test-probe");
                    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)?;
                    drop(file);
                    std::fs::remove_file(probe)
                },
            )
        }

        fn home(&self) -> PathBuf {
            PathBuf::from(self.get("HOME").unwrap())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only the directory successfully created by this fixture is owned for removal.
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn a_portable_copy_and_its_config_folder_are_refused_before_any_probe() {
        let fixture = Fixture::new();
        let stable_install = fixture.root.join("stable portable install");
        let stable_data = stable_install.join(PORTABLE_DIR);
        std::fs::create_dir_all(&stable_data).unwrap();
        std::fs::write(stable_install.join(PORTABLE_MARKER), b"owned portable fixture").unwrap();
        std::fs::write(stable_install.join("figcraft.exe"), b"portable copy, never run").unwrap();
        std::fs::write(stable_data.join("state.json"), b"stable settings must not change").unwrap();
        for platform in [Platform::Windows, Platform::Linux, Platform::Mac] {
            for candidate in [&stable_data, &stable_data.join("nested nightly")] {
                let probed = std::cell::Cell::new(false);
                let result = validate_for(
                    platform,
                    candidate,
                    |key| fixture.get(key),
                    Some(&fixture.exe),
                    |_| {
                        probed.set(true);
                        Ok(())
                    },
                );
                assert!(result.is_err(), "a different portable install is not a private Nightly profile");
                assert!(!probed.get(), "reject portable data before a writable probe");
                assert!(!stable_data.join("nested nightly").exists());
                assert_eq!(std::fs::read(stable_data.join("state.json")).unwrap(), b"stable settings must not change");
            }
        }
        let private = fixture.root.join("figcraft-data-backup/nightly");
        assert!(fixture.check(Platform::Linux, &private).is_ok(), "a merely similar name is not a portable copy");
    }

    #[cfg(unix)]
    #[test]
    fn recovery_aliases_cannot_redirect_to_stable_missing_or_profile_root_data() {
        let fixture = Fixture::new();
        let stable = fixture.home().join(".config/FigCraft/Recovery");
        std::fs::create_dir_all(&stable).unwrap();
        std::fs::write(stable.join("stable-autosave"), b"stable recovery bytes").unwrap();
        for (index, target) in [stable.clone(), fixture.root.join("missing recovery target")].into_iter().enumerate() {
            let private = fixture.root.join(format!("nightly-{index}"));
            std::fs::create_dir(&private).unwrap();
            std::os::unix::fs::symlink(&target, private.join("Recovery")).unwrap();
            let probed = std::cell::Cell::new(false);
            let result = validate_for(
                Platform::Linux,
                &private,
                |key| fixture.get(key),
                Some(&fixture.exe),
                |_| {
                    probed.set(true);
                    Ok(())
                },
            );
            assert!(result.is_err(), "Recovery must not escape the private profile or follow a dangling alias");
            assert!(!probed.get());
        }
        let private = fixture.root.join("nightly-self");
        std::fs::create_dir(&private).unwrap();
        std::os::unix::fs::symlink(&private, private.join("Recovery")).unwrap();
        assert!(fixture.check(Platform::Linux, &private).is_err(), "Recovery cannot treat all profile settings as recovery entries");
        assert_eq!(std::fs::read(stable.join("stable-autosave")).unwrap(), b"stable recovery bytes");
    }

    #[cfg(unix)]
    #[test]
    fn recovery_alias_to_a_private_subfolder_keeps_the_profile_usable() {
        let fixture = Fixture::new();
        let private = fixture.root.join("nightly-private");
        let recovery = private.join("private-recovery");
        std::fs::create_dir_all(&recovery).unwrap();
        std::os::unix::fs::symlink(&recovery, private.join("Recovery")).unwrap();
        assert!(fixture.check(Platform::Linux, &private).is_ok());
        assert_eq!(std::fs::canonicalize(private.join("Recovery")).unwrap(), recovery);
    }

    #[test]
    fn the_stable_settings_folder_and_its_ancestors_are_refused() {
        let fixture = Fixture::new();
        // Each platform fences the folder its own stable build resolves (`services::platform_dir`):
        // `%APPDATA%` on Windows, `~/.config` on Linux and Application Support on macOS.
        let stable_folders = [
            (Platform::Windows, fixture.root.join("roaming/FigCraft")),
            (Platform::Linux, fixture.home().join(".config/FigCraft")),
            (Platform::Mac, fixture.home().join("Library/Application Support/FigCraft")),
        ];
        for (platform, stable) in stable_folders {
            std::fs::create_dir_all(&stable).unwrap();
            std::fs::write(stable.join("state.json"), b"stable settings").unwrap();
            for root in [stable.clone(), stable.join("nightly")] {
                let calls = std::cell::Cell::new(0);
                let result = validate_for(
                    platform,
                    &root,
                    |key| fixture.get(key),
                    Some(&fixture.exe),
                    |_| {
                        calls.set(calls.get() + 1);
                        Ok(())
                    },
                );
                assert!(result.unwrap_err().contains("overlap"));
                assert_eq!(calls.get(), 0);
            }
            assert_eq!(std::fs::read(stable.join("state.json")).unwrap(), b"stable settings");
        }
    }

    #[test]
    fn overlap_check_handles_ancestor_descendant_and_prefix_lookalikes() {
        let fixture = Fixture::new();
        let root = fixture.home().join("nightly/profiles/figcraft");
        for platform in [Platform::Windows, Platform::Linux, Platform::Mac] {
            assert!(overlaps(&root, &root.join("Recovery"), platform));
            assert!(overlaps(&root.join("Recovery"), &root, platform));
            assert!(!overlaps(&root, &root.with_file_name("figcraft-other"), platform));
        }
    }

    #[test]
    fn linux_fences_app_xdg_subtrees_but_allows_launcher_sibling() {
        let mut fixture = Fixture::new();
        for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"] {
            fixture.env.insert(key.into(), fixture.root.join(key).into_os_string());
        }
        for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"] {
            for name in ["FigCraft", "figcraft"] {
                let stable = PathBuf::from(fixture.get(key).unwrap()).join(name);
                std::fs::create_dir_all(&stable).unwrap();
                std::fs::write(stable.join("stable-sentinel"), b"stable bytes").unwrap();
                for candidate in [stable.clone(), stable.join("nightly")] {
                    assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("overlap"));
                }
                assert_eq!(std::fs::read(stable.join("stable-sentinel")).unwrap(), b"stable bytes");
            }
        }
        let data = PathBuf::from(fixture.get("XDG_DATA_HOME").unwrap());
        let nightly = data.join("ArtCraftLauncher/nightly/profiles/figcraft");
        assert_eq!(fixture.check(Platform::Linux, &nightly).unwrap(), nightly);
        assert_eq!(std::fs::read_dir(nightly).unwrap().count(), 0);
    }

    #[test]
    fn mac_fences_only_app_library_subtrees_and_the_entire_bundle() {
        let mut fixture = Fixture::new();
        fixture.exe = fixture.root.join("install/FigCraft Nightly.app/Contents/MacOS/FigCraft");
        std::fs::create_dir_all(fixture.exe.parent().unwrap()).unwrap();
        std::fs::write(&fixture.exe, b"fixture executable, never run").unwrap();
        let library = fixture.home().join("Library");
        for suffix in [
            "Application Support/FigCraft",
            "Caches/FigCraft",
            "Preferences/FigCraft",
            "Preferences/ai.storyteller.figcraft.plist",
            "Saved Application State/ai.storyteller.figcraft.savedState",
        ] {
            let stable = library.join(suffix);
            assert!(fixture.check(Platform::Mac, &stable.join("nightly")).unwrap_err().contains("overlap"));
            assert!(!stable.exists());
        }
        let inside_resources = fixture.exe.parent().unwrap().parent().unwrap().join("Resources/profile");
        assert!(fixture.check(Platform::Mac, &inside_resources).unwrap_err().contains("overlap"));
        assert!(!inside_resources.exists());
        let valid = library.join("Application Support/ArtCraftLauncher/nightly/profiles/figcraft");
        assert_eq!(fixture.check(Platform::Mac, &valid).unwrap(), valid);
    }

    #[test]
    fn windows_roaming_local_and_install_fences_remain_active() {
        let fixture = Fixture::new();
        for key in ["APPDATA", "LOCALAPPDATA"] {
            let stable = PathBuf::from(fixture.get(key).unwrap()).join("FigCraft");
            assert!(fixture.check(Platform::Windows, &stable.join("nightly")).unwrap_err().contains("overlap"));
            assert!(!stable.exists());
        }
        assert!(fixture.check(Platform::Windows, &fixture.exe.parent().unwrap().join("profile")).is_err());
        assert!(fixture.check(Platform::Windows, &fixture.root.join("Launcher/profiles/figcraft")).is_ok());
    }

    #[test]
    fn windows_default_msi_install_folders_are_fenced() {
        let mut fixture = Fixture::new();
        let local = PathBuf::from(fixture.get("LOCALAPPDATA").unwrap());
        let per_user = local.join("Programs").join("ArtCraft").join("FigCraft");
        assert!(fixture.check(Platform::Windows, &per_user.join("nightly")).unwrap_err().contains("overlap"));
        for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
            let base = fixture.root.join(format!("machine {key}"));
            fixture.env.insert(key.into(), base.clone().into_os_string());
            let machine = base.join("ArtCraft").join("FigCraft");
            assert!(fixture.check(Platform::Windows, &machine.join("nightly")).unwrap_err().contains("overlap"));
            assert!(!machine.exists());
        }
        let sibling = local.join("Programs").join("ArtCraft").join("ArtCraftLauncher").join("nightly").join("figcraft");
        assert!(fixture.check(Platform::Windows, &sibling).is_ok(), "the Launcher's own Programs sibling is not FigCraft's install");
    }

    #[test]
    fn a_marker_that_no_figcraft_copy_sits_beside_is_someone_elses_file() {
        let fixture = Fixture::new();
        let unrelated = fixture.root.join("user folder");
        std::fs::create_dir_all(&unrelated).unwrap();
        std::fs::write(unrelated.join(PORTABLE_MARKER), b"another app's marker").unwrap();
        assert!(fixture.check(Platform::Windows, &unrelated.join("Launcher/nightly/figcraft")).is_ok());
        // Beside a FigCraft executable, the same marker makes it a portable copy.
        std::fs::write(unrelated.join("figcraft.exe"), b"portable copy, never run").unwrap();
        assert!(fixture.check(Platform::Windows, &unrelated.join("nightly")).unwrap_err().contains("portable"));
    }

    #[cfg(unix)]
    #[test]
    fn linux_wrapper_override_keeps_the_stable_xdg_folders_fenced() {
        let mut fixture = Fixture::new();
        // What the wrapper exports with XDG_CONFIG_HOME and XDG_DATA_HOME unset.
        let (config_base, data_base) = (fixture.home().join(".config"), fixture.home().join(".local/share"));
        for (key, base) in [("XDG_CONFIG_HOME", &config_base), ("XDG_DATA_HOME", &data_base)] {
            fixture.env.insert(key.into(), base.join("figcraft-nightly").into_os_string());
            fixture.env.insert(format!("{STABLE_XDG_KEY_PREFIX}{key}"), base.clone().into_os_string());
        }
        let stable = config_base.join("FigCraft");
        std::fs::create_dir_all(&stable).unwrap();
        std::fs::write(stable.join("state.json"), b"stable settings").unwrap();
        let profile = data_base.join("figcraft-nightly/profile");
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&stable, &profile).unwrap();
        assert!(fixture.check(Platform::Linux, &profile).unwrap_err().contains("overlap"));
        assert_eq!(std::fs::read_dir(&stable).unwrap().count(), 1, "nothing is written into the stable folder");
        std::fs::remove_file(&profile).unwrap();
        assert_eq!(fixture.check(Platform::Linux, &profile).unwrap(), profile, "a normal Nightly profile stays valid");
    }

    #[test]
    fn a_leftover_write_probe_does_not_block_a_writable_profile() {
        let fixture = Fixture::new();
        let dir = fixture.home().join("nightly");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("nightly-write-probe"), b"left by a killed run").unwrap();
        ensure_writable(&dir).unwrap();
        ensure_writable(&dir).unwrap();
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, ["nightly-write-probe"], "only this attempt's own probe is removed");
    }

    #[test]
    fn linux_fences_the_stable_flatpak_data_folder() {
        let fixture = Fixture::new();
        let flatpak = fixture.home().join(".var/app/ai.storyteller.figcraft");
        for candidate in [flatpak.clone(), flatpak.join("data/nightly")] {
            assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("overlap"));
        }
        assert!(!flatpak.exists());
        assert!(fixture.check(Platform::Linux, &fixture.home().join(".var/app/ai.storyteller.launcher/nightly")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn any_profile_entry_alias_must_stay_inside_the_profile() {
        let fixture = Fixture::new();
        let stable = fixture.home().join(".config/FigCraft/Plugins");
        std::fs::create_dir_all(&stable).unwrap();
        std::fs::write(stable.join("stable-plugin"), b"stable plugin bytes").unwrap();
        for name in ["Plugins", "logs", "state.json"] {
            let private = fixture.root.join(format!("nightly-{name}"));
            std::fs::create_dir(&private).unwrap();
            std::os::unix::fs::symlink(&stable, private.join(name)).unwrap();
            let probed = std::cell::Cell::new(false);
            let result = validate_for(
                Platform::Linux,
                &private,
                |key| fixture.get(key),
                Some(&fixture.exe),
                |_| {
                    probed.set(true);
                    Ok(())
                },
            );
            assert!(result.is_err(), "{name} must not lead outside the profile");
            assert!(!probed.get());
        }
        let private = fixture.root.join("nightly-inside");
        std::fs::create_dir_all(private.join("real-plugins")).unwrap();
        std::fs::write(private.join("state.json"), b"{}").unwrap();
        std::os::unix::fs::symlink(private.join("real-plugins"), private.join("Plugins")).unwrap();
        assert!(fixture.check(Platform::Linux, &private).is_ok(), "regular files and links that stay inside are fine");
        assert_eq!(std::fs::read(stable.join("stable-plugin")).unwrap(), b"stable plugin bytes");
    }

    #[test]
    fn ambiguous_environment_and_non_native_sandbox_are_refused_before_creation() {
        let mut fixture = Fixture::new();
        let candidate = fixture.home().join("nightly/profiles/figcraft");
        fixture.env.insert("XDG_CACHE_HOME".into(), "relative/cache".into());
        assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("absolute"));
        fixture.env.remove("XDG_CACHE_HOME");
        fixture.env.insert("FIGCRAFT_CONFIG_DIR".into(), fixture.home().join("custom-stable").into_os_string());
        assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("FIGCRAFT_CONFIG_DIR"));
        fixture.env.remove("FIGCRAFT_CONFIG_DIR");
        fixture.env.insert("FLATPAK_ID".into(), "ai.storyteller.figcraft".into());
        assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("Flatpak"));
        // The Nightly's own Flatpak bundle is a separate sandbox: its profile is allowed.
        fixture.env.insert("FLATPAK_ID".into(), NIGHTLY_FLATPAK_ID.into());
        assert!(fixture.check(Platform::Linux, &fixture.home().join("nightly/profile")).is_ok());
        fixture.env.remove("FLATPAK_ID");
        fixture.env.remove("HOME");
        assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("HOME"));
        assert!(!candidate.exists());
    }

    #[test]
    fn non_directory_and_failed_write_probe_never_fall_back_to_stable() {
        let fixture = Fixture::new();
        let file = fixture.home().join("not-a-directory");
        std::fs::write(&file, b"do not replace me").unwrap();
        assert!(fixture.check(Platform::Linux, &file).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"do not replace me");
        let candidate = fixture.home().join("nightly");
        let calls = std::cell::Cell::new(0);
        let result = validate_for(
            Platform::Linux,
            &candidate,
            |key| fixture.get(key),
            Some(&fixture.exe),
            |_| {
                calls.set(calls.get() + 1);
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected write refusal"))
            },
        );
        assert!(result.unwrap_err().contains("not writable"));
        assert_eq!(calls.get(), 1);
        assert!(!fixture.home().join(".config/FigCraft").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_dangling_aliases_cannot_create_or_modify_stable_data() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let stable = fixture.home().join(".config/FigCraft");
        std::fs::create_dir_all(&stable).unwrap();
        std::fs::write(stable.join("sentinel"), b"untouched").unwrap();
        let alias = fixture.home().join("alias");
        symlink(&stable, &alias).unwrap();
        assert!(fixture.check(Platform::Linux, &alias.join("nightly")).unwrap_err().contains("overlap"));
        let missing = fixture.home().join("missing-target");
        let dangling = fixture.home().join("dangling");
        symlink(&missing, &dangling).unwrap();
        assert!(fixture.check(Platform::Linux, &dangling.join("nightly")).is_err());
        assert!(!missing.exists());
        assert!(!stable.join("nightly").exists());
        assert_eq!(std::fs::read(stable.join("sentinel")).unwrap(), b"untouched");
    }
}
