//! A WASM mod's view: its connection, the last frame the host sent, and the
//! state that lives here between frames (text fields, list selections and
//! scroll, canvases' pan and zoom). Clicks and keys become the protocol's
//! events; moves and views are held while an answer is due, so a slow
//! module is sent only the latest.

use super::{
    Error,
    canvas::CanvasState,
    link::{Inbound, Link, Status, Target},
    theme,
};
use crate::{
    config::{Config, FontConfig, Theme},
    search_input::{self, SearchInput},
};
use gpui::{
    AppContext, Context, Entity, FocusHandle, Focusable, ScrollStrategy, Subscription,
    UniformListScrollHandle, Window,
};
use std::{
    collections::{BTreeMap, HashMap},
    time::{Duration, Instant},
};
use wasm_mods_protocol::{
    Event, Mods, Move, Node, Offer, PointerKind, WindowContext,
    wire::{In, Message},
};

/// How long an unanswered batch holds back moves before they go anyway.
const HOLD_MAX: Duration = Duration::from_secs(1);

/// Virtual lists whose scroll is kept after they leave the frame, at most.
const MAX_KEPT_SCROLLS: usize = 64;

/// The last frame: what to draw and the actions offered under it.
pub(crate) struct Frame {
    pub ui: Node,
    pub actions: Vec<Offer>,
}

pub(crate) struct Field {
    pub input: Entity<SearchInput>,
    /// The module's revision of the text; edits carry it.
    revision: u64,
    /// The text the module last set, which is not sent back as an edit.
    module_value: String,
    masked: bool,
    pub focused: bool,
    _changed: Subscription,
}

/// A list's selection here: it moves at once on a click or key, before the
/// module answers.
pub(crate) struct ListSel {
    pub selected: Option<usize>,
    pub count: usize,
    pub local: bool,
}

/// A virtual list's scroll here, and what it last told the module.
pub(crate) struct VirtualList {
    pub handle: UniformListScrollHandle,
    /// The rows last said to show: first, count.
    pub sent: Option<(usize, usize)>,
    /// The module's last `scroll_to` revision done.
    pub revision: Option<u64>,
}

/// Where the connection is, as the view shows it.
pub(crate) enum Shown {
    Connecting,
    Connected,
    /// The host went away; the last frame stays until another answers.
    Reconnecting,
    Failed(Error),
}

/// What a poll found for the window.
#[derive(Default)]
pub(crate) struct Polled {
    pub changed: bool,
    /// The module asked to close its view.
    pub close: bool,
}

/// What the window tells the view each tick: its theme, fonts, and folder.
pub(crate) struct Surroundings<'a> {
    pub theme: &'a Theme,
    pub theme_name: &'a str,
    pub ui: &'a FontConfig,
    pub mono: &'a FontConfig,
    pub folder: Option<String>,
}

pub(crate) struct WasmView {
    pub(super) link: Option<Link>,
    pub(super) shown: Shown,
    pub(super) frame: Option<Frame>,
    /// The room it has, last told the host.
    pub(super) size: Option<(f32, f32)>,
    pub(super) theme: Theme,
    pub(super) ui_font: FontConfig,
    pub(super) mono: FontConfig,
    sent_theme: Option<wasm_mods_protocol::Theme>,
    sent_window: Option<Option<WindowContext>>,
    folder: Option<String>,
    seq: u64,
    /// Batches sent and not answered yet: seq → when.
    sent: BTreeMap<u64, Instant>,
    /// Moves and views held while an answer is due: the latest of each.
    held: Vec<(String, Event)>,
    held_mods: Mods,
    pub(super) fields: HashMap<String, Field>,
    pub(super) lists: HashMap<String, ListSel>,
    pub(super) virtual_lists: HashMap<String, VirtualList>,
    /// The list the arrow keys move.
    pub(super) key_list: Option<String>,
    /// The frame's page (a virtual list with `pages`): PageUp, PageDown,
    /// Home, and End scroll it.
    pub(super) page_list: Option<String>,
    pub(super) canvases: HashMap<String, CanvasState>,
    pub(super) open_popup: Option<String>,
    pub(super) focus: FocusHandle,
}

