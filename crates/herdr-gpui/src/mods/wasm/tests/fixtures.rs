//! Fake hosts and installs for the tests that start processes and sockets.

use super::*;
pub(super) use serde_json::{Value, json};
pub(super) use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// The token every fake host opens with: 48 hex digits.
pub(super) const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

/// Polls `poll` until it returns something, for at most ten seconds.
pub(super) fn eventually<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(found) = poll() {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

/// Installs a host `version` under `paths` that says it runs protocols 4
/// and 5 and, when started, runs `on_start` (a shell snippet with `$T`,
/// `$P`, and `$H` set to its token, port, and host files).
pub(super) fn install_host(paths: &Paths, version: &str, on_start: &str) {
    use std::os::unix::fs::PermissionsExt;
    let dir = paths.share.join("host").join(version);
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("wasm-mods-host");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = --version ]; then echo '{{\"version\": \"{version}\", \"protocol\": [4, 5]}}'; exit 0; fi\n\
             while [ $# -gt 0 ]; do case \"$1\" in\n\
               --token-file) T=$2;; --port-file) P=$2;; --host-file) H=$2;;\n\
             esac; shift 2; done\n\
             {on_start}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Writes the run file naming a host at `port`.
pub(super) fn write_run_file(paths: &Paths, port: u16, version: &str) {
    fs::create_dir_all(&paths.run).unwrap();
    fs::write(
        paths.run.join("host"),
        format!("{port} {TOKEN} {version}\n"),
    )
    .unwrap();
}

/// A host on a socket: it answers each hello with `greeting`, keeps every
/// line it is sent, and can drop its connections as a killed host would.
pub(super) struct FakeHost {
    pub(super) port: u16,
    pub(super) received: Arc<Mutex<Vec<Value>>>,
    pub(super) open: Arc<Mutex<Vec<TcpStream>>>,
    pub(super) connections: Arc<Mutex<usize>>,
}

impl FakeHost {
    pub(super) fn start(greeting: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(Mutex::new(Vec::new()));
        let open = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(Mutex::new(0));
        let (log, streams, count) = (received.clone(), open.clone(), connections.clone());
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                *count.lock().unwrap() += 1;
                streams.lock().unwrap().push(stream.try_clone().unwrap());
                let (log, greeting) = (log.clone(), greeting.clone());
                thread::spawn(move || serve(stream, &log, &greeting));
            }
        });
        Self {
            port,
            received,
            open,
            connections,
        }
    }

    /// Closes every open connection.
    pub(super) fn drop_connections(&self) {
        for stream in self.open.lock().unwrap().drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    pub(super) fn connections(&self) -> usize {
        *self.connections.lock().unwrap()
    }

    /// The first line received that `pick` accepts.
    pub(super) fn wait_for(&self, pick: impl Fn(&Value) -> bool) -> Value {
        eventually(|| {
            self.received
                .lock()
                .unwrap()
                .iter()
                .find(|line| pick(line))
                .cloned()
        })
    }

    /// Every event received, in order.
    pub(super) fn events(&self) -> Vec<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter_map(|line| line.pointer("/events/events"))
            .filter_map(Value::as_array)
            .flatten()
            .cloned()
            .collect()
    }
}

fn serve(stream: TcpStream, log: &Mutex<Vec<Value>>, greeting: &[Value]) {
    let mut writer = stream.try_clone().unwrap();
    let mut lines = BufReader::new(stream).lines();
    let Some(Ok(hello)) = lines.next() else {
        return;
    };
    log.lock()
        .unwrap()
        .push(serde_json::from_str(&hello).unwrap());
    for line in greeting {
        if writeln!(writer, "{line}").is_err() {
            return;
        }
    }
    for line in lines.map_while(Result::ok) {
        log.lock()
            .unwrap()
            .push(serde_json::from_str(&line).unwrap());
    }
}

pub(super) fn ready() -> Value {
    json!({ "ready": { "name": "demo", "description": "", "load_ms": 1.0 } })
}

/// A frame drawing `ui`, offering `actions`.
pub(super) fn frame(ui: Value, actions: Value) -> Value {
    json!({ "frame": {
        "title": "demo", "ui": ui, "actions": actions,
        "ack": 0, "call_ms": 0.0, "got": 0.0, "sent": 0.0,
    } })
}

/// A target for module `demo` with its paths under `root`.
pub(super) fn target(root: &Path) -> Target {
    Target {
        paths: Paths::under(root),
        mod_dir: root.join("mods/demo"),
        mod_name: "demo".into(),
        module: "demo".into(),
    }
}
