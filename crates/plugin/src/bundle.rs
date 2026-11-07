//! Plugin bundles: a plugin as a plugin directory keeps and serves it (its manifest, its module and
//! a few optional asset files) in one uncompressed binary form, so nothing about it can expand
//! beyond the bytes that arrived. The SHA-256 of those bytes names the bundle: a server lists it
//! with every entry and the app checks a download against it before reading anything else.
//!
//! ```text
//! "FCPLUGB1"                          magic
//! u32 LE length, manifest JSON        (UTF-8, `figcraft-plugin.json`)
//! u32 LE length, WebAssembly module
//! u32 LE asset count, then per asset: u32 LE length, name (UTF-8), u32 LE length, bytes
//! ```
//!
//! Everything in a bundle is untrusted: [`Bundle::decode`] reads it without allocating more than
//! its own size, and [`Bundle::check`] validates the manifest (and the API version it asks for), its
//! version, the module (compiled, never run) and the asset names before anyone keeps or installs it.

use std::cmp::Ordering;

use sha2::{Digest, Sha256};

use crate::manifest::Manifest;
use crate::sandbox::{Limits, check_module};

/// The first bytes of every bundle.
pub const MAGIC: &[u8; 8] = b"FCPLUGB1";
/// Most asset files in a bundle, and their longest name.
pub const MAX_ASSETS: usize = 32;
const MAX_ASSET_NAME: usize = 64;
/// Longest version (`1.2.3`), and most of its numbers.
const MAX_VERSION: usize = 32;
const MAX_VERSION_PARTS: usize = 4;

/// A plugin as a directory keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bundle {
    /// The manifest's JSON text.
    pub manifest: String,
    /// The module the manifest's `main` names.
    pub wasm: Vec<u8>,
    /// Files that travel with the plugin (an icon, a licence), installed in its `assets` folder.
    pub assets: Vec<Asset>,
}

/// A file in a bundle: a plain file name (no folders) and its bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub data: Vec<u8>,
}

/// SHA-256 of `bytes`, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A directory version: 1–4 whole numbers separated by dots (`1.2.0`).
pub fn parse_version(s: &str) -> Option<Vec<u64>> {
    if s.is_empty() || s.len() > MAX_VERSION {
        return None;
    }
    let parts: Vec<u64> =
        s.split('.').map(|p| if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) { None } else { p.parse().ok() }).collect::<Option<_>>()?;
    (parts.len() <= MAX_VERSION_PARTS).then_some(parts)
}

/// How version `a` compares with `b` (missing numbers count as 0: `1.2` = `1.2.0`); `None` when
/// either isn't a directory version.
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parse_version(a)?, parse_version(b)?);
    let n = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    Some((0..n).map(|i| at(&a, i).cmp(&at(&b, i))).find(|o| o.is_ne()).unwrap_or(Ordering::Equal))
}

/// Version `candidate` is newer than `installed` (false when either isn't a directory version).
pub fn newer(candidate: &str, installed: &str) -> bool {
    compare_versions(candidate, installed) == Some(Ordering::Greater)
}

/// An asset's name: 1–64 letters, digits, `.`, `-` or `_`, not starting with a dot, and not a name
/// Windows reserves for a device (`con`, `nul.txt`…), so it is a plain file name everywhere.
pub fn check_asset_name(name: &str) -> Result<(), String> {
    let plain = !name.is_empty()
        && name.len() <= MAX_ASSET_NAME
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    let stem = name.split('.').next().unwrap_or_default().to_ascii_lowercase();
    let device = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt")) && stem.len() == 4 && stem.as_bytes().get(3).is_some_and(u8::is_ascii_digit));
    if !plain || device {
        return Err(format!(
            "asset `{}` must be a plain file name: 1–{MAX_ASSET_NAME} letters, digits, `.`, `-` or `_`",
            name.chars().take(40).collect::<String>()
        ));
    }
    Ok(())
}

