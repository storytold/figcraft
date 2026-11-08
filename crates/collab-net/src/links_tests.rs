//! The `figcraft://` registration against a fake registry (tests never write the real one).

use std::collections::BTreeMap;
use std::path::PathBuf;

use figcraft_engine::invite_links::LinkRegistrar;

use crate::links::{KEY, Registry, UrlScheme, installed_build};

#[derive(Default)]
struct Fake {
    values: BTreeMap<(String, String), String>,
    deny: bool,
}

impl Registry for Fake {
    fn get(&self, key: &str, name: &str) -> Option<String> {
        self.values.get(&(key.to_string(), name.to_string())).cloned()
    }
    fn set(&mut self, key: &str, name: &str, value: &str) -> Result<(), String> {
        if self.deny {
            return Err("access denied".into());
        }
        self.values.insert((key.to_string(), name.to_string()), value.to_string());
        Ok(())
    }
    fn remove_tree(&mut self, key: &str) -> Result<(), String> {
        self.values.retain(|(k, _), _| k != key && !k.starts_with(&format!("{key}\\")));
        Ok(())
    }
}

fn exe(path: &str) -> PathBuf {
    PathBuf::from(path)
}

#[test]
fn registers_the_scheme_for_this_copy() {
    let mut s = UrlScheme::new(Fake::default(), exe(r"C:\Apps\ClawDesign\figcraft.exe"), true);
    assert!(!s.registered());
    s.set(true).unwrap();
    assert!(s.registered());
    let reg = s.registry();
    assert_eq!(reg.get(KEY, "").as_deref(), Some("URL:ClawDesign invite link"));
    assert_eq!(reg.get(KEY, "URL Protocol").as_deref(), Some(""), "marks the key as a URL scheme");
    assert_eq!(reg.get(&format!(r"{KEY}\DefaultIcon"), "").as_deref(), Some(r#""C:\Apps\ClawDesign\figcraft.exe",0"#));
    assert_eq!(reg.get(&format!(r"{KEY}\shell\open\command"), "").as_deref(), Some(r#""C:\Apps\ClawDesign\figcraft.exe" "%1""#));
    assert_eq!(reg.values.len(), 4, "nothing else is written");
    s.set(false).unwrap();
    assert!(!s.registered());
    assert!(s.registry().values.is_empty(), "unregistering removes the whole key");
}

#[test]
fn leaves_another_copys_registration() {
    let mut other = UrlScheme::new(Fake::default(), exe(r"C:\Other\figcraft.exe"), true);
    other.set(true).unwrap();
    let reg = Fake { values: other.registry().values.clone(), deny: false };
    let mut s = UrlScheme::new(reg, exe(r"C:\Apps\figcraft.exe"), true);
    assert!(!s.registered(), "another copy is the opener");
    s.set(false).unwrap();
    assert_eq!(s.registry().get(&format!(r"{KEY}\shell\open\command"), "").as_deref(), Some(r#""C:\Other\figcraft.exe" "%1""#));
    // Turning it on takes it over.
    s.set(true).unwrap();
    assert!(s.registered());
}

#[test]
fn errors_are_reported() {
    let mut s = UrlScheme::new(Fake { deny: true, ..Fake::default() }, exe(r"C:\Apps\figcraft.exe"), true);
    assert_eq!(s.set(true).unwrap_err(), "access denied");
    assert!(!s.registered());
}

#[test]
fn cargo_builds_are_not_installed() {
    let dir = std::env::temp_dir().join(format!("figcraft-links-{}", std::process::id()));
    let target = dir.join("target");
    let bin = target.join("release");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(target.join("CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55").unwrap();
    assert!(!installed_build(&bin.join("figcraft.exe")), "inside a Cargo build directory");
    let installed = dir.join("Programs").join("ClawDesign");
    std::fs::create_dir_all(&installed).unwrap();
    // Debug builds (these tests) never count as installed either.
    assert_eq!(installed_build(&installed.join("figcraft.exe")), !cfg!(debug_assertions));
    let _ = std::fs::remove_dir_all(&dir);
    // The test binary itself runs from a Cargo build directory.
    assert!(!installed_build(&std::env::current_exe().unwrap()));
}
