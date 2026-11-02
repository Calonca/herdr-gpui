use super::{fixtures::*, *};
use link::Link;

#[test]
fn a_new_build_is_installed_over_the_open_connection() {
    let root = tempfile::tempdir().unwrap();
    let host = FakeHost::start(vec![ready()]);
    let target = target(root.path());
    install_host(&target.paths, "0.2.0", "exit 1");
    write_run_file(&target.paths, host.port, "0.2.0");
    fs::create_dir_all(&target.mod_dir).unwrap();
    fs::write(target.mod_dir.join("demo.wasm"), b"one").unwrap();
    let installed = target.paths.share.join("modules/demo.wasm");
    let link = Link::spawn(target.clone()).unwrap();
    host.wait_for(|line| line.get("hello").is_some());
    assert_eq!(fs::read(&installed).unwrap(), b"one");

    fs::write(root.path().join("mods/demo/demo.wasm"), b"two").unwrap();
    assert!(link.reinstall("demo"));
    eventually(|| (fs::read(&installed).ok()? == b"two").then_some(()));
    // The host swaps the build in itself; the view keeps its connection.
    thread::sleep(Duration::from_millis(300));
    assert_eq!(host.connections(), 1);
}

#[test]
fn another_module_needs_a_new_link() {
    let root = tempfile::tempdir().unwrap();
    let host = FakeHost::start(vec![ready()]);
    let target = target(root.path());
    install_host(&target.paths, "0.2.0", "exit 1");
    write_run_file(&target.paths, host.port, "0.2.0");
    let link = Link::spawn(target).unwrap();
    assert!(!link.reinstall("other"));
}

#[test]
fn what_arrives_wakes_the_window() {
    let root = tempfile::tempdir().unwrap();
    let host = FakeHost::start(vec![ready()]);
    let target = target(root.path());
    install_host(&target.paths, "0.2.0", "exit 1");
    write_run_file(&target.paths, host.port, "0.2.0");
    let link = Link::spawn(target).unwrap();
    let arrivals = link.arrivals();
    eventually(|| arrivals.try_recv().ok());
}
