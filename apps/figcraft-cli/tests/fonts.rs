//! Headless runs lay text out with the installed fonts, like the desktop app, unless
//! `--no-system-fonts` or `FIGCRAFT_NO_SYSTEM_FONTS=1` opts out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

use serde_json::{Value, json};

/// The result of one `figcraft-cli run` step (script file `name`); `opt_out` sets the environment
/// variable.
fn run_one(name: &str, step: Value, args: &[&str], opt_out: bool) -> Value {
    let d = std::env::temp_dir().join(format!("figcraft-cli-fonts-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let script = d.join(format!("{name}.json"));
    std::fs::write(&script, json!([step]).to_string()).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_figcraft-cli"));
    cmd.arg("run").arg(&script).args(args);
    if opt_out {
        cmd.env("FIGCRAFT_NO_SYSTEM_FONTS", "1");
    } else {
        cmd.env_remove("FIGCRAFT_NO_SYSTEM_FONTS");
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let line: Value = serde_json::from_slice(&out.stdout).unwrap();
    line["result"].clone()
}

fn families(args: &[&str], opt_out: bool) -> Vec<String> {
    serde_json::from_value(run_one("families", json!({"command": "text.fonts"}), args, opt_out)).unwrap()
}

#[test]
fn headless_runs_resolve_installed_families_unless_opted_out() {
    let built_in = families(&["--no-system-fonts"], false);
    assert_eq!(families(&[], true), built_in, "FIGCRAFT_NO_SYSTEM_FONTS=1 opts out like the flag");
    // Any family this machine adds (scanned here, in the test); none (CI): nothing to check.
    figcraft_text::scan_system_fonts();
    let Some(installed) = figcraft_text::family_names().into_iter().find(|f| !built_in.contains(f)) else {
        eprintln!("skipped: no installed fonts beyond the built-in ones");
        return;
    };
    assert!(families(&[], false).contains(&installed), "headless runs list {installed}");
    let face = json!({"command": "text.fonts", "params": {"family": installed, "axes": true}});
    assert_eq!(run_one("face", face.clone(), &[], false)["family"], json!(installed), "text uses the installed family");
    assert_ne!(run_one("face", face, &["--no-system-fonts"], false)["family"], json!(installed), "opted out, it falls back");
}

#[test]
fn info_lists_the_fonts_a_file_uses_that_are_missing() {
    let d = std::env::temp_dir().join(format!("figcraft-cli-missing-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let (script, file) = (d.join("make.json"), d.join("doc.figcraft"));
    let text =
        |family: &str| json!({"command": "text.create", "params": {"text": "Hamburgefonstiv", "x": 0, "y": 0, "props": {"fontFamily": family}}});
    std::fs::write(&script, json!([text("FigCraft Missing CLI"), text("FigCraft Missing CLI")]).to_string()).unwrap();
    let cli = || Command::new(env!("CARGO_BIN_EXE_figcraft-cli"));
    let made = cli().arg("run").arg(&script).arg("--out").arg(&file).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));

    let out = cli().arg("info").arg(&file).arg("--json").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["missingFonts"], json!([{"family": "FigCraft Missing CLI", "layers": 2}]));

    let out = cli().arg("info").arg(&file).output().unwrap();
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.contains("missing font \"FigCraft Missing CLI\": 2 text layers"), "{shown}");
}
