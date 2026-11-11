//! The user name a desktop start takes while none has been chosen: the system account's, so
//! comments, board stickies and live sessions carry the designer's name rather than the default
//! "You" (`figcraft_engine::cmd::comment::DEFAULT_AUTHOR`). A name set in Preferences ▸ Your name
//! is saved and wins from then on (`crate::use_system_user_name`).
//!
//! - Windows: the account's first and last name, else its display name (only a domain or work
//!   account has one), read through the Windows Runtime's `Windows.System.User`: a safe API,
//!   where the Win32 `GetUserNameExW` needs unsafe, which our workspace forbids. Without either
//!   (an account with no name, or account info access turned off in Settings › Privacy), the
//!   logon name.
//! - Linux, the BSDs: the full name in the running account's `/etc/passwd` entry — the line whose
//!   uid field is our uid, asked of the kernel rather than inferred: an environment variable can say
//!   anything, and the line named by the logon name can only ever be somebody else's. The name is
//!   the GECOS field up to its first comma, as `chfn` sets it. Accounts from a directory service
//!   (LDAP, SSSD) have no line in that file at all, under their uid or their name, and take the
//!   logon name.
//! - macOS: the full name Directory Services keeps for the account, from the system's own lookup
//!   (`/usr/bin/id -F`), else the logon name. `objc2-app-kit` binds no `NSFullUserName`, and reading
//!   Directory Services directly needs unsafe, which our workspace forbids. A directory service
//!   that doesn't answer must not hold up the start, so like the Windows call it is asked on its
//!   own thread with a timeout ([`LOOKUP_TIMEOUT`]), and its `id` process is stopped and waited
//!   for when that deadline passes instead of being left running for the next start to repeat.
//! - Runs the app keeps no preferences for (no settings folder: `FIGCRAFT_CONFIG_DIR` set empty),
//!   runs started for an agent (`--control`, `FIGCRAFT_CONTROL_PORT`) and the browser build,
//!   `figcraft-cli` and the MCP server, which never call this, keep the default: what they sign or
//!   save must stay the same on every machine (the desktop start passes that decision as `ask`).
//!
//! Every source is untrusted: the name still goes through `pref.userName`, as one typed in
//! Preferences does, which trims it, drops control characters and caps it at
//! `figcraft_engine::live::MAX_NAME`.

use std::ffi::OsString;

/// The variables holding the logon name; the first with a usable one wins.
#[cfg(target_os = "windows")]
const LOGON_VARS: &[&str] = &["USERNAME"];
#[cfg(not(target_os = "windows"))]
const LOGON_VARS: &[&str] = &["USER", "LOGNAME"];

/// How long a host account lookup may take before the logon name is used instead. Both the
/// Windows Runtime call and macOS's `id -F` go through the directory services of the machine, and
/// neither is guaranteed to answer when one of them is misconfigured or unreachable. The Windows
/// call is left to its own thread when it overruns, a Windows Runtime call not being one anyone can
/// interrupt; the `id` process is stopped and waited for at the same deadline.
#[cfg(any(target_os = "windows", target_os = "macos", all(test, unix)))]
const LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// The most of `/etc/passwd` read: far more than any real one holds.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
const MAX_PASSWD_BYTES: u64 = 4 << 20;

/// The name of the account running FigCraft: its full name where the system has one, else its
/// logon name; `None` without either.
pub fn system() -> Option<String> {
    let login = logon_name(|var| std::env::var_os(var));
    #[cfg(target_os = "windows")]
    {
        if let Some(name) = windows_account_name() {
            return Some(name);
        }
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    {
        if let Some(name) = passwd_full_name() {
            return Some(name);
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(name) = directory_services_full_name() {
            return Some(name);
        }
    }
    login
}

/// The logon name: the first of [`LOGON_VARS`] that holds a Unicode, non-blank one.
fn logon_name(var: impl Fn(&str) -> Option<OsString>) -> Option<String> {
    LOGON_VARS.iter().find_map(|name| Some(var(name)?.into_string().ok()?.trim().to_owned()).filter(|v| !v.is_empty()))
}

/// The account's name from the Windows Runtime ([`account_name`]), bounded by [`ask_with_timeout`].
/// `None` when there is none, or on any failure (logged).
#[cfg(target_os = "windows")]
fn windows_account_name() -> Option<String> {
    ask_with_timeout("account lookup", || match account_name() {
        Ok(name) => Some(name).filter(|n| !n.trim().is_empty()),
        Err(e) => {
            log::debug!("user name: account lookup failed: {e}");
            None
        }
    })
}

/// `ask`'s answer, asked on a thread of its own so a host lookup that never answers can't hold up
/// opening the window: `what` names it in the log, [`LOOKUP_TIMEOUT`] bounds the wait. A lookup
/// still running when the timeout passes carries on and finishes on its own, its own thread
/// waiting for it (so an `id -F` child is still waited for and reaped); the start doesn't.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn ask_with_timeout(what: &str, ask: impl FnOnce() -> Option<String> + Send + 'static) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    if let Err(e) = std::thread::Builder::new().name("user-name".into()).spawn(move || {
        let _ = tx.send(ask());
    }) {
        log::debug!("user name: no {what} thread: {e}");
        return None;
    }
    match rx.recv_timeout(LOOKUP_TIMEOUT) {
        Ok(answer) => answer,
        Err(_) => {
            log::debug!("user name: {what} gave no answer within {LOOKUP_TIMEOUT:?}");
            None
        }
    }
}

