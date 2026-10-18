//! Design token bundles: a zip with a DTCG resolver document (`tokens.resolver.json`) whose sets
//! and modifier contexts point (`$ref`) at a token file each — `sets/<set>.tokens.json` and
//! `modifiers/<modifier>/<context>.tokens.json` — the layout the resolver module describes for
//! tokens kept in several files.
//!
//! Reading takes any zip of token files: a resolver document in it (the shallowest one) is
//! followed; without one, `sets/<name>.json` become sets, `modifiers/<m>/<c>.json` contexts of
//! modifier `m`, and other JSON files sets named after the file. Zips from elsewhere are
//! untrusted: entries, sizes and paths are capped and checked.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use figcraft_doc::tokens::{self as dt, DesignTokens, OrderItem, Source, TokenContext, TokenModifier, TokenSet};
use serde_json::Value;

use crate::{FormatError, io_err, json_options, put};

/// The resolver document's name in a bundle.
pub const RESOLVER: &str = "tokens.resolver.json";
/// Most entries read from a bundle.
const MAX_ENTRIES: usize = 4000;
/// Largest entry read.
const MAX_ENTRY: u64 = 32 << 20;
/// Most bytes read from all entries together.
const MAX_TOTAL: u64 = 256 << 20;

/// Whether `bytes` start like a zip.
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// `name` as a file name: letters, digits, `-`, `_`, `.` and spaces kept, anything else `_`; never
/// empty, `.` or `..`.
fn file_name(name: &str) -> String {
    let s: String = name.trim().chars().take(100).map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '_' }).collect();
    let s = s.trim_matches(['.', ' ']).to_string();
    if s.is_empty() { "_".into() } else { s }
}

/// `base` made unique among `taken` (case-insensitively, for file systems that ignore case).
fn unique(base: String, taken: &mut BTreeSet<String>) -> String {
    let mut name = base.clone();
    let mut k = 2;
    while !taken.insert(name.to_lowercase()) {
        name = format!("{base} {k}");
        k += 1;
    }
    name
}

/// The file each set and context goes to in a bundle.
fn layout(t: &DesignTokens) -> BTreeMap<(String, Option<String>), String> {
    let mut out = BTreeMap::new();
    let mut sets = BTreeSet::new();
    for s in &t.sets {
        let f = unique(file_name(&s.name), &mut sets);
        out.insert((s.name.clone(), None), format!("sets/{f}.tokens.json"));
    }
    let mut mods = BTreeSet::new();
    for m in &t.modifiers {
        let dir = unique(file_name(&m.name), &mut mods);
        let mut ctxs = BTreeSet::new();
        for c in &m.contexts {
            let f = unique(file_name(&c.name), &mut ctxs);
            out.insert((m.name.clone(), Some(c.name.clone())), format!("modifiers/{dir}/{f}.tokens.json"));
        }
    }
    out
}

/// Write `t` as a bundle (`name`: the resolver document's `name` when the tokens have none).
pub fn write(t: &DesignTokens, name: &str) -> Result<Vec<u8>, FormatError> {
    let files = layout(t);
    let key = |src: &Source| match src {
        Source::Set(s) => (s.clone(), None),
        Source::Context { modifier, context } => (modifier.clone(), Some(context.clone())),
    };
    let resolver = dt::resolver_with(t, name, &|src| files.get(&key(src)).cloned());
    let pretty = |v: &Value| serde_json::to_vec_pretty(v).map_err(io_err);
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        put(&mut z, RESOLVER, json_options(), &pretty(&resolver)?)?;
        for s in &t.sets {
            if let Some(f) = files.get(&(s.name.clone(), None)) {
                put(&mut z, f, json_options(), &pretty(&tree(&s.tokens))?)?;
            }
        }
        for m in &t.modifiers {
            for c in &m.contexts {
                if let Some(f) = files.get(&(m.name.clone(), Some(c.name.clone()))) {
                    put(&mut z, f, json_options(), &pretty(&tree(&c.tokens))?)?;
                }
            }
        }
        z.finish().map_err(io_err)?;
    }
    Ok(buf.into_inner())
}

fn tree(v: &Value) -> Value {
    if v.is_object() { v.clone() } else { Value::Object(Default::default()) }
}

