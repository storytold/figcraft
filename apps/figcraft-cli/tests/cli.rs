//! The binary end to end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
        assert!(err.contains("USAGE"), "{args:?}");
        assert!(!err.contains("panicked"));
    }
    let out = bin().args(["info", "/no/such/file.figcraft"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
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
