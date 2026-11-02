//! WASM mods in the window: a view made when the mod's app is first shown,
//! told the window's theme and folder and polled on every tick, and made
//! again when the mod's files change.

use super::{Paths, Surroundings, Target, WasmView};
use crate::HerdrWindow;
use gpui::{AppContext, Context, Entity, Focusable, Window};

impl HerdrWindow {
    /// The view of WASM mod `name`, made now if it has none yet; `None`
    /// for a Luau mod.
    pub(crate) fn wasm_view(
        &mut self,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<WasmView>> {
        let dir = self.mods.dir.as_ref()?.join(name);
        let entry = self.mods.entries.get_mut(name)?;
        let wasm = entry.manifest.as_ref()?.wasm.as_ref()?;
        if let Some(view) = &entry.wasm_view {
            return Some(view.clone());
        }
        let target = Paths::from_home().map(|paths| Target {
            paths,
            mod_dir: dir,
            mod_name: name.to_owned(),
            module: wasm.module.clone(),
        });
        let view = cx.new(|cx| WasmView::new(target, cx));
        entry.wasm_view = Some(view.clone());
        entry.wasm_reloaded = false;
        Some(view)
    }

    /// Gives the keyboard to WASM mod `name`'s view; false for a Luau mod.
    pub(crate) fn focus_wasm_view(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.wasm_view(name, cx) else {
            return false;
        };
        window.focus(&view.focus_handle(cx), cx);
        true
    }

    /// Runs on every tick: the shown WASM mod gets a view, every view hears
    /// the window's theme and folder and takes what its host sent, and a
    /// mod whose files changed starts again.
    pub(crate) fn poll_wasm_mods(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.mod_app().map(str::to_owned);
        if let Some(name) = &shown {
            self.wasm_view(name, cx);
        }
        let folder = self.wasm_folder(cx);
        let around = |folder: Option<String>| Surroundings {
            theme: &self.theme,
            theme_name: &self.config.theme,
            ui: &self.config.ui,
            mono: &self.config.terminal,
            folder,
        };
        let mut close = false;
        let mut wakes = Vec::new();
        for (name, entry) in &mut self.mods.entries {
            let module = entry
                .manifest
                .as_ref()
                .and_then(|manifest| manifest.wasm.as_ref())
                .map(|wasm| wasm.module.clone());
            let reloaded = std::mem::take(&mut entry.wasm_reloaded);
            let kept = match (&module, &entry.wasm_view) {
                (Some(module), Some(view)) if reloaded => view.read(cx).reload(module),
                _ => !reloaded,
            };
            if module.is_none() || !kept {
                // Dropping the view closes its connection; the next tick
                // makes a new one if the app shows.
                entry.wasm_view = None;
                continue;
            }
            let Some(view) = &entry.wasm_view else {
                continue;
            };
            let surroundings = around(folder.clone());
            let (polled, wake) = view.update(cx, |view, cx| {
                view.surround(surroundings, cx);
                (view.poll(window, cx), view.take_arrivals())
            });
            wakes.extend(wake);
            close |= polled.close && shown.as_deref() == Some(name.as_str());
        }
        if close {
            self.leave_mod_app(window, cx);
        }
        // A view's frames are drawn as they arrive, not up to a tick later:
        // that wait would cap how often a module can answer moving input.
        for arrivals in wakes {
            cx.spawn_in(window, async move |this, cx| {
                while arrivals.recv().await.is_ok() {
                    if this
                        .update_in(cx, |this, window, cx| this.poll_wasm_mods(window, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
    }

    /// Where a module that asks works, on this computer: the focused
    /// workspace's folder, or with no Herdr 1, the stand-in shell's. None for
    /// a workspace on another machine.
    fn wasm_folder(&self, cx: &gpui::App) -> Option<String> {
        self.wasm_workspace_folder().or_else(|| {
            let folder = self.core_fallback_folder(cx)?;
            Some(folder.to_string_lossy().into_owned())
        })
    }

    fn wasm_workspace_folder(&self) -> Option<String> {
        let endpoint = self.endpoints.get(self.selected_endpoint)?;
        if endpoint.connection.target.remote_host().is_some() {
            return None;
        }
        let snapshot = self.live.snapshot.as_deref()?;
        let id = snapshot.focused_workspace_id.as_deref()?;
        snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == id)
            .map(|workspace| workspace.new_workspace_cwd.clone())
            .filter(|path| !path.is_empty())
    }
}
