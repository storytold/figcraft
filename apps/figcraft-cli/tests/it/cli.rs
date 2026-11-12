//! The binary end to end.

use std::io::Write;
use std::process::{Command, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_figcraft-cli"))
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-cli-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Conversion and image commands must report the Penpot loader's notes too.
#[test]
fn penpot_import_notes_reach_convert_export_and_render() {
    use figcraft_doc::proto::{Action, Easing, Interaction, Transition, Trigger};
    use figcraft_doc::{Document, NodeKind};
    let d = tmp("penpot-notes");
    let mut doc = Document::new("Prototype");
    let page = doc.first_page().unwrap();
    let home = doc.create(page, NodeKind::Frame, "Home", 0.0, 0.0, 40.0, 40.0).unwrap();
    let dest = doc.create(page, NodeKind::Frame, "Next", 60.0, 0.0, 40.0, 40.0).unwrap();
    doc.node_mut(home).unwrap().interactions.push(Interaction {
        trigger: Trigger::OnClick,
        actions: vec![Action::Navigate { dest, preserve_scroll: false }],
        transition: Transition::Instant,
        duration_ms: 0,
        easing: Easing::Linear,
        delay_ms: 0,
    });
    let input = d.join("source.penpot");
    std::fs::write(&input, figcraft_format::penpot::export(&doc).unwrap().0).unwrap();
    for (command, name) in [("convert", "out.figcraft"), ("export", "out.svg"), ("render", "out.png")] {
        let mut cmd = bin();
        cmd.arg(command).arg(&input);
        if command != "convert" {
            cmd.arg("-o");
        }
        let out = cmd.arg(d.join(name)).output().unwrap();
        let notes = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{command}: {notes}");
        assert!(notes.contains("note: Imported 1 Penpot prototype connection"), "{command}: {notes}");
    }
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn option_terminator_allows_dash_prefixed_filenames() {
    let dir = tmp("option-terminator");
    let doc = figcraft_doc::Document::new("Dash-prefixed file");
    let bytes = figcraft_format::save_bytes(&doc, None).unwrap();
    for name in ["--drawing.figcraft", "-h", "--json", "--no-system-fonts", "--"] {
        std::fs::write(dir.join(name), &bytes).unwrap();
        let output = bin().current_dir(&dir).args(["info", "--json", "--no-system-fonts", "--", name]).output().unwrap();
        assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
        let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(info["file"], name);
        assert_eq!(info["name"], "Dash-prefixed file");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn global_font_flag_respects_option_values() {
    // A value starting with `-` is written with `=` (RFC-0012).
    for filter in ["--filter=--", "--filter=--no-system-fonts"] {
        let output = bin().args(["commands", filter, "--no-system-fonts", "--json"]).output().unwrap();
        assert!(output.status.success(), "{filter}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap().is_array());
    }
}

#[test]
fn global_font_flag_can_precede_the_subcommand() {
    let output = bin().args(["--no-system-fonts", "commands", "--json"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout).unwrap().is_empty());
}

#[test]
fn bad_arguments_exit_2_without_panicking() {
    for args in [
        &[][..],
        &["bogus"],
        &["export"],
        &["export", "x.figcraft"],
        &["info", "--nope"],
        &["render", "a", "-o"],
        &["perf", "--shapes", "lots"],
        &["mcp", "--attach", "notaport"],
        // An attached app confines itself: roots go to `figcraft --control`.
        &["mcp", "--attach", "7979", "--automation-read-root", "."],
        &["mcp", "--automation-write-root"],
        &["convert", "a"],
    ] {
        let out = bin().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("Usage: figcraft-cli") || err.contains("try '--help'"), "{args:?}: {err}");
        assert!(!err.contains("panicked"));
    }
    let out = bin().args(["info", "/no/such/file.figcraft"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}

/// `--help` / `-h` after a subcommand prints that subcommand's help on stdout and exits 0, as it
/// does first (#1155), whatever else is on the line; nothing runs.
#[test]
fn help_after_a_subcommand_prints_the_usage() {
    for args in [
        &["--help"][..],
        &["-h"],
        &["help"],
        &["mcp", "--help"],
        &["mcp", "-h"],
        &["info", "--help"],
        &["export", "--help"],
        &["export", "x.figcraft", "-o", "out", "--help"],
        &["convert", "-h"],
        &["render", "--help"],
        &["run", "--help"],
        &["commands", "--help"],
        &["plugin", "publish", "--help"],
        &["perf", "--help"],
        &["mcp", "--attach", "notaport", "--help"],
        &["mcp", "--bogus", "--help"],
    ] {
        let out = bin().args(args).stdin(Stdio::null()).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("Usage: figcraft-cli"), "{args:?}: {text}");
        assert!(out.stderr.is_empty(), "{args:?}: nothing on stderr, got {}", String::from_utf8_lossy(&out.stderr));
    }
    // A value that happens to be `--help` is still a value, not a request for help.
    let out = bin().args(["commands", "--filter=--help"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Usage:"), "--filter took `--help` as its text");
}

#[test]
fn run_export_info_convert() {
    let d = tmp("run");
    let script = d.join("s.jsonl");
    std::fs::write(&script, "{\"command\":\"frame.create\",\"params\":{\"x\":0,\"y\":0,\"width\":100,\"height\":50,\"name\":\"A\",\"props\":{\"fill\":\"#ff0000\"}}}\n{\"command\":\"frame.create\",\"params\":{\"x\":200,\"y\":0,\"width\":30,\"height\":30,\"name\":\"B\"}}\n").unwrap();
    let file = d.join("doc.figcraft");
    let out = bin().arg("run").arg(&script).arg("--out").arg(&file).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|l| l["ok"] == true));

    let info = bin().args(["info", "--json"]).arg(&file).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    assert_eq!(v["pages"][0]["frames"].as_array().unwrap().len(), 2);

    let dir = d.join("png");
    assert!(bin().arg("export").arg(&file).arg("-o").arg(&dir).args(["--scale", "2"]).status().unwrap().success());
    let a = std::fs::read(dir.join("A.png")).unwrap();
    assert_eq!(&a[16..24], &[0, 0, 0, 200, 0, 0, 0, 100]);
    assert!(dir.join("B.png").exists());

    let svg = d.join("b.svg");
    assert!(bin().arg("export").arg(&file).arg("-o").arg(&svg).args(["--node", "B"]).status().unwrap().success());
    assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));

    let json = d.join("doc.json");
    assert!(bin().arg("convert").arg(&file).arg(&json).status().unwrap().success());
    let png = d.join("doc.png");
    assert!(bin().arg("convert").arg(&json).arg(&png).status().unwrap().success());
    assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));

    // A failing step: nonzero exit, error line printed.
    let bad = d.join("bad.json");
    std::fs::write(&bad, r#"[{"command":"select.all"},{"command":"nope"}]"#).unwrap();
    let out = bin().arg("run").arg(&bad).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"ok\":false"));
    std::fs::remove_dir_all(&d).ok();
}

