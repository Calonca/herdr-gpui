//! Lists: plain ones, every row built, and virtual ones of `window.total`
//! rows of which a frame carries only those around what shows. A virtual
//! list builds only the rows on screen, tells the module which those are
//! (`VisibleRange`), and leaves rows not sent yet blank.

use super::{
    theme::{Colors, hsla},
    tree::{Build, Ctx, node, role, sized, z},
    view::WasmView,
};
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, SharedString, Window, div, prelude::*,
    uniform_list,
};
use std::ops::Range;
use wasm_mods_protocol::{Event, Font, ListItem, ListStyle, ListWindow, Node, Size};

/// How a list's rows look.
#[derive(Clone, Copy)]
struct Rows {
    selected: Option<usize>,
    dense: bool,
    soft: bool,
    /// A virtual list's row height: its rows are that tall, and a click
    /// also sends `Clicked "<list>#<row>"`.
    height: Option<f32>,
}

pub(super) fn list(
    v: &mut WasmView,
    b: &mut Build,
    n: &Node,
    ctx: Ctx,
    window: &mut Window,
    cx: &mut Context<WasmView>,
) -> AnyElement {
    let Node::List {
        id,
        items,
        selected,
        layout,
        dense,
        style,
        font,
        ..
    } = n
    else {
        return div().into_any_element();
    };
    let rows = Rows {
        selected: v.lists.get(id).map_or(*selected, |list| list.selected),
        dense: *dense,
        soft: *style == ListStyle::Soft,
        height: None,
    };
    let built: Vec<AnyElement> = items
        .iter()
        .enumerate()
        .map(|(index, item)| row(v, b, id, item, index, rows, window, cx))
        .collect();
    let mut el = div().flex().flex_col().gap(z(1.)).children(built);
    if *font == Font::Mono {
        el = el.font_family(b.mono.clone());
    }
    el = sized(el, layout, ctx.in_row);
    if layout.width == Size::Auto {
        el = el.w_full();
    }
    el.into_any_element()
}

/// One row of list `id`; `index` counts from the list's first row.
#[allow(clippy::too_many_arguments)]
fn row(
    v: &mut WasmView,
    b: &mut Build,
    id: &str,
    item: &ListItem,
    index: usize,
    rows: Rows,
    window: &mut Window,
    cx: &mut Context<WasmView>,
) -> AnyElement {
    let t = b.t;
    if item.header {
        let header = div()
            .px(z(8.))
            .pt(z(8.))
            .pb(z(2.))
            .text_size(z(11.))
            .text_color(t.dim)
            .child(item.label.clone());
        return match rows.height {
            Some(height) => header.h(z(height)).overflow_hidden().into_any_element(),
            None => header.into_any_element(),
        };
    }
    let selected = rows.selected == Some(index);
    let filled = selected && !rows.soft;
    let fg = if filled {
        t.accent_fg
    } else if item.disabled {
        t.dim
    } else {
        t.fg
    };
    // A virtual row with content of its own is flush: its lines meet.
    let flush = rows.height.is_some() && item.child.is_some();
    let mut el = div()
        .id(SharedString::from(format!("l-{id}-{index}")))
        .flex()
        .flex_row()
        .w_full()
        .gap(z(8.))
        .px(z(8.))
        .py(z(if flush {
            0.
        } else if rows.dense {
            3.
        } else {
            6.
        }))
        .rounded(z(if flush { 0. } else { 6. }))
        .items_center()
        .text_color(fg);
    if let Some(height) = rows.height {
        el = el.h(z(height)).flex_shrink_0().overflow_hidden();
    }
    if selected {
        el = el.bg(if rows.soft { t.selected } else { t.accent });
    } else {
        let hover = t.hover;
        el = el.hover(move |style| style.bg(hover));
    }
    if item.disabled {
        el = el.opacity(0.5);
    } else {
        let (list_id, virtual_row) = (id.to_owned(), rows.height.is_some());
        el = el
            .cursor_pointer()
            .on_click(cx.listener(move |v, event: &ClickEvent, window, cx| {
                if let Some(list) = v.lists.get_mut(&list_id) {
                    list.selected = Some(index);
                    list.local = true;
                }
                v.key_list = Some(list_id.clone());
                let id = list_id.clone();
                let mut events = vec![Event::Selected { id, index }];
                if virtual_row {
                    // A click, not the keys: a folder opens on one click.
                    let id = format!("{list_id}#{index}");
                    events.push(Event::Clicked { id });
                }
                if event.click_count() >= 2 {
                    let id = list_id.clone();
                    events.push(Event::Activated { id, index });
                }
                v.send_events(events, None);
                window.focus(&v.focus, cx);
                cx.notify();
            }));
    }
    if let Some(child) = &item.child {
        let inner = Ctx {
            in_row: flush,
            virtual_row: rows.height.is_some(),
        };
        let inner = node(v, b, child, inner, window, cx);
        // A flush row's content takes its whole height: lines sit one under
        // another and their fills meet.
        let cell = if flush {
            el = el.items_stretch();
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_hidden()
        } else {
            div().flex_1().min_w_0().overflow_hidden()
        };
        return el.child(cell.child(inner)).into_any_element();
    }
    parts(el, item, t, selected, filled, fg, id, index, cx).into_any_element()
}

