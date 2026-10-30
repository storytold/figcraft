//! The server's files on disk and reading them back without trusting them: atomic writes (a
//! temporary file beside the target, flushed, then renamed over it), the snapshot format and the
//! append-only log format.
//!
//! - `snapshot.bin`: `FCSN`, a format byte, the generation (u64 LE), a CRC-32 of the rest (u32 LE),
//!   then the session state ([`SessionState`] in the wire encoding).
//! - `log.bin`: `FCLG`, a format byte, the generation of the snapshot it continues, then records:
//!   length (u32 LE), CRC-32 (u32 LE), one [`Journal`] entry in the wire encoding.
//!
//! Compaction writes the next generation's snapshot, then an empty log of that generation. A log
//! whose generation isn't the snapshot's is usually from before a compaction that was cut short
//! (what it holds is in the snapshot already), but no checksum covers either generation, so the
//! store replays such a log anyway and reports any change the snapshot lacked. A log ends at its
//! first damaged record (a write cut short).

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use figcraft_collab::message;
use figcraft_collab::{Journal, SessionState};

pub const SNAPSHOT: &str = "snapshot.bin";
pub const LOG: &str = "log.bin";
pub const META: &str = "meta.json";
pub const LINKS: &str = "links.json";
pub const LIBRARY: &str = "library.bin";
pub const USAGE: &str = "usage.json";
pub const ACTIONS: &str = "actions.json";

const SNAP_MAGIC: &[u8; 4] = b"FCSN";
const LOG_MAGIC: &[u8; 4] = b"FCLG";
const FORMAT: u8 = 1;
const SNAP_HEADER: usize = 4 + 1 + 8 + 4;
pub const LOG_HEADER: usize = 4 + 1 + 8;
const RECORD_HEADER: usize = 8;

/// Free space on the disk holding `path`, in bytes, where it can be told (Unix).
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        rustix::fs::statvfs(path).ok().map(|s| s.f_bavail.saturating_mul(s.f_frsize))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// `path` with `.tmp` added: where its next version is written first.
pub fn temp_of(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(OsStr::to_os_string).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Replace `path` with `bytes` atomically: a reader sees the old or the new content, never a mix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_as(path, bytes, false)
}

/// [`write_atomic`] for a secret: on Unix only the owner can read it, from the moment it exists.
pub fn write_atomic_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_as(path, bytes, true)
}

fn write_atomic_as(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let tmp = temp_of(path);
    {
        let mut o = fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut f = o.open(&tmp)?;
        counted(true);
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_dir(path.parent());
    Ok(())
}

/// Make a rename in `dir` durable (directories can't be opened for that on Windows).
pub fn sync_dir(dir: Option<&Path>) {
    #[cfg(unix)]
    if let Some(d) = dir
        && let Ok(f) = File::open(d)
    {
        let _ = f.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Read a file of at most `max` bytes (a longer one is refused without being read).
pub fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let f = File::open(path).map_err(|e| format!("can't open {}: {e}", name(path)))?;
    counted(false);
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len > max {
        return Err(format!("{} is {} bytes, over the {max}-byte cap", name(path), len));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    f.take(max).read_to_end(&mut bytes).map_err(|e| format!("can't read {}: {e}", name(path)))?;
    Ok(bytes)
}

#[cfg(test)]
thread_local! {
    /// Files read and written on this thread (tests: what an action costs, #1074).
    static IO: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((0, 0)) };
}

/// Count a file read or write (a log append counts as a write).
pub(crate) fn counted(write: bool) {
    #[cfg(test)]
    IO.with(|io| {
        let (r, w) = io.get();
        io.set(if write { (r, w + 1) } else { (r + 1, w) });
    });
    #[cfg(not(test))]
    let _ = write;
}

/// The files read and written on this thread since the last call: `(reads, writes)`.
#[cfg(test)]
pub(crate) fn take_io() -> (u32, u32) {
    IO.with(|io| io.replace((0, 0)))
}

fn name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn crc(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).and_then(|s| s.try_into().ok()).map(u32::from_le_bytes)
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).and_then(|s| s.try_into().ok()).map(u64::from_le_bytes)
}

