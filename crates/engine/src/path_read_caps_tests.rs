//! Paths callers give are read only when they are regular files within a cap that fits their
//! kind (#1114): a folder, a device or an oversized file is refused with a clear error before
//! anything is read, and the session is left as it was.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Session;
use crate::file_access::read_capped;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("figcraft-caps-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("folder")).unwrap();
        Self(d)
    }
    fn p(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().into_owned()
    }
    /// A sparse file of `len` bytes (nothing is written).
    fn sized(&self, rel: &str, len: u64) -> String {
        std::fs::File::create(self.0.join(rel)).unwrap().set_len(len).unwrap();
        self.p(rel)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `cmd` with `params` fails with a message containing `why`, and changes no document.
fn refused(s: &mut Session, cmd: &str, params: Value, why: &str) {
    let before = (format!("{:?}", s.docs.iter().map(|d| &d.doc).collect::<Vec<_>>()), s.active);
    let e = s.execute(cmd, &params).unwrap_err().to_string();
    assert!(e.contains(why), "{cmd} {params}: {e}");
    assert_eq!((format!("{:?}", s.docs.iter().map(|d| &d.doc).collect::<Vec<_>>()), s.active), before, "{cmd} {params}");
}

const FOLDER: &str = "is a folder, not a file";
const DEVICE: &str = "isn't a regular file";

#[test]
fn the_capped_read_refuses_folders_devices_and_oversized_files() {
    let d = Dir::new("read");
    std::fs::write(d.0.join("four.txt"), b"four").unwrap();
    assert_eq!(read_capped(d.p("four.txt"), 4).unwrap(), b"four");
    let e = read_capped(d.p("four.txt"), 3).unwrap_err().to_string();
    assert!(e.contains("is 4 bytes, more than the 3 bytes allowed here"), "{e}");
    assert!(read_capped(d.p("folder"), u64::MAX).unwrap_err().to_string().contains(FOLDER));
    if cfg!(unix) {
        // An endless source: refused, not read until memory runs out.
        assert!(read_capped("/dev/zero", u64::MAX).unwrap_err().to_string().contains(DEVICE));
    }
}

#[test]
fn documents_are_refused_as_folders_and_devices() {
    let d = Dir::new("docs");
    let mut s = Session::new();
    for cmd in ["file.open", "file.import"] {
        refused(&mut s, cmd, json!({"path": d.p("folder")}), FOLDER);
        if cfg!(unix) {
            refused(&mut s, cmd, json!({"path": "/dev/zero"}), DEVICE);
        }
    }
    const { assert!(crate::cmd::file::MAX_DOCUMENT_FILE > 7_400_000_000, "a real 7.4 GB file still opens") };
}

#[test]
fn placed_images_are_capped_at_what_a_document_takes() {
    let d = Dir::new("images");
    let mut s = Session::new();
    let big = d.sized("huge.png", figcraft_doc::ImageData::MAX_BYTES + 1);
    refused(&mut s, "image.place", json!({"path": big}), "more than the 256 MB allowed here");
    refused(&mut s, "image.place", json!({"path": d.p("folder")}), FOLDER);
    if cfg!(unix) {
        refused(&mut s, "image.place", json!({"path": "/dev/zero"}), DEVICE);
    }
}

#[test]
fn fonts_and_plugins_are_refused_as_folders() {
    let d = Dir::new("plugins");
    let mut s = Session::new();
    refused(&mut s, "text.loadFont", json!({"path": d.p("folder")}), FOLDER);
    // A plugin folder whose manifest is a folder.
    std::fs::create_dir_all(d.0.join("plugin").join("figcraft-plugin.json")).unwrap();
    refused(&mut s, "plugin.import", json!({"path": d.p("plugin")}), FOLDER);
    assert!(s.prefs.plugins.is_empty());
}