impl WasmView {
    pub(crate) fn new(target: Result<Target, Error>, cx: &mut Context<Self>) -> Self {
        let (link, shown) = match target.and_then(Link::spawn) {
            Ok(link) => (Some(link), Shown::Connecting),
            Err(error) => (None, Shown::Failed(error)),
        };
        Self {
            link,
            shown,
            frame: None,
            size: None,
            theme: Theme::default(),
            ui_font: Config::default().ui,
            mono: Config::default().terminal,
            sent_theme: None,
            sent_window: None,
            folder: None,
            seq: 0,
            sent: BTreeMap::new(),
            held: Vec::new(),
            held_mods: Mods::default(),
            fields: HashMap::new(),
            lists: HashMap::new(),
            virtual_lists: HashMap::new(),
            key_list: None,
            page_list: None,
            canvases: HashMap::new(),
            open_popup: None,
            focus: cx.focus_handle(),
        }
    }

    /// The window's theme, fonts, and folder: the host hears them when they
    /// changed.
    pub(crate) fn surround(&mut self, around: Surroundings<'_>, cx: &mut Context<Self>) {
        if self.theme != *around.theme || self.ui_font != *around.ui || self.mono != *around.mono {
            self.theme = around.theme.clone();
            self.ui_font = around.ui.clone();
            self.mono = around.mono.clone();
            for field in self.fields.values() {
                let (font, theme) = (self.ui_font.clone(), self.theme.clone());
                field
                    .input
                    .update(cx, |input, cx| input.set_appearance(font, theme, cx));
            }
            cx.notify();
        }
        let theme = theme::protocol(around.theme, around.theme_name, &around.mono.family);
        if self.sent_theme.as_ref() != Some(&theme) && self.connected() {
            self.send(&In::Theme(theme.clone()));
            self.sent_theme = Some(theme);
        }
        self.folder = around.folder;
        let window = self.folder.clone().map(|folder| WindowContext {
            folder,
            branch: None,
            machine: None,
        });
        if self.sent_window.as_ref() != Some(&window) && self.connected() {
            self.send(&In::Window(window.clone()));
            self.sent_window = Some(window);
        }
    }

    fn connected(&self) -> bool {
        matches!(self.shown, Shown::Connected)
    }

    /// Takes what the host sent since the last tick.
    pub(crate) fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Polled {
        let mut polled = Polled::default();
        let Some(link) = &self.link else {
            return polled;
        };
        let Inbound {
            frame,
            messages,
            status,
        } = link.take();
        if let Some(status) = status {
            polled.changed = true;
            self.status(status);
        }
        if let Some(frame) = frame {
            polled.changed = true;
            self.receive(*frame, window, cx);
        }
        for message in messages {
            polled.changed = true;
            polled.close |= matches!(message, Message::Close);
            self.receive(message, window, cx);
        }
        if polled.changed {
            cx.notify();
        }
        polled
    }

    fn status(&mut self, status: Status) {
        self.shown = match status {
            Status::Connecting | Status::Lost if self.frame.is_some() => Shown::Reconnecting,
            Status::Connecting | Status::Lost => Shown::Connecting,
            Status::Connected => {
                // A new connection: everything it needs is said again, and
                // nothing is still waiting on the old one.
                self.sent_theme = None;
                self.sent_window = None;
                self.sent.clear();
                self.held.clear();
                Shown::Connected
            }
            Status::Failed(error) => Shown::Failed(error),
        };
        if matches!(self.shown, Shown::Connected) {
            if let Some((width, height)) = self.size {
                self.send(&In::Size { width, height });
            }
            for list in self.virtual_lists.values_mut() {
                list.sent = None;
            }
        }
    }

    pub(super) fn receive(
        &mut self,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match message {
            Message::Frame {
                mut ui,
                actions,
                ack,
                ..
            } => {
                self.answered(ack);
                self.adopt(&ui, cx);
                super::canvas::take_shapes(self, &mut ui);
                self.frame = Some(Frame { ui, actions });
            }
            Message::Focus { id } => {
                if let Some(field) = self.fields.get(&id) {
                    window.focus(&field.input.focus_handle(cx), cx);
                }
            }
            Message::OpenUrl { url } => {
                if url.starts_with("https://") || url.starts_with("http://") {
                    cx.open_url(&url);
                }
            }
            Message::Open { module, .. } => {
                tracing::info!(%module, "a WASM mod asked to open another module, which Herdr UI does not do yet");
            }
            Message::Close | Message::Ready { .. } | Message::Error { .. } => {}
        }
    }

    /// The room the view has, from layout: told to the host when it changed.
    pub(super) fn resized(&mut self, size: (f32, f32)) {
        if self.size == Some(size) {
            return;
        }
        self.size = Some(size);
        let (width, height) = size;
        self.send(&In::Size { width, height });
    }

