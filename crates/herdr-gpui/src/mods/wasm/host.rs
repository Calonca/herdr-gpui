//! Finding, starting, and sharing wasm-mods-host, as the WASM mods release's
//! own bridge does, with the same folders on every system:
//!
//! - hosts are installed side by side as
//!   `~/.local/share/wasm-mods/host/<version>/wasm-mods-host`, and the newest
//!   one whose protocol range holds this build's protocol runs;
//! - modules are `~/.local/share/wasm-mods/modules/<name>.wasm`, a mod
//!   folder's own module copied there under the mod's name;
//! - Herdr UI's windows share one host through `~/.cache/wasm-mods/run/herdr-ui/host`
//!   (`<port> <token> <version>`), which the host writes as it listens and
//!   removes when it exits idle. The folder is the owner's alone, and the
//!   token reaches the host through a file there, never its command line.
//!
//! Everything here blocks: it runs on a WASM mod's connection thread.

use super::Error;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};

/// The host's file name in its version folder.
const HOST: &str = "wasm-mods-host";
/// The largest module a mod folder may hold.
const MAX_MODULE: u64 = 64 << 20;
/// How long a host may take to listen once started.
const LISTEN_MAX: Duration = Duration::from_secs(15);
/// A start not finished within this is taken over: its window went away.
/// Well past [`LISTEN_MAX`], so a slow start is never taken for a dead one.
const START_MAX: Duration = Duration::from_secs(40);

/// Where hosts, modules, and the run folder are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Paths {
    /// `~/.local/share/wasm-mods`.
    pub share: PathBuf,
    /// `~/.cache/wasm-mods/run/herdr-ui`.
    pub run: PathBuf,
}

impl Paths {
    pub(crate) fn from_home() -> Result<Self, Error> {
        let home = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or(Error::NoHome)?;
        Ok(Self::under(&home))
    }

    /// The folders under `home`.
    pub(crate) fn under(home: &Path) -> Self {
        Self {
            share: home.join(".local/share/wasm-mods"),
            run: home.join(".cache/wasm-mods/run/herdr-ui"),
        }
    }

    fn modules(&self) -> PathBuf {
        self.share.join("modules")
    }

    fn host_file(&self) -> PathBuf {
        self.run.join("host")
    }
}

/// An installed host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Installed {
    pub version: Version,
    /// The protocols it runs, from `--version --json`.
    pub protocol: (u32, u32),
    pub path: PathBuf,
}

impl Installed {
    fn runs(&self, protocol: u32) -> bool {
        (self.protocol.0..=self.protocol.1).contains(&protocol)
    }
}

/// `major.minor.patch`; a `-pre` or `+build` part is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub(super) fn parse(text: &str) -> Option<Self> {
        let core = text.trim().split(['-', '+']).next()?;
        let mut parts = core.split('.').map(str::parse::<u32>);
        let version = Self(
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        );
        parts.next().is_none().then_some(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Where a host listens and the token it opens connections with.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub port: u16,
    pub token: String,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// The run file's `<port> <token> <version>`; the token is 48 hex digits.
pub(super) fn parse_run_file(text: &str) -> Option<(Endpoint, Version)> {
    let line = text.strip_suffix('\n')?;
    let mut parts = line.split(' ');
    let port = parts.next()?.parse().ok()?;
    let token = parts.next()?;
    let valid = token.len() == 48 && token.bytes().all(|b| b.is_ascii_hexdigit());
    // A host from before releases wrote no version.
    let version = match parts.next() {
        Some(version) => Version::parse(version)?,
        None => Version(0, 0, 0),
    };
    (valid && parts.next().is_none()).then(|| {
        let token = token.to_owned();
        (Endpoint { port, token }, version)
    })
}

/// Every installed host that answers `--version --json` with its own
/// folder's version.
pub(super) fn installed(paths: &Paths) -> Vec<Installed> {
    let Ok(entries) = fs::read_dir(paths.share.join("host")) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let version = Version::parse(entry.file_name().to_str()?)?;
            let path = entry.path().join(HOST);
            let protocol = ask_version(&path, version)?;
            Some(Installed {
                version,
                protocol,
                path,
            })
        })
        .collect()
}

