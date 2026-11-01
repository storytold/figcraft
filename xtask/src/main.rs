//! Project tasks: `cargo xtask <task>`.
//!
//! - `ci`      fmt check, clippy -D warnings, tests, the translation report, layers, assets, docs, wasm
//!   check
//! - `test`    the workspace tests (nextest when installed) and doctests
//! - `i18n-report` the UI strings, command labels and menu wordings each language lacks (never fails
//!   on them: a missing translation reads English)
//! - `layers`  crate dependency layering check
//! - `assets`  every file under assets/ and docs/ has a row in ATTRIBUTION.md
//! - `docs`    no table row twice and no status-line item listed twice in ROADMAP.md and docs/*.md
//!   (what a merge that keeps both sides leaves)
//! - `parity`  menu-catalog coverage → docs/parity-checklist.md
//! - `version` print the version, or `version set X.Y.Z`
//! - `icon`    regenerate the app icon PNGs/ICO from assets/app-icon/figcraft.svg (needs resvg)
//! - `wasm`    check the core crates and web app for wasm32, and build the plugin SDK and examples
//! - `plugins` format-check, lint, test and build the plugin SDK and example plugins (`plugins/`, a
//!   workspace of its own) for wasm32: `<target>/plugins/dist/<example>/` holds each manifest and
//!   module, ready for Plugins ▸ Development ▸ Import plugin from manifest…
//! - `bench`   release-build timings at 1k/10k/50k layers (`bench 1000,10000` for other sizes) and
//!   with a synthetic design system used by 2,000 instances

mod docs_check;
mod icon;
mod layers;

use std::process::{Command, ExitCode, Stdio};

