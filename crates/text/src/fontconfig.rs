//! Optional Linux Fontconfig enumeration. Conventional font folders remain the fallback.
use std::io::{self, Read};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fd::OwnedFd;
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, Signal, WaitId, WaitIdOptions, kill_process_group, pidfd_open, waitid};

// Enumerate each file value separately before escaping: commas in filenames are not separators.
// The escape converter prefixes backslash to both backslash and newline, preserving Unix bytes.
const FORMAT: &str = r"%{[]file{%{file[0]|escape(\\\n)}\n}}";

#[derive(Clone, Copy)]
struct Limits {
    stdout: usize,
    stderr: usize,
    time: Duration,
}

const LIMITS: Limits = Limits { stdout: 16 * 1024 * 1024, stderr: 64 * 1024, time: Duration::from_secs(10) };

pub(super) fn files() -> io::Result<Vec<PathBuf>> {
    let mut command = Command::new("fc-list");
    command.args(["--format", FORMAT]);
    decode(&query(command, LIMITS, true)?)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn decode(bytes: &[u8]) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut path = Vec::new();
    let mut escaped = false;
    for &byte in bytes {
        if escaped {
            if !matches!(byte, b'\\' | b'\n') {
                return Err(invalid("invalid Fontconfig filename escape"));
            }
            path.push(byte);
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'\n' {
            let file = PathBuf::from(std::ffi::OsString::from_vec(std::mem::take(&mut path)));
            if !file.is_absolute() {
                return Err(invalid("Fontconfig returned an empty or relative filename"));
            }
            files.push(file);
        } else if byte == 0 {
            return Err(invalid("Fontconfig returned a filename containing NUL"));
        } else {
            path.push(byte);
        }
    }
    if escaped || !path.is_empty() {
        return Err(invalid("incomplete Fontconfig filename record"));
    }
    files.sort_unstable();
    files.dedup();
    Ok(files)
}

/// Keep the leader unreaped until the owned process group has been stopped, even on success.
/// Reaping first could allow its numeric process-group id to be reused before cleanup.
struct Group {
    child: Child,
    done: bool,
}

impl Group {
    fn finish(&mut self) -> io::Result<ExitStatus> {
        match kill_process_group(Pid::from_child(&self.child), Signal::KILL) {
            Ok(()) | Err(Errno::SRCH) => {}
            Err(error) => return Err(error.into()),
        }
        let status = self.child.wait()?;
        self.done = true;
        Ok(status)
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if !self.done {
            let _ = kill_process_group(Pid::from_child(&self.child), Signal::KILL);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn query(mut command: Command, limits: Limits, use_pidfd: bool) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + limits.time;
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    let mut group = Group { child: command.spawn()?, done: false };
    let pid = Pid::from_child(&group.child);
    let mut stdout = group.child.stdout.take().ok_or_else(|| invalid("missing Fontconfig stdout pipe"))?;
    let mut stderr = group.child.stderr.take().ok_or_else(|| invalid("missing Fontconfig stderr pipe"))?;
    fcntl_setfl(&stdout, fcntl_getfl(&stdout)? | OFlags::NONBLOCK)?;
    fcntl_setfl(&stderr, fcntl_getfl(&stderr)? | OFlags::NONBLOCK)?;
    let descriptor: Option<OwnedFd> = if use_pidfd {
        match pidfd_open(pid, PidfdFlags::empty()) {
            Ok(fd) => Some(fd),
            // Linux before 5.3 has no child-exit descriptor. Retain compatibility using
            // WNOWAIT and a positive bounded status interval beneath the same deadline.
            Err(Errno::NOSYS) => None,
            Err(error) => return Err(error.into()),
        }
    } else {
        None
    };
    let (mut out_open, mut err_open, mut exited) = (true, true, false);
    let mut output = Vec::new();
    let mut errors = 0;
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Fontconfig query timed out"));
        }
        if out_open {
            out_open = drain(&mut stdout, &mut output, limits.stdout, deadline)?;
        }
        if err_open {
            // Count stderr without retaining its content or letting a noisy child block stdout.
            let mut chunk = Vec::new();
            err_open = drain(&mut stderr, &mut chunk, limits.stderr.saturating_sub(errors), deadline)?;
            errors += chunk.len();
        }
        if descriptor.is_none() {
            exited = waitid(WaitId::Pid(pid), WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT)?.is_some();
        }
        if exited {
            if out_open || err_open {
                return Err(invalid("Fontconfig exited while an inherited output pipe remained open"));
            }
            return if group.finish()?.success() { Ok(output) } else { Err(io::Error::other("Fontconfig query failed")) };
        }
        let mut watched = Vec::with_capacity(3);
        if out_open {
            watched.push(PollFd::new(&stdout, PollFlags::IN));
        }
        if err_open {
            watched.push(PollFd::new(&stderr, PollFlags::IN));
        }
        if let Some(fd) = &descriptor {
            watched.push(PollFd::new(fd, PollFlags::IN));
        }
        let remaining =
            deadline.checked_duration_since(Instant::now()).ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Fontconfig query timed out"))?;
        let interval = if descriptor.is_some() { remaining } else { remaining.min(Duration::from_millis(25)) };
        let timeout = Timespec::try_from(interval).map_err(io::Error::other)?;
        match poll(&mut watched, Some(&timeout)) {
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
        if watched.iter().any(|fd| fd.revents().intersects(PollFlags::ERR | PollFlags::NVAL)) {
            return Err(invalid("Fontconfig output polling failed"));
        }
        if descriptor.is_some() {
            exited = watched.last().is_some_and(|fd| fd.revents().contains(PollFlags::IN));
        }
    }
}

/// Drain a nonblocking pipe, retaining at most its declared cap. True means a writer remains.
fn drain(reader: &mut impl Read, output: &mut Vec<u8>, cap: usize, deadline: Instant) -> io::Result<bool> {
    let mut buffer = [0; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Fontconfig query timed out"));
        }
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(false),
            Ok(count) => {
                let bytes = buffer.get(..count).ok_or_else(|| invalid("invalid Fontconfig read length"))?;
                if count > cap.saturating_sub(output.len()) {
                    return Err(invalid("Fontconfig output limit exceeded"));
                }
                output.try_reserve(count).map_err(io::Error::other)?;
                output.extend_from_slice(bytes);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
pub(super) fn test_command(command: Command) -> io::Result<Vec<u8>> {
    query(command, LIMITS, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    fn shell(script: &str) -> Command {
        // Test peers use POSIX shell utilities; font discovery itself only invokes fc-list.
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    fn small() -> Limits {
        Limits { stdout: 8, stderr: 8, time: Duration::from_secs(2) }
    }

    #[test]
    fn escaped_records_preserve_unix_filenames_and_deduplicate() {
        let decoded = decode(b"/plain,a.ttf\n/fonts/new\\\nline\\\\name/\xff.ttf\n/plain,a.ttf\n").unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].as_os_str().as_bytes(), b"/fonts/new\nline\\name/\xff.ttf");
        assert_eq!(decoded[1].as_os_str().as_bytes(), b"/plain,a.ttf");
        assert!(decode(b"").unwrap().is_empty());
        for bad in [b"/wrong\\q\n".as_slice(), b"/incomplete\\", b"/no-newline", b"relative/file\n", b"/nul\0.ttf\n", b"\n"] {
            assert!(decode(bad).is_err(), "{bad:?}");
        }
        // A valid first record never hides an incomplete second record.
        assert!(decode(b"/good.ttf\n/bad").is_err());
    }

    #[test]
    fn a_query_accepts_exact_caps_but_never_partial_or_failed_output() {
        for pidfd in [true, false] {
            assert_eq!(query(shell("printf 12345678; printf 12345678 >&2"), small(), pidfd).unwrap(), b"12345678");
            assert!(query(shell("printf 123456789"), small(), pidfd).is_err());
            assert!(query(shell("printf 123456789 >&2"), small(), pidfd).is_err());
            assert!(query(shell("printf valid; exit 1"), small(), pidfd).is_err());
        }
        assert_eq!(query(Command::new("no-such-figcraft-fontconfig-tool"), small(), true).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("figcraft-fontconfig-peer-{}-{next}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn stopped(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return true };
        // A grandchild may briefly be a zombie pending its system reaper; it is no longer running.
        stat.rsplit_once(") ").is_some_and(|(_, fields)| fields.starts_with('Z'))
    }

    #[test]
    fn timeout_and_inherited_pipes_stop_owned_processes() {
        for pidfd in [true, false] {
            for mode in ["timeout", "inherited", "overflow"] {
                let directory = Directory::new();
                let ids = directory.0.join("ids");
                let script = match mode {
                    "timeout" => "sleep 30 & printf '%s %s' \"$$\" \"$!\" > \"$1\"; wait",
                    "overflow" => "sleep 30 & printf '%s %s' \"$$\" \"$!\" > \"$1\"; printf 123456789",
                    _ => "sleep 30 & printf '%s %s' \"$$\" \"$!\" > \"$1\"; printf valid",
                };
                let mut command = shell(script);
                command.arg("fontconfig-test").arg(&ids);
                let error =
                    query(command, Limits { stdout: if mode == "overflow" { 8 } else { 64 }, stderr: 64, time: Duration::from_secs(1) }, pidfd)
                        .unwrap_err();
                assert_eq!(error.kind(), if mode == "timeout" { io::ErrorKind::TimedOut } else { io::ErrorKind::InvalidData });
                let pids: Vec<u32> = std::fs::read_to_string(ids).unwrap().split_whitespace().map(|value| value.parse().unwrap()).collect();
                assert_eq!(pids.len(), 2);
                assert!(!std::path::Path::new(&format!("/proc/{}", pids[0])).exists(), "direct child was not reaped");
                // SIGKILL delivery and orphan adoption are asynchronous. Observe only our peers,
                // with a positive bounded wait rather than assuming they disappear in one syscall.
                let deadline = Instant::now() + Duration::from_secs(2);
                while !stopped(pids[1]) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(stopped(pids[1]), "descendant retained the query's pipe");
            }
        }
    }
}