/// The account's name ([`chosen`] from its first, last and display names). Empty when the account
/// has none of them.
#[cfg(target_os = "windows")]
fn account_name() -> windows::core::Result<String> {
    use windows::System::{KnownUserProperties, User};
    // The account this process runs as. `GetDefault` names that one; `FindAllAsync` enumerates the
    // session's users, so it is the fallback, and only when it answers with exactly one — a
    // machine with two signed-in accounts (fast user switching) gets no name from us rather than a
    // guess at which of them this is.
    let user = match User::GetDefault() {
        Ok(user) => user,
        Err(e) => {
            log::debug!("user name: no default account: {e}");
            let users = User::FindAllAsync()?.join()?;
            if users.Size()? != 1 {
                return Ok(String::new());
            }
            users.GetAt(0)?
        }
    };
    let first = user_property(&user, "first name", KnownUserProperties::FirstName);
    let last = user_property(&user, "last name", KnownUserProperties::LastName);
    Ok(chosen(&first, &last, || user_property(&user, "display name", KnownUserProperties::DisplayName)).unwrap_or_default())
}

/// One of `user`'s text properties (`what` names it in the log); empty when the account doesn't
/// have it, or when the lookup fails (logged), so the next source is still tried.
#[cfg(target_os = "windows")]
fn user_property(user: &windows::System::User, what: &str, name: fn() -> windows::core::Result<windows::core::HSTRING>) -> String {
    use windows::Foundation::IReference;
    use windows::core::{HSTRING, Interface};
    let read = || -> windows::core::Result<String> {
        let value = user.GetPropertyAsync(&name()?)?.join()?;
        Ok(value.cast::<IReference<HSTRING>>()?.Value()?.to_string())
    };
    read().unwrap_or_else(|e| {
        log::debug!("user name: account {what}: {e}");
        String::new()
    })
}

/// Which of the account's names is the user name: its first and last name ([`joined`]), else its
/// display name (asked only then) — Windows has a display name only for a domain or work account,
/// a first and last name for any account. `None` when all are blank.
#[cfg(any(test, target_os = "windows"))]
fn chosen(first: &str, last: &str, display: impl FnOnce() -> String) -> Option<String> {
    joined(first, last).or_else(|| Some(display().trim().to_owned()).filter(|n| !n.is_empty()))
}