    pub(super) fn send(&self, message: &In) {
        if let Some(link) = &self.link {
            link.send(message);
        }
    }

    /// Sends `events`; moves and views are held while an answer is due.
    pub(super) fn send_events(&mut self, events: Vec<Event>, mods: Option<Mods>) {
        let waiting = self
            .sent
            .values()
            .next()
            .is_some_and(|at| at.elapsed() < HOLD_MAX);
        if !events.is_empty() && waiting && events.iter().all(|event| latest_only(event).is_some())
        {
            for event in events {
                let key = latest_only(&event).unwrap_or_default();
                self.held.retain(|(held, _)| *held != key);
                self.held.push((key, event));
            }
            if let Some(mods) = mods {
                self.held_mods = mods;
            }
            return;
        }
        let mut all: Vec<Event> = std::mem::take(&mut self.held)
            .into_iter()
            .map(|(_, event)| event)
            .collect();
        all.extend(events);
        if all.is_empty() || !self.connected() {
            return;
        }
        let mods = mods.unwrap_or(self.held_mods);
        self.seq += 1;
        self.sent.insert(self.seq, Instant::now());
        let events = all
            .iter()
            .filter_map(|event| serde_json::to_value(event).ok())
            .collect();
        let seq = self.seq;
        self.send(&In::Events { seq, events, mods });
    }

    /// An answer covers every batch sent up to its ack; held moves go now.
    fn answered(&mut self, ack: u64) {
        self.sent.retain(|seq, _| *seq > ack);
        if self.sent.is_empty() && !self.held.is_empty() {
            self.send_events(Vec::new(), None);
        }
    }

    /// The fields and lists of a new tree: inputs for new fields, new text
    /// where the module changed a revision, scroll handles for virtual lists.
    fn adopt(&mut self, ui: &Node, cx: &mut Context<Self>) {
        let mut fields = Vec::new();
        let mut lists = Vec::new();
        let mut first_key_list = None;
        let mut page_list = None;
        let mut nodes = vec![ui];
        while let Some(node) = nodes.pop() {
            match node {
                Node::TextField {
                    id,
                    value,
                    revision,
                    password,
                    placeholder,
                    ..
                } => {
                    fields.push(id.clone());
                    self.adopt_field(id, value, *revision, *password, placeholder, cx);
                }
                Node::List {
                    id,
                    items,
                    selected,
                    keyboard,
                    window,
                    pages,
                    ..
                } => {
                    lists.push(id.clone());
                    // A selection made here and not answered yet stays.
                    let pending =
                        !self.sent.is_empty() && self.lists.get(id).is_some_and(|list| list.local);
                    let mine = self.lists.get(id).and_then(|list| list.selected);
                    let count = window
                        .as_ref()
                        .map_or(items.len(), |window| window.total as usize);
                    let selected = if pending { mine } else { *selected };
                    self.lists.insert(
                        id.clone(),
                        ListSel {
                            selected,
                            count,
                            local: pending,
                        },
                    );
                    if *keyboard && first_key_list.is_none() {
                        first_key_list = Some(id.clone());
                    }
                    if window.is_some() {
                        self.virtual_lists
                            .entry(id.clone())
                            .or_insert_with(|| VirtualList {
                                handle: UniformListScrollHandle::new(),
                                sent: None,
                                revision: None,
                            });
                        if *pages && page_list.is_none() {
                            page_list = Some(id.clone());
                        }
                    }
                    nodes.extend(items.iter().filter_map(|item| item.child.as_deref()));
                }
                Node::Column { children, .. } | Node::Row { children, .. } => {
                    nodes.extend(children.iter().rev());
                }
                Node::Popup { child, .. } => nodes.push(child),
                _ => {}
            }
        }
        self.fields.retain(|id, _| fields.contains(id));
        // A list gone from the frame takes no keys; a virtual one keeps its
        // scroll, so going back finds it where it was.
        self.lists.retain(|id, _| lists.contains(id));
        // Scrolls of lists gone are kept for going back, but not without end.
        if self.virtual_lists.len() > MAX_KEPT_SCROLLS {
            self.virtual_lists.retain(|id, _| lists.contains(id));
        }
        self.page_list = page_list;
        if self
            .key_list
            .as_ref()
            .is_none_or(|list| !self.lists.contains_key(list))
        {
            self.key_list = first_key_list;
        }
    }

