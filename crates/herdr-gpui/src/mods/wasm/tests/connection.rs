use super::{fixtures::*, *};
use link::{Link, Status};
use wasm_mods_protocol::wire::{In, Message};

/// A link to `host`, installed as version 0.2.0 under `root`.
fn link_to(root: &Path, host: &FakeHost) -> Link {
    let target = target(root);
    install_host(&target.paths, "0.2.0", "exit 1");
    write_run_file(&target.paths, host.port, "0.2.0");
    Link::spawn(target).unwrap()
}

/// Waits for a status that `pick` accepts.
fn status(link: &Link, pick: impl Fn(&Status) -> bool) -> Status {
    eventually(|| link.take().status.filter(|status| pick(status)))
}

#[test]
fn the_link_greets_the_host_and_takes_its_frames() {
    let root = tempfile::tempdir().unwrap();
    let ui = serde_json::to_value(wasm_mods_protocol::ui::text("hi")).unwrap();
    let host = FakeHost::start(vec![ready(), frame(ui, json!([]))]);
    let link = link_to(root.path(), &host);
    let hello = host.wait_for(|line| line.get("hello").is_some());
    assert_eq!(hello["hello"]["token"], TOKEN);
    assert_eq!(hello["hello"]["module"], "demo");
    assert_eq!(hello["hello"]["native_canvas"], true);
    let frame = eventually(|| link.take().frame);
    assert!(matches!(*frame, Message::Frame { .. }));
    link.send(&In::Size {
        width: 640.,
        height: 480.,
    });
    let size = host.wait_for(|line| line.get("size").is_some());
    assert_eq!(size["size"]["width"], 640.);
}

#[test]
fn only_the_newest_frame_waits_for_the_view() {
    let root = tempfile::tempdir().unwrap();
    let text = |text| serde_json::to_value(wasm_mods_protocol::ui::text(text)).unwrap();
    let host = FakeHost::start(vec![
        ready(),
        frame(text("one"), json!([])),
        frame(text("two"), json!([])),
    ]);
    let link = link_to(root.path(), &host);
    // Both frames arrive before the view looks.
    thread::sleep(Duration::from_millis(300));
    let frame = eventually(|| link.take().frame);
    let Message::Frame { ui, .. } = *frame else {
        panic!("not a frame");
    };
    assert_eq!(ui, wasm_mods_protocol::ui::text("two"));
}

#[test]
fn the_link_connects_again_after_the_host_goes_away() {
    let root = tempfile::tempdir().unwrap();
    let host = FakeHost::start(vec![ready()]);
    let link = link_to(root.path(), &host);
    status(&link, |status| matches!(status, Status::Connected));
    host.drop_connections();
    // The run file still names the same port, where the host answers again.
    status(&link, |status| matches!(status, Status::Connected));
    assert_eq!(host.connections(), 2);
    let hellos = host
        .received
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.get("hello").is_some())
        .count();
    assert_eq!(hellos, 2);
}

#[test]
fn a_host_that_refuses_the_module_is_not_asked_again() {
    let root = tempfile::tempdir().unwrap();
    let refusal = json!({ "error": { "message": "no module named demo" } });
    let host = FakeHost::start(vec![refusal]);
    let link = link_to(root.path(), &host);
    host.wait_for(|line| line.get("hello").is_some());
    // The host closes once it said why.
    thread::sleep(Duration::from_millis(100));
    host.drop_connections();
    let refused = status(&link, |status| matches!(status, Status::Failed(_)));
    let Status::Failed(Error::Host(message)) = refused else {
        panic!("expected the host's refusal, got {refused:?}");
    };
    assert_eq!(message, "no module named demo");
    thread::sleep(Duration::from_millis(800));
    assert_eq!(host.connections(), 1);
}

#[test]
fn a_dead_port_is_forgotten() {
    let root = tempfile::tempdir().unwrap();
    let target = target(root.path());
    install_host(&target.paths, "0.2.0", "exit 1");
    // Nothing listens on a port taken and let go.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    write_run_file(&target.paths, port, "0.2.0");
    let run_file = target.paths.run.join("host");
    let link = Link::spawn(target).unwrap();
    let failed = status(&link, |status| matches!(status, Status::Failed(_)));
    assert!(
        matches!(
            failed,
            Status::Failed(Error::Connect { .. }) | Status::Failed(Error::Exited { .. })
        ),
        "{failed:?}"
    );
    eventually(|| (!run_file.exists()).then_some(()));
}

#[test]
fn dropping_the_link_closes_the_connection() {
    let root = tempfile::tempdir().unwrap();
    let host = FakeHost::start(vec![ready()]);
    let link = link_to(root.path(), &host);
    status(&link, |status| matches!(status, Status::Connected));
    drop(link);
    // The host sees the connection close: a read on it ends.
    let stream = host.open.lock().unwrap().pop().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buffer = [0; 1];
    let read = std::io::Read::read(&mut &stream, &mut buffer);
    assert!(matches!(read, Ok(0) | Err(_)));
}

#[test]
fn a_host_that_closes_before_greeting_is_forgotten() {
    let root = tempfile::tempdir().unwrap();
    // Says nothing, as a host that does not know the token closes at once.
    let host = FakeHost::start(Vec::new());
    let target = target(root.path());
    let run_file = target.paths.run.join("host");
    let link = link_to(root.path(), &host);
    host.wait_for(|line| line.get("hello").is_some());
    host.drop_connections();
    let failed = status(&link, |status| matches!(status, Status::Failed(_)));
    assert!(
        matches!(failed, Status::Failed(Error::Closed)),
        "{failed:?}"
    );
    eventually(|| (!run_file.exists()).then_some(()));
}