/// A plain row's parts: tree guides, dot, icon, label, detail, key, and its
/// trailing button.
#[allow(clippy::too_many_arguments)]
fn parts(
    mut el: gpui::Stateful<gpui::Div>,
    item: &ListItem,
    t: &Colors,
    selected: bool,
    filled: bool,
    fg: gpui::Hsla,
    id: &str,
    index: usize,
    cx: &mut Context<WasmView>,
) -> gpui::Stateful<gpui::Div> {
    if !item.tree.is_empty() {
        el = el.child(div().text_color(t.dim).child(item.tree.clone()));
    }
    if let Some(dot) = item.dot {
        let color = if filled { fg } else { hsla(dot) };
        let mark = div().w(z(8.)).h(z(8.)).rounded_full().flex_shrink_0();
        el = el.child(if item.ring {
            mark.border_1().border_color(color)
        } else {
            mark.bg(color)
        });
    }
    if !item.icon.is_empty() {
        let mark = match item.icon.as_str() {
            "terminal" => ">_",
            "widget" | "connected" => "▦",
            other => other,
        };
        let color = item.icon_color.map_or(t.dim, hsla);
        el = el.child(div().text_color(color).child(mark.to_owned()));
    }
    let mut label = if item.label_spans.is_empty() {
        div().child(item.label.clone())
    } else {
        div()
            .flex()
            .flex_row()
            .children(item.label_spans.iter().map(|span| {
                div()
                    .text_color(span.color.map_or(fg, hsla))
                    .child(span.text.clone())
            }))
    };
    if let Some(r) = item.role
        && !selected
    {
        label = label.text_color(role(r, t));
    }
    if item.bold {
        label = label.font_weight(FontWeight::SEMIBOLD);
    }
    let mut main = div().flex().flex_col().flex_1().min_w_0().child(label);
    if !item.sub.is_empty() {
        main = main.child(div().flex().flex_row().text_size(z(11.5)).children(
            item.sub.iter().map(|span| {
                div()
                    .text_color(span.color.map_or(t.dim, hsla))
                    .child(span.text.clone())
            }),
        ));
    }
    el = el.child(main);
    if !item.detail.is_empty() {
        let color = if filled {
            fg
        } else {
            item.detail_color.map_or(t.dim, hsla)
        };
        el = el.child(
            div()
                .text_size(z(12.))
                .text_color(color)
                .child(item.detail.clone()),
        );
    }
    if !item.key.is_empty() {
        el = el.child(
            div()
                .text_size(z(11.5))
                .text_color(t.accent)
                .child(item.key.clone()),
        );
    }
    if let Some(trailing) = &item.trailing {
        let clicked = trailing.id.clone();
        let (hover_bg, hover_fg) = (t.hover, t.fg);
        el = el.child(
            div()
                .id(SharedString::from(format!("l-{id}-{index}-t")))
                .px(z(6.))
                .rounded(z(4.))
                .text_color(if filled { fg } else { t.dim })
                .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
                .cursor_pointer()
                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                    v.send_events(
                        vec![Event::Clicked {
                            id: clicked.clone(),
                        }],
                        None,
                    );
                    cx.stop_propagation();
                }))
                .child(trailing.text.clone()),
        );
    }
    el
}

/// One height for every row of a virtual list. Code (dense and monospace:
/// a diff's lines) sits tight, line under line.
fn row_height(dense: bool, font: Font) -> f32 {
    match (dense, font) {
        (true, Font::Mono) => 20.,
        (true, _) => 26.,
        _ => 36.,
    }
}

