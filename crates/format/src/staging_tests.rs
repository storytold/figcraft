//! The staging write behind `save_file`: a pre-existing file or symlink at a staging name is never
//! truncated or followed, concurrent savers of one file never corrupt it, and a failed write
//! cleans up after itself and leaves the destination alone.

use figcraft_doc::Document;

use crate::{save_file, write_file_atomic};

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-staging-tests-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A symlink `from` → `to`; false when the platform won't make one (Windows needs privileges).
#[cfg(unix)]
fn symlink(from: &std::path::Path, to: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(to, from).is_ok()
}

#[cfg(windows)]
fn symlink(from: &std::path::Path, to: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_file(to, from).is_ok()
}

#[test]
fn an_existing_staging_file_is_never_written_through() {
    let d = dir("existing");
    let path = d.join("a.figcraft");
    // The old deterministic staging name, held by an unrelated file: saving must pick its own
    // staging name instead of truncating this one.
    let old = d.join("a.figcraft.tmp");
    std::fs::write(&old, b"not yours").unwrap();
    save_file(&Document::new("Mine"), &path, None).unwrap();
    assert_eq!(std::fs::read(&old).unwrap(), b"not yours", "an unrelated file at a staging name is untouched");
    assert_eq!(crate::load_file(&path).unwrap().0.name, "Mine");
    // Just the file and the untouched staging file: no new staging leftovers.
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_staging_symlink_is_never_followed() {
    let d = dir("symlink");
    let path = d.join("a.figcraft");
    let victim = d.join("victim.txt");
    std::fs::write(&victim, b"innocent").unwrap();
    let old = d.join("a.figcraft.tmp");
    if !symlink(&old, &victim) {
        return; // Making a symlink needs privileges on some Windows setups.
    }
    save_file(&Document::new("Mine"), &path, None).unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"innocent", "the symlink was never written through");
    assert_eq!(crate::load_file(&path).unwrap().0.name, "Mine");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn concurrent_savers_of_one_file_never_interleave() {
    let d = dir("concurrent");
    let path = d.join("a.figcraft");
    let names = ["W0", "W1", "W2", "W3"];
    for round in 0..5 {
        let mut handles = Vec::new();
        for name in names {
            let (path, doc) = (path.clone(), Document::new(name));
            handles.push(std::thread::spawn(move || save_file(&doc, &path, None)));
        }
        for h in handles {
            // On Windows a rename over a file another writer just replaced can lose the race.
            if cfg!(windows) {
                let _ = h.join().unwrap();
            } else {
                h.join().unwrap().unwrap();
            }
        }
        // Whatever landed is one writer's complete file, never a mix: a torn archive wouldn't load.
        let (doc, _) = crate::load_file(&path).unwrap();
        assert!(names.contains(&doc.name.as_str()), "round {round}: {:?}", doc.name);
    }
    // No staging file was left stuck.
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_failed_write_leaves_the_destination_alone_and_cleans_up() {
    let d = dir("failure");
    // The destination is a directory: the rename fails, our staging file is removed, the
    // directory is untouched.
    let taken = d.join("folder.figcraft");
    std::fs::create_dir_all(&taken).unwrap();
    assert!(save_file(&Document::new("Mine"), &taken, None).is_err());
    assert!(write_file_atomic(&taken, b"payload").is_err());
    assert!(taken.is_dir());
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no staging file left behind");
    // A missing parent directory fails without creating anything anywhere.
    assert!(write_file_atomic(&d.join("no").join("such.figcraft"), b"payload").is_err());
    assert!(!d.join("no").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A read-only directory refuses the staging file, and the old destination stays.
        let ro = d.join("read-only");
        std::fs::create_dir_all(&ro).unwrap();
        let dest = ro.join("keep.figcraft");
        std::fs::write(&dest, b"keep").unwrap();
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_mode(0o500);
        std::fs::set_permissions(&ro, perms).unwrap();
        // Running as root would defeat the read-only directory: skip instead of failing.
        let probe = ro.join(".probe");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
        } else {
            assert!(write_file_atomic(&dest, b"payload").is_err());
            assert_eq!(std::fs::read(&dest).unwrap(), b"keep");
            assert_eq!(std::fs::read_dir(&ro).unwrap().count(), 1, "no staging file left behind");
        }
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&ro, perms).unwrap();
    }
    let _ = std::fs::remove_dir_all(&d);
}
