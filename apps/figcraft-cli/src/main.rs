//! `figcraft-cli`: FigCraft without a window. See the library crate for the commands.

fn main() {
    // `args_os`: a non-Unicode argument is clap's usage error, not a panic.
    let code = figcraft_cli::run(std::env::args_os().skip(1), &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
