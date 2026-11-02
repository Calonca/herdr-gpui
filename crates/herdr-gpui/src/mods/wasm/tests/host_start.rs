use super::{fixtures::*, *};
use host::{Version, parse_run_file};

#[test]
fn versions_compare_by_number_and_ignore_pre_releases() {
    assert_eq!(Version::parse("0.2.0"), Some(Version(0, 2, 0)));
    assert_eq!(Version::parse("1.10.3-rc1"), Some(Version(1, 10, 3)));
    assert!(Version(0, 10, 0) > Version(0, 9, 9));
    for bad in ["", "1.2", "1.2.3.4", "v1.2.3", "a.b.c"] {
        assert_eq!(Version::parse(bad), None, "{bad}");
    }
}

#[test]
fn the_run_file_names_port_token_and_version() {
    let (endpoint, version) = parse_run_file(&format!("4242 {TOKEN} 0.2.0\n")).unwrap();
    assert_eq!((endpoint.port, endpoint.token.as_str()), (4242, TOKEN));
    assert_eq!(version, Version(0, 2, 0));
    // A host from before releases wrote no version.
    let (_, version) = parse_run_file(&format!("4242 {TOKEN}\n")).unwrap();
    assert_eq!(version, Version(0, 0, 0));
    for bad in [
        String::new(),
        format!("4242 {TOKEN} 0.2.0"),
        "4242 short 0.2.0\n".to_owned(),
        format!("port {TOKEN} 0.2.0\n"),
        format!("4242 {TOKEN} 0.2.0 extra\n"),
    ] {
        assert!(parse_run_file(&bad).is_none(), "{bad:?}");
    }
}

#[test]
fn the_token_stays_out_of_debug_output() {
    let (endpoint, _) = parse_run_file(&format!("4242 {TOKEN} 0.2.0\n")).unwrap();
    assert!(!format!("{endpoint:?}").contains(TOKEN));
}

#[test]
fn no_installed_host_is_reported() {
    let root = tempfile::tempdir().unwrap();
    let error = host::ensure(&Paths::under(root.path()), 5).unwrap_err();
    assert!(matches!(error, Error::NoHost { protocol: 5 }), "{error:?}");
}

#[test]
fn the_newest_host_that_runs_the_protocol_is_picked() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    install_host(&paths, "0.1.0", "");
    install_host(&paths, "0.2.0", "");
    // A folder that is not a host, and one whose host names another version.
    fs::create_dir_all(paths.share.join("host/0.3.0")).unwrap();
    install_host(&paths, "0.4.0", "");
    fs::rename(
        paths.share.join("host/0.4.0"),
        paths.share.join("host/0.5.0"),
    )
    .unwrap();
    let hosts = host::installed(&paths);
    assert_eq!(hosts.len(), 2);
    let best = host::pick(&hosts, 5).unwrap();
    assert_eq!(best.version, Version(0, 2, 0));
    assert!(host::pick(&hosts, 6).is_none());
}

/// Starts a fake host that writes `port` as it listens, as the real one does.
fn listening(port: u16, starts: &Path) -> String {
    format!(
        "echo started >> '{}'\n\
         printf '{port}\\n' > \"$P.tmp\" && mv \"$P.tmp\" \"$P\"\n\
         printf '{port} %s 0.2.0\\n' \"$(cat \"$T\")\" > \"$H\"",
        starts.display()
    )
}

#[test]
fn a_host_is_started_once_and_shared_through_the_run_file() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    let starts = root.path().join("starts");
    install_host(&paths, "0.2.0", &listening(4242, &starts));
    let first = host::ensure(&paths, 5).unwrap();
    assert_eq!(first.port, 4242);
    assert_eq!(first.token.len(), 48);
    // The token file and port file are gone; only the run file stays.
    let left: Vec<String> = fs::read_dir(&paths.run)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, ["host"]);
    let second = host::ensure(&paths, 5).unwrap();
    assert_eq!(second, first);
    assert_eq!(fs::read_to_string(&starts).unwrap().lines().count(), 1);
}

