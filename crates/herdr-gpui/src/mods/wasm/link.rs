//! One WASM mod's connection to wasm-mods-host, on a thread of its own: it
//! installs the module, finds or starts a host, says hello, and reads what
//! the host sends; a second thread writes what the view sends. When the
//! host goes away it finds or starts one again and says hello again.
//!
//! The view's end is mailboxes, as a Luau mod worker's: the newest frame
//! (an older one not yet drawn is dropped), a short queue of the host's
//! other messages, the connection's status, and a bounded queue of lines
//! out. A one-slot wake tells the window something arrived, so it takes it
//! at once rather than on its next tick; nothing here waits on the window.

use super::{
    Error,
    host::{self, Endpoint, Paths},
};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpStream},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::Duration,
};
use wasm_mods_protocol::wire::{In, Message};

/// The host's messages other than frames kept until the view takes them.
const MAX_MESSAGES: usize = 32;
/// Lines the view may queue for the host before more are dropped.
const MAX_OUT: usize = 256;
/// The longest line read from the host: a frame of a large diff.
const LINE_MAX: u64 = 32 << 20;
/// Waits between tries to reach a host again, growing to the last.
const BACKOFF: [Duration; 4] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];
/// The wait before looking again for a host that is not installed.
const NO_HOST_RETRY: Duration = Duration::from_secs(5);

/// What the link shows: the module, where its files are, and where hosts are.
#[derive(Clone, Debug)]
pub(crate) struct Target {
    pub paths: Paths,
    pub mod_dir: PathBuf,
    pub mod_name: String,
    pub module: String,
}

/// Where the connection is.
#[derive(Debug)]
pub(crate) enum Status {
    /// Finding or starting a host, or connecting to it.
    Connecting,
    /// Connected and greeted: the view sends its size, theme, and folder
    /// again to each new connection.
    Connected,
    /// The host went away; finding another.
    Lost,
    /// Cannot connect, or the host refused the module, and why. A refusal
    /// is final for this link.
    Failed(Error),
}

/// What arrived since the view last took it.
#[derive(Default)]
pub(crate) struct Inbound {
    pub frame: Option<Box<Message>>,
    pub messages: Vec<Message>,
    pub status: Option<Status>,
}

#[derive(Default)]
struct FromHost {
    frame: Option<Box<Message>>,
    messages: VecDeque<Message>,
    status: Option<Status>,
}

#[derive(Default)]
struct ToHost {
    lines: VecDeque<String>,
    /// The connection the lines are for; the writer of an older one stops.
    generation: u64,
    stopped: bool,
    /// The open connection, so dropping the view can end its blocked read.
    stream: Option<TcpStream>,
    /// The name the open connection said hello with.
    module: Option<String>,
}

struct Shared {
    from_host: Mutex<FromHost>,
    to_host: Mutex<ToHost>,
    wake: Condvar,
    /// Tells the window something arrived, so it takes it now rather than
    /// on its next tick. One slot: a wake already pending covers the rest.
    arrived: async_channel::Sender<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The view's end. Dropping it stops the threads; nothing waits for them.
pub(crate) struct Link {
    shared: Arc<Shared>,
    target: Target,
    arrivals: async_channel::Receiver<()>,
}

impl Link {
    pub(crate) fn spawn(target: Target) -> Result<Self, Error> {
        let (arrived, arrivals) = async_channel::bounded(1);
        let shared = Arc::new(Shared {
            from_host: Mutex::default(),
            to_host: Mutex::default(),
            wake: Condvar::new(),
            arrived,
        });
        let worker = Arc::clone(&shared);
        let kept = target.clone();
        thread::Builder::new()
            .name(format!("wasm-mod-{}", target.mod_name))
            .spawn(move || run(&target, &worker))
            .map_err(Error::Thread)?;
        Ok(Self {
            shared,
            target: kept,
            arrivals,
        })
    }

