//! `figcraft-cli export … -o out.xml` (Android Vector Drawable) end to end.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_figcraft-cli"))
}

#[test]
fn xml_export_writes_a_drawable_and_prints_its_notes() {
    let d = std::env::temp_dir().join(format!("figcraft-cli-vd-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let script = d.join("s.jsonl");
    std::fs::write(
        &script,
        r##"{"command":"shape.rectangle","params":{"x":0,"y":0,"width":24,"height":24,"name":"Dot","props":{"fill":"#ff0000","effects":[{"type":"dropShadow"}]}}}"##,
    )
    .unwrap();
    let file = d.join("doc.figcraft");
    let out = bin().arg("run").arg(&script).arg("--out").arg(&file).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let xml = d.join("icon.xml");
    let out = bin().arg("export").arg(&file).arg("-o").arg(&xml).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = std::fs::read_to_string(&xml).unwrap();
    assert!(text.contains("<vector") && text.contains("android:fillColor=\"#FF0000\""), "{text}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("note: drop shadows were left out (Dot)"));

    // --format xml into a folder; --vd-compat ignore prints nothing.
    let dir = d.join("out");
    let out = bin().arg("export").arg(&file).arg("-o").arg(&dir).args(["--format", "xml", "--vd-compat", "ignore"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("Dot.xml").exists());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("note:"));

    let out = bin().arg("export").arg(&file).arg("-o").arg(&xml).args(["--vd-compat", "sloppy"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--vd-compat"));
    let _ = std::fs::remove_dir_all(&d);
}
