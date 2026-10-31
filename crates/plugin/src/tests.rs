//! The sandbox (modules assembled from WAT text at test time) and the manifest.

use serde_json::{Value, json};

use crate::{Access, Host, Limits, Manifest, PluginError, check_module, run};

/// Records what a plugin asked for and answers `call` with a canned reply.
#[derive(Default)]
struct Mock {
    calls: Vec<(String, String)>,
    notes: Vec<String>,
    logs: Vec<String>,
    refuse: bool,
}

impl Host for Mock {
    fn call(&mut self, command: &str, params: &str) -> Result<String, String> {
        self.calls.push((command.into(), params.into()));
        if self.refuse { Err(format!("`{command}` refused")) } else { Ok(format!("{{\"ran\":\"{command}\"}}")) }
    }
    fn selection(&mut self) -> String {
        "[2,3]".into()
    }
    fn notify(&mut self, message: &str) {
        self.notes.push(message.into());
    }
    fn log(&mut self, line: &str) {
        self.logs.push(line.into());
    }
}

const IMPORTS: &str = r#"
  (import "figcraft" "call" (func $call (param i32 i32 i32 i32) (result i32)))
  (import "figcraft" "selection" (func $selection (result i32)))
  (import "figcraft" "reply_len" (func $reply_len (result i32)))
  (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
  (import "figcraft" "log" (func $log (param i32 i32)))
  (import "figcraft" "notify" (func $notify (param i32 i32)))
  (import "figcraft" "set_output" (func $set_output (param i32 i32)))
"#;

/// A module with the host imports, one page of memory, `data` segments and `figcraft_run`'s body.
fn module(data: &str, body: &str) -> Vec<u8> {
    module_with(data, body, "(memory (export \"memory\") 1)", "")
}

fn module_with(data: &str, body: &str, memory: &str, extra: &str) -> Vec<u8> {
    let wat = format!("(module {IMPORTS} {memory} {data} {extra} (func (export \"figcraft_run\") (param $n i32) (result i32) {body}))");
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

fn small() -> Limits {
    Limits { fuel: 2_000_000, ..Limits::default() }
}

fn go(bytes: &[u8], input: &str) -> (Result<crate::RunOutput, PluginError>, Mock) {
    let mut host = Mock::default();
    let r = run(bytes, input, &small(), &mut host);
    (r, host)
}

#[test]
fn echoes_input_calls_commands_and_reads_replies() {
    let m = module(
        r#"(data (i32.const 0) "shape.rectangle") (data (i32.const 32) "{\"x\":1}") (data (i32.const 64) "made it")"#,
        r#"
        ;; The input is waiting in the reply buffer.
        (drop (call $reply_read (i32.const 1024) (local.get $n)))
        (call $log (i32.const 1024) (local.get $n))
        (if (i32.ne (call $call (i32.const 0) (i32.const 15) (i32.const 32) (i32.const 7)) (i32.const 0)) (then (return (i32.const 1))))
        ;; Hand the command's reply back as the output.
        (call $set_output (i32.const 2048) (call $reply_read (i32.const 2048) (call $reply_len)))
        (call $notify (i32.const 64) (i32.const 7))
        (i32.const 0)"#,
    );
    check_module(&m, &Limits::default()).unwrap();
    let (r, host) = go(&m, r#"{"command":"run"}"#);
    let out = r.unwrap();
    assert_eq!(out.output, r#"{"ran":"shape.rectangle"}"#);
    assert_eq!(out.logs, vec![r#"{"command":"run"}"#.to_string()]);
    assert_eq!(host.logs, out.logs);
    assert_eq!(host.calls, vec![("shape.rectangle".into(), r#"{"x":1}"#.into())]);
    assert_eq!(host.notes, vec!["made it".to_string()]);
    assert_eq!(out.calls, 7);
    assert!(out.fuel > 0);
}

#[test]
fn selection_and_refusals_reach_the_plugin() {
    let m = module(
        r#"(data (i32.const 0) "node.rename")"#,
        r#"
        (drop (call $selection))
        (drop (call $reply_read (i32.const 512) (i32.const 64)))
        ;; Empty params count as {}; status 1 when refused, the message in the reply.
        (if (i32.eq (call $call (i32.const 0) (i32.const 11) (i32.const 0) (i32.const 0)) (i32.const 1))
          (then (call $set_output (i32.const 1024) (call $reply_read (i32.const 1024) (call $reply_len)))))
        (i32.const 0)"#,
    );
    let mut host = Mock { refuse: true, ..Mock::default() };
    let out = run(&m, "{}", &small(), &mut host).unwrap();
    assert_eq!(out.output, "`node.rename` refused");
    assert_eq!(host.calls[0].1, "{}");
}

#[test]
fn a_nonzero_status_fails_the_run_with_its_output() {
    let m = module(r#"(data (i32.const 0) "nothing selected")"#, "(call $set_output (i32.const 0) (i32.const 16)) (i32.const 2)");
    assert_eq!(go(&m, "{}").0, Err(PluginError::Failed("nothing selected".into())));
    let m = module("", "(i32.const 7)");
    assert!(matches!(go(&m, "{}").0, Err(PluginError::Failed(m)) if m.contains("status 7")));
}

#[test]
fn an_endless_loop_runs_out_of_fuel() {
    let m = module("", "(loop $l (br $l)) (i32.const 0)");
    assert_eq!(go(&m, "{}").0, Err(PluginError::OutOfFuel));
    // Host calls cost fuel too, and their number is capped.
    let m = module("", "(loop $l (call $log (i32.const 0) (i32.const 0)) (br $l)) (i32.const 0)");
    assert_eq!(go(&m, "{}").0, Err(PluginError::OutOfFuel));
    let mut host = Mock::default();
    let limits = Limits { max_calls: 10, ..small() };
    assert!(matches!(run(&m, "{}", &limits, &mut host), Err(PluginError::Limit(_))));
}

#[test]
fn logs_over_the_default_line_budget_fail_before_forwarding() {
    // One byte over 8 KiB, all valid UTF-8 in the module's zero-filled memory.
    let m = module("", "(call $log (i32.const 0) (i32.const 8193)) (i32.const 0)");
    let (result, host) = go(&m, "{}");
    assert!(matches!(result, Err(PluginError::Limit(_))), "an oversized log line must fail the run");
    assert!(host.logs.is_empty());
}

#[test]
fn logs_over_the_default_byte_budget_fail_before_forwarding() {
    let m = module(
        "",
        "(local $i i32)
         (loop $logs
           (call $log (i32.const 0) (i32.const 8192))
           (local.set $i (i32.add (local.get $i) (i32.const 1)))
           (br_if $logs (i32.lt_u (local.get $i) (i32.const 129)))) (i32.const 0)",
    );
    // The default fuel: copying a megabyte of log bytes out of the plugin is itself charged.
    let mut host = Mock::default();
    let result = run(&m, "{}", &Limits::default(), &mut host);
    assert!(matches!(result, Err(PluginError::Limit(_))), "logs over 1 MiB must fail the run");
    assert_eq!(host.logs.len(), 128);
    assert_eq!(host.logs.iter().map(String::len).sum::<usize>(), 1024 * 1024);
}

#[test]
fn logs_over_the_count_budget_fail_before_forwarding() {
    let m = module(
        "",
        "(call $log (i32.const 0) (i32.const 0))
         (call $log (i32.const 0) (i32.const 0))
         (call $log (i32.const 0) (i32.const 0)) (i32.const 0)",
    );
    let limits = Limits { max_logs: 2, ..small() };
    let mut host = Mock::default();
    assert_eq!(run(&m, "{}", &limits, &mut host), Err(PluginError::Limit("more than 2 log lines".into())));
    assert_eq!(host.logs, vec![String::new(), String::new()]);
}

#[test]
fn logs_check_the_line_budget_before_reading_guest_memory() {
    let limits = Limits { max_log_line_bytes: 4, ..small() };
    let m = module(r#"(data (i32.const 0) "line")"#, "(call $log (i32.const 0) (i32.const 4)) (i32.const 0)");
    let mut host = Mock::default();
    let out = run(&m, "{}", &limits, &mut host).unwrap();
    assert_eq!(out.logs, vec!["line".to_string()]);
    assert_eq!(host.logs, out.logs);
    // Both an ordinary oversize line and invalid pointers/lengths are rejected by the byte
    // budget, before an out-of-bounds read or a guest-to-host allocation can happen.
    for (ptr, len) in [(0, 5), (65535, 5), (-1, 5), (0, -1)] {
        let m = module("", &format!("(call $log (i32.const {ptr}) (i32.const {len})) (i32.const 0)"));
        let mut host = Mock::default();
        assert!(matches!(run(&m, "{}", &limits, &mut host), Err(PluginError::Limit(m)) if m.contains("log line")));
        assert!(host.logs.is_empty());
    }
}

#[test]
fn logs_count_utf8_bytes_and_check_the_total_before_reading_memory() {
    let data = r#"(data (i32.const 0) "\c3\a9")"#;
    let two_lines = "(call $log (i32.const 0) (i32.const 2)) (call $log (i32.const 0) (i32.const 2))";
    let limits = Limits { max_log_line_bytes: 2, max_log_bytes: 4, ..small() };
    let m = module(data, &format!("{two_lines} (i32.const 0)"));
    let mut host = Mock::default();
    let out = run(&m, "{}", &limits, &mut host).unwrap();
    assert_eq!(out.logs, vec!["é".to_string(), "é".to_string()]);
    assert_eq!(host.logs, out.logs);
    assert_eq!(out.logs.iter().map(String::len).sum::<usize>(), 4);
    // The third line exceeds the total; its invalid pointer must never be read or forwarded.
    let m = module(data, &format!("{two_lines} (call $log (i32.const 65535) (i32.const 2)) (i32.const 0)"));
    let mut host = Mock::default();
    assert_eq!(run(&m, "{}", &limits, &mut host), Err(PluginError::Limit("more than 4 log bytes".into())));
    assert_eq!(host.logs, out.logs);
    let limits = Limits { max_log_bytes: 3, ..limits };
    let m = module(data, &format!("{two_lines} (i32.const 0)"));
    let mut host = Mock::default();
    assert_eq!(run(&m, "{}", &limits, &mut host), Err(PluginError::Limit("more than 3 log bytes".into())));
    assert_eq!(host.logs, vec!["é".to_string()]);
}

#[test]
fn logs_observe_zero_budgets_and_the_exact_line_count() {
    let empty = "(call $log (i32.const 0) (i32.const 0))";
    let limits = Limits { max_logs: 2, max_log_line_bytes: 0, max_log_bytes: 0, ..small() };
    let m = module("", &format!("{empty} {empty} (i32.const 0)"));
    let mut host = Mock::default();
    let out = run(&m, "{}", &limits, &mut host).unwrap();
    assert_eq!(out.logs, vec![String::new(), String::new()]);
    assert_eq!(host.logs, out.logs);
    for limits in [Limits { max_logs: 0, ..limits }, Limits { max_log_line_bytes: 1, ..limits }] {
        let bytes = if limits.max_logs == 0 { 0 } else { 1 };
        let m = module("", &format!("(call $log (i32.const 0) (i32.const {bytes})) (i32.const 0)"));
        let mut host = Mock::default();
        assert!(matches!(run(&m, "{}", &limits, &mut host), Err(PluginError::Limit(_))));
        assert!(host.logs.is_empty());
    }
}

#[test]
fn logs_get_fresh_budgets_on_callbacks_after_success_and_failure() {
    let m = module_with(
        r#"(data (i32.const 0) "x")"#,
        "(call $log (i32.const 0) (i32.const 1))
         (call $log (i32.const 0) (i32.const 1))
         (if (i32.eqz (global.get $called))
           (then (global.set $called (i32.const 1)) (call $log (i32.const 0) (i32.const 1)))) (i32.const 0)",
        "(memory (export \"memory\") 1)",
        "(global $called (mut i32) (i32.const 0))",
    );
    let limits = Limits { max_logs: 2, max_log_line_bytes: 1, max_log_bytes: 2, ..small() };
    let mut instance = crate::Instance::new(&m, &limits).unwrap();
    let mut host = Mock::default();
    assert!(matches!(instance.call("{}", &mut host), Err(PluginError::Limit(_))));
    assert_eq!(host.logs, vec!["x".to_string(), "x".to_string()]);
    for _ in 0..2 {
        let mut host = Mock::default();
        let out = instance.call("{}", &mut host).unwrap();
        assert_eq!(out.logs, vec!["x".to_string(), "x".to_string()]);
        assert_eq!(host.logs, out.logs);
    }
}

#[test]
fn logs_still_observe_the_general_message_limit() {
    let limits = Limits { max_message: 3, max_log_line_bytes: 4, ..small() };
    let m = module(r#"(data (i32.const 0) "line")"#, "(call $log (i32.const 0) (i32.const 4)) (i32.const 0)");
    let mut host = Mock::default();
    assert_eq!(run(&m, "{}", &limits, &mut host), Err(PluginError::Limit("a 4-byte string is too long".into())));
    assert!(host.logs.is_empty());
}

#[test]
fn reply_and_output_imports_obey_host_call_limit() {
    for call in
        ["(drop (call $reply_len))", "(drop (call $reply_read (i32.const 0) (i32.const 2)))", "(call $set_output (i32.const 0) (i32.const 2))"]
    {
        let m = module(r#"(data (i32.const 0) "ok")"#, &format!("{call} {call} (i32.const 0)"));
        let limits = Limits { max_calls: 1, ..small() };
        assert!(matches!(run(&m, "{}", &limits, &mut Mock::default()), Err(PluginError::Limit(_))), "{call}");
        let limits = Limits { max_calls: 2, ..small() };
        assert_eq!(run(&m, "{}", &limits, &mut Mock::default()).unwrap().calls, 2, "{call}");
    }
}

#[test]
fn repeated_bulk_imports_spend_fuel_for_bytes_even_without_call_overhead() {
    for call in ["(drop (call $reply_read (i32.const 0) (i32.const 4096)))", "(call $set_output (i32.const 0) (i32.const 4096))"] {
        let m = module("", &format!("{} (i32.const 0)", call.repeat(16)));
        let limits = Limits { fuel: 40_000, call_fuel: 0, ..small() };
        assert_eq!(run(&m, &"x".repeat(4096), &limits, &mut Mock::default()), Err(PluginError::OutOfFuel), "{call}");
    }
}

#[test]
fn reply_copy_charges_actual_bytes_and_each_callback_gets_fresh_limits() {
    let m = module("", "(call $set_output (i32.const 0) (call $reply_read (i32.const 0) (i32.const 2147483647))) (i32.const 0)");
    let limits = Limits { max_calls: 2, fuel: 1000, call_fuel: 0, ..small() };
    let mut instance = crate::Instance::new(&m, &limits).unwrap();
    assert_eq!(instance.call(&"x".repeat(2048), &mut Mock::default()), Err(PluginError::OutOfFuel));
    for input in ["hello", "again"] {
        let out = instance.call(input, &mut Mock::default()).unwrap();
        assert_eq!(out.output, input);
        assert_eq!(out.calls, 2);
    }
}

#[test]
fn memory_is_capped() {
    // Declared larger than the cap: refused at instantiation.
    let big = module_with("", "(i32.const 0)", "(memory (export \"memory\") 2000)", "");
    assert_eq!(go(&big, "{}").0, Err(PluginError::OutOfMemory));
    // Growing past the cap fails (memory.grow returns -1) and the plugin can tell.
    let grow = module(
        r#"(data (i32.const 0) "refused")"#,
        r#"(if (i32.eq (memory.grow (i32.const 1500)) (i32.const -1)) (then (call $set_output (i32.const 0) (i32.const 7)))) (i32.const 0)"#,
    );
    assert_eq!(go(&grow, "{}").0.unwrap().output, "refused");
    // Within the cap it grows.
    let ok = module("", "(drop (memory.grow (i32.const 10))) (i32.const 0)");
    assert!(go(&ok, "{}").0.is_ok());
}

#[test]
fn traps_fail_cleanly() {
    let cases = [
        module("", "unreachable"),
        module("", "(i32.div_u (i32.const 1) (i32.const 0))"),
        // Out-of-bounds strings and replies.
        module("", "(call $log (i32.const 65530) (i32.const 100)) (i32.const 0)"),
        module("", "(call $log (i32.const -1) (i32.const 2)) (i32.const 0)"),
        module("", "(drop (call $reply_read (i32.const 65535) (i32.const 100))) (i32.const 0)"),
        // Not UTF-8.
        module(r#"(data (i32.const 0) "\ff\fe")"#, "(call $log (i32.const 0) (i32.const 2)) (i32.const 0)"),
        // Endless recursion exhausts the interpreter's stack, not the app's.
        module_with("", "(call $rec) (i32.const 0)", "(memory (export \"memory\") 1)", "(func $rec (call $rec))"),
    ];
    for m in &cases {
        let r = go(m, "{}").0;
        assert!(matches!(r, Err(PluginError::Trap(_)) | Err(PluginError::Limit(_))), "{r:?}");
    }
    // A huge length is refused before anything is read.
    let m = module("", "(call $log (i32.const 0) (i32.const 2147483647)) (i32.const 0)");
    assert!(matches!(go(&m, "{}").0, Err(PluginError::Limit(_))));
}

#[test]
fn modules_without_the_abi_are_refused() {
    let invalid = |wat: &str| {
        let bytes = wat::parse_str(wat).unwrap();
        let e = check_module(&bytes, &Limits::default()).unwrap_err();
        assert!(matches!(e, PluginError::InvalidModule(_)), "{e:?}");
        assert_eq!(go(&bytes, "{}").0.unwrap_err(), e);
        e.to_string()
    };
    let wasi = invalid(
        r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
           (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) (i32.const 0)))"#,
    );
    assert!(wasi.contains("no file system"), "{wasi}");
    invalid(
        r#"(module (import "figcraft" "call" (func (param i32) (result i32))) (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) (i32.const 0)))"#,
    );
    invalid(
        r#"(module (import "figcraft" "launch_rockets" (func)) (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) (i32.const 0)))"#,
    );
    invalid(r#"(module (func (export "figcraft_run") (param i32) (result i32) (i32.const 0)))"#);
    invalid(r#"(module (memory (export "memory") 1))"#);
    invalid(r#"(module (memory (export "memory") 1) (func (export "figcraft_run") (result i32) (i32.const 0)))"#);
    for junk in [&b""[..], b"\0asm", b"\0asm\x01\0\0\0\x01", b"not wasm at all", &[0xff; 64]] {
        assert!(matches!(check_module(junk, &Limits::default()), Err(PluginError::InvalidModule(_))));
    }
    let tiny = Limits { module_bytes: 8, ..Limits::default() };
    assert!(matches!(check_module(&module("", "(i32.const 0)"), &tiny), Err(PluginError::InvalidModule(m)) if m.contains("larger")));
}

#[test]
fn mangled_modules_never_panic() {
    // A cheap fuzz: flip bytes of a valid module all over and run what still compiles.
    let base = module(
        r#"(data (i32.const 0) "node.get")"#,
        "(drop (call $call (i32.const 0) (i32.const 8) (i32.const 0) (i32.const 0))) (loop $l (br_if $l (i32.const 0))) (i32.const 0)",
    );
    let mut seed = 0x2545_f491_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let limits = Limits { fuel: 100_000, ..Limits::default() };
    for _ in 0..600 {
        let mut m = base.clone();
        for _ in 0..1 + next() % 4 {
            let i = next() as usize % m.len();
            m[i] = next() as u8;
        }
        if next() % 5 == 0 {
            m.truncate(next() as usize % m.len());
        }
        let _ = check_module(&m, &limits);
        let mut host = Mock::default();
        let _ = run(&m, "{}", &limits, &mut host);
    }
}

fn manifest(extra: Value) -> Result<Manifest, String> {
    let mut v = json!({"id": "org.example.tidy", "name": "Tidy", "version": "1.0.0", "main": "tidy.wasm"});
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    Manifest::from_value(&v)
}

#[test]
fn manifests_validate_and_fill_defaults() {
    let m = manifest(json!({})).unwrap();
    assert_eq!(m.permissions.access, Access::Read, "read-only unless asked");
    assert_eq!(m.commands.len(), 1, "one `run` command when none are listed");
    assert_eq!(m.command(None).unwrap().id, "run");
    let m = manifest(json!({
        "permissions": {"access": "write", "commands": ["node.*", "shape.rectangle"]},
        "commands": [{"id": "grid", "label": "Swatch grid", "params": [
            {"name": "columns", "type": "number", "default": 4, "min": 1, "max": 12},
            {"name": "palette", "type": "text", "default": "#f00,#0f0"},
            {"name": "rounded", "type": "boolean"},
            {"name": "size", "type": "select", "options": ["S", "M", "L"], "default": "M"}
        ]}, {"id": "other", "label": ""}]
    }))
    .unwrap();
    assert!(m.permissions.allows("node.rename") && m.permissions.allows("shape.rectangle"));
    assert!(!m.permissions.allows("shape.ellipse") && !m.permissions.allows("file.save"));
    assert_eq!(m.command(Some("other")).unwrap().label, "other", "an empty label falls back to the id");
    let grid = m.command(Some("grid")).unwrap();
    let p = grid.resolve_params(&json!({"columns": 40, "extra": [1]})).unwrap();
    assert_eq!(p, json!({"columns": 12.0, "palette": "#f00,#0f0", "rounded": false, "size": "M", "extra": [1]}));
    assert_eq!(grid.resolve_params(&json!({"columns": "3"})).unwrap()["columns"], json!(3.0));
    assert!(grid.resolve_params(&json!({"size": "XL"})).is_err());
    assert!(grid.resolve_params(&json!({"rounded": "yes"})).is_err());
    assert!(grid.resolve_params(&json!({"columns": "NaN"})).is_err());
    assert!(grid.resolve_params(&json!([1])).is_err());
    assert!(m.command(Some("missing")).is_none());
}

#[test]
fn bad_manifests_are_refused() {
    let bad = [
        json!({"id": ""}),
        json!({"id": "has space"}),
        json!({"id": "x".repeat(200)}),
        json!({"name": ""}),
        json!({"name": "line\nbreak"}),
        json!({"main": "../escape.wasm"}),
        json!({"main": "/abs/plugin.wasm"}),
        json!({"main": "C:\\plugin.wasm"}),
        json!({"main": "plugin.js"}),
        json!({"main": "a//b.wasm"}),
        json!({"api": crate::API_VERSION + 1}),
        json!({"api": 0}),
        json!({"permissions": {"access": "root"}}),
        json!({"permissions": {"commands": ["*.x"]}}),
        json!({"permissions": {"commands": [""]}}),
        json!({"commands": [{"id": "a", "label": "A"}, {"id": "a", "label": "B"}]}),
        json!({"commands": [{"id": "a", "label": "A", "params": [{"name": "s", "type": "select"}]}]}),
        json!({"commands": [{"id": "a", "label": "A", "params": [{"name": "n", "type": "number", "min": 5, "max": 1}]}]}),
        json!({"commands": [{"id": "a", "label": "A", "params": [{"name": "n"}, {"name": "n"}]}]}),
        json!({"commands": [{"id": "a", "label": "A", "params": [{"name": "n", "type": "date"}]}]}),
        json!({"commands": [{"id": "a", "label": "A", "params": [{"name": "i", "type": "image", "data": "iVBORw0KGgo="}]}]}),
        json!({"commands": "run"}),
    ];
    for extra in bad {
        assert!(manifest(extra.clone()).is_err(), "accepted {extra}");
    }
    let deep = "[".repeat(10_000);
    let huge = format!("{{\"id\":\"{}\"}}", "a".repeat(300_000));
    for junk in ["", "null", "[]", "{", "\"id\"", "{\"id\": 5}", &deep, &huge] {
        assert!(Manifest::parse(junk).is_err(), "accepted {}", junk.chars().take(40).collect::<String>());
    }
}

#[test]
fn api_may_be_a_version_string() {
    assert_eq!(manifest(json!({"api": "1.0.0"})).unwrap().api, 1);
    assert_eq!(manifest(json!({"api": "2"})).unwrap().api, 2);
    assert_eq!(manifest(json!({"api": 2})).unwrap().api, 2);
    assert!(manifest(json!({"api": format!("{}.0.0", crate::API_VERSION + 1)})).is_err());
    assert!(manifest(json!({"api": "x"})).is_err());
}