    /// A new build of the same module: installs it over the old one, and the
    /// host swaps it in on the open connection, keeping the module's state.
    /// False when `module` is another module, which needs a new link. Should
    /// the new build install under another name (the folder gained or lost
    /// its own `.wasm`), the connection is dropped and the link says hello
    /// again with the new name.
    pub(crate) fn reinstall(&self, module: &str) -> bool {
        if module != self.target.module {
            return false;
        }
        let (target, shared) = (self.target.clone(), Arc::clone(&self.shared));
        let installing = thread::Builder::new()
            .name(format!("wasm-mod-install-{}", target.mod_name))
            .spawn(move || {
                let installed = host::install_module(
                    &target.paths,
                    &target.mod_dir,
                    &target.mod_name,
                    &target.module,
                );
                match installed {
                    Ok(name) => {
                        let out = lock(&shared.to_host);
                        if out.module.as_deref() != Some(name.as_str())
                            && let Some(stream) = &out.stream
                        {
                            let _ = stream.shutdown(Shutdown::Both);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "a new build of a WASM mod was not installed")
                    }
                }
            });
        installing.is_ok()
    }

    /// Queues `message` for the host; dropped while not connected, or when
    /// the host has not read the last [`MAX_OUT`] lines.
    pub(crate) fn send(&self, message: &In) {
        let Ok(line) = serde_json::to_string(message) else {
            return;
        };
        let mut out = lock(&self.shared.to_host);
        if out.stream.is_none() {
            return;
        }
        if out.lines.len() == MAX_OUT {
            tracing::warn!("wasm-mods-host is not reading; a message to it was dropped");
            return;
        }
        out.lines.push_back(line);
        drop(out);
        self.shared.wake.notify_all();
    }

    /// Ready whenever something arrived since the last [`Link::take`]; it
    /// closes once the link's threads are gone.
    pub(crate) fn arrivals(&self) -> async_channel::Receiver<()> {
        self.arrivals.clone()
    }

    pub(crate) fn take(&self) -> Inbound {
        let mut inbox = lock(&self.shared.from_host);
        Inbound {
            frame: inbox.frame.take(),
            messages: inbox.messages.drain(..).collect(),
            status: inbox.status.take(),
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let mut out = lock(&self.shared.to_host);
        out.stopped = true;
        if let Some(stream) = out.stream.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        drop(out);
        self.shared.wake.notify_all();
    }
}

fn report(shared: &Shared, status: Status) {
    lock(&shared.from_host).status = Some(status);
    arrived(shared);
}

/// Wakes the window; a full slot means a wake is already on its way.
fn arrived(shared: &Shared) {
    let _ = shared.arrived.try_send(());
}

/// Whether the view let go.
fn stopped(shared: &Shared) -> bool {
    lock(&shared.to_host).stopped
}

/// Waits `wait`, or less when the view lets go; whether to go on.
fn pause(shared: &Shared, wait: Duration) -> bool {
    let out = lock(&shared.to_host);
    let (out, _) = shared
        .wake
        .wait_timeout_while(out, wait, |out| !out.stopped)
        .unwrap_or_else(PoisonError::into_inner);
    !out.stopped
}

/// Waits until the view lets go.
fn wait_stopped(shared: &Shared) {
    let out = lock(&shared.to_host);
    let _stopped = shared
        .wake
        .wait_while(out, |out| !out.stopped)
        .unwrap_or_else(PoisonError::into_inner);
}

fn run(target: &Target, shared: &Arc<Shared>) {
    let mut generation = 0;
    let mut tries = 0;
    while !stopped(shared) {
        report(shared, Status::Connecting);
        let prepared = host::install_module(
            &target.paths,
            &target.mod_dir,
            &target.mod_name,
            &target.module,
        )
        .and_then(|module| {
            let endpoint = host::ensure(&target.paths, wasm_mods_protocol::PROTOCOL_VERSION)?;
            Ok((module, endpoint))
        });
        let (module, endpoint) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                report(shared, Status::Failed(error));
                if !pause(shared, NO_HOST_RETRY) {
                    return;
                }
                continue;
            }
        };
        generation += 1;
        match session(shared, &module, &endpoint, generation) {
            // Trying again would fail the same way; the window makes a new
            // view when the mod's files change.
            Ended::Refused(message) => {
                report(shared, Status::Failed(Error::Host(message)));
                return wait_stopped(shared);
            }
            Ended::Unreachable(error) => {
                host::forget(&target.paths, endpoint.port);
                report(shared, Status::Failed(error));
                let wait = BACKOFF[tries.min(BACKOFF.len() - 1)];
                tries += 1;
                if !pause(shared, wait) {
                    return;
                }
            }
            Ended::Lost => {
                tries = 0;
                report(shared, Status::Lost);
                if !pause(shared, BACKOFF[0]) {
                    return;
                }
            }
        }
    }
}

