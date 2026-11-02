//! Keys as the protocol's actions, never raw keys: arrows (and `j`/`k`)
//! move the frame's list or are sent as moves, PageUp/PageDown and
//! Home/End scroll its page, Enter activates or opens, Space is Primary,
//! Tab is Secondary (Shift held with Shift+Tab), and Escape is Back or
//! dismisses a popup. In a text field every key types except Enter, which
//! submits it, Escape, which leaves it, and Tab.

use super::view::WasmView;
use gpui::{Context, KeyDownEvent, ScrollStrategy, Window};
use wasm_mods_protocol::{Action, Event, Mods, Move};

impl WasmView {
    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        let field = self.focused_field(window, cx).map(str::to_owned);
        let mut events = Vec::new();
        if let Some(id) = field {
            match key {
                "enter" => events.push(Event::Submitted { id }),
                "escape" => window.focus(&self.focus, cx),
                "tab" => self.tab(modifiers.shift, window, cx),
                _ => return,
            }
        } else if key == "tab" {
            self.tab(modifiers.shift, window, cx);
        } else {
            events = self.action_events(key, modifiers.shift, cx);
            // A key that means nothing here goes on to the window's bindings;
            // a page key that scrolled the page here sends nothing.
            let paged = matches!(key, "pageup" | "pagedown" | "home" | "end");
            if events.is_empty() && !paged {
                return;
            }
        }
        if !events.is_empty() {
            self.send_events(events, None);
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Tab (Shift+Tab: `back`): the module's Secondary action with Shift in
    /// its modifiers; in a text field, the next or previous field.
    fn tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.focused_field(window, cx).is_some() {
            if back {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            return;
        }
        let mods = Mods {
            shift: back,
            ..Mods::default()
        };
        self.send_events(vec![Event::Action(Action::Secondary)], Some(mods));
    }

    /// The events `key` (outside a field) stands for.
    fn action_events(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) -> Vec<Event> {
        let step = match key {
            "up" | "k" => Some(Move::Up),
            "down" | "j" => Some(Move::Down),
            "pageup" => Some(Move::PageUp),
            "pagedown" => Some(Move::PageDown),
            "home" => Some(Move::Home),
            "end" => Some(Move::End),
            "left" => Some(Move::Left),
            "right" => Some(Move::Right),
            _ => None,
        };
        // A key the module offers by name: that action.
        let named = (!shift)
            .then(|| {
                let frame = self.frame.as_ref()?;
                frame
                    .actions
                    .iter()
                    .find(|offer| {
                        matches!(offer.action, Action::Key(_)) && offer.key.as_deref() == Some(key)
                    })
                    .map(|offer| offer.action.clone())
            })
            .flatten();
        if let Some(action) = named {
            return vec![Event::Action(action)];
        }
        let list = self
            .key_list
            .clone()
            .filter(|id| self.lists.get(id).is_some_and(|list| list.count > 0));
        if let Some(step @ (Move::PageUp | Move::PageDown | Move::Home | Move::End)) = step
            && self.page_by(step)
        {
            return Vec::new();
        }
        match (step, &list) {
            (Some(step), Some(id)) if !matches!(step, Move::Left | Move::Right) => {
                return self.move_selection(id, step, cx);
            }
            (Some(step), _) => return vec![Event::Action(Action::Move(step))],
            _ => {}
        }
        let selected = list
            .as_ref()
            .and_then(|id| Some((id.clone(), self.lists.get(id)?.selected?)));
        match key {
            "enter" => match selected {
                Some((id, index)) => vec![Event::Activated { id, index }],
                None => vec![Event::Action(Action::Open)],
            },
            "escape" => vec![match &self.open_popup {
                Some(id) => Event::Dismissed { id: id.clone() },
                None => Event::Action(Action::Back),
            }],
            "space" => vec![Event::Action(Action::Primary)],
            _ => Vec::new(),
        }
    }

    /// Moves list `id`'s selection here at once, and tells the module.
    fn move_selection(&mut self, id: &str, step: Move, cx: &mut Context<Self>) -> Vec<Event> {
        let Some(list) = self.lists.get_mut(id) else {
            return Vec::new();
        };
        let current = list.selected.map_or(-1, |selected| selected as i64);
        let by: i64 = match step {
            Move::Up => -1,
            Move::Down => 1,
            Move::PageUp => -10,
            Move::PageDown => 10,
            Move::Home => i64::MIN / 2,
            _ => i64::MAX / 2,
        };
        let last = list.count as i64 - 1;
        let next = current.saturating_add(by).clamp(0, last) as usize;
        list.selected = Some(next);
        list.local = true;
        // A virtual list scrolls to it; the others scroll with their container.
        if let Some(virtual_list) = self.virtual_lists.get(id) {
            virtual_list
                .handle
                .scroll_to_item(next, ScrollStrategy::Nearest);
        }
        cx.notify();
        vec![Event::Selected {
            id: id.to_owned(),
            index: next,
        }]
    }
}