/// The protocols the host at `path` runs, if it says it is `version`.
fn ask_version(path: &Path, version: Version) -> Option<(u32, u32)> {
    #[derive(serde::Deserialize)]
    struct Answer {
        version: String,
        protocol: (u32, u32),
    }
    if !path.is_file() {
        return None;
    }
    let output = herdr_client::new_command(path)
        .args(["--version", "--json"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let answer: Answer = serde_json::from_slice(&output.stdout).ok()?;
    (Version::parse(&answer.version) == Some(version)).then_some(answer.protocol)
}

/// The newest of `hosts` that runs `protocol`.
pub(super) fn pick(hosts: &[Installed], protocol: u32) -> Option<&Installed> {
    hosts
        .iter()
        .filter(|host| host.runs(protocol))
        .max_by_key(|host| host.version)
}

/// Copies the mod folder's `<module>.wasm` into the shared modules folder as
/// `<mod_name>.wasm` when it differs, and returns the name the host knows it
/// by. Without one in the folder, the installed `<module>` runs as it is.
pub(crate) fn install_module(
    paths: &Paths,
    mod_dir: &Path,
    mod_name: &str,
    module: &str,
) -> Result<String, Error> {
    let own = mod_dir.join(format!("{module}.wasm"));
    // As a mod's other files, a link is never followed.
    match fs::symlink_metadata(&own) {
        Ok(meta) if meta.is_file() && meta.len() <= MAX_MODULE => {}
        Ok(meta) if meta.is_file() => return Err(Error::TooLarge { limit: MAX_MODULE }),
        _ => return Ok(module.to_owned()),
    }
    let modules = paths.modules();
    let shared = modules.join(format!("{mod_name}.wasm"));
    if same_file(&own, &shared)? {
        return Ok(mod_name.to_owned());
    }
    fs::create_dir_all(&modules).map_err(Error::io(&modules))?;
    // Written beside it and renamed: a host reloads the module whole.
    let temp = modules.join(format!(".{mod_name}.wasm.{}", hex(&random::<4>()?)));
    fs::copy(&own, &temp).map_err(Error::io(&temp))?;
    fs::rename(&temp, &shared).map_err(|source| {
        let _ = fs::remove_file(&temp);
        Error::Io {
            path: shared.clone(),
            source,
        }
    })?;
    Ok(mod_name.to_owned())
}

/// Whether both files hold the same bytes, by SHA-256, reading at most
/// [`MAX_MODULE`] of each; a missing `b` differs.
fn same_file(a: &Path, b: &Path) -> Result<bool, Error> {
    let digest = |path: &Path| -> io::Result<_> {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_MODULE + 1)
            .read_to_end(&mut bytes)?;
        Ok(Sha256::digest(bytes))
    };
    let mine = digest(a).map_err(Error::io(a))?;
    Ok(digest(b).is_ok_and(|theirs| theirs == mine))
}

/// Where a host that runs `protocol` listens: the one the run file names,
/// else one started now, the newest installed. A host the run file names is
/// used while it runs `protocol` and no newer one is installed; a newer one
/// takes over, and the old one exits once idle.
pub(crate) fn ensure(paths: &Paths, protocol: u32) -> Result<Endpoint, Error> {
    let hosts = installed(paths);
    let best = pick(&hosts, protocol).ok_or(Error::NoHost { protocol })?;
    make_private(&paths.run)?;
    let usable = |found: &Option<(Endpoint, Version)>| {
        found.as_ref().and_then(|(endpoint, version)| {
            let host = hosts.iter().find(|host| host.version == *version)?;
            (host.runs(protocol) && *version >= best.version).then(|| endpoint.clone())
        })
    };
    let lock = paths.run.join("starting");
    let until = Instant::now() + START_MAX + Duration::from_secs(5);
    while Instant::now() < until {
        if let Some(endpoint) = usable(&read_run_file(paths)) {
            return Ok(endpoint);
        }
        if fs::create_dir(&lock).is_err() {
            // Another window is starting one; a start that never finished
            // (its window closed) is taken over.
            let stale = fs::metadata(&lock)
                .and_then(|meta| meta.modified())
                .is_ok_and(|at| {
                    SystemTime::now()
                        .duration_since(at)
                        .is_ok_and(|age| age > START_MAX)
                });
            // Renamed away first, so of two windows that both found it
            // stale only one removes it, never a fresh lock made meanwhile.
            if stale && let Ok(suffix) = random::<6>() {
                let taken = paths.run.join(format!("starting-stale-{}", hex(&suffix)));
                if fs::rename(&lock, &taken).is_ok() {
                    let _ = fs::remove_dir(&taken);
                }
            }
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        let started = (|| {
            if let Some(endpoint) = usable(&read_run_file(paths)) {
                return Ok(endpoint);
            }
            start(paths, best)
        })();
        let _ = fs::remove_dir(&lock);
        return started;
    }
    Err(Error::Starting)
}

fn read_run_file(paths: &Paths) -> Option<(Endpoint, Version)> {
    parse_run_file(&fs::read_to_string(paths.host_file()).ok()?)
}

/// Forgets the host at `port`, gone away, unless another window has already
/// written a new one.
pub(crate) fn forget(paths: &Paths, port: u16) {
    if read_run_file(paths).is_some_and(|(endpoint, _)| endpoint.port == port) {
        let _ = fs::remove_file(paths.host_file());
    }
}

/// Starts `host` and waits until it listens. A thread reaps it once it
/// exits by itself, idle.
fn start(paths: &Paths, host: &Installed) -> Result<Endpoint, Error> {
    let token = hex(&random::<24>()?);
    let id = hex(&random::<6>()?);
    let token_file = paths.run.join(format!("token-{id}"));
    let port_file = paths.run.join(format!("port-{id}"));
    write_private(&token_file, &token)?;
    let modules = paths.modules();
    let spawned = herdr_client::new_command(&host.path)
        .arg("--token-file")
        .arg(&token_file)
        .arg("--port-file")
        .arg(&port_file)
        .arg("--host-file")
        .arg(paths.host_file())
        .arg("--modules")
        .arg(&modules)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let version = host.version.to_string();
    let child = match spawned {
        Ok(child) => child,
        Err(source) => {
            let _ = fs::remove_file(&token_file);
            return Err(Error::Start { version, source });
        }
    };
    let port = wait_listening(child, &port_file, &version);
    let _ = fs::remove_file(&port_file);
    let _ = fs::remove_file(&token_file);
    let endpoint = Endpoint { port: port?, token };
    // The host writes the run file itself; one from before releases did not.
    let line = format!("{} {} {}\n", endpoint.port, endpoint.token, host.version);
    if fs::read_to_string(paths.host_file()).ok().as_deref() != Some(line.as_str()) {
        write_private(&paths.host_file(), &line)?;
    }
    Ok(endpoint)
}

/// The port the host writes to `port_file`, then hands `child` to a thread
/// that waits for it.
fn wait_listening(mut child: Child, port_file: &Path, version: &str) -> Result<u16, Error> {
    let until = Instant::now() + LISTEN_MAX;
    let result = loop {
        let port = fs::read_to_string(port_file)
            .ok()
            .and_then(|text| text.strip_suffix('\n')?.parse().ok());
        if let Some(port) = port {
            break Ok(port);
        }
        if let Ok(Some(status)) = child.try_wait() {
            let version = version.to_owned();
            return Err(Error::Exited { version, status });
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            let version = version.to_owned();
            return Err(Error::TooSlow { version });
        }
        thread::sleep(Duration::from_millis(20));
    };
    let reaper = thread::Builder::new()
        .name("wasm-mods-host-wait".into())
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(error) = reaper {
        tracing::warn!(%error, "could not start a thread to wait for wasm-mods-host");
    }
    result
}

/// Makes `dir` (and its parents) and closes it to everyone but its owner.
fn make_private(dir: &Path) -> Result<(), Error> {
    fs::create_dir_all(dir).map_err(Error::io(dir))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `create_dir_all` leaves a folder that was already there as it was.
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(Error::io(dir))?;
    }
    Ok(())
}

/// Writes `text` to a file only its owner can read, whole: written beside
/// and renamed, so a reader never sees half of it.
fn write_private(path: &Path, text: &str) -> Result<(), Error> {
    let temp = path.with_extension(format!("tmp{}", hex(&random::<6>()?)));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&temp)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .and_then(|()| fs::rename(&temp, path));
    written.map_err(|source: io::Error| {
        let _ = fs::remove_file(&temp);
        Error::Io {
            path: path.to_owned(),
            source,
        }
    })
}

fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(Error::Random)?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