/// Normalise a path inside the zip: `/` separators, `.` and `..` resolved (`None` when it climbs
/// out of the zip).
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Read a bundle: the tokens and warnings.
pub fn read(bytes: &[u8]) -> Result<(DesignTokens, Vec<String>), FormatError> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| FormatError::Corrupt(e.to_string()))?;
    let mut warn = Vec::new();
    let mut files: BTreeMap<String, Value> = BTreeMap::new();
    let mut total = 0u64;
    if z.len() > MAX_ENTRIES {
        warn.push(format!("only the first {MAX_ENTRIES} entries are read"));
    }
    for i in 0..z.len().min(MAX_ENTRIES) {
        let Ok(mut f) = z.by_index(i) else { continue };
        if f.is_dir() {
            continue;
        }
        let Some(name) = normalize(f.name()) else { continue };
        // Folders zip tools add, hidden files and anything that isn't JSON.
        if name.starts_with("__MACOSX/") || name.split('/').any(|p| p.starts_with('.')) || !name.to_lowercase().ends_with(".json") {
            continue;
        }
        if f.size() > MAX_ENTRY || total + f.size() > MAX_TOTAL {
            warn.push(format!("{name}: too large, skipped"));
            continue;
        }
        let mut data = Vec::new();
        if f.by_ref().take(MAX_ENTRY + 1).read_to_end(&mut data).is_err() || data.len() as u64 > MAX_ENTRY {
            warn.push(format!("{name}: couldn't be read"));
            continue;
        }
        total += data.len() as u64;
        match serde_json::from_slice::<Value>(&data) {
            Ok(v) => {
                files.insert(name, v);
            }
            Err(e) => warn.push(format!("{name}: {e}")),
        }
    }
    // The shallowest resolver document, if any.
    let resolver = files
        .iter()
        .filter(|(n, v)| n.to_lowercase().ends_with(".resolver.json") || (n.to_lowercase().ends_with("resolver.json") && dt::is_resolver(v)))
        .min_by_key(|(n, _)| (n.matches('/').count(), n.len()))
        .map(|(n, v)| (n.clone(), v.clone()));
    if let Some((name, doc)) = resolver {
        let dir = name.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
        let load = |f: &str| -> Option<Value> {
            let (file, _) = f.split_once('#').unwrap_or((f, ""));
            files.get(&normalize(&format!("{dir}{file}"))?).cloned()
        };
        let (t, w) = dt::from_resolver(&doc, &load).map_err(FormatError::Corrupt)?;
        warn.extend(w);
        return Ok((t, warn));
    }
    // No resolver: the folders say what each file is.
    let mut t = DesignTokens::default();
    let stem = |n: &str| {
        let f = n.rsplit('/').next().unwrap_or(n);
        let f = f.get(..f.len().saturating_sub(5)).unwrap_or(f);
        f.strip_suffix(".tokens").unwrap_or(f).to_string()
    };
    for (name, v) in &files {
        if dt::is_resolver(v) || !v.is_object() {
            continue;
        }
        let parts: Vec<&str> = name.split('/').collect();
        let modifier = parts.iter().position(|p| p.eq_ignore_ascii_case("modifiers")).and_then(|i| parts.get(i + 1).filter(|_| i + 2 < parts.len()));
        match modifier {
            Some(m) => {
                let ctx = TokenContext { name: stem(name), tokens: v.clone() };
                match t.modifiers.iter_mut().find(|x| x.name == *m) {
                    Some(x) => x.contexts.push(ctx),
                    None => t.modifiers.push(TokenModifier {
                        name: (*m).to_string(),
                        description: String::new(),
                        default: Some(ctx.name.clone()),
                        contexts: vec![ctx],
                    }),
                }
            }
            None => {
                let n = dt::unique_name(&t, &stem(name));
                t.sets.push(TokenSet { name: n, description: String::new(), tokens: v.clone() });
            }
        }
    }
    t.order = t.sets.iter().map(|s| OrderItem::Set(s.name.clone())).chain(t.modifiers.iter().map(|m| OrderItem::Modifier(m.name.clone()))).collect();
    if t.sets.is_empty() && t.modifiers.is_empty() {
        warn.push("no token files found".into());
    }
    Ok((t, warn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> DesignTokens {
        DesignTokens {
            sets: vec![TokenSet { name: "core/base".into(), description: String::new(), tokens: json!({"x": {"$type": "number", "$value": 1}}) }],
            modifiers: vec![TokenModifier {
                name: "theme".into(),
                description: String::new(),
                contexts: vec![
                    TokenContext { name: "light".into(), tokens: json!({"y": {"$type": "number", "$value": "{x}"}}) },
                    TokenContext { name: "dark".into(), tokens: json!({"y": {"$type": "number", "$value": 2}}) },
                ],
                default: Some("light".into()),
            }],
            order: vec![OrderItem::Modifier("theme".into()), OrderItem::Set("core/base".into())],
            ..Default::default()
        }
    }

    #[test]
    fn bundles_round_trip_with_a_file_per_set_and_context() {
        let t = sample();
        let bytes = write(&t, "Example").unwrap();
        assert!(is_zip(&bytes));
        let mut z = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let names: BTreeSet<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
        let want: BTreeSet<String> =
            [RESOLVER, "sets/core_base.tokens.json", "modifiers/theme/light.tokens.json", "modifiers/theme/dark.tokens.json"]
                .map(String::from)
                .into();
        assert_eq!(names, want);
        let mut text = String::new();
        z.by_name(RESOLVER).unwrap().read_to_string(&mut text).unwrap();
        let r: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(r["sets"]["core/base"]["sources"], json!([{"$ref": "sets/core_base.tokens.json"}]));
        assert_eq!(r["modifiers"]["theme"]["contexts"]["dark"], json!([{"$ref": "modifiers/theme/dark.tokens.json"}]));
        let (back, warn) = read(&bytes).unwrap();
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!((back.sets, back.modifiers, back.order), (t.sets, t.modifiers, t.order));
    }

    #[test]
    fn zips_without_a_resolver_read_by_folder() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = json_options();
            put(&mut z, "tokens/sets/core.json", o, br#"{"a": {"$type": "number", "$value": 1}}"#).unwrap();
            put(&mut z, "tokens/modifiers/brand/acme.tokens.json", o, br#"{"b": {"$type": "number", "$value": 2}}"#).unwrap();
            put(&mut z, "tokens/modifiers/brand/globex.tokens.json", o, br#"{"b": {"$type": "number", "$value": 3}}"#).unwrap();
            put(&mut z, "__MACOSX/._core.json", o, b"junk").unwrap();
            put(&mut z, "readme.txt", o, b"hi").unwrap();
            put(&mut z, "broken.json", o, b"{").unwrap();
            z.finish().unwrap();
        }
        let (t, warn) = read(buf.get_ref()).unwrap();
        assert_eq!(t.sets.len(), 1);
        assert_eq!(t.modifiers[0].contexts.len(), 2);
        assert_eq!(warn.len(), 1, "the broken file: {warn:?}");
        assert!(read(b"PK\x03\x04 nope").is_err());
        assert_eq!(normalize("a/../../b"), None);
        assert_eq!(normalize("./a/./b/../c.json").as_deref(), Some("a/c.json"));
    }
}
