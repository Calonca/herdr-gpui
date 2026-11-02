//! Mod failures, from `error-mods.ftl`.

use super::Localize;
use crate::{
    i18n::fl,
    mods::{Error, wasm},
};

impl Localize for Error {
    fn localize(&self) -> String {
        match self {
            // keep-sorted start
            Self::Busy(name) => fl!("error-mods-busy", name = name.as_str()),
            Self::DisabledName(name) => fl!("error-mods-disabled-name", name = name.as_str()),
            // A mod's own diagnostics, for its author: where in the mod it
            // stopped, with Luau's words, as a parser's are shown.
            Self::Mod(error) => error.to_string(),
            Self::NotRunning(name) => fl!("error-mods-not-running", name = name.as_str()),
            Self::Spawn { name, .. } => fl!("error-mods-spawn", name = name.as_str()),
            Self::TooManyDisabled(limit) => fl!("error-mods-too-many-disabled", limit = *limit),
            Self::Wasm(error) => error.localize(),
            // keep-sorted end
        }
    }
}

impl Localize for wasm::Error {
    fn localize(&self) -> String {
        match self {
            // keep-sorted start
            Self::Closed => fl!("error-mods-wasm-closed"),
            Self::Connect { port, .. } => fl!("error-mods-wasm-connect", port = *port),
            Self::Exited { version, .. } => {
                fl!("error-mods-wasm-exited", version = version.as_str())
            }
            Self::Host(message) => fl!("error-mods-wasm-host", message = message.as_str()),
            Self::Io { path, .. } => fl!("error-mods-wasm-io", path = path.display().to_string()),
            Self::NoHome => fl!("error-mods-wasm-no-home"),
            Self::NoHost { protocol } => fl!("error-mods-wasm-no-host", protocol = *protocol),
            Self::Random(_) => fl!("error-mods-wasm-random"),
            Self::Start { version, .. } => fl!("error-mods-wasm-start", version = version.as_str()),
            Self::Starting => fl!("error-mods-wasm-starting"),
            Self::Thread(_) => fl!("error-mods-wasm-thread"),
            Self::TooLarge { limit } => fl!("error-mods-wasm-too-large", limit = *limit),
            Self::TooSlow { version } => fl!("error-mods-wasm-slow", version = version.as_str()),
            // keep-sorted end
        }
    }
}
