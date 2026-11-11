//! The shapes [`crate::user_name`] accepts and rejects: the GECOS forms of a `/etc/passwd` entry,
//! the logon-name variables, and the names the Windows Runtime returns. The file is a synthetic
//! one; no real account or personal name is in it.

use std::ffi::OsString;

use super::*;

/// The deadline half of the macOS lookup, which is otherwise untestable: no test can make Directory
/// Services unresponsive, so the child here is a `sleep` and the waiting code is the same. The wait
/// must end at its deadline with the child stopped and reaped, not run to the child's own end.
#[cfg(unix)]
#[test]
fn a_child_that_never_answers_is_stopped_at_its_deadline() {
    use std::process::Stdio;
    // `sleep` itself, not a shell standing in front of it: the process the check below looks for
    // has to be the one the wait is meant to stop.
    let Ok(child) = std::process::Command::new("sleep").arg("30").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return; // no `sleep` to time out on: the waiting code is not what changed
    };
    let started = std::time::Instant::now();
    let pid = child.id();
    let deadline = started + std::time::Duration::from_millis(200);
    assert!(wait_with_deadline(child, deadline, "sleep").is_none(), "the deadline ends the wait, not the child");
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "and it ends it at the deadline, not after the sleep: {:?}", started.elapsed());
    // Stopped *and* waited for: a child that was only killed would still be a process, and
    // `kill -0` finds zombies. So nothing is left for the next start to pile up behind.
    let still_there = std::process::Command::new("sh").arg("-c").arg(format!("kill -0 {pid}")).status().is_ok_and(|status| status.success());
    assert!(!still_there, "the child is reaped, not left running or waiting");
}

const PASSWD: &[u8] = b"# local accounts\n\
    #ada:x:1000:1000:Older Ada,commented out:/home/not-ada:/bin/sh\n\
    \t#ada2:x:1000:1000:Older Ada,indented comment:/home/not-ada2:/bin/sh\n\
    root:x:0:0:root:/root:/bin/bash\n\
    ada:x:1000:1000:Ada Lovelace,Room 1,+44 20,,:/home/ada:/bin/zsh\n\
    anna:x:1001:1001:&  Smith:/home/anna:/bin/sh\r\n\
    bob:x:1002:1002::/home/bob:/bin/sh\n\
    carl:x:1003:1003:   ,office:/home/carl:/bin/sh\n\
    dee:x:1004:1004:Dee \xe9:/home/dee:/bin/sh\n\
    ren:x:1008:1008:Rene Dupont,Office \xe9:/home/ren:/bin/sh\n\
    eve:x:1005\n\
    zoe:x:1006:1006:Zo\xc3\xab Kowalska:/home/zoe:/bin/sh\n\
    zoe:x:1007:1007:Someone Else:/home/zoe2:/bin/sh\n\
    gus:x:1009:1009:\x07\x02:/home/gus:/bin/sh\n\
    +ldapuser::::::";

#[test]
fn the_passwd_full_name_is_the_gecos_name_without_office_and_phones() {
    assert_eq!(full_name(PASSWD, "0").as_deref(), Some("root"));
    // The commented-out line above with the same uid is not an account, and is skipped as
    // `getpwuid` skips it; the real entry below is the one.
    assert_eq!(full_name(PASSWD, "1000").as_deref(), Some("Ada Lovelace"));
    // `&` is the entry's own name, capitalised; a Windows line ending is no part of the name.
    assert_eq!(full_name(PASSWD, "1001").as_deref(), Some("Anna  Smith"));
    // The first line with our uid is the one, as `getpwuid` reads it.
    assert_eq!(full_name(PASSWD, "1006").as_deref(), Some("Zoë Kowalska"));
    assert_eq!(full_name(PASSWD, "1007").as_deref(), Some("Someone Else"));
    // A Latin-1 in the office field behind the name costs nothing: only the name half is read.
    assert_eq!(full_name(PASSWD, "1008").as_deref(), Some("Rene Dupont"));
}

