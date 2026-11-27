//! Launcher-owned nightly-v1 bootstrap. Selected before logging, updates, fonts or saved state.
//! Stable launches retain their original arguments and directory policy. No environment mutation.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub const VERSION: &str = "nightly-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Profile {
    Stable,
    NightlyV1(PathBuf),
}

impl Profile {
    pub fn is_nightly(&self) -> bool {
        matches!(self, Self::NightlyV1(_))
    }

    pub fn root(&self) -> Option<&Path> {
        match self {
            Self::Stable => None,
            Self::NightlyV1(root) => Some(root),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    pub profile: Profile,
    pub args: Vec<OsString>,
}

static PROFILE: OnceLock<Profile> = OnceLock::new();

pub fn current() -> &'static Profile {
    PROFILE.get().unwrap_or(&Profile::Stable)
}

pub fn install(profile: Profile) -> Result<(), String> {
    PROFILE.set(profile).map_err(|_| "the launch profile was already selected".into())
}

/// Recognize only the agreed literal selector/root pair before `--`. Everything after it is
/// a file, even when it looks like a profile flag. No filesystem or startup hook runs here.
///
/// Every other flag is refused for a nightly launch, which is how Nightly stays away from the
/// control channel and the automation roots the stable app can be started with.
pub fn parse(args: Vec<OsString>) -> Result<Launch, String> {
    let mut selector = None;
    let mut root = None;
    let mut other = Vec::new();
    let mut files = Vec::new();
    let mut iter = args.iter();
    let mut delimiter = false;
    while let Some(arg) = iter.next() {
        if arg == "--" {
            delimiter = true;
            files.extend(iter.cloned());
            break;
        }
        if arg == "--launch-profile" || arg == "--profile-root" {
            let slot = if arg == "--launch-profile" { &mut selector } else { &mut root };
            if slot.is_some() {
                return Err("the profile selector and root must each occur exactly once".into());
            }
            let value = iter.next().filter(|v| !v.is_empty()).ok_or("the profile selector/root needs a value")?;
            if value.to_string_lossy().starts_with("--") {
                return Err("the profile selector/root needs a value".into());
            }
            *slot = Some(value.clone());
        } else if arg.to_string_lossy().starts_with("--launch-profile=") || arg.to_string_lossy().starts_with("--profile-root=") {
            return Err("the nightly protocol requires separate literal selector/root argument tokens".into());
        } else {
            other.push(arg.clone());
        }
    }
    match (selector, root) {
        (None, None) => Ok(Launch { profile: Profile::Stable, args }),
        (Some(version), Some(root)) if version == VERSION => {
            if !delimiter || !other.is_empty() {
                return Err("nightly-v1 accepts only its selector/root followed by -- and file names".into());
            }
            let root = PathBuf::from(root);
            if !root.is_absolute() || root.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err("the nightly profile root must be absolute and contain no parent traversal".into());
            }
            Ok(Launch { profile: Profile::NightlyV1(root), args: std::iter::once(OsString::from("--")).chain(files).collect() })
        }
        (Some(_), Some(_)) => Err("unsupported launch profile (expected nightly-v1)".into()),
        _ => Err("--launch-profile and --profile-root must be supplied together".into()),
    }
}

/// The Launcher's version check: the selector/root pair followed by exactly `--version` (or `-V`),
/// answered like a plain version query. Only [`parse`]'s lexical checks apply; the root is never
/// validated or touched, and `-- --version` stays a file name.
pub fn version_query(args: &[OsString]) -> bool {
    let Some((last, pair)) = args.split_last() else { return false };
    if last != "--version" && last != "-V" {
        return false;
    }
    let lexical = pair.iter().cloned().chain(std::iter::once(OsString::from("--"))).collect();
    matches!(parse(lexical), Ok(Launch { profile: Profile::NightlyV1(_), args }) if args.len() == 1)
}

/// Validation finishes before the caller's first startup effect. Stable never calls the nightly
/// root validator, so its old portable/config resolution stays unchanged.
pub fn prepare(mut launch: Launch, validate: impl FnOnce(&Path) -> Result<PathBuf, String>) -> Result<Launch, String> {
    if let Profile::NightlyV1(root) = &launch.profile {
        launch.profile = Profile::NightlyV1(validate(root)?);
    }
    Ok(launch)
}