fn run(cmd: &mut Command) -> bool {
    eprintln!("$ {cmd:?}");
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(std::path::Path::to_path_buf).unwrap_or_default()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().map(String::as_str).unwrap_or("help");
    let ok = match task {
        "ci" => ci(),
        "test" => test(),
        "i18n-report" => i18n_report(),
        "layers" => layers::check(&root()),
        "assets" => assets(),
        "docs" => docs_check::check(&root()),
        "parity" => parity(),
        "version" => version(args.get(1).map(String::as_str), args.get(2).map(String::as_str)),
        "icon" => icon::generate(&root()),
        "wasm" => wasm(),
        "plugins" => plugins(),
        "bench" => bench(args.get(1).map_or("1000,10000,50000", String::as_str)),
        _ => {
            eprintln!("tasks: ci, test, i18n-report, layers, assets, docs, parity, version [set X.Y.Z], icon, wasm, plugins, bench [SIZES]");
            task == "help"
        }
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn ci() -> bool {
    run(cargo().args(["fmt", "--all", "--check"]))
        && run(cargo().args(["clippy", "--workspace", "--all-targets", "--keep-going", "--", "-D", "warnings"]))
        && test()
        && i18n_report()
        && layers::check(&root())
        && assets()
        && docs_check::check(&root())
        && wasm()
}

/// Unit and integration tests with nextest when it is installed (`cargo install cargo-nextest`),
/// else with `cargo test`; then the doctests, which nextest does not run. Examples are not built:
/// clippy's `--all-targets` already checks them.
fn test() -> bool {
    let nextest = cargo().args(["nextest", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    let units = if nextest {
        run(cargo().args(["nextest", "run", "--workspace", "--lib", "--bins", "--tests"]))
    } else {
        eprintln!("xtask: cargo-nextest not installed; running unit and integration tests with `cargo test`");
        run(cargo().args(["test", "--workspace", "--lib", "--bins", "--tests"]))
    };
    units && run(cargo().args(["test", "--workspace", "--doc"]))
}

/// Prints what each language's catalog lacks, from the test that lists it. `--workspace --lib`
/// reuses the binaries `test` built.
fn i18n_report() -> bool {
    run(cargo().args(["test", "-q", "--workspace", "--lib", "--", "--exact", "i18n::tests::missing_translations_report", "--nocapture"]))
}

fn wasm() -> bool {
    run(cargo().args([
        "check",
        "--keep-going",
        "--target",
        "wasm32-unknown-unknown",
        "-p",
        "figcraft-collab",
        "-p",
        "figcraft-collab-net",
        "-p",
        "figcraft-engine",
        "-p",
        "figcraft-ui-egui",
        "-p",
        "figcraft-web",
    ])) && plugins()
}

/// The example plugins in `plugins/examples` (each folder's name is its package; its module is
/// `<name with _>.wasm`).
const EXAMPLE_PLUGINS: &[&str] = &["tidy-names", "swatch-grid", "counter", "export-names"];

/// The plugin workspace: fmt, clippy and build for wasm32, host tests, then each example's manifest
/// and module side by side in `<target>/plugins/dist/<example>/`.
fn plugins() -> bool {
    let root = root();
    let manifest = root.join("plugins").join("Cargo.toml");
    let target = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("target"));
    let ws = |args: &[&str]| {
        let mut c = cargo();
        // Keep the caller's target; Cargo separates host and wasm artifacts itself.
        c.args(args).arg("--manifest-path").arg(&manifest).env("CARGO_TARGET_DIR", &target);
        c
    };
    let wasm = ["--target", "wasm32-unknown-unknown", "--locked"];
    let ok = run(&mut ws(&["fmt", "--all", "--check"]))
        && run(ws(&["clippy", "--workspace", "--release"]).args(wasm).args(["--", "-D", "warnings"]))
        && run(&mut ws(&["test", "--workspace", "--locked"]))
        && run(ws(&["build", "--workspace", "--release"]).args(wasm));
    if !ok {
        return false;
    }
    for name in EXAMPLE_PLUGINS {
        let module = format!("{}.wasm", name.replace('-', "_"));
        let dist = target.join("plugins/dist").join(name);
        let copied = std::fs::create_dir_all(&dist)
            .and_then(|()| std::fs::copy(root.join("plugins/examples").join(name).join("figcraft-plugin.json"), dist.join("figcraft-plugin.json")))
            .and_then(|_| std::fs::copy(target.join("wasm32-unknown-unknown/release").join(&module), dist.join(&module)));
        match copied {
            Ok(bytes) => eprintln!("plugins: {} ({} KB)", dist.display(), bytes / 1024),
            Err(e) => {
                eprintln!("plugins: {name}: {e}");
                return false;
            }
        }
    }
    true
}

/// Engine/render timings (`figcraft-cli perf`) and UI frame timings (an ignored test driving the
/// whole UI headless), release builds, for each comma-separated document size in layers.
fn bench(sizes: &str) -> bool {
    run(cargo().args(["run", "--release", "-p", "figcraft-cli", "--", "perf", "--layers", sizes]))
        && run(cargo().env("FIGCRAFT_BENCH_LAYERS", sizes).args([
            "test",
            "--release",
            "-p",
            "figcraft-ui-egui",
            "--lib",
            "perf_ui_frames",
            "--",
            "--ignored",
            "--nocapture",
        ]))
}

/// Every asset file must be listed in ATTRIBUTION.md.
fn assets() -> bool {
    let root = root();
    let attribution = std::fs::read_to_string(root.join("ATTRIBUTION.md")).unwrap_or_default();
    let mut missing = Vec::new();
    for dir in ["assets", "docs/brand", "docs/images"] {
        walk(&root.join(dir), &mut |p| {
            let rel = p.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') || name.ends_with(".md") || name.ends_with(".txt") {
                return;
            }
            // Listed by path, or by any ancestor directory written as `dir/`.
            let mut listed = attribution.contains(&format!("`{rel}`"));
            let mut cur = rel.as_str();
            while let Some((parent, _)) = cur.rsplit_once('/') {
                if attribution.contains(&format!("`{parent}/`")) {
                    listed = true;
                    break;
                }
                cur = parent;
            }
            if !listed {
                missing.push(rel);
            }
        });
    }
    if !missing.is_empty() {
        eprintln!("assets without an ATTRIBUTION.md row:\n  {}", missing.join("\n  "));
        return false;
    }
    eprintln!("assets: ok");
    true
}

fn walk(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() && !ft.is_symlink() {
            walk(&p, f);
        } else if !ft.is_dir() {
            f(&p);
        }
    }
}

fn parity() -> bool {
    let extra = [
        "file.open",
        "file.import",
        "file.placeImage",
        "file.save",
        "file.saveAs",
        "file.export",
        "file.exportSvg",
        "edit.copyAsText",
        "edit.copyAsCss",
        "edit.copyAsSvg",
        "edit.copyAsPng",
        "edit.copyLink",
    ];
    let (done, total, missing) = figcraft_engine::catalog::parity(&extra);
    let pct = done as f64 * 100.0 / total.max(1) as f64;
    // The generation date: the date of the last commit (no clock dependency in xtask).
    let date = Command::new("git")
        .args(["log", "-1", "--format=%cs"])
        .current_dir(root())
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|d| d.trim().to_string())
        .unwrap_or_default();
    let mut md = format!(
        "# Parity checklist\n\n> **Generated:** {date} by `cargo xtask parity`. Do not edit by hand.\n\nFigCraft commands registered for the incumbent's main menu and toolbar ({} commands in the engine).\n\n**{done} / {total} menu items ({pct:.1}%)**\n\n",
        figcraft_engine::all_commands().len()
    );
    if missing.is_empty() {
        md.push_str("Every catalog item has a command.\n");
    } else {
        md.push_str("## Missing\n\n");
        for m in &missing {
            md.push_str(&format!("- `{m}`\n"));
        }
    }
    md.push_str("\nMenu coverage measures breadth only; see docs/target-app-parity.md for depth and fidelity.\n");
    let path = root().join("docs/parity-checklist.md");
    if let Err(e) = std::fs::write(&path, md) {
        eprintln!("{}: {e}", path.display());
        return false;
    }
    eprintln!("parity: {done}/{total} ({pct:.1}%) → docs/parity-checklist.md");
    true
}

fn version(sub: Option<&str>, value: Option<&str>) -> bool {
    let path = root().join("Cargo.toml");
    let Ok(text) = std::fs::read_to_string(&path) else { return false };
    let cur = text
        .lines()
        .skip_while(|l| !l.starts_with("[workspace.package]"))
        .find_map(|l| l.strip_prefix("version = \"").and_then(|r| r.strip_suffix('"')))
        .unwrap_or("0.0.0")
        .to_string();
    match (sub, value) {
        (Some("set"), Some(v)) => {
            let valid =
                v.split(['-', '+']).next().is_some_and(|core| core.split('.').count() == 3 && core.split('.').all(|p| p.parse::<u64>().is_ok()));
            if !valid {
                eprintln!("version must look like 1.2.3 or 1.2.3-rc.1");
                return false;
            }
            let new = text.replacen(&format!("version = \"{cur}\""), &format!("version = \"{v}\""), 1);
            if std::fs::write(&path, new).is_err() {
                return false;
            }
            eprintln!("version {cur} → {v}");
            run(cargo().args(["update", "--workspace"]))
        }
        _ => {
            println!("{cur}");
            true
        }
    }
}
