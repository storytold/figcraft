//! `figcraft-cli export` with PDF print options: bleed, marks, CMYK; bad lengths are usage errors.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_figcraft-cli"))
}

#[test]
fn pdf_export_with_bleed_marks_and_cmyk() {
    let d = std::env::temp_dir().join(format!("figcraft-cli-print-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let script = d.join("s.jsonl");
    std::fs::write(&script, r##"{"command":"frame.create","params":{"x":0,"y":0,"width":100,"height":50,"name":"A","props":{"fill":"#ff0000"}}}"##)
        .unwrap();
    let file = d.join("doc.figcraft");
    assert!(bin().arg("run").arg(&script).arg("--out").arg(&file).status().unwrap().success());

    let pdf = d.join("print.pdf");
    let out = bin().arg("export").arg(&file).arg("-o").arg(&pdf).args(["--bleed", "0.125in", "--crop-marks", "--cmyk"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&std::fs::read(&pdf).unwrap()).into_owned();
    // 9 pt of bleed, then 3 + 18 + 3 pt for the marks.
    assert!(s.contains("/MediaBox[0 0 166 116]") && s.contains("/BleedBox[24 24 142 92]") && s.contains("/TrimBox[33 33 133 83]"), "{s}");
    // Without options: the frame's size, no boxes.
    let plain = d.join("plain.pdf");
    assert!(bin().arg("export").arg(&file).arg("-o").arg(&plain).status().unwrap().success());
    let s = String::from_utf8_lossy(&std::fs::read(&plain).unwrap()).into_owned();
    assert!(s.contains("/MediaBox[0 0 100 50]") && !s.contains("/TrimBox"), "{s}");
    // A directory of PDFs gets them too.
    let dir = d.join("out");
    assert!(bin().arg("export").arg(&file).arg("-o").arg(&dir).args(["--format", "pdf", "--bleed", "3mm"]).status().unwrap().success());
    assert!(String::from_utf8_lossy(&std::fs::read(dir.join("A.pdf")).unwrap()).contains("/TrimBox["));
    // A bleed that isn't a length is a usage error.
    let out = bin().arg("export").arg(&file).arg("-o").arg(&pdf).args(["--bleed", "lots"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);
}
