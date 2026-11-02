use std::{io, path::PathBuf, process::ExitStatus};

/// Why a WASM mod could not be shown. The host's own refusals (a module it
/// cannot load) arrive as its words, [`Error::Host`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("wasm-mods-host closed the connection before it was ready")]
    Closed,
    #[error("could not reach wasm-mods-host on port {port}")]
    Connect {
        port: u16,
        #[source]
        source: io::Error,
    },
    #[error("wasm-mods-host {version} exited before it listened ({status})")]
    Exited { version: String, status: ExitStatus },
    #[error("wasm-mods-host: {0}")]
    Host(String),
    #[error("{} could not be read or written", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("no home folder to find wasm-mods-host in")]
    NoHome,
    #[error("no installed wasm-mods-host runs protocol {protocol}")]
    NoHost { protocol: u32 },
    #[error("could not make a token for wasm-mods-host")]
    Random(#[source] getrandom::Error),
    #[error("wasm-mods-host is still starting in another window")]
    Starting,
    #[error("could not start wasm-mods-host {version}")]
    Start {
        version: String,
        #[source]
        source: io::Error,
    },
    #[error("could not start a thread for a WASM mod")]
    Thread(#[source] io::Error),
    #[error("the mod's module is larger than {limit} bytes")]
    TooLarge { limit: u64 },
    #[error("wasm-mods-host {version} did not listen in time")]
    TooSlow { version: String },
}

impl Error {
    pub(super) fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}