#[test]
fn no_usable_passwd_name_falls_back_to_the_logon_name() {
    for (uid, why) in [
        ("1002", "an empty GECOS field"),
        ("1003", "a full name of blanks"),
        ("1004", "a full name that isn't UTF-8 (Latin-1)"),
        ("1005", "a cut-off entry"),
        ("1009", "a full name of control characters only"),
        ("4242", "no line for our uid"),
        ("", "the empty uid field of a NIS `+` line"),
        ("100", "a prefix of another uid, not a whole match"),
        ("ada", "a logon name is no uid, and the line named by it is somebody else's"),
    ] {
        assert_eq!(full_name(PASSWD, uid), None, "{why} ({uid:?})");
    }
    assert_eq!(full_name(b"", "1000"), None, "an empty file");
    assert_eq!(full_name(b"ada:x:1000:1000:Ada Lovelace", "1000").as_deref(), Some("Ada Lovelace"), "no shell or home");
}

#[test]
fn the_logon_name_skips_blank_and_non_unicode_variables() {
    let env =
        |vars: &'static [(&'static str, &'static str)]| move |name: &str| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(*v));
    let first = LOGON_VARS.first().copied().unwrap_or_default();
    assert_eq!(logon_name(env(&[])), None);
    assert_eq!(logon_name(|name| (name == first).then(|| OsString::from(" ada \t"))).as_deref(), Some("ada"));
    assert_eq!(logon_name(|name| (name == first).then(|| OsString::from(" \t "))), None);
    #[cfg(not(target_os = "windows"))]
    {
        assert_eq!(logon_name(env(&[("USER", "ada"), ("LOGNAME", "root")])).as_deref(), Some("ada"), "USER first");
        assert_eq!(logon_name(env(&[("USER", "  "), ("LOGNAME", "ada")])).as_deref(), Some("ada"));
        use std::os::unix::ffi::OsStringExt;
        let bad = |name: &str| match name {
            "USER" => Some(OsString::from_vec(b"caf\xe9".to_vec())),
            "LOGNAME" => Some(OsString::from("ada")),
            _ => None,
        };
        assert_eq!(logon_name(bad).as_deref(), Some("ada"), "a name that isn't UTF-8 is skipped");
    }
}

#[test]
fn a_first_and_last_name_join_into_one() {
    assert_eq!(joined("Ada", "Lovelace").as_deref(), Some("Ada Lovelace"));
    assert_eq!(joined(" Zoë ", "\tKowalska ").as_deref(), Some("Zoë Kowalska"));
    // Either alone is the name; with neither the display name is tried.
    assert_eq!(joined("Ada", "").as_deref(), Some("Ada"));
    assert_eq!(joined(" ", "Lovelace").as_deref(), Some("Lovelace"));
    assert_eq!(joined("", ""), None);
    assert_eq!(joined(" \t", " "), None);
}

#[test]
fn the_first_and_last_name_win_over_the_display_name() {
    let asked = std::cell::Cell::new(false);
    let display = |name: &'static str| {
        let asked = &asked;
        move || {
            asked.set(true);
            name.to_owned()
        }
    };
    assert_eq!(chosen("Ada", "Lovelace", display("Contoso Admin")).as_deref(), Some("Ada Lovelace"));
    assert!(!asked.get(), "the display name is asked only without a first or last name");
    assert_eq!(chosen("", "Lovelace", display("Contoso Admin")).as_deref(), Some("Lovelace"));
    // Without either (a domain account may have only a display name), the display name.
    assert_eq!(chosen(" ", "", display(" Ada Lovelace ")).as_deref(), Some("Ada Lovelace"));
    assert!(asked.get());
    assert_eq!(chosen("", "", display(" \t")), None, "no name at all: the logon name follows");
}

#[test]
fn capitalised_upper_cases_the_first_letter_only() {
    assert_eq!(capitalised("ada"), "Ada");
    assert_eq!(capitalised("éva"), "Éva");
    assert_eq!(capitalised("ß"), "SS");
    assert_eq!(capitalised(""), "");
}