/// `export` into a folder names the files as the app's export does (#1154): brackets, `+`, `&`,
/// `%`, `#` and commas stay, what file systems refuse becomes `_`.
#[test]
fn export_file_names_keep_what_the_app_keeps() {
    let d = tmp("names");
    let script = d.join("s.json");
    let names = ["Homepage - desktop (live + verbeteringen)", "Q&A, 50% #1 [v2]", "a:b*c?d|e"];
    let steps: Vec<serde_json::Value> = names
        .iter()
        .enumerate()
        .map(|(i, n)| serde_json::json!({"command": "frame.create", "params": {"x": i * 200, "y": 0, "width": 20, "height": 20, "name": n}}))
        .collect();
    std::fs::write(&script, serde_json::to_string(&steps).unwrap()).unwrap();
    let file = d.join("doc.figcraft");
    assert!(bin().arg("run").arg(&script).arg("--out").arg(&file).output().unwrap().status.success());
    let dir = d.join("out");
    let out = bin().arg("export").arg(&file).arg("-o").arg(&dir).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    for want in ["Homepage - desktop (live + verbeteringen).png", "Q&A, 50% #1 [v2].png", "a_b_c_d_e.png"] {
        assert!(dir.join(want).is_file(), "{want} in {:?}", std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>());
    }
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn run_takes_the_documented_gradient_shorthand() {
    // #252: only MCP used to accept it; `run` said "`fills` must be paints or colours".
    let d = tmp("gradient");
    let script = d.join("s.json");
    std::fs::write(
        &script,
        r##"[{"command":"shape.rectangle","params":{"x":0,"y":0,"width":200,"height":100}},
            {"command":"node.set","params":{"ids":[2],"fills":[{"type":"linearGradient","stops":["#ff0000","#0000ff"],"from":[0,0.5],"to":[1,0.5]}]}}]"##,
    )
    .unwrap();
    let file = d.join("g.figcraft");
    let out = bin().arg("run").arg(&script).arg("--out").arg(&file).output().unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let json = d.join("g.json");
    assert!(bin().arg("convert").arg(&file).arg(&json).status().unwrap().success());
    let doc = std::fs::read_to_string(&json).unwrap();
    assert!(doc.contains("\"kind\":\"linear\"") || doc.contains("\"kind\": \"linear\""), "{doc}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mcp_over_stdio() {
    let mut child = bin().arg("mcp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#).unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
        writeln!(stdin, "garbage").unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"create_node","arguments":{{"type":"ellipse"}}}}}}"#)
            .unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(lines[1]["error"]["code"], -32700);
    assert_eq!(lines[2]["result"]["isError"], false);
}

