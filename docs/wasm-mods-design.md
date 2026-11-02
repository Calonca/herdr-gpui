# WASM mods in Herdr UI: design

A proof of concept. Mods written once as sandboxed WebAssembly modules for
[herdr-wasm-mods](https://github.com/Calonca/herdr-wasm-mods) (the
`wasm-mods-host` runtime, protocol 5, release 0.2.0) also run in Herdr UI,
drawn natively in its theme. The same `.wasm` file runs unchanged in both
apps.

## Why

Herdr UI's own mods are Luau ([mods.md](mods.md)): UI as plain data in a
sandbox with no files, network, or processes. That is the right default, but
some mods need more: a todo list kept in a file, a diff of the checkout. The
WASM mods runtime already does that safely. It runs each module in its own
host process, asks the user before a module touches a folder or runs git, and
pins each answer to the module's SHA-256. If Herdr UI can draw what that host
sends, a mod author writes one module and it runs in every app that speaks the
protocol.

## Goals

- `todo` and `diff` (and `hello`) from the 0.2.0 release run in Herdr UI as
  activity bar apps, drawn with GPUI in Herdr UI's theme, from the very
  `.wasm` bytes the release ships.
- Typing in todo's field; diff's paging, Tab and Shift+Tab, and file tree;
  the Allow prompt; the view reconnecting after the host is killed.
- A WASM mod is installed the way a Luau mod is: a folder under
  `<config>/herdr/gpui-mods/` with a `mod.toml`.
- A small, readable change to Herdr UI that its maintainers could take.

## Non-goals

- Downloading or building hosts. Herdr UI uses an installed host and says how
  to install one when there is none.
- WASM mods in the status bar, sidebar, or panel. The activity bar app comes
  first because todo and diff need the room. The renderer does not depend on
  where it is placed, so other places can come later.
- `Open` (a module opening another module in a new tab, as the git graph opens
  a commit's diff), pictures, and remote machines (`WindowContext.machine`).
  Each is logged and ignored, or shown as unsupported.
- Platforms the host is not released for. Hosts are released for Linux now
  and macOS hosts are on their way, with the same `~/.local/share` and
  `~/.cache` folders, so nothing here is Linux-specific: hashing, copying,
  and the run folder are done in Rust, not with shell commands. Windows has
  no host. Where none is installed, a WASM mod says so.

## Anti-goals

- Checking permissions again in Herdr UI. The host enforces them and draws its
  own Allow prompt as an ordinary frame. A second check in the app would
  disagree with the host sooner or later.
- Blocking the UI thread. Finding the host, starting it, connecting, and
  reading and parsing JSON all happen on a worker thread.
- Running a WASM mod's code in the app process. The module runs only in the
  host.

## Decisions

Each was my call, made from the code. The brief allowed that.

1. **Manifest.** A WASM mod is a mod folder whose `mod.toml` has a `[wasm]`
   table:

   ```toml
   name = "wasm-todo"
   api = 1

   [activity]
   title = "Todo"
   icon = "icon.svg"

   [wasm]
   module = "todo"      # runs todo.wasm
   ```

   `herdr-mods` parses `[wasm]` and does not require `main.luau` when it is
   present. A new build of the folder's `.wasm` is installed over the old
   one on the open connection, and the host swaps it in keeping the
   module's state; the view keeps its own (a canvas's zoom). Changing
   which module the mod names makes a new view. `[wasm]` with a
   `main.luau` is refused, so a folder is one kind of mod or the other. The scanner, the reload on save, `[mods] disabled`,
   `herdr-gpui mod`, and `mod open` work unchanged.
   *Why:* the brief asked for this, and it adds about thirty lines to
   `herdr-mods` instead of a second discovery path.

2. **Where the module comes from.** This matches the release's own bridge
   exactly. If the folder holds `<module>.wasm`, it is copied into the shared
   modules folder `~/.local/share/wasm-mods/modules/<mod name>.wasm` when its
   SHA-256 differs (written beside it, then renamed). Otherwise the shared
   `<module>.wasm` already installed there is run.
   *Why:* the host's permissions are keyed by the installed name plus the
   module's SHA-256. With the release's folder name (`wasm-todo`) and the same
   bytes, an answer given in another app counts here too. This is the "write
   once" proof.

3. **Place.** Only `[activity]`: the mod's tree fills the app body under its
   title. A `[wasm]` mod without `[activity]` is refused by the manifest
   check, with a clear message.