/// Reads length-prefixed parts of untrusted bytes.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.0.len() {
            return Err("the bundle is cut short".into());
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u32(&mut self) -> Result<usize, String> {
        let b: [u8; 4] = self.take(4)?.try_into().map_err(|_| "the bundle is cut short".to_string())?;
        usize::try_from(u32::from_le_bytes(b)).map_err(|_| "the bundle is damaged".to_string())
    }

    /// A length, then that many bytes (the length can't exceed what's left, so nothing
    /// allocates beyond the input).
    fn part(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()?;
        self.take(n)
    }

    fn text(&mut self, what: &str) -> Result<String, String> {
        String::from_utf8(self.part()?.to_vec()).map_err(|_| format!("the bundle's {what} isn't UTF-8 text"))
    }
}

fn push_part(out: &mut Vec<u8>, bytes: &[u8]) {
    // Parts are capped far below 4 GiB by the limits checked before encoding and after decoding.
    out.extend(u32::try_from(bytes.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend(bytes);
}

impl Bundle {
    /// Its binary form (see the module docs).
    pub fn encode(&self) -> Vec<u8> {
        let size =
            MAGIC.len() + 12 + self.manifest.len() + self.wasm.len() + self.assets.iter().map(|a| 8 + a.name.len() + a.data.len()).sum::<usize>();
        let mut out = Vec::with_capacity(size);
        out.extend(MAGIC);
        push_part(&mut out, self.manifest.as_bytes());
        push_part(&mut out, &self.wasm);
        out.extend(u32::try_from(self.assets.len()).unwrap_or(u32::MAX).to_le_bytes());
        for a in &self.assets {
            push_part(&mut out, a.name.as_bytes());
            push_part(&mut out, &a.data);
        }
        out
    }

    /// Read a bundle of at most `max` bytes (untrusted input: never panics, allocates at most its
    /// size, refuses trailing bytes).
    pub fn decode(bytes: &[u8], max: usize) -> Result<Bundle, String> {
        if bytes.len() > max {
            return Err(format!("the bundle is {} KB, over the {} KB limit", bytes.len() / 1024, max / 1024));
        }
        let mut r = Reader(bytes);
        if r.take(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
            return Err("that isn't a ClawDesign plugin bundle".into());
        }
        let manifest = r.text("manifest")?;
        let wasm = r.part()?.to_vec();
        let count = r.u32()?;
        if count > MAX_ASSETS {
            return Err(format!("the bundle has more than {MAX_ASSETS} assets"));
        }
        let mut assets = Vec::with_capacity(count);
        for _ in 0..count {
            let name = r.text("asset name")?;
            let data = r.part()?.to_vec();
            assets.push(Asset { name, data });
        }
        if !r.0.is_empty() {
            return Err(format!("{} bytes after the bundle", r.0.len()));
        }
        Ok(Bundle { manifest, wasm, assets })
    }

    /// Validate everything before the bundle is kept or installed: the manifest (with the plugin
    /// API version it asks for), a directory version (`1.2.0`), the module (compiled and checked
    /// for what it imports, never run) and the asset names (plain, unique ignoring case). The
    /// manifest.
    pub fn check(&self) -> Result<Manifest, String> {
        let m = Manifest::parse(&self.manifest)?;
        if parse_version(&m.version).is_none() {
            return Err(format!(
                "a plugin in a directory needs a version of 1–{MAX_VERSION_PARTS} numbers like 1.0.0 in its manifest (got `{}`)",
                m.version.chars().take(40).collect::<String>()
            ));
        }
        check_module(&self.wasm, &Limits::default()).map_err(|e| format!("{}: {e}", m.name))?;
        if self.assets.len() > MAX_ASSETS {
            return Err(format!("more than {MAX_ASSETS} assets"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for a in &self.assets {
            check_asset_name(&a.name)?;
            if !seen.insert(a.name.to_ascii_lowercase()) {
                return Err(format!("two assets are called `{}`", a.name));
            }
        }
        Ok(m)
    }
}
