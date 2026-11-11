//! Release and source Flatpaks must allow release checks and host update helpers.
fn finish_args(manifest: &str) -> Vec<&str> {
    manifest
        .split("finish-args:")
        .nth(1)
        .unwrap()
        .lines()
        .take_while(|line| line.is_empty() || line.starts_with(' ') || line.starts_with('#'))
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(|arg| arg.trim_matches(['\'', '"']))
        .collect()
}

#[test]
fn flatpak_update_permissions() {
    let manifests = [
        include_str!("../../../packaging/linux/flatpak/ai.storyteller.figcraft.bundle.yml"),
        include_str!("../../../packaging/linux/flatpak/ai.storyteller.figcraft.yml"),
    ];
    let args = manifests.map(finish_args);
    assert_eq!(args[0], args[1], "source and release permissions must agree");
    for permissions in args {
        for required in ["--share=network", "--talk-name=org.freedesktop.Flatpak"] {
            assert!(permissions.contains(&required), "missing update permission: {required}");
        }
    }
}
