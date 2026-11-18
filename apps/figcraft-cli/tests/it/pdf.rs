//! `figcraft-cli export --format pdf` and `convert … .pdf` end to end.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_figcraft-cli"))
}

fn pages(pdf: &[u8]) -> usize {
    let s = String::from_utf8_lossy(pdf);
    s.matches("/Type/Page").count() - s.matches("/Type/Pages").count()
}

#[test]
fn pdf_export_one_page_per_frame() {
    let d = std::env::temp_dir().join(format!("figcraft-cli-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    // Two frames and a loose rectangle (not a frame: left out of the frames PDF).
    let script = d.join("s.jsonl");
    std::fs::write(
        &script,
        [
            r##"{"command":"frame.create","params":{"x":0,"y":0,"width":100,"height":50,"name":"A","props":{"fill":"#ff0000"}}}"##,
            r##"{"command":"frame.create","params":{"x":200,"y":0,"width":30,"height":40,"name":"B"}}"##,
            r##"{"command":"shape.rectangle","params":{"x":0,"y":300,"width":500,"height":20}}"##,
        ]
        .join("\n"),
    )
    .unwrap();
    let file = d.join("doc.figcraft");
    let out = bin().arg("run").arg(&script).arg("--out").arg(&file).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    // The whole page: the frames, one PDF page each, sized in points.
    let pdf = d.join("page.pdf");
    let out = bin().arg("export").arg(&file).arg("-o").arg(&pdf).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    assert_eq!(pages(&bytes), 2);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("/MediaBox[0 0 100 50]") && s.contains("/MediaBox[0 0 30 40]"), "{s}");

    // `--format pdf` into a directory: a file per frame; `--node`: that layer alone.
    let dir = d.join("out");
    assert!(bin().arg("export").arg(&file).arg("-o").arg(&dir).args(["--format", "pdf"]).status().unwrap().success());
    assert_eq!(pages(&std::fs::read(dir.join("A.pdf")).unwrap()), 1);
    assert!(dir.join("B.pdf").exists());
    let one = d.join("b.pdf");
    assert!(bin().arg("export").arg(&file).arg("-o").arg(&one).args(["--node", "B"]).status().unwrap().success());
    assert!(String::from_utf8_lossy(&std::fs::read(&one).unwrap()).contains("/MediaBox[0 0 30 40]"));

    // `convert` writes the same frames PDF.
    let conv = d.join("conv.pdf");
    assert!(bin().arg("convert").arg(&file).arg(&conv).status().unwrap().success());
    assert_eq!(pages(&std::fs::read(&conv).unwrap()), 2);
    let _ = std::fs::remove_dir_all(&d);
}