    fn adopt_field(
        &mut self,
        id: &str,
        value: &str,
        revision: u64,
        password: bool,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(field) = self.fields.get_mut(id) {
            // Up: the module set the text. Down: a new connection whose
            // module counts from the start again, and edits must carry its
            // revision or it drops them.
            if revision != field.revision {
                field.revision = revision;
                field.module_value = value.to_owned();
                let value = value.to_owned();
                field.input.update(cx, |input, cx| {
                    input.set_text_at(&value, value.len(), cx);
                });
            }
            if field.masked != password {
                field.masked = password;
                field
                    .input
                    .update(cx, |input, cx| input.set_masked(password, cx));
            }
            return;
        }
        let (font, theme) = (self.ui_font.clone(), self.theme.clone());
        let (value, placeholder) = (value.to_owned(), placeholder.to_owned());
        let input = cx.new(|cx| {
            let mut input = SearchInput::new(cx);
            input.set_placeholder(&placeholder, cx);
            input.set_appearance(font, theme, cx);
            input.set_masked(password, cx);
            input.set_text_at(&value, value.len(), cx);
            input
        });
        let owned = id.to_owned();
        let changed = cx.subscribe(&input, move |view, input, _: &search_input::Changed, cx| {
            let Some(field) = view.fields.get(&owned) else {
                return;
            };
            let value = input.read(cx).text().to_owned();
            // The module's own text, set here: not an edit of the user's.
            if value == field.module_value {
                return;
            }
            let (id, revision) = (owned.clone(), field.revision);
            view.send_events(
                vec![Event::Edited {
                    id,
                    value,
                    revision,
                }],
                None,
            );
        });
        self.fields.insert(
            id.to_owned(),
            Field {
                input,
                revision,
                module_value: value,
                masked: password,
                focused: false,
                _changed: changed,
            },
        );
    }

    /// Tells the module which fields gained or lost the keyboard.
    pub(super) fn track_field_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mut events = Vec::new();
        for (id, field) in &mut self.fields {
            let focused = field.input.read(cx).focus.is_focused(window);
            if focused != field.focused {
                field.focused = focused;
                events.push(Event::Focus {
                    id: id.clone(),
                    focused,
                });
            }
        }
        if !events.is_empty() {
            self.send_events(events, None);
        }
    }

    /// The field that has the keyboard, if one does.
    pub(crate) fn focused_field(&self, window: &Window, cx: &gpui::App) -> Option<&str> {
        self.fields
            .iter()
            .find(|(_, field)| field.input.read(cx).focus.is_focused(window))
            .map(|(id, _)| id.as_str())
    }

    /// Scrolls the frame's page list a page up or down, or to its start or
    /// end; false when the frame has none. The module hears where it is.
    pub(super) fn page_by(&mut self, step: Move) -> bool {
        let Some(id) = &self.page_list else {
            return false;
        };
        let Some(list) = self.virtual_lists.get(id) else {
            return false;
        };
        let total = self.lists.get(id).map_or(0, |list| list.count);
        if total == 0 {
            return true;
        }
        let (first, count) = list.sent.unwrap_or((0, 0));
        // The range counts rows partly showing at both ends: a page keeps a
        // row or so.
        let page = count.saturating_sub(3).max(1);
        let middle = first + count / 2;
        let handle = &list.handle;
        match step {
            Move::PageUp => {
                handle.scroll_to_item(middle.saturating_sub(page), ScrollStrategy::Center)
            }
            Move::PageDown => {
                handle.scroll_to_item((middle + page).min(total - 1), ScrollStrategy::Center)
            }
            Move::Home => handle.scroll_to_item(0, ScrollStrategy::Top),
            _ => handle.scroll_to_item(total - 1, ScrollStrategy::Bottom),
        }
        true
    }
}

/// For an event of which only the latest counts (a pointer move or drag, a
/// view, a visible range): what it replaces is keyed by this.
fn latest_only(event: &Event) -> Option<String> {
    match event {
        Event::Pointer {
            id,
            kind: kind @ (PointerKind::Move | PointerKind::Drag),
            ..
        } => Some(format!("p:{id}:{kind:?}")),
        Event::View { id, .. } => Some(format!("v:{id}")),
        Event::VisibleRange { id, .. } => Some(format!("r:{id}")),
        _ => None,
    }
}

impl Focusable for WasmView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
