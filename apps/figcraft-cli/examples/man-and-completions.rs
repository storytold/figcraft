//! Writes figcraft-cli's shell completions and recursive man pages into a directory:
//!
//! ```sh
//! cargo run -p figcraft-cli --example man-and-completions -- <out-dir>
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[path = "../src/args.rs"]
#[expect(dead_code, reason = "Generation uses clap's command metadata, not the parsed fields or runtime parser.")]
mod args;

use std::path::Path;
use std::process::ExitCode;

use clap::{CommandFactory, ValueEnum};
use clap_complete::Shell;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), None) = (args.next(), args.next()) else {
        eprintln!("usage: man-and-completions <out-dir>");
        return ExitCode::from(2);
    };
    let dir = Path::new(&dir);
    match generate(dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}: {e}", dir.display());
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn generate(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut command = args::Cli::command();
    let name = command.get_name().to_owned();
    for &shell in Shell::value_variants() {
        clap_complete::generate_to(shell, &mut command, &name, dir)?;
    }
    // Completions build the command and add synthetic help subcommands. A fresh command keeps
    // those out of the man pages (such as figcraft-cli-help-export.1).
    clap_mangen::generate_to(args::Cli::command(), dir)
}
