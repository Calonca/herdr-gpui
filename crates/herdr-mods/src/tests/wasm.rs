use super::*;

const WASM_MANIFEST: &str = "name = \"wasm-todo\"\napi = 1\n\
     [activity]\ntitle = \"Todo\"\nicon = \"icon.svg\"\n\
     [wasm]\nmodule = \"todo\"\n";
const ICON: &str = "<svg/>";

fn problem(text: &str) -> (String, Problem) {
    match Manifest::parse(text).unwrap_err() {
        Error::Manifest { at, problem } => (at, problem),
        other => panic!("expected a manifest error, got {other:?}"),
    }
}

#[test]
fn a_wasm_manifest_names_its_module() {
    let manifest = Manifest::parse(WASM_MANIFEST).unwrap();
    assert_eq!(manifest.wasm.unwrap().module, "todo");
    assert!(Manifest::parse(MANIFEST).unwrap().wasm.is_none());
}

#[test]
fn a_wasm_module_name_is_plain() {
    let underscored = WASM_MANIFEST.replace("\"todo\"", "\"git_graph\"");
    assert_eq!(
        Manifest::parse(&underscored).unwrap().wasm.unwrap().module,
        "git_graph"
    );
    for bad in ["\"../todo\"", "\"Todo\"", "\"\"", "\"a/b\""] {
        let (at, _) = problem(&WASM_MANIFEST.replace("\"todo\"", bad));
        assert_eq!(at, "wasm.module", "{bad}");
    }
}

#[test]
fn a_wasm_mod_is_an_activity_app() {
    let (at, problem) = problem("name = \"wasm-todo\"\napi = 1\n[wasm]\nmodule = \"todo\"\n");
    assert_eq!(at, "wasm");
    assert!(matches!(problem, Problem::Expected { .. }), "{problem:?}");
}

#[test]
fn a_wasm_mod_needs_no_main_luau() {
    let sources = Sources::new(files(&[("mod.toml", WASM_MANIFEST), ("icon.svg", ICON)])).unwrap();
    assert_eq!(sources.manifest.wasm.as_ref().unwrap().module, "todo");
    assert_eq!(sources.modules().count(), 0);
}

#[test]
fn a_wasm_mod_has_no_luau() {
    let error = Sources::new(files(&[
        ("mod.toml", WASM_MANIFEST),
        ("icon.svg", ICON),
        ("main.luau", "return 1"),
    ]))
    .unwrap_err();
    assert!(matches!(error, Error::WasmWithLuau), "{error:?}");
}

#[test]
fn a_new_build_of_a_folders_module_changes_its_stamp() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("mod.toml"), WASM_MANIFEST).unwrap();
    let before = Stamp::take(root.path()).unwrap();
    std::fs::write(root.path().join("todo.wasm"), b"\0asm").unwrap();
    assert_ne!(Stamp::take(root.path()).unwrap(), before);
}
