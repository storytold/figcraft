//! `save_file` (API 4): the manifest permission, the host call with its bytes, and the limits.

use serde_json::json;

use crate::{Host, Instance, Limits, Manifest, PluginError};

/// A host that records the files offered to it (and refuses names starting with `no`).
#[derive(Default)]
struct Saves(Vec<(String, String, Vec<u8>)>);

impl Host for Saves {
    fn call(&mut self, _command: &str, _params: &str) -> Result<String, String> {
        Ok("{}".into())
    }
    fn selection(&mut self) -> String {
        "[]".into()
    }
    fn notify(&mut self, _message: &str) {}
    fn save_file(&mut self, name: &str, mime: &str, data: &[u8]) -> Result<(), String> {
        if name.starts_with("no") {
            return Err("nope".into());
        }
        self.0.push((name.into(), mime.into(), data.to_vec()));
        Ok(())
    }
}

/// A host that keeps the default `save_file`.
struct Plain;

impl Host for Plain {
    fn call(&mut self, _command: &str, _params: &str) -> Result<String, String> {
        Ok("{}".into())
    }
    fn selection(&mut self) -> String {
        "[]".into()
    }
    fn notify(&mut self, _message: &str) {}
}

/// Memory: name at 0, MIME at 100, data at 200. `call` is the arguments of `save_file`; its status
/// is the run's status, and its reply is the output.
fn module(name: &str, mime: &str, data: &[u8], call: &str) -> Vec<u8> {
    let bytes = |b: &[u8]| b.iter().map(|b| format!("\\{b:02x}")).collect::<String>();
    let wat = format!(
        r#"(module
          (import "figcraft" "save_file" (func $save_file (param i32 i32 i32 i32 i32 i32) (result i32)))
          (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "figcraft" "set_output" (func $set_output (param i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{}") (data (i32.const 100) "{}") (data (i32.const 200) "{}")
          (func (export "figcraft_run") (param $n i32) (result i32) (local $st i32)
            (local.set $st {call})
            (call $set_output (i32.const 1000) (call $reply_read (i32.const 1000) (i32.const 100)))
            (local.get $st)))"#,
        bytes(name.as_bytes()),
        bytes(mime.as_bytes()),
        bytes(data)
    );
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

fn call(name: &str, mime: &str, data: &[u8]) -> String {
    format!(
        "(call $save_file (i32.const 0) (i32.const {}) (i32.const 100) (i32.const {}) (i32.const 200) (i32.const {}))",
        name.len(),
        mime.len(),
        data.len()
    )
}

fn manifest(api: u32, save_file: bool) -> String {
    json!({"id": "org.test.save", "name": "Save", "main": "p.wasm", "api": api, "permissions": {"saveFile": save_file}}).to_string()
}

#[test]
fn the_host_gets_the_name_the_type_and_the_exact_bytes() {
    let data = [0u8, 1, 2, 255, 0x80, b'x'];
    let wasm = module("out.jsx", "text/plain", &data, &call("out.jsx", "text/plain", &data));
    let mut host = Saves::default();
    Instance::new(&wasm, &Limits::default()).unwrap().call("{}", &mut host).unwrap();
    assert_eq!(host.0, vec![("out.jsx".to_string(), "text/plain".to_string(), data.to_vec())]);
}

#[test]
fn a_refusal_returns_status_one_with_the_reason() {
    let wasm = module("no.txt", "text/plain", b"x", &call("no.txt", "text/plain", b"x"));
    // The module returns the status as its own, so the run fails with the reason as its message.
    let err = Instance::new(&wasm, &Limits::default()).unwrap().call("{}", &mut Saves::default()).unwrap_err();
    assert_eq!(err, PluginError::Failed("nope".into()));
}

#[test]
fn a_host_without_saving_refuses_by_default() {
    let wasm = module("a.txt", "text/plain", b"x", &call("a.txt", "text/plain", b"x"));
    let err = Instance::new(&wasm, &Limits::default()).unwrap().call("{}", &mut Plain).unwrap_err();
    assert_eq!(err, PluginError::Failed("saving files isn't available here".into()));
}

#[test]
fn too_much_data_is_refused_and_bad_utf8_a_trap() {
    let limits = Limits { max_message: 64, ..Limits::default() };
    let data = [b'x'; 65];
    let wasm = module("a.txt", "text/plain", &data, &call("a.txt", "text/plain", &data));
    let mut host = Saves::default();
    // Status 1 with the reason, which the module passes on as its own failure.
    let err = Instance::new(&wasm, &limits).unwrap().call("{}", &mut host).unwrap_err();
    assert_eq!(err, PluginError::Failed("a file to save is limited to 64 bytes, this one has 65".into()));
    assert!(host.0.is_empty());

    // The name points at a lone 0xff byte: not UTF-8.
    let wasm = module(
        "a",
        "text/plain",
        &[0xff],
        "(call $save_file (i32.const 200) (i32.const 1) (i32.const 100) (i32.const 1) (i32.const 200) (i32.const 1))",
    );
    let err = Instance::new(&wasm, &Limits::default()).unwrap().call("{}", &mut host).unwrap_err();
    assert!(matches!(err, PluginError::Trap(_)), "{err:?}");
    assert!(host.0.is_empty());
}

#[test]
fn a_wrong_signature_is_rejected() {
    let wat = r#"(module (import "figcraft" "save_file" (func (param i32 i32) (result i32)))
        (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) (i32.const 0)))"#;
    let wasm = wat::parse_str(wat).unwrap();
    assert!(matches!(crate::check_module(&wasm, &Limits::default()), Err(PluginError::InvalidModule(m)) if m.contains("wrong signature")));
}

#[test]
fn the_permission_needs_api_four_and_survives_a_round_trip() {
    let err = Manifest::parse(&manifest(3, true)).unwrap_err();
    assert!(err.contains("saveFile needs plugin API 4"), "{err}");
    let m = Manifest::parse(&manifest(4, true)).unwrap();
    assert!(m.permissions.save_file);
    let again = Manifest::parse(&serde_json::to_string(&m).unwrap()).unwrap();
    assert_eq!(m, again);
    assert!(serde_json::to_string(&m).unwrap().contains("\"saveFile\":true"));
    // Older manifests stay valid and don't grow the key.
    let old = Manifest::parse(&manifest(3, false)).unwrap();
    assert!(!old.permissions.save_file);
    assert!(!serde_json::to_string(&old).unwrap().contains("saveFile"));
}
