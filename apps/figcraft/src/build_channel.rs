//! The build-time nightly identity shared by the build script and its pure regression tests.
pub fn is_nightly_version(version: &str) -> bool {
    let Some((base, suffix)) = version.split_once("-nightly.") else { return false };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let base: Vec<_> = base.split('.').collect();
    let parts: Vec<_> = suffix.split('.').collect();
    base.len() == 3
        && base.iter().all(|s| digits(s))
        && parts.len() == 4
        && parts[0].len() == 8
        && digits(parts[0])
        && digits(parts[1])
        && digits(parts[2])
        && parts[3].strip_prefix('g').is_some_and(|sha| sha.len() == 12 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

pub fn windows_icon_path(nightly: bool) -> &'static str {
    if nightly { "../../assets/app-icon/figcraft-nightly.ico" } else { "../../assets/app-icon/figcraft.ico" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_resource_uses_the_flavor_specific_icon_without_changing_stable() {
        assert_eq!(windows_icon_path(false), "../../assets/app-icon/figcraft.ico");
        assert_eq!(windows_icon_path(true), "../../assets/app-icon/figcraft-nightly.ico");
    }

    #[test]
    fn explicit_feed_identity_is_required_for_a_nightly_build() {
        assert!(is_nightly_version("0.18.1-nightly.20261120.17.1.gabcdef012345"));
        for invalid in [
            "",
            "0.18.1",
            "0.18.1-nightly",
            "0.18.1-nightly.20261120",
            "0.18.1-nightly.20261120.17.1.gABCDEF012345",
            "0.18.1-nightly.20261120.17.1.gabcdef012345\n",
        ] {
            assert!(!is_nightly_version(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn version_cannot_inject_a_path_or_an_extra_build_directive() {
        for invalid in [
            "../0.18.1-nightly.20261120.1.1.gabcdef012345",
            "0.18.1-nightly.20261120.1.1.gabcdef012345/other",
            "0.18.1-nightly.20261120.1.1.gabcdef012345\ncargo:rustc-cfg=other",
        ] {
            assert!(!is_nightly_version(invalid));
        }
    }
}
