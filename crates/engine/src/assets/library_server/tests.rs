//! Team library helpers: links, what is shown of them, and the archive a server keeps.

use super::*;

#[test]
fn links_archives_and_junk() {
    assert!(is_server_link("figcraft://join?host=h:1&file=f&token=t") && is_server_link(" WS://h:1/?file=f&token=t"));
    assert!(!is_server_link("C:/kits/ui.figcraft") && !is_server_link("/srv/ws://x"));
    assert_eq!(shown_path("figcraft://join?host=h:1&file=f&token=secret&x=1"), "figcraft://join?host=h:1&file=f&token=…&x=1");
    assert_eq!(shown_path("ws://h:1/?file=f&token=secret"), "ws://h:1/?file=f&token=…");
    assert_eq!(shown_path("/kits/token=ui.figcraft"), "/kits/token=ui.figcraft", "files are shown as they are");

    let mut s = Session::new();
    let f = s.execute("frame.create", &serde_json::json!({"x": 0, "y": 0, "width": 10, "height": 10, "name": "Chip"})).unwrap();
    s.execute("component.create", &serde_json::json!({"ids": [f["id"]]})).unwrap();
    s.execute("library.publish", &serde_json::json!({})).unwrap();
    let lib = s.doc().unwrap().published.clone().unwrap();
    let back = read_archive(&archive(&lib).unwrap()).unwrap();
    assert_eq!(back.manifest, lib.manifest);
    assert!(read_archive(b"not a zip").is_err());
    let plain = figcraft_format::save_bytes(&Document::new("x"), None).unwrap();
    assert!(read_archive(&plain).unwrap_err().contains("without a library"));
    // Without the app's networking, a team library is a clear error.
    assert!(
        s.execute("library.add", &serde_json::json!({"path": "ws://127.0.0.1:9/?file=f&token=t"})).unwrap_err().to_string().contains("desktop app")
    );
}
