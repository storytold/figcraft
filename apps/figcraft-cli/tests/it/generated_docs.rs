//! Generated CLI documentation uses the parser's command definition, without running commands.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "../../examples/man-and-completions.rs"]
#[expect(dead_code, reason = "These tests call generate directly; the example's main is smoke-tested separately.")]
mod generator;

struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tmp(name: &str) -> std::io::Result<TempDir> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("figcraft-cli-test-{}-{n}-{name}", std::process::id()));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(TempDir(dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

#[test]
fn completions_name_the_cli_its_subcommands_and_options() -> std::io::Result<()> {
    let temp = tmp("completions")?;
    let dir = temp.path();
    generator::generate(dir)?;
    for name in ["figcraft-cli.bash", "figcraft-cli.elv", "figcraft-cli.fish", "_figcraft-cli.ps1", "_figcraft-cli"] {
        let text = fs::read_to_string(dir.join(name))?;
        for word in [
            "figcraft-cli",
            "info",
            "export",
            "convert",
            "render",
            "run",
            "commands",
            "mcp",
            "plugin",
            "perf",
            "publish",
            "no-system-fonts",
            "format",
            "automation-read-root",
            "server",
        ] {
            assert!(text.contains(word), "{name} does not contain {word}");
        }
        assert!(!text.contains("man-and-completions"), "{name} names the example instead of the CLI");
    }
    Ok(())
}

#[test]
fn bash_dispatch_states_have_matching_option_blocks() -> std::io::Result<()> {
    let temp = tmp("bash-completion-states")?;
    let dir = temp.path();
    generator::generate(dir)?;
    let text = fs::read_to_string(dir.join("figcraft-cli.bash"))?;
    let states: Vec<_> =
        text.lines().filter_map(|line| line.trim().strip_prefix("cmd=\"")?.strip_suffix('"')).filter(|state| !state.is_empty()).collect();
    assert!(states.len() > 1, "the completion has no subcommand dispatch states");
    for state in states {
        let block = format!("{state})");
        assert!(text.lines().any(|line| line.trim() == block), "Bash dispatch state {state} has no option block");
    }
    Ok(())
}

#[test]
fn man_pages_cover_every_subcommand_without_synthetic_help_pages() -> std::io::Result<()> {
    let temp = tmp("man-pages")?;
    let dir = temp.path();
    generator::generate(dir)?;
    let mut pages = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "1") {
            pages.push(path.file_name().unwrap_or_default().to_string_lossy().into_owned());
        }
    }
    pages.sort();
    assert_eq!(
        pages,
        [
            "figcraft-cli-commands.1",
            "figcraft-cli-connect-publish.1",
            "figcraft-cli-connect.1",
            "figcraft-cli-convert.1",
            "figcraft-cli-export.1",
            "figcraft-cli-info.1",
            "figcraft-cli-mcp.1",
            "figcraft-cli-perf.1",
            "figcraft-cli-plugin-publish.1",
            "figcraft-cli-plugin.1",
            "figcraft-cli-render.1",
            "figcraft-cli-run.1",
            "figcraft-cli.1",
        ]
    );
    for name in &pages {
        let text = fs::read_to_string(dir.join(name))?.replace("\\-", "-");
        assert!(text.contains(".SH NAME"), "{name} has no NAME section");
        assert!(text.contains(name.trim_end_matches(".1")), "{name} has the wrong command name");
        assert!(!text.contains("man-and-completions"), "{name} names the example instead of the CLI");
    }
    for (page, option) in [
        ("figcraft-cli.1", "--no-system-fonts"),
        ("figcraft-cli-export.1", "--format"),
        ("figcraft-cli-export.1", "--automation-read-root"),
        ("figcraft-cli-plugin-publish.1", "--server"),
        ("figcraft-cli-connect-publish.1", "--dry-run"),
    ] {
        let text = fs::read_to_string(dir.join(page))?.replace("\\-", "-");
        assert!(text.contains(option), "{page} does not contain {option}");
    }
    Ok(())
}

#[test]
fn generation_creates_the_output_directory_and_can_run_again() -> std::io::Result<()> {
    let temp = tmp("generated-docs-repeat")?;
    let dir = temp.path();
    let output = dir.join("new").join("docs");
    assert!(!output.exists());
    generator::generate(&output)?;
    let page = output.join("figcraft-cli.1");
    let first = fs::read(&page)?;
    assert!(!first.is_empty());
    fs::write(&page, "old page")?;
    generator::generate(&output)?;
    assert_eq!(fs::read(page)?, first);
    assert_eq!(fs::read_dir(output)?.count(), 18);
    Ok(())
}

#[test]
fn generation_returns_filesystem_errors() -> std::io::Result<()> {
    let temp = tmp("generated-docs-errors")?;
    let dir = temp.path();
    let file = dir.join("not-a-directory");
    fs::write(&file, "keep me")?;
    assert!(generator::generate(&file).is_err());
    assert_eq!(fs::read_to_string(&file)?, "keep me");
    // A valid directory can still fail when writing one of the generated pages.
    fs::create_dir(dir.join("figcraft-cli.1"))?;
    assert!(generator::generate(dir).is_err());
    Ok(())
}