#[cfg(unix)]
#[test]
fn the_run_folder_is_the_owners_alone() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    install_host(
        &paths,
        "0.2.0",
        &listening(4242, &root.path().join("starts")),
    );
    fs::create_dir_all(&paths.run).unwrap();
    fs::set_permissions(&paths.run, fs::Permissions::from_mode(0o755)).unwrap();
    host::ensure(&paths, 5).unwrap();
    let mode = fs::metadata(&paths.run).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn a_forgotten_host_is_started_again() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    let starts = root.path().join("starts");
    install_host(&paths, "0.2.0", &listening(4242, &starts));
    write_run_file(&paths, 1111, "0.2.0");
    assert_eq!(host::ensure(&paths, 5).unwrap().port, 1111);
    // Another port is not this host's to forget.
    host::forget(&paths, 2222);
    assert_eq!(host::ensure(&paths, 5).unwrap().port, 1111);
    host::forget(&paths, 1111);
    assert_eq!(host::ensure(&paths, 5).unwrap().port, 4242);
    assert_eq!(fs::read_to_string(&starts).unwrap().lines().count(), 1);
}

#[test]
fn a_newer_installed_host_takes_over_from_an_older_running_one() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    install_host(&paths, "0.1.0", "");
    install_host(
        &paths,
        "0.2.0",
        &listening(4242, &root.path().join("starts")),
    );
    write_run_file(&paths, 1111, "0.1.0");
    assert_eq!(host::ensure(&paths, 5).unwrap().port, 4242);
}

#[test]
fn a_stale_start_lock_is_taken_over() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    install_host(
        &paths,
        "0.2.0",
        &listening(4242, &root.path().join("starts")),
    );
    let lock = paths.run.join("starting");
    fs::create_dir_all(&lock).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(60);
    fs::File::open(&lock).unwrap().set_modified(old).unwrap();
    assert_eq!(host::ensure(&paths, 5).unwrap().port, 4242);
    assert!(!lock.exists());
}

#[test]
fn a_host_that_exits_before_listening_is_reported() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    install_host(&paths, "0.2.0", "exit 3");
    let error = host::ensure(&paths, 5).unwrap_err();
    assert!(
        matches!(&error, Error::Exited { version, .. } if version == "0.2.0"),
        "{error:?}"
    );
    assert!(!paths.run.join("starting").exists());
}

#[test]
fn a_mods_own_module_is_installed_under_its_name() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    let dir = root.path().join("mods/wasm-todo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("todo.wasm"), b"\0asm one").unwrap();
    let name = host::install_module(&paths, &dir, "wasm-todo", "todo").unwrap();
    assert_eq!(name, "wasm-todo");
    let installed = paths.share.join("modules/wasm-todo.wasm");
    assert_eq!(fs::read(&installed).unwrap(), b"\0asm one");
    // Unchanged, it is left as it is; changed, it is replaced.
    let before = fs::metadata(&installed).unwrap().modified().unwrap();
    host::install_module(&paths, &dir, "wasm-todo", "todo").unwrap();
    assert_eq!(
        fs::metadata(&installed).unwrap().modified().unwrap(),
        before
    );
    fs::write(dir.join("todo.wasm"), b"\0asm two").unwrap();
    host::install_module(&paths, &dir, "wasm-todo", "todo").unwrap();
    assert_eq!(fs::read(&installed).unwrap(), b"\0asm two");
    let modules: Vec<_> = fs::read_dir(paths.share.join("modules")).unwrap().collect();
    assert_eq!(modules.len(), 1, "no temporary copy is left");
}

#[test]
fn without_its_own_module_a_mod_runs_the_installed_one() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    let dir = root.path().join("mods/wasm-todo");
    fs::create_dir_all(&dir).unwrap();
    let name = host::install_module(&paths, &dir, "wasm-todo", "todo").unwrap();
    assert_eq!(name, "todo");
    assert!(!paths.share.join("modules").exists());
}