/// A snapshot file's bytes.
pub fn encode_snapshot(generation: u64, state: &SessionState) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(SNAP_HEADER);
    out.extend_from_slice(SNAP_MAGIC);
    out.push(FORMAT);
    out.extend_from_slice(&generation.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut out = message::encode_into(state, out).map_err(|e| e.to_string())?;
    let sum = crc(&out[SNAP_HEADER..]);
    out[SNAP_HEADER - 4..SNAP_HEADER].copy_from_slice(&sum.to_le_bytes());
    Ok(out)
}

/// A snapshot's generation and body, when its header and checksum are right.
fn snapshot_body(bytes: &[u8]) -> Result<(u64, &[u8]), String> {
    if bytes.get(..4) != Some(SNAP_MAGIC.as_slice()) {
        return Err("the snapshot isn't one (wrong header)".into());
    }
    if bytes.get(4) != Some(&FORMAT) {
        return Err("the snapshot is in a format this version doesn't read".into());
    }
    let (Some(generation), Some(sum), Some(body)) = (u64_at(bytes, 5), u32_at(bytes, 13), bytes.get(SNAP_HEADER..)) else {
        return Err("the snapshot is cut short".into());
    };
    if crc(body) != sum {
        return Err("the snapshot is damaged (checksum mismatch)".into());
    }
    Ok((generation, body))
}

/// A snapshot's generation, after checking its header and checksum (no decoding).
pub fn check_snapshot(bytes: &[u8]) -> Result<u64, String> {
    snapshot_body(bytes).map(|(g, _)| g)
}

/// A snapshot's generation and session state.
pub fn decode_snapshot(bytes: &[u8]) -> Result<(u64, SessionState), String> {
    let (generation, body) = snapshot_body(bytes)?;
    let state = message::decode::<SessionState>(body, body.len()).map_err(|e| format!("the snapshot can't be read: {e}"))?;
    Ok((generation, state))
}

/// An empty log continuing snapshot `generation`.
pub fn log_header(generation: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(LOG_HEADER);
    out.extend_from_slice(LOG_MAGIC);
    out.push(FORMAT);
    out.extend_from_slice(&generation.to_le_bytes());
    out
}

/// One log record.
pub fn record(entry: &Journal) -> Result<Vec<u8>, String> {
    let body = message::encode(entry).map_err(|e| e.to_string())?;
    let len = u32::try_from(body.len()).map_err(|_| "a journal entry over 4 GiB".to_string())?;
    let mut out = Vec::with_capacity(RECORD_HEADER + body.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&crc(&body).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// What a log holds.
#[derive(Debug)]
pub struct LogRead {
    pub generation: u64,
    pub entries: Vec<Journal>,
    /// Why it ends before the end of the file (a damaged or cut-short record), if it does.
    pub damaged: Option<String>,
}

/// Read a log: its generation and the entries up to the first damaged record. Only a header that
/// isn't a log's is an error.
pub fn read_log(bytes: &[u8]) -> Result<LogRead, String> {
    if bytes.get(..4) != Some(LOG_MAGIC.as_slice()) {
        return Err("the log isn't one (wrong header)".into());
    }
    if bytes.get(4) != Some(&FORMAT) {
        return Err("the log is in a format this version doesn't read".into());
    }
    let generation = u64_at(bytes, 5).ok_or("the log is cut short")?;
    let mut at = LOG_HEADER;
    let mut entries = Vec::new();
    let mut damaged = None;
    while at < bytes.len() {
        let (Some(len), Some(sum)) = (u32_at(bytes, at), u32_at(bytes, at + 4)) else {
            damaged = Some(format!("a record cut short at byte {at}"));
            break;
        };
        let start = at + RECORD_HEADER;
        let Some(body) = usize::try_from(len).ok().and_then(|n| bytes.get(start..start.checked_add(n)?)) else {
            damaged = Some(format!("a record cut short at byte {at}"));
            break;
        };
        if crc(body) != sum {
            damaged = Some(format!("a damaged record at byte {at}"));
            break;
        }
        match message::decode::<Journal>(body, body.len()) {
            Ok(e) => entries.push(e),
            Err(e) => {
                damaged = Some(format!("an unreadable record at byte {at} ({e})"));
                break;
            }
        }
        at = start + body.len();
    }
    Ok(LogRead { generation, entries, damaged })
}