#[test]
fn mcp_over_stdio_confined_to_automation_roots() {
    let d = tmp("roots");
    for sub in ["in", "out", "outside"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    let p = |rel: &str| d.join(rel).to_string_lossy().into_owned();
    // A root that isn't a folder stops the server before it serves.
    let out = bin().args(["mcp", "--automation-read-root", &p("missing")]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--automation-read-root"));
    let mut child = bin()
        .args(["mcp", "--automation-read-root", &p("in"), "--automation-write-root", &p("out")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        let call = |id: u32, tool: &str, args: serde_json::Value| {
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": args}}).to_string()
        };
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#).unwrap();
        writeln!(stdin, "{}", call(2, "create_node", serde_json::json!({"type": "ellipse"}))).unwrap();
        writeln!(stdin, "{}", call(3, "save_file", serde_json::json!({"path": p("outside/a.figcraft")}))).unwrap();
        writeln!(stdin, "{}", call(4, "save_file", serde_json::json!({"path": p("out/a.figcraft")}))).unwrap();
        writeln!(stdin, "{}", call(5, "open_file", serde_json::json!({"path": p("outside/a.figcraft")}))).unwrap();
        writeln!(stdin, "{}", call(6, "open_file", serde_json::json!({"path": p("out/a.figcraft")}))).unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 6);
    assert!(lines[0]["result"]["instructions"].as_str().unwrap().contains("only these folders may be read"));
    let error = |i: usize| lines[i]["result"]["isError"] == true;
    assert!(!error(1) && error(2) && !error(3) && error(4) && !error(5), "{lines:?}");
    assert!(lines[2]["result"]["content"][0]["text"].as_str().unwrap().contains("automation path rejected"));
    assert!(d.join("out/a.figcraft").is_file() && !d.join("outside/a.figcraft").exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn commands_json_lists_everything() {
    let out = bin().args(["commands", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.as_array().unwrap().len() > 200);
}

/// The automation roots confine the file subcommands too, not just `mcp` (#833): a read outside a
/// root, and a write outside a write root, are refused and leave no file behind. Without the flags
/// the same commands behave as they always did.
#[test]
fn file_subcommands_are_confined_to_automation_roots() {
    let d = tmp("file-roots");
    let _ = std::fs::remove_dir_all(&d);
    for sub in ["in", "out"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    let p = |rel: &str| d.join(rel).to_string_lossy().into_owned();
    let out_of = |o: &std::process::Output| String::from_utf8_lossy(&o.stderr).into_owned();

    // A document to work on, made unconfined.
    std::fs::write(d.join("in/make.json"), br#"[{"command":"shape.rectangle","params":{"x":0,"y":0,"width":40,"height":20}}]"#).unwrap();
    let made = bin().args(["run", &p("in/make.json"), "--out", &p("in/doc.figcraft")]).output().unwrap();
    assert!(made.status.success(), "{}", out_of(&made));

    // Inside the roots: renders as usual.
    let ok = bin()
        .args(["render", &p("in/doc.figcraft"), "-o", &p("out/ok.png"), "--automation-read-root", &p("in"), "--automation-write-root", &p("out")])
        .output()
        .unwrap();
    assert!(ok.status.success(), "{}", out_of(&ok));
    assert!(d.join("out/ok.png").is_file());

    // A write outside the write root is refused, and nothing is written.
    let escaped = d.join("escaped.png");
    let refused = bin()
        .args([
            "render",
            &p("in/doc.figcraft"),
            "-o",
            &escaped.to_string_lossy(),
            "--automation-read-root",
            &p("in"),
            "--automation-write-root",
            &p("out"),
        ])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(out_of(&refused).contains("may be written"), "{}", out_of(&refused));
    assert!(!escaped.exists(), "the refused render wrote a file anyway");

    // No write root at all: writing is refused outright, reading still works.
    let no_write = bin().args(["render", &p("in/doc.figcraft"), "-o", &p("out/no.png"), "--automation-read-root", &p("in")]).output().unwrap();
    assert!(!no_write.status.success());
    assert!(out_of(&no_write).contains("no folder may be written"), "{}", out_of(&no_write));

    // A read outside every root is refused: `info` on the document with only `out` readable.
    let read_refused = bin().args(["info", &p("in/doc.figcraft"), "--automation-read-root", &p("out")]).output().unwrap();
    assert!(!read_refused.status.success());
    assert!(out_of(&read_refused).contains("may be read"), "{}", out_of(&read_refused));

    // `convert` and `run` take them as well, and a root that is not a folder is refused.
    let missing = bin().args(["convert", &p("in/doc.figcraft"), &p("out/c.svg"), "--automation-read-root", &p("missing")]).output().unwrap();
    assert!(!missing.status.success());
    assert!(out_of(&missing).contains("automation-read-root"), "{}", out_of(&missing));

    // Unconfined, the same render still writes wherever it is told.
    let plain = bin().args(["render", &p("in/doc.figcraft"), "-o", &escaped.to_string_lossy()]).output().unwrap();
    assert!(plain.status.success(), "{}", out_of(&plain));
    assert!(escaped.is_file());
    let _ = std::fs::remove_dir_all(&d);
}

/// An output path that is a symbolic link is written through, to its target, as `std::fs::write`
/// always did. Routing the CLI's writes through `file_access` for #833 would otherwise have
/// replaced the link with a regular file, because that write stages the bytes and renames them
/// into place.
#[cfg(unix)]
#[test]
fn an_unconfined_write_follows_a_symbolic_link() {
    let d = tmp("symlink-out");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("real")).unwrap();
    let p = |rel: &str| d.join(rel).to_string_lossy().into_owned();
    std::fs::write(d.join("in.json"), br#"[{"command":"shape.rectangle","params":{"x":0,"y":0,"width":40,"height":20}}]"#).unwrap();
    let made = bin().args(["run", &p("in.json"), "--out", &p("doc.figcraft")]).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));

    let target = d.join("real/target.png");
    std::fs::write(&target, b"placeholder").unwrap();
    let link = d.join("link.png");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let out = bin().args(["render", &p("doc.figcraft"), "-o", &link.to_string_lossy()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link is still a link");
    let written = std::fs::read(&target).unwrap();
    assert!(written.starts_with(b"\x89PNG"), "the target holds the render, not the placeholder");
    let _ = std::fs::remove_dir_all(&d);
}
