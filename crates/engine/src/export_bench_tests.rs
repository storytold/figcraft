//! Export timings over the example files: every top-level frame of each `examples/*.figcraft`
//! exported as PNG, JPG and WebP at 2x through [`export_file`], as Export and `figcraft-cli export`
//! do. Run in release:
//! `cargo test --release -p figcraft-engine --lib export_timings -- --ignored --nocapture`.
//!
//! `FIGCRAFT_EXPORT_BENCH_RUNS` sets the timed runs (default 7; one warm-up run first),
//! `FIGCRAFT_EXPORT_BENCH_FORMATS` the formats (default `png,jpg,webp`) and
//! `FIGCRAFT_EXPORT_BENCH_OUT` a folder to write the files of the last run to (to compare outputs).

use std::time::Instant;

use figcraft_doc::{Document, ExportFormat, ExportSetting, NodeId};

use crate::cmd::file::{export_file, load_path};

/// The example documents and their top-level frames.
fn examples() -> Vec<(String, Document, Vec<NodeId>)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<_> =
        std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "figcraft")).collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let (loaded, _) = load_path(&p.to_string_lossy(), None).unwrap();
            let doc = loaded.doc;
            let frames: Vec<NodeId> = doc.pages.iter().flat_map(|pg| doc.top_frames(*pg)).collect();
            (p.file_stem().unwrap().to_string_lossy().into_owned(), doc, frames)
        })
        .collect()
}

fn format(name: &str) -> ExportFormat {
    match name {
        "png" => ExportFormat::Png,
        "jpg" => ExportFormat::Jpg,
        "webp" => ExportFormat::Webp,
        "svg" => ExportFormat::Svg,
        "pdf" => ExportFormat::Pdf,
        other => panic!("unknown format {other}"),
    }
}

/// Export every frame in `format` at 2x: (milliseconds, files).
fn run(docs: &[(String, Document, Vec<NodeId>)], format: ExportFormat) -> (f64, Vec<(String, Vec<u8>)>) {
    let setting = ExportSetting { format, ..Default::default() };
    let mut files = Vec::new();
    let t = Instant::now();
    for (name, doc, frames) in docs {
        for f in frames {
            let (bytes, _) = export_file(doc, &[*f], &setting, 2.0).unwrap();
            files.push((format!("{name}-{}.{}", f.0, format.ext()), bytes));
        }
    }
    (t.elapsed().as_secs_f64() * 1000.0, files)
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn export_timings() {
    let runs: usize = std::env::var("FIGCRAFT_EXPORT_BENCH_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(7).max(1);
    let formats = std::env::var("FIGCRAFT_EXPORT_BENCH_FORMATS").unwrap_or_else(|_| "png,jpg,webp".into());
    let out = std::env::var("FIGCRAFT_EXPORT_BENCH_OUT").ok();
    let docs = examples();
    let frames: usize = docs.iter().map(|d| d.2.len()).sum();
    println!("{} documents, {frames} frames, {runs} runs", docs.len());
    let mut total = vec![0.0; runs];
    for name in formats.split(',').map(str::trim).filter(|f| !f.is_empty()) {
        let fmt = format(name);
        let _ = run(&docs, fmt);
        let mut times = Vec::with_capacity(runs);
        let mut last = Vec::new();
        for t in &mut total {
            let (ms, files) = run(&docs, fmt);
            *t += ms;
            times.push(ms);
            last = files;
        }
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            for (file, bytes) in &last {
                std::fs::write(std::path::Path::new(dir).join(file), bytes).unwrap();
            }
        }
        report(name, &mut times);
    }
    report("total", &mut total);
}

/// Print the median and minimum of `times` (and every run).
fn report(name: &str, times: &mut [f64]) {
    let runs: Vec<String> = times.iter().map(|t| format!("{t:.1}")).collect();
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    println!("{name:>6}: median {median:8.1} ms  min {:8.1} ms  runs [{}]", times[0], runs.join(", "));
}
