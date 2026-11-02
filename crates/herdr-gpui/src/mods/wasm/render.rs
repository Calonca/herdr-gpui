//! The view on screen: the last frame's tree under the actions it offers,
//! popups over it, and where the connection is.
// i18n: migrated

use super::{
    theme::Colors,
    tree::{self, Build, Ctx, z},
    view::{Shown, WasmView},
};
use crate::i18n::{errors::Localize, fl};
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, MouseButton, Render, SharedString,
    WeakEntity, Window, div, prelude::*,
};
use wasm_mods_protocol::{Action, Event, Offer};

/// The view's key context, for bindings that apply only in it.
const KEY_CONTEXT: &str = "WasmModView"; // i18n: data

impl Render for WasmView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.track_field_focus(window, cx);
        let t = Colors::of(&self.theme);
        let root = div()
            .id("wasm-mod-view")
            .debug_selector(|| "wasm-mod-view".into())
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|v, _, window, cx| {
                    if v.focused_field(window, cx).is_none() {
                        window.focus(&v.focus, cx);
                    }
                }),
            )
            .child(size_probe(cx.entity().downgrade()))
            .bg(t.bg)
            .text_color(t.fg)
            .text_size(z(13.))
            .font_family(SharedString::from(self.ui_font.family.clone()));
        let note = match &self.shown {
            Shown::Connected => None,
            Shown::Connecting => Some((fl!("wasm-mods-connecting"), t.dim)),
            Shown::Reconnecting => Some((fl!("wasm-mods-reconnecting"), t.dim)),
            Shown::Failed(error) => Some((error.localize(), t.ansi[1])),
        };
        let Some(frame) = self.frame.take() else {
            let (text, color) = note.unwrap_or_else(|| (fl!("wasm-mods-connecting"), t.dim));
            return root
                .p(z(14.))
                .child(div().text_color(color).child(text))
                .into_any_element();
        };
        self.open_popup = None;
        let mut b = Build {
            popups: Vec::new(),
            t: &t,
            mono: SharedString::from(self.mono.family.clone()),
        };
        let body = tree::node(self, &mut b, &frame.ui, Ctx::default(), window, cx);
        let popups = std::mem::take(&mut b.popups);
        let hints = (!frame.actions.is_empty()).then(|| hints(&frame.actions, &t, cx));
        // The frame scrolls, unless a virtual list in it does.
        let virtual_list = tree::holds_virtual(&frame.ui);
        let content = div()
            .id("wasm-mod-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p(z(14.))
            .child(body);
        let content = if virtual_list {
            content.overflow_hidden()
        } else {
            content.overflow_y_scroll()
        };
        self.frame = Some(frame);
        let badge = note.map(|(text, color)| {
            div()
                .absolute()
                .top(z(6.))
                .right(z(10.))
                .max_w(gpui::relative(0.6))
                .px(z(6.))
                .rounded(z(4.))
                .bg(t.raised)
                .text_size(z(11.))
                .text_color(color)
                .child(text)
        });
        root.child(content)
            .children(hints)
            .children(popups)
            .children(badge)
            .into_any_element()
    }
}

/// Tells the view its size when layout changes it.
fn size_probe(view: WeakEntity<WasmView>) -> impl IntoElement {
    gpui::canvas(
        move |bounds, _, cx| {
            let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let _ = view.update(cx, |v, _| v.resized(size));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// What the user can do now: the module's offers, clickable.
fn hints(actions: &[Offer], t: &Colors, cx: &mut Context<WasmView>) -> AnyElement {
    let fg = t.fg;
    div()
        .id("wasm-mod-hints")
        .flex()
        .flex_row()
        .flex_wrap()
        .flex_shrink_0()
        .px(z(14.))
        .py(z(6.))
        .gap(z(14.))
        .border_t_1()
        .border_color(t.border)
        .text_size(z(11.5))
        .text_color(t.dim)
        .children(actions.iter().enumerate().map(|(index, offer)| {
            let glyph = match &offer.action {
                Action::Back => "esc".to_owned(),
                Action::Open => "⏎".to_owned(),
                Action::Primary => "space".to_owned(),
                Action::Secondary => "tab".to_owned(),
                Action::Move(_) => "↑↓".to_owned(),
                Action::Key(_) => offer.key.clone().unwrap_or_default(),
            };
            let action = offer.action.clone();
            div()
                .id(("wasm-mod-hint", index))
                .flex()
                .flex_row()
                .gap(z(5.))
                .cursor_pointer()
                .hover(move |style| style.text_color(fg))
                .on_click(cx.listener(move |v, _: &ClickEvent, _, _| {
                    v.send_events(vec![Event::Action(action.clone())], None);
                }))
                .child(
                    div()
                        .px(z(5.))
                        .rounded(z(4.))
                        .bg(t.raised)
                        .text_color(t.muted)
                        .font_weight(FontWeight::MEDIUM)
                        .child(glyph),
                )
                .child(offer.label.clone())
        }))
        .into_any_element()
}
