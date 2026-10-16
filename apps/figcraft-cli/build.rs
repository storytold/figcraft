//! Windows: give the main thread the stack Linux and macOS give it (see `main_thread_stack`).
fn main() {
    main_thread_stack();
}

/// Windows starts the main thread with 1 MB of stack, Linux and macOS with 8 MB. Layers nest up
/// to 256 deep in an export, and that recursion needs the same room everywhere.
fn main_thread_stack() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    println!("cargo::rustc-link-arg-bins={}", if msvc { "/STACK:8388608" } else { "-Wl,--stack,8388608" });
}
