use super::*;
use herdr_mods::Sources;

/// The shipped WASM mod `name`'s files.
fn example(manifest: &[u8], icon: &[u8]) -> Sources {
    Sources::new([
        ("mod.toml".to_owned(), manifest.to_vec()),
        ("icon.svg".to_owned(), icon.to_vec()),
    ])
    .unwrap()
}

#[test]
fn the_shipped_wasm_mods_are_apps_naming_their_modules() {
    let examples = [
        (
            example(
                include_bytes!("../../../mods/wasm-diff/mod.toml"),
                include_bytes!("../../../mods/wasm-diff/icon.svg"),
            ),
            "wasm-diff",
            "diff",
        ),
        (
            example(
                include_bytes!("../../../mods/wasm-hello/mod.toml"),
                include_bytes!("../../../mods/wasm-hello/icon.svg"),
            ),
            "wasm-hello",
            "hello",
        ),
        (
            example(
                include_bytes!("../../../mods/wasm-todo/mod.toml"),
                include_bytes!("../../../mods/wasm-todo/icon.svg"),
            ),
            "wasm-todo",
            "todo",
        ),
    ];
    for (sources, name, module) in examples {
        assert_eq!(sources.manifest.name, name);
        assert_eq!(sources.manifest.wasm.as_ref().unwrap().module, module);
        let activity = sources.manifest.activity.as_ref().unwrap();
        assert!(
            sources
                .icon(&activity.icon)
                .unwrap()
                .contains("currentColor")
        );
        assert_eq!(sources.modules().count(), 0);
    }
}

#[test]
fn the_worker_reports_a_wasm_mod_without_running_luau() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("wasm-todo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("mod.toml"),
        include_bytes!("../../../mods/wasm-todo/mod.toml"),
    )
    .unwrap();
    fs::write(
        dir.join("icon.svg"),
        include_bytes!("../../../mods/wasm-todo/icon.svg"),
    )
    .unwrap();
    let worker = crate::mods::worker::Handle::spawn(dir).unwrap();
    let mut seen = Vec::new();
    let manifest = eventually(|| {
        let report = worker.take();
        let manifest = report.manifest.clone();
        seen.push(report);
        manifest
    });
    assert_eq!(manifest.wasm.unwrap().module, "todo");
    assert!(
        seen.iter()
            .all(|report| report.load_error.as_ref().is_none_or(Option::is_none))
    );
}