/// A first and a last name as one name (`Ada` + `Lovelace` → `Ada Lovelace`), either alone when
/// the other is blank; `None` when both are.
#[cfg(any(test, target_os = "windows"))]
fn joined(first: &str, last: &str) -> Option<String> {
    let name = [first.trim(), last.trim()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ");
    Some(name).filter(|n| !n.is_empty())
}

/// The running account's full name in `/etc/passwd` ([`full_name`]), in the line whose uid field is
/// the one the kernel gives for this process; `None` when the file can't be read or has no line for
/// it. The uid comes from the system rather than from the environment or from procfs ownership: a
/// logon name can say anything, and a non-dumpable process's `/proc/self` reads as root (proc(5)).
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
fn passwd_full_name() -> Option<String> {
    use std::io::Read;
    let uid = rustix::process::getuid().as_raw().to_string();
    let mut passwd = Vec::new();
    match std::fs::File::open("/etc/passwd").and_then(|f| f.take(MAX_PASSWD_BYTES).read_to_end(&mut passwd)) {
        Ok(_) => full_name(&passwd, &uid),
        Err(e) => {
            log::debug!("user name: /etc/passwd: {e}");
            None
        }
    }
}

/// The full name in the entry of a `passwd` file (`name:password:UID:GID:GECOS:home:shell` lines)
/// whose uid field reads exactly `uid`, as the system's own lookup reads it. The name is the
/// GECOS field up to its first comma (the rest is office and phone numbers), with `&` standing for
/// the entry's name capitalised, a BSD convention. `None` without an entry, with a full name that
/// has nothing any cleaning could keep (empty, blanks, control characters), or with a *name* that
/// isn't UTF-8: the office and phone fields behind the first comma are never read, so a Latin-1 in
/// one of them keeps nothing.
#[cfg(any(test, all(unix, not(target_os = "macos"), not(target_arch = "wasm32"))))]
fn full_name(passwd: &[u8], uid: &str) -> Option<String> {
    // As `getpwuid` reads the file: a line whose first character that is neither a space nor a tab is
    // `#` is a comment in the file, not an account, so it is skipped rather than matched. A
    // commented-out `#ada:x:1000:...` would otherwise answer for uid 1000 and outrank the live
    // `ada:x:1000:...` line below it, indented just as much, since the uid is still its third field.
    let is_comment = |line: &[u8]| matches!(line.iter().find(|b| !matches!(**b, b' ' | b'\t')), Some(&b'#'));
    let entries: Vec<Vec<&[u8]>> =
        passwd.split(|&b| b == b'\n').filter(|line| !is_comment(line)).map(|line| line.split(|&b| b == b':').collect()).collect();
    // Our own entry, and only ours. The entry named by the logon name is not a candidate: an
    // account from a directory service has no line in this file under its uid or its name, so a
    // match by name can only ever land on somebody else's local entry - and a full name is
    // somebody's, so guessing wrong signs their work. The uid is matched whole, not as a prefix, so
    // `100` never takes `1000`'s entry, and the first line with our uid is the one, as `getpwuid`.
    let entry = entries.iter().find(|fields| fields.get(2) == Some(&uid.as_bytes()))?;
    let account = std::str::from_utf8(entry.first()?).ok()?;
    // Only the part of GECOS before the first comma has to be UTF-8. The office and phone fields
    // behind it are never read, and a Latin-1 in one of them must not cost the person their name.
    let gecos = entry.get(4)?;
    let name = match gecos.iter().position(|&b| b == b',') {
        Some(comma) => &gecos[..comma],
        None => gecos,
    };
    let name = std::str::from_utf8(name).ok()?;
    let name = if name.contains('&') { name.replace('&', &capitalised(account)) } else { name.to_owned() };
    // A full name with nothing in it that a cleaning could keep - blanks, control characters - is no
    // name at all: `None` lets the logon name have its turn, rather than one stray control character
    // in the GECOS field leaving the person with the placeholder. The name itself is handed on as it
    // stands, each app cleaning a name the same way it cleans one typed in.
    let nothing_left = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().is_empty();
    (!nothing_left).then(|| name.trim().to_owned())
}

/// The macOS account's full name, from the system's own Directory Services lookup (`id -F`, which
/// prints the login name when the account has no real name), bounded by [`ask_with_timeout`] so the
/// window never waits on it; `None` when it fails.
#[cfg(target_os = "macos")]
fn directory_services_full_name() -> Option<String> {
    ask_with_timeout("id -F", lookup_id_dash_f)
}

/// Wait for `child` until `deadline`; `None` if it has not answered by then, in which case it is
/// stopped and waited for, so a lookup that never finishes leaves no process behind for the next
/// start to add to. `what` names it in the log. Split out from the `id -F` lookup because a test
/// cannot make Directory Services unresponsive, but it can hand a wait like this a child that never
/// ends, which is what `a_child_that_never_answers_is_stopped_at_its_deadline` does.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn wait_with_deadline(mut child: std::process::Child, deadline: std::time::Instant, what: &str) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(10)),
            Ok(None) => {
                log::debug!("user name: {what} gave no answer within {LOOKUP_TIMEOUT:?}");
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(e) => {
                log::debug!("user name: {what}: {e}");
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The system's own `id -F`, run as `/usr/bin/id`: a GNU coreutils `id` earlier in `PATH`, easy to
/// have on a Mac with coreutils installed, has no `-F` and would only print an error. Stopped and
/// waited for if it outlives [`LOOKUP_TIMEOUT`]: a directory service that never answers would
/// otherwise leave an `id` process waiting on every start. The name it prints is far below what a
/// pipe holds, so reading its output after it finished cannot block.
#[cfg(target_os = "macos")]
fn lookup_id_dash_f() -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = match Command::new("/usr/bin/id").arg("-F").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(e) => {
            log::debug!("user name: id -F: {e}");
            return None;
        }
    };
    let (out, err) = (child.stdout.take(), child.stderr.take());
    let status = wait_with_deadline(child, std::time::Instant::now() + LOOKUP_TIMEOUT, "id -F")?;
    let mut text = String::new();
    if status.success() {
        if let Some(mut pipe) = out {
            let _ = pipe.read_to_string(&mut text);
        }
    } else {
        if let Some(mut pipe) = err {
            let _ = pipe.read_to_string(&mut text);
        }
        log::debug!("user name: id -F: {}", text.trim());
        return None;
    }
    Some(text.trim().to_owned()).filter(|n| !n.is_empty())
}

/// `word` with its first letter upper case (`ada` → `Ada`).
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
#[path = "user_name_tests.rs"]
mod tests;
