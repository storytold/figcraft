//! The project library's file (`projects.json`) next to the preferences.
//!
//! Kept apart from `state.json`, so a build that doesn't know about it rewrites the preferences
//! and leaves the library alone. Every save writes a staging file, flushes it to disk and renames
//! it over the old one (a crash leaves either version whole), after copying the old one to
//! `projects.json.bak`. A file that can't be read is never overwritten: a damaged one is renamed
//! aside first (`projects.json.damaged-<time>`), and the last good copy takes its place.

use std::io::Write;
use std::path::{Path, PathBuf};

use figcraft_engine::projects::Projects;
use figcraft_ui_egui::ProjectsNotice;

pub const FILE: &str = "projects.json";
pub const BACKUP: &str = "projects.json.bak";

/// What reading the library found.
#[derive(Debug, Default)]
pub struct Loaded {
    /// `None`: there was none yet (a first run with this build).
    pub projects: Option<Projects>,
    /// What to tell the user about it.
    pub notice: Option<ProjectsNotice>,
    /// Why saving must not happen (the file is there but couldn't be read).
    pub blocked: Option<String>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `dir/name-<time>` (another number when that is taken).
fn aside(dir: &Path, name: &str) -> PathBuf {
    let stamp = now();
    (0..100)
        .map(|i| dir.join(if i == 0 { format!("{name}-{stamp}") } else { format!("{name}-{stamp}-{i}") }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{name}-{stamp}-x")))
}

/// The backup's library, when it reads as one.
fn backup(dir: &Path) -> Option<Projects> {
    let text = std::fs::read_to_string(dir.join(BACKUP)).ok()?;
    Projects::from_json(&text).ok().map(|(p, _)| p)
}

/// Read the library kept in `dir`.
pub fn read(dir: &Path) -> Loaded {
    let path = dir.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Gone with its backup still there (removed by hand): the backup comes back.
            return match backup(dir) {
                Some(mut p) => {
                    p.unsaved = true;
                    Loaded { projects: Some(p), notice: Some(ProjectsNotice::Restored { kept: None }), blocked: None }
                }
                None => Loaded::default(),
            };
        }
        Err(e) => {
            log::warn!("projects: can't read {}: {e}", path.display());
            let why = format!("{}: {e}", path.display());
            return Loaded { projects: None, notice: Some(ProjectsNotice::Unreadable { error: why.clone() }), blocked: Some(why) };
        }
    };
    match Projects::from_json(&text) {
        Ok((mut p, 0)) => {
            p.unsaved = false;
            Loaded { projects: Some(p), notice: None, blocked: None }
        }
        Ok((mut p, fixes)) => {
            // Repaired: the file as it was stays aside, the repaired library is kept.
            let kept = aside(dir, "projects.json.repaired");
            let kept = std::fs::copy(&path, &kept).ok().map(|_| kept.to_string_lossy().into_owned());
            log::warn!("projects: repaired {fixes} references in {}", path.display());
            p.unsaved = true;
            Loaded { projects: Some(p), notice: Some(ProjectsNotice::Repaired { fixes, kept }), blocked: None }
        }
        Err(e) => {
            log::warn!("projects: {} is damaged: {e}", path.display());
            let damaged = aside(dir, "projects.json.damaged");
            if let Err(e) = std::fs::rename(&path, &damaged) {
                // It can't be moved aside: leave it, and don't write over it.
                let why = format!("{}: {e}", path.display());
                return Loaded { projects: None, notice: Some(ProjectsNotice::Unreadable { error: why.clone() }), blocked: Some(why) };
            }
            let kept = Some(damaged.to_string_lossy().into_owned());
            match backup(dir) {
                Some(mut p) => {
                    p.unsaved = true;
                    Loaded { projects: Some(p), notice: Some(ProjectsNotice::Restored { kept }), blocked: None }
                }
                None => Loaded { projects: Some(Projects::default()), notice: Some(ProjectsNotice::Damaged { kept }), blocked: None },
            }
        }
    }
}

/// Keep `projects` in `dir`: the old file becomes the backup, the new one takes its place whole.
pub fn write(dir: &Path, projects: &Projects) -> Result<(), String> {
    let path = dir.join(FILE);
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    std::fs::create_dir_all(dir).map_err(fail)?;
    if path.exists() {
        std::fs::copy(&path, dir.join(BACKUP)).map_err(fail)?;
    }
    let tmp = dir.join(format!("{FILE}.tmp-{}", std::process::id()));
    let staged = std::fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(projects.to_json().as_bytes())?;
        f.sync_all()
    });
    if let Err(e) = staged.and_then(|()| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(e));
    }
    Ok(())
}

/// A first run with the library: the recent design files join it, in no folder.
pub fn seeded(recent_files: &[String]) -> Projects {
    let mut p = Projects::default();
    let now = now();
    for path in recent_files.iter().rev() {
        if figcraft_engine::cmd::projects::is_project_file(Path::new(path)) {
            p.touch(path, None, now);
        }
    }
    p.unsaved = !p.files.is_empty();
    p
}
