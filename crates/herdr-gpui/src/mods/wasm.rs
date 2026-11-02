//! WASM mods (`docs/wasm-mods-design.md`): a mod folder whose `mod.toml`
//! has `[wasm]` runs a sandboxed WebAssembly module in wasm-mods-host, the
//! runtime WASM mods share across apps, and this module draws what the host
//! sends with the window's theme. The host enforces the module's
//! permissions and draws its own Allow prompt as an ordinary frame, so
//! nothing here grants or checks them.
//!
//! The UI thread never touches the host: each shown mod has a connection
//! thread ([`link`]) that finds or starts the host ([`host`]), and the
//! window's tick takes what arrived, as it does for Luau mods.

mod canvas;
mod error;
mod host;
mod keys;
mod link;
mod list;
mod render;
mod scrollbar;
mod theme;
mod tree;
mod view;
mod window;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

pub(crate) use error::Error;
pub(crate) use host::Paths;
pub(crate) use link::Target;
pub(crate) use view::{Surroundings, WasmView};