/// How a connection ended.
enum Ended {
    /// The host said why it cannot run the module, and closed.
    Refused(String),
    /// No host listened at the endpoint.
    Unreachable(Error),
    /// It went away, or the view let go.
    Lost,
}

/// One connection: hello, then lines until it closes.
fn session(shared: &Arc<Shared>, module: &str, endpoint: &Endpoint, generation: u64) -> Ended {
    let port = endpoint.port;
    let stream = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(stream) => stream,
        Err(source) => return Ended::Unreachable(Error::Connect { port, source }),
    };
    let _ = stream.set_nodelay(true);
    let (Ok(writer), Ok(shutdown)) = (stream.try_clone(), stream.try_clone()) else {
        return Ended::Lost;
    };
    let hello = In::Hello {
        token: endpoint.token.clone(),
        module: module.to_owned(),
        arg: None,
        images: None,
        open: None,
        native_canvas: true,
    };
    {
        let mut out = lock(&shared.to_host);
        if out.stopped {
            return Ended::Lost;
        }
        out.generation = generation;
        out.lines.clear();
        if let Ok(line) = serde_json::to_string(&hello) {
            out.lines.push_back(line);
        }
        out.stream = Some(shutdown);
        out.module = Some(module.to_owned());
    }
    let writing = Arc::clone(shared);
    let spawned = thread::Builder::new()
        .name("wasm-mod-write".into())
        .spawn(move || write_lines(&writing, writer, generation));
    if spawned.is_err() {
        return Ended::Lost;
    }
    let ended = read_lines(shared, stream);
    let mut out = lock(&shared.to_host);
    if let Some(stream) = out.stream.take() {
        let _ = stream.shutdown(Shutdown::Both);
    }
    out.lines.clear();
    drop(out);
    shared.wake.notify_all();
    ended
}

fn write_lines(shared: &Shared, mut stream: TcpStream, generation: u64) {
    loop {
        let lines: Vec<String> = {
            let out = lock(&shared.to_host);
            let mut out = shared
                .wake
                .wait_while(out, |out| {
                    !out.stopped
                        && out.generation == generation
                        && out.stream.is_some()
                        && out.lines.is_empty()
                })
                .unwrap_or_else(PoisonError::into_inner);
            if out.stopped || out.generation != generation || out.stream.is_none() {
                return;
            }
            out.lines.drain(..).collect()
        };
        for line in lines {
            let written = stream
                .write_all(line.as_bytes())
                .and_then(|()| stream.write_all(b"\n"));
            if written.is_err() {
                return;
            }
        }
    }
}

fn read_lines(shared: &Shared, stream: TcpStream) -> Ended {
    let mut reader = BufReader::with_capacity(1 << 16, stream);
    let mut line = Vec::new();
    let mut refused = None;
    let mut greeted = false;
    loop {
        line.clear();
        match (&mut reader)
            .take(LINE_MAX + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => break,
            Ok(_) if line.len() as u64 > LINE_MAX => {
                tracing::warn!("wasm-mods-host sent a line too long to read; reconnecting");
                break;
            }
            Ok(_) => {}
        }
        let message = match serde_json::from_slice::<Message>(&line) {
            Ok(message) => message,
            Err(error) => {
                tracing::debug!(%error, "a line from wasm-mods-host this build does not read");
                continue;
            }
        };
        let mut inbox = lock(&shared.from_host);
        match message {
            Message::Ready { .. } => {
                greeted = true;
                inbox.status = Some(Status::Connected);
            }
            Message::Frame { .. } => inbox.frame = Some(Box::new(message)),
            Message::Error { message } => refused = Some(message),
            other if inbox.messages.len() < MAX_MESSAGES => inbox.messages.push_back(other),
            _ => tracing::warn!("a WASM mod's view is not taking messages; one was dropped"),
        }
        drop(inbox);
        arrived(shared);
    }
    match refused {
        Some(message) => Ended::Refused(message),
        // Closed before greeting: not this host (a wrong token closes at
        // once), or one that died starting.
        None if !greeted => Ended::Unreachable(Error::Closed),
        None => Ended::Lost,
    }
}
