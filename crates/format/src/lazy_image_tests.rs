//! On-demand lazy image loading, bounded encoded-byte LRU eviction, content-hash integrity, and
//! streaming atomic pass-through save (#1313).

#![cfg(not(target_arch = "wasm32"))]

use std::io::Write;
use std::sync::Arc;

use figcraft_doc::{Document, ImageData, NodeKind};

use super::*;

fn png(r: u8, g: u8, b: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 3, image::Rgba([r, g, b, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-lazy-images-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn lazy_open_indexes_all_images_without_loading_bytes_into_memory() {
    let dir = temp_dir("open");
    let path = dir.join("gallery.figcraft");
    let mut doc = Document::new("Gallery");
    let mut expected = Vec::new();
    for i in 0..8u8 {
        let bytes = png(i * 30, 10, 200 - i * 20);
        let (hash, w, h) = add_image(&mut doc, bytes.clone()).unwrap();
        expected.push((hash, w, h, bytes));
    }
    save_file(&doc, &path, None).unwrap();

    let loaded = load_file_with_versions(&path).unwrap();
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert_eq!(loaded.images_past_budget, 0);
    assert_eq!(loaded.doc.images.len(), expected.len());
    for (hash, w, h, _) in &expected {
        let img = loaded.doc.images.get(hash).unwrap();
        assert!(img.is_lazy(), "image {hash} should be lazy on open");
        assert_eq!((img.width, img.height), (*w, *h));
        assert!(img.as_in_memory().is_none());
    }
    // Resolving on demand returns exact original bytes.
    for (hash, _, _, raw) in &expected {
        let img = loaded.doc.images.get(hash).unwrap();
        assert_eq!(img.bytes().as_deref().map(Vec::as_slice), Some(raw.as_slice()));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn encoded_lru_cache_stays_strictly_within_byte_budget() {
    let dir = temp_dir("lru");
    let path = dir.join("bounded.figcraft");
    let mut doc = Document::new("Bounded");
    let mut hashes = Vec::new();
    let mut single_len = 0u64;
    for i in 0..10u8 {
        let bytes = png(i * 25, i * 13, 255 - i * 20);
        single_len = single_len.max(bytes.len() as u64);
        let (hash, _, _) = add_image(&mut doc, bytes.clone()).unwrap();
        hashes.push((hash, bytes));
    }
    save_file(&doc, &path, None).unwrap();

    // Budget fits at most 2 encoded images at once.
    let budget = single_len * 2;
    let file = std::fs::File::open(&path).unwrap();
    let len = file.metadata().unwrap().len();
    let loaded = load_file_from(&path, file, len, budget).unwrap();

    // Every image resolves identically even though total images = 5x cache budget.
    for (hash, raw) in &hashes {
        let img = loaded.doc.images.get(hash).unwrap();
        assert_eq!(img.bytes().as_deref().map(Vec::as_slice), Some(raw.as_slice()));
    }
    // Re-reading evicted earliest images still succeeds on demand.
    assert_eq!(loaded.doc.images.get(&hashes[0].0).unwrap().bytes().as_deref().map(Vec::as_slice), Some(hashes[0].1.as_slice()));

    // Direct EncodedLru byte-accounting & eviction invariant check.
    let mut lru = image_store::EncodedLru::new(budget);
    for (hash, raw) in &hashes {
        lru.insert(hash.clone(), Arc::new(raw.clone()));
        assert!(lru.bytes() <= budget, "lru bytes {} exceeded budget {budget}", lru.bytes());
        assert!(lru.len() <= 2);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lazy_load_rejects_corrupted_content_hash_on_disk() {
    let dir = temp_dir("corrupt");
    let path = dir.join("tampered.figcraft");
    let mut doc = Document::new("Tampered");
    let raw = png(12, 34, 56);
    let (hash, _, _) = add_image(&mut doc, raw.clone()).unwrap();
    assert!(ImageData::claims_content_hash(&hash));
    save_file(&doc, &path, None).unwrap();

    // Rewrite the ZIP replacing `images/<hash>` with mismatched bytes under the same hash key.
    let orig = std::fs::read(&path).unwrap();
    let mut reader = zip::ZipArchive::new(std::io::Cursor::new(&orig)).unwrap();
    let names: Vec<String> = reader.file_names().map(str::to_string).collect();
    let mut out = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for name in names {
            writer.start_file(&name, stored).unwrap();
            if name == format!("images/{hash}") {
                let mut bad = raw.clone();
                bad[0] ^= 0xff;
                writer.write_all(&bad).unwrap();
            } else {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut reader.by_name(&name).unwrap(), &mut data).unwrap();
                writer.write_all(&data).unwrap();
            }
        }
        writer.finish().unwrap();
    }
    std::fs::write(&path, out).unwrap();

    let loaded = load_file_with_versions(&path).unwrap();
    let img = loaded.doc.images.get(&hash).unwrap();
    assert!(img.bytes().is_none(), "hash mismatch must be rejected instead of returning corrupted bytes");
    // Streaming save must also refuse to propagate corrupted bytes.
    let copy = dir.join("copy.figcraft");
    assert!(save_file(&loaded.doc, &copy, None).is_err());
    assert!(!copy.exists(), "failed streaming save must not leave destination behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn streaming_save_in_place_and_save_as_preserve_all_lazy_and_new_images() {
    let dir = temp_dir("save-inplace");
    let path = dir.join("live.figcraft");
    let mut doc = Document::new("Live");
    let page = doc.first_page().unwrap();
    doc.create(page, NodeKind::Rectangle, "Box", 0.0, 0.0, 50.0, 50.0).unwrap();
    let img_a_bytes = png(10, 20, 30);
    let img_b_bytes = png(40, 50, 60);
    let (hash_a, _, _) = add_image(&mut doc, img_a_bytes.clone()).unwrap();
    let (hash_b, _, _) = add_image(&mut doc, img_b_bytes.clone()).unwrap();
    save_file(&doc, &path, None).unwrap();

    // Open lazily, add a third in-memory image, and save IN PLACE over `path`.
    let mut loaded = load_file_with_versions(&path).unwrap();
    assert!(loaded.doc.images[&hash_a].is_lazy());
    assert!(loaded.doc.images[&hash_b].is_lazy());
    let img_c_bytes = png(70, 80, 90);
    let (hash_c, _, _) = add_image(&mut loaded.doc, img_c_bytes.clone()).unwrap();

    save_file(&loaded.doc, &path, None).unwrap();

    // Existing lazy handles on `loaded.doc` must still resolve after the in-place swap.
    assert_eq!(loaded.doc.images[&hash_a].bytes().as_deref().map(Vec::as_slice), Some(img_a_bytes.as_slice()));
    assert_eq!(loaded.doc.images[&hash_b].bytes().as_deref().map(Vec::as_slice), Some(img_b_bytes.as_slice()));

    // Reopen from disk and verify all three images round-tripped identically.
    let reopened = load_file_with_versions(&path).unwrap();
    assert_eq!(reopened.doc.images.len(), 3);
    assert_eq!(reopened.doc.images[&hash_a].bytes().as_deref().map(Vec::as_slice), Some(img_a_bytes.as_slice()));
    assert_eq!(reopened.doc.images[&hash_b].bytes().as_deref().map(Vec::as_slice), Some(img_b_bytes.as_slice()));
    assert_eq!(reopened.doc.images[&hash_c].bytes().as_deref().map(Vec::as_slice), Some(img_c_bytes.as_slice()));

    // Save As to a separate path, remove the original, and verify the copy is self-contained.
    let copy = dir.join("copy.figcraft");
    save_file(&reopened.doc, &copy, None).unwrap();
    std::fs::remove_file(&path).unwrap();
    let from_copy = load_file_with_versions(&copy).unwrap();
    assert_eq!(from_copy.doc.images.len(), 3);
    assert_eq!(from_copy.doc.images[&hash_a].bytes().as_deref().map(Vec::as_slice), Some(img_a_bytes.as_slice()));
    assert_eq!(from_copy.doc.images[&hash_b].bytes().as_deref().map(Vec::as_slice), Some(img_b_bytes.as_slice()));
    assert_eq!(from_copy.doc.images[&hash_c].bytes().as_deref().map(Vec::as_slice), Some(img_c_bytes.as_slice()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn streaming_save_in_place_reuses_lazy_store_without_polluting_lru_cache() {
    let dir = temp_dir("no-lru-bloat");
    let path = dir.join("stream.figcraft");
    let mut doc = Document::new("Stream");
    let mut hashes = Vec::new();
    for i in 0..6u8 {
        let bytes = png(i * 40, 100, 150);
        let (h, _, _) = add_image(&mut doc, bytes.clone()).unwrap();
        hashes.push((h, bytes));
    }
    save_file(&doc, &path, None).unwrap();

    // Build a store directly so we can inspect its LRU cache occupancy after pass-through save.
    let file = std::fs::File::open(&path).unwrap();
    let store = ZipImageStore::new(path.clone(), zip::ZipArchive::new(file).unwrap(), 1024 * 1024);
    let mut lazy_doc = Document::new("Stream");
    for (h, raw) in &hashes {
        lazy_doc.images.insert(h.clone(), ImageData::lazy(h.clone(), raw.len() as u64, 4, 3, store.clone()));
    }
    assert_eq!(store.cached_entries(), 0);
    assert_eq!(store.disk_reads(), 0);

    let dest = dir.join("out.figcraft");
    save_file(&lazy_doc, &dest, None).unwrap();
    assert_eq!(store.cached_entries(), 0, "streaming pass-through save must not populate the LRU cache");
    assert_eq!(store.cached_bytes(), 0);
    assert_eq!(store.disk_reads(), hashes.len() as u64);

    let reopened = load_file_with_versions(&dest).unwrap();
    for (h, raw) in &hashes {
        assert_eq!(reopened.doc.images[h].bytes().as_deref().map(Vec::as_slice), Some(raw.as_slice()));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn bench_lazy_large_archive() {
    use std::time::Instant;

    let dir = temp_dir("bench");
    let path = dir.join("200mb.figcraft");
    let count = 200usize;
    let payload_size = 1024 * 1024; // 1 MiB per image -> 200 MiB total

    let mut doc = Document::new("Bench200MiB");
    let mut base = png(10, 20, 30);
    base.resize(payload_size, 0xAB);
    for i in 0..count {
        let mut bytes = base.clone();
        bytes[payload_size - 4..].copy_from_slice(&(i as u32).to_le_bytes());
        let hash = image_hash(&bytes);
        doc.images.insert(hash, ImageData::new(Arc::new(bytes), 4, 3));
    }

    let t0 = Instant::now();
    save_file(&doc, &path, None).unwrap();
    let initial_save_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let archive_bytes = std::fs::metadata(&path).unwrap().len();

    // Eager byte-buffer open (the old load_path behavior: read whole file + unpack all images)
    let t1 = Instant::now();
    let raw_file = std::fs::read(&path).unwrap();
    let eager = load_bytes_with_versions(&raw_file).unwrap();
    let eager_open_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let eager_in_mem: u64 = eager.doc.images.values().filter_map(|i| i.as_in_memory()).map(|b| b.len() as u64).sum();
    drop(eager);
    drop(raw_file);

    // Lazy seekable open (new load_file_with_versions)
    let t2 = Instant::now();
    let lazy = load_file_with_versions(&path).unwrap();
    let lazy_open_ms = t2.elapsed().as_secs_f64() * 1000.0;
    let lazy_in_mem: u64 = lazy.doc.images.values().filter_map(|i| i.as_in_memory()).map(|b| b.len() as u64).sum();

    // Streaming pass-through save in place (new save_file) vs buffered save_bytes
    let t3 = Instant::now();
    let copy_buf = save_bytes(&lazy.doc, None).unwrap();
    let buffered_save_ms = t3.elapsed().as_secs_f64() * 1000.0;
    drop(copy_buf);

    let lazy_fresh = load_file_with_versions(&path).unwrap();
    let t4 = Instant::now();
    save_file(&lazy_fresh.doc, &path, None).unwrap();
    let streaming_save_ms = t4.elapsed().as_secs_f64() * 1000.0;

    println!(
        "200x1MiB archive ({:.1} MiB): initial_save={initial_save_ms:.1}ms | eager_open={eager_open_ms:.1}ms (in_mem={:.1} MiB) | lazy_open={lazy_open_ms:.2}ms (in_mem={:.1} MiB, {} images, past_budget={}) | buffered_save={buffered_save_ms:.1}ms | streaming_save_inplace={streaming_save_ms:.1}ms",
        archive_bytes as f64 / (1024.0 * 1024.0),
        eager_in_mem as f64 / (1024.0 * 1024.0),
        lazy_in_mem as f64 / (1024.0 * 1024.0),
        lazy.doc.images.len(),
        lazy.images_past_budget,
    );
    assert_eq!(lazy.doc.images.len(), count);
    assert_eq!(lazy_in_mem, 0);
    assert_eq!(lazy.images_past_budget, 0);
    let _ = std::fs::remove_dir_all(&dir);
}
