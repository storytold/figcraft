//! Windows: embed the icon and version info in figcraft.exe and size its main thread's stack
//! (no-op elsewhere). Everywhere: compile in the browser build when `FIGCRAFT_WEB_DIST` names it
//! (Share ▸ View on another device; see `web_viewer`).

use std::io::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(windows_slim_errors)");
    println!("cargo::rerun-if-changed=../../assets/app-icon/figcraft.ico");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let mut res = winresource::WindowsResource::new();
        let ico = "../../assets/app-icon/figcraft.ico";
        if std::path::Path::new(ico).exists() {
            res.set_icon(ico);
        }
        res.set("ProductName", "FigCraft");
        res.set("FileDescription", "FigCraft");
        res.set("CompanyName", "ArtCraft");
        if let Err(e) = res.compile() {
            println!("cargo::warning=winresource: {e}");
        }
        // Windows starts the main thread with 1 MB of stack, Linux and macOS with 8 MB. Layers
        // nest up to 256 deep in an export, and that recursion needs the same room everywhere.
        let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
        println!("cargo::rustc-link-arg-bins={}", if msvc { "/STACK:8388608" } else { "-Wl,--stack,8388608" });
    }
    let sha = std::env::var("FIGCRAFT_BUILD_SHA").unwrap_or_default();
    println!("cargo::rustc-env=FIGCRAFT_BUILD_SHA={sha}");
    println!("cargo::rerun-if-env-changed=FIGCRAFT_BUILD_SHA");
    web_viewer();
}

/// The most the compiled-in browser build may take, gzipped (it is about 7 MiB).
const MAX_WEB_BYTES: u64 = 64 << 20;

/// `FIGCRAFT_WEB_DIST`: the browser build's site (`trunk build --release` in `apps/figcraft-web`
/// puts it in `dist/web`; a path relative to the repository's root, or absolute). Its files are
/// gzipped into `OUT_DIR` and listed in `web_site.rs` for the app to serve to phones; without it
/// the list is empty and the app has no View on another device.
fn web_viewer() {
    println!("cargo::rerun-if-env-changed=FIGCRAFT_WEB_DIST");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let mut rows = Vec::new();
    if let Some(dist) = std::env::var_os("FIGCRAFT_WEB_DIST").filter(|d| !d.is_empty()) {
        let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
        let dist = manifest.join("../..").join(dist);
        println!("cargo::rerun-if-changed={}", dist.display());
        match bundle(&dist, &out.join("web")) {
            Ok(files) if files.iter().any(|(p, _)| p == "index.html") => rows = files,
            Ok(_) => println!("cargo::warning=FIGCRAFT_WEB_DIST={}: no index.html there; building without the browser viewer", dist.display()),
            Err(e) => println!("cargo::warning=FIGCRAFT_WEB_DIST={}: {e}; building without the browser viewer", dist.display()),
        }
    }
    let mut src =
        String::from("/// The browser build's files: (path from the site's root, gzipped bytes).\npub const FILES: &[(&str, &[u8])] = &[\n");
    for (path, gz) in &rows {
        src.push_str(&format!("    ({path:?}, include_bytes!({:?})),\n", gz.display().to_string()));
    }
    src.push_str("];\n");
    if let Err(e) = std::fs::write(out.join("web_site.rs"), src) {
        println!("cargo::warning=can't write web_site.rs: {e}");
    }
}

/// Gzip every file under `dist` (but dotfiles and the sample header files for static hosts) into
/// `into`: (its path from `dist` with `/`s, the gzipped copy).
fn bundle(dist: &Path, into: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    walk(dist, dist, 0, &mut found)?;
    found.sort();
    std::fs::create_dir_all(into)?;
    let mut rows = Vec::new();
    let mut total = 0u64;
    for (i, (rel, path)) in found.into_iter().enumerate() {
        let gz = into.join(format!("{i}.gz"));
        let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&gz)?, flate2::Compression::best());
        enc.write_all(&std::fs::read(&path)?)?;
        enc.finish()?;
        total += std::fs::metadata(&gz)?.len();
        if total > MAX_WEB_BYTES {
            return Err(std::io::Error::other(format!("more than {} MiB gzipped", MAX_WEB_BYTES >> 20)));
        }
        rows.push((rel, gz));
    }
    Ok(rows)
}

/// How many folders deep [`walk`] goes under the browser build's folder (trunk writes one or two).
const MAX_WEB_DEPTH: usize = 16;

/// Every file under `dir` (in `root`, `depth` folders down) as (its path from `root` with `/`s,
/// its path), but dotfiles, `_headers` and symbolic links: a link can lead outside the browser
/// build, or back up into it without end, and trunk writes none.
fn walk(root: &Path, dir: &Path, depth: usize, found: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    if depth > MAX_WEB_DEPTH {
        return Err(std::io::Error::other(format!("folders more than {MAX_WEB_DEPTH} deep")));
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name.starts_with('.') || name == "_headers" {
            continue;
        }
        // The entry itself, not where a link leads.
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            println!("cargo::warning=FIGCRAFT_WEB_DIST: {} is a symbolic link; left out", path.display());
        } else if kind.is_dir() {
            walk(root, &path, depth + 1, found)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            found.push((rel, path));
        }
    }
    Ok(())
}