4. **The renderer is ported into Herdr UI** as `crates/herdr-gpui/src/mods/wasm/`.
   It is ported from the existing native GPUI renderer for the protocol (about
   2,100 lines), onto gpui-pre 0.3.6, Herdr UI's `Theme`, its `SearchInput`
   text field, and its own scrollbars. It is not a shared `wasm-mods-gpui`
   crate.
   *Why:* the two apps differ in GPUI version, theme, text input, and zoom.
   A shared crate would need an abstraction over all four before it had a
   second user, and the rules here ask for no speculative layers. Once both
   renderers have settled, extracting one is a sensible follow-up.

5. **Protocol types** come from `wasm-mods-protocol` as a git dependency
   pinned to the v0.2.0 release commit (`b619c01`). `deny.toml` allows that
   one repository.
   *Why:* there is one definition of the wire, with no mirror types. The crate
   depends only on serde and serde_json. Publishing it to crates.io would make
   the git entry unnecessary, and is a follow-up for its owners.

6. **Finding and starting the host** is the bridge's algorithm, in Rust, on
   the connection thread:
   - Installed hosts are `~/.local/share/wasm-mods/host/<version>/wasm-mods-host`,
     each asked once for `--version --json`. The newest one whose protocol
     range holds the protocol this build speaks is chosen.
   - Herdr UI keeps its own run folder, `~/.cache/wasm-mods/run/herdr-ui/`
     (mode 700). Its `host` file holds `<port> <token> <version>`, and every
     Herdr UI window shares it. A `starting` directory, made with `mkdir`, is
     the start lock, and a stale one is taken over after 15 s.
   - A start writes a 48-hex-digit token to a private file and runs the host
     with `--token-file`, `--port-file`, and `--host-file`. It is started as a
     plain child through `herdr_client::new_command` (not in a herdr-core
     terminal, so it also works with Herdr 1), and a thread reaps it. The host
     exits by itself after ten idle minutes.
   *Why:* this keeps the host's own security model (the token never appears on
   a command line or on screen) and its lifetime rules.

7. **Connection.** There is one thread per shown mod. It finds the host,
   connects, sends `hello` (with `native_canvas: true`), and then reads lines.
   It talks to the UI through the same kind of mutex mailbox Luau mod workers
   use: the latest frame (coalesced), a short bounded queue of other host
   messages, and a bounded queue of outgoing lines. A one-slot wake has the
   window take what arrived at once, not on its next 16 ms tick, which would
   cap how often a module answers moving input. Nothing is unbounded, and
   only the newest frame is ever drawn.

8. **Reconnecting.** When the host goes away, the view keeps its last frame
   and shows "reconnecting…". The thread forgets the run file if it still
   names the dead port, finds or starts a host again with a backoff from
   250 ms to 2 s, and says hello again. The view then resends its size, theme,
   and folder. A host that closed after reporting an error (the module failed
   to load) is not retried. The error stays on screen until the files change.

9. **Theme and folder.** The protocol's theme comes from Herdr UI's `Theme`:
   background, surface, foreground, muted, primary, border, and the 16 ANSI
   colors. The fonts are the configured mono font. The module's folder is the
   focused workspace's path on the local host (the `focus` mod topic), with no
   branch. For a workspace on another machine, the module is told there is no
   folder.

10. **Keys.** These follow the renderer's protocol mapping:
    - Arrows, `j`/`k`, PageUp/PageDown, and Home/End move lists or are sent
      as moves.
    - Enter activates or opens, Space is Primary, Escape is Back (or dismisses
      a popup).
    - Tab and Shift+Tab are Secondary, with Shift in the modifiers when it is
      held. They are bound in the view's own key context, ahead of the
      window's focus cycling.

    In a text field every key types, except Enter (submit), Escape (leave the
    field), and Tab.

11. **Errors** have their own `mods::wasm::Error` (`thiserror`), wrapped once
    in `mods::Error`. User-visible text appears only at the drawing boundary.

## Known limitations of this proof of concept

- A virtual list (the diff's page) has a scrollbar of its own, as Settings
  and the review have theirs: Herdr UI has no shared scrollbar component
  yet. Rows the list has not laid out count at the first row's height.
- Pictures (`Shape::Image`, `ImageSrc`) are not drawn, and `Open`, which
  opens another module in a new tab, is logged and ignored.
- The module is told only the focused workspace's folder, without its
  branch.

## What is left to the builder

- The exact split of the renderer files, within the 1,000-line limit.
- Visual details that the Luau mod renderer already settles: spacing, tones,
  hover.

## How to try it

See the "WASM mods" section of [mods.md](mods.md).
