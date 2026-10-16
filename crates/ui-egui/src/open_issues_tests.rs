//! Opening a native file with a damaged image: the app says what was lost.

use base64::Engine as _;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// A 1×1 PNG.
const PNG: [u8; 67] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x00, 0x00, 0x02,
    0x00, 0x00, 0x01, 0xe2, 0x21, 0xbc, 0x33, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn opening_a_file_with_a_damaged_image_toasts_what_was_lost() {
    let dir = std::env::temp_dir().join(format!("figcraft-ui-open-issues-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pics.figcraft");
    let mut s = Session::new();
    let data = base64::engine::general_purpose::STANDARD.encode(PNG);
    s.execute("file.import", &json!({"data": data, "name": "dot.png"})).unwrap();
    s.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    // Images are stored as they are: change one byte of the PNG, not its checksum.
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes.windows(PNG.len()).position(|w| w == PNG).unwrap();
    bytes[at + 30] ^= 0xff;
    std::fs::write(&path, bytes).unwrap();

    let mut u = Ui::new(Session::new());
    u.app.run("file.open", json!({"path": path.to_string_lossy()}));
    u.frame(vec![]);
    let toast = u.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(toast.starts_with("pics: image ") && toast.contains("couldn't be read"), "{toast}");
    let _ = std::fs::remove_dir_all(&dir);
}
