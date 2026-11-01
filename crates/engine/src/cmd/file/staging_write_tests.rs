use super::*;

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-engine-staging-{name}-{}", std::process::id()));
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
    let path = d.join("shot.svg");
    // The old deterministic staging name, held by an unrelated file: the write must pick its
    // own staging name instead of truncating this one.
    let old = d.join("shot.svg.tmp-figcraft");
    std::fs::write(&old, b"not yours").unwrap();
    write(&path.to_string_lossy(), b"<svg/>").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"<svg/>");
    assert_eq!(std::fs::read(&old).unwrap(), b"not yours", "an unrelated file at a staging name is untouched");
    // Just the file and the untouched staging file: no new staging leftovers.
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_staging_symlink_is_never_followed() {
    let d = dir("symlink");
    let path = d.join("shot.svg");
    let victim = d.join("victim.txt");
    std::fs::write(&victim, b"innocent").unwrap();
    let old = d.join("shot.svg.tmp-figcraft");
    if !symlink(&old, &victim) {
        return; // Making a symlink needs privileges on some Windows setups.
    }
    write(&path.to_string_lossy(), b"<svg/>").unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"innocent", "the symlink was never written through");
    assert_eq!(std::fs::read(&path).unwrap(), b"<svg/>");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn concurrent_writers_of_one_file_never_interleave() {
    let d = dir("concurrent");
    let path = d.join("shot.svg");
    for round in 0..5u8 {
        let mut handles = Vec::new();
        for w in 0u8..4 {
            let (path, bytes) = (path.clone(), vec![b'0' + w; 8192]);
            handles.push(std::thread::spawn(move || write(&path.to_string_lossy(), &bytes)));
        }
        let mut wins = 0;
        for h in handles {
            // On Windows a rename over a file another writer just replaced can lose the race.
            if h.join().unwrap().is_ok() {
                wins += 1;
            }
        }
        assert!(wins > 0, "round {round}: every writer lost");
        // Whatever landed is one writer's complete payload, never a mix.
        let got = std::fs::read(&path).unwrap();
        assert_eq!(got.len(), 8192, "round {round}: a complete payload landed");
        assert!(got.windows(2).all(|w| w[0] == w[1]), "round {round}: one writer's payload, not a mix");
    }
    // No staging file was left stuck (a failed writer removes its own).
    let left = std::fs::read_dir(&d).unwrap().count();
    assert!(left <= if cfg!(windows) { 5 } else { 1 }, "{left} files left in the staging directory");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_failed_write_leaves_the_destination_alone_and_cleans_up() {
    let d = dir("failure");
    // The destination is a directory: the rename fails, our staging file is removed and the
    // directory is untouched.
    let taken = d.join("over-a-directory");
    std::fs::create_dir_all(&taken).unwrap();
    assert!(write(&taken.to_string_lossy(), b"payload").is_err());
    assert!(taken.is_dir());
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no staging file left behind");
    // A missing parent directory fails without creating anything anywhere.
    assert!(write(&d.join("no").join("such.svg").to_string_lossy(), b"payload").is_err());
    assert!(!d.join("no").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A read-only directory refuses the staging file, and the old destination stays.
        let ro = d.join("read-only");
        std::fs::create_dir_all(&ro).unwrap();
        let dest = ro.join("keep.svg");
        std::fs::write(&dest, b"keep").unwrap();
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_mode(0o500);
        std::fs::set_permissions(&ro, perms).unwrap();
        // Running as root would defeat the read-only directory: skip instead of failing.
        let probe = ro.join(".probe");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
        } else {
            assert!(write(&dest.to_string_lossy(), b"payload").is_err());
            assert_eq!(std::fs::read(&dest).unwrap(), b"keep");
            assert_eq!(std::fs::read_dir(&ro).unwrap().count(), 1, "no staging file left behind");
        }
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&ro, perms).unwrap();
    }
    let _ = std::fs::remove_dir_all(&d);
}