/// Resolve the selected private root before the caller's first startup effect.
pub fn validate_root(root: &Path, env: impl Fn(&str) -> Option<OsString>, exe: Option<&Path>) -> Result<PathBuf, String> {
    crate::profile_root::validate(root, env, exe, crate::profile_root::ensure_writable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    fn absolute() -> &'static str {
        if cfg!(windows) { r"C:\NightlyData\profiles\figcraft" } else { "/nightly-data/profiles/figcraft" }
    }
    fn nightly() -> Vec<OsString> {
        args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--"])
    }

    #[test]
    fn stable_arguments_and_directory_policy_are_unchanged() {
        for argv in [args(&[]), args(&["--control", "8765", "a.figcraft"]), args(&["--", "--launch-profile", "file.figcraft"])] {
            let launch = prepare(parse(argv.clone()).unwrap(), |_| panic!("stable must never validate a nightly root")).unwrap();
            assert_eq!(launch, Launch { profile: Profile::Stable, args: argv });
        }
    }

    #[test]
    fn exact_version_and_root_are_parsed_without_reinterpreting_file_names() {
        let mut argv = nightly();
        argv.extend(args(&["--profile-root", "image with spaces.figcraft"]));
        let launch = parse(argv).unwrap();
        assert_eq!(launch.profile, Profile::NightlyV1(PathBuf::from(absolute())));
        assert_eq!(launch.args, args(&["--", "--profile-root", "image with spaces.figcraft"]));
    }

    #[test]
    fn missing_duplicate_unknown_and_conflicting_profile_arguments_are_refused() {
        for argv in [
            args(&["--launch-profile", VERSION]),
            args(&["--profile-root", absolute()]),
            args(&["--launch-profile", "future-v2", "--profile-root", absolute(), "--"]),
            args(&["--launch-profile", VERSION, "--launch-profile", VERSION, "--profile-root", absolute(), "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--profile-root", absolute(), "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", "relative", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", "", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--control", "8765", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--automation-write-root", "/tmp", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute()]),
        ] {
            assert!(parse(argv).is_err());
        }
    }

    #[test]
    fn the_launcher_version_check_is_a_version_query_only_in_its_exact_form() {
        let pair = &["--launch-profile", VERSION, "--profile-root", absolute()];
        let reversed = &["--profile-root", absolute(), "--launch-profile", VERSION];
        for flag in ["--version", "-V"] {
            for form in [pair, reversed] {
                let mut argv = args(form);
                argv.push(flag.into());
                assert!(version_query(&argv), "{argv:?}");
            }
        }
        for argv in [
            args(&["--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--", "--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--version", "--"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--control", "8765", "--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--version", "--version"]),
            args(&["--version", "--launch-profile", VERSION, "--profile-root", absolute()]),
            args(&["--launch-profile", "future-v2", "--profile-root", absolute(), "--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", "relative", "--version"]),
            args(&["--launch-profile", VERSION, "--version"]),
            args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--help"]),
        ] {
            assert!(!version_query(&argv), "{argv:?}");
        }
        // Outside the query the strict protocol still refuses the version flag.
        assert!(parse(args(&["--launch-profile", VERSION, "--profile-root", absolute(), "--version"])).is_err());
    }

    #[test]
    fn failed_root_validation_never_reaches_startup_or_falls_back_to_stable() {
        let reached = Cell::new(false);
        let result = prepare(parse(nightly()).unwrap(), |_| Err("read-only or unsafe profile".into())).map(|_| reached.set(true));
        assert!(result.is_err());
        assert!(!reached.get());
    }

    #[test]
    fn successful_root_validation_keeps_the_persistent_root_not_the_binary_tag() {
        let checked = PathBuf::from(absolute()).join("checked");
        let launch = prepare(parse(nightly()).unwrap(), |_| Ok(checked.clone())).unwrap();
        assert_eq!(launch.profile.root(), Some(checked.as_path()));
        assert!(launch.profile.is_nightly());
    }
}
