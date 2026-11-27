//! Build-time channel is distinct from a launch selector. A runtime environment cannot promote a stable build.
use crate::launch_profile::Profile;
use std::ffi::OsString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildFlavor {
    Stable,
    NightlyV1,
}

pub const COMPILED: BuildFlavor = if cfg!(figcraft_nightly_v1) { BuildFlavor::NightlyV1 } else { BuildFlavor::Stable };

pub fn require_profile(flavor: BuildFlavor, profile: &Profile) -> Result<(), String> {
    match (flavor, profile.is_nightly()) {
        (BuildFlavor::NightlyV1, false) => Err("Open FigCraft Nightly from ArtCraft Launcher".into()),
        (BuildFlavor::Stable, true) => Err("This is a stable FigCraft build; nightly-v1 requires the nightly build".into()),
        _ => Ok(()),
    }
}

pub fn needs_native_error(flavor: BuildFlavor, desktop_session: bool) -> bool {
    desktop_session && flavor == BuildFlavor::NightlyV1
}

pub fn version_query(args: &[OsString]) -> bool {
    matches!(args, [arg] if arg == "--version" || arg == "-V")
}

pub fn version(flavor: BuildFlavor, stable: &str) -> String {
    match flavor {
        BuildFlavor::Stable => stable.to_owned(),
        BuildFlavor::NightlyV1 => format!("{stable} (nightly-v1)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_nightly_desktop_refusal_requests_a_native_dialog() {
        assert!(needs_native_error(BuildFlavor::NightlyV1, true));
        assert!(!needs_native_error(BuildFlavor::NightlyV1, false));
        assert!(!needs_native_error(BuildFlavor::Stable, true));
        assert!(!needs_native_error(BuildFlavor::Stable, false));
    }

    #[test]
    fn nightly_build_without_a_profile_is_refused_before_startup() {
        let started = std::cell::Cell::new(false);
        let result = require_profile(BuildFlavor::NightlyV1, &Profile::Stable).map(|()| started.set(true));
        assert!(result.unwrap_err().contains("ArtCraft Launcher"));
        assert!(!started.get());
    }
    #[test]
    fn stable_and_nightly_flavors_accept_only_their_own_profile() {
        let nightly = Profile::NightlyV1(std::path::PathBuf::from("already validated root"));
        assert!(require_profile(BuildFlavor::Stable, &Profile::Stable).is_ok());
        assert!(require_profile(BuildFlavor::Stable, &nightly).is_err());
        assert!(require_profile(BuildFlavor::NightlyV1, &nightly).is_ok());
    }
    #[test]
    fn nightly_version_query_is_unambiguous_and_never_reinterprets_file_arguments() {
        assert_eq!(version(BuildFlavor::Stable, "0.18.1"), "0.18.1");
        assert_eq!(version(BuildFlavor::NightlyV1, "0.18.1"), "0.18.1 (nightly-v1)");
        assert!(version_query(&["--version".into()]));
        assert!(version_query(&["-V".into()]));
        assert!(!version_query(&["--".into(), "--version".into()]));
        assert!(!version_query(&[]));
    }
}