pub(super) fn virtual_list(
    v: &mut WasmView,
    b: &Build,
    n: &Node,
    range: &ListWindow,
    ctx: Ctx,
    cx: &mut Context<WasmView>,
) -> AnyElement {
    let Node::List {
        id, layout, font, ..
    } = n
    else {
        return div().into_any_element();
    };
    let total = range.total as usize;
    let Some(list) = v.virtual_lists.get_mut(id) else {
        return div().into_any_element();
    };
    if let Some(scroll) = range.scroll_to
        && list.revision != Some(scroll.revision)
    {
        list.revision = Some(scroll.revision);
        let row = scroll.row.min(total.saturating_sub(1) as u64) as usize;
        list.handle.scroll_to_item(row, gpui::ScrollStrategy::Top);
    }
    let handle = list.handle.clone();
    let list_id = id.clone();
    let rows = uniform_list(
        SharedString::from(format!("list-{id}")),
        total,
        cx.processor(move |v: &mut WasmView, shown: Range<usize>, window, cx| {
            virtual_rows(v, &list_id, shown, window, cx)
        }),
    )
    .track_scroll(&handle)
    .size_full();
    let thumb = super::scrollbar::thumb_for(id, &handle, b.t, cx);
    let el = div().relative().w_full().child(rows).children(thumb);
    let mut el = super::scrollbar::follow_drag(el, id, &handle, cx);
    if *font == Font::Mono {
        el = el.font_family(b.mono.clone());
    }
    el = match layout.height {
        Size::Fixed(height) => el.h(z(height)),
        _ if ctx.in_row => el.h_full(),
        _ => el.flex_1().min_h_0(),
    };
    el = match layout.width {
        Size::Fill if ctx.in_row => el.flex_1().min_w_0(),
        Size::Fixed(width) => el.w(z(width)).flex_shrink_0(),
        _ => el,
    };
    el.into_any_element()
}

/// The rows of virtual list `id` in `shown`, and the module told that they
/// show.
fn virtual_rows(
    v: &mut WasmView,
    id: &str,
    shown: Range<usize>,
    window: &mut Window,
    cx: &mut Context<WasmView>,
) -> Vec<AnyElement> {
    let total = v.lists.get(id).map_or(0, |list| list.count);
    let count = shown.end - shown.start;
    // A one-row range is the list measuring itself, not what shows.
    if let Some(list) = v.virtual_lists.get_mut(id)
        && (count > 1 || total <= 1)
        && list.sent != Some((shown.start, count))
    {
        list.sent = Some((shown.start, count));
        let (first, count) = (shown.start as u64, count as u64);
        let id = id.to_owned();
        v.send_events(vec![Event::VisibleRange { id, first, count }], None);
    }
    let blank = |shown: Range<usize>| shown.map(|_| div().into_any_element()).collect();
    let Some(frame) = v.frame.take() else {
        return blank(shown);
    };
    let Some(Node::List {
        items,
        selected,
        dense,
        style,
        font,
        window: Some(range),
        ..
    }) = find_list(&frame.ui, id)
    else {
        v.frame = Some(frame);
        return blank(shown);
    };
    let t = Colors::of(&v.theme);
    let mut b = Build {
        popups: Vec::new(),
        t: &t,
        mono: SharedString::from(v.mono.family.clone()),
    };
    let height = row_height(*dense, *font);
    let rows = Rows {
        selected: v.lists.get(id).map_or(*selected, |list| list.selected),
        dense: *dense,
        soft: *style == ListStyle::Soft,
        height: Some(height),
    };
    let first = range.first as usize;
    let built = shown
        .map(
            |index| match index.checked_sub(first).and_then(|at| items.get(at)) {
                Some(item) => row(v, &mut b, id, item, index, rows, window, cx),
                None => div().w_full().h(z(height)).into_any_element(),
            },
        )
        .collect();
    v.frame = Some(frame);
    built
}

/// The list with this id in the tree.
fn find_list<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
    match n {
        Node::List { id: found, .. } if found == id => Some(n),
        Node::List { items, .. } => items
            .iter()
            .filter_map(|item| item.child.as_deref())
            .find_map(|child| find_list(child, id)),
        Node::Column { children, .. } | Node::Row { children, .. } => {
            children.iter().find_map(|child| find_list(child, id))
        }
        Node::Popup { child, .. } => find_list(child, id),
        _ => None,
    }
}
