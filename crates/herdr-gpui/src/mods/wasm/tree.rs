//! A frame's tree as GPUI elements. Lengths are the protocol's logical
//! pixels; colors come from the window's theme through [`Colors`], or from
//! the module where it names its own.

use super::{
    canvas, list,
    theme::{Colors, hsla},
    view::WasmView,
};
use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, MouseButton, MouseDownEvent, Pixels,
    SharedString, Stateful, Window, div, prelude::*, px, relative,
};
use wasm_mods_protocol::{
    Align, ButtonKind, Event, Font, Justify, Layout, Node, Pad, Paint, Role, Rounding, Scroll,
    Size, Span, TextStyle, Variant, Wrap,
};

/// A protocol length on screen.
pub(super) fn z(value: f32) -> Pixels {
    px(value)
}

/// Where a node is drawn: in a row (else a column), and in a virtual
/// list's row, whose clicks are the list's.
#[derive(Clone, Copy, Default)]
pub(super) struct Ctx {
    pub in_row: bool,
    pub virtual_row: bool,
}

/// What one render shares: popups collected for the top, and the colors.
pub(super) struct Build<'a> {
    pub popups: Vec<AnyElement>,
    pub t: &'a Colors,
    pub mono: SharedString,
}

pub(super) fn role(role: Role, t: &Colors) -> Hsla {
    match role {
        Role::Text | Role::Heading => t.fg,
        Role::Subtext => t.muted,
        Role::Muted => t.dim,
        Role::Accent => t.accent,
        Role::Danger => t.ansi[1],
        Role::Success => t.ansi[2],
        Role::Warning => t.ansi[3],
        Role::Info => t.ansi[4],
        Role::Space => t.bg,
        Role::Surface => t.surface,
        Role::Border | Role::Divider => t.border,
    }
}

fn paint(paint: &Paint, t: &Colors) -> Hsla {
    match paint {
        Paint::Rgba(color) => hsla(*color),
        Paint::Role(r) => role(*r, t),
    }
}

fn pad(pad: Pad) -> Option<f32> {
    match pad {
        Pad::Auto => None,
        Pad::None => Some(0.),
        Pad::Xs => Some(4.),
        Pad::S => Some(6.),
        Pad::M => Some(10.),
        Pad::L => Some(14.),
    }
}

/// Width and height from a layout, in a row or a column.
pub(super) fn sized<E: Styled>(mut el: E, layout: &Layout, in_row: bool) -> E {
    match layout.width {
        // Filling part of a row: clipped to its share, so halves side by
        // side do not overlap.
        Size::Fill if in_row => el = el.flex_1().min_w_0().overflow_hidden(),
        Size::Fill => el = el.w_full(),
        Size::Fixed(width) => el = el.w(z(width)).flex_shrink_0(),
        Size::Auto | Size::Hug => {}
    }
    match layout.height {
        Size::Fill if in_row => el = el.h_full(),
        Size::Fill => el = el.flex_1().min_h_0(),
        Size::Fixed(height) => el = el.h(z(height)),
        Size::Auto | Size::Hug => {}
    }
    el
}

/// Whether the tree holds a virtual list, which scrolls itself and fills the
/// room it is given.
pub(super) fn holds_virtual(node: &Node) -> bool {
    match node {
        Node::List { window, .. } => window.is_some(),
        Node::Column { children, .. } | Node::Row { children, .. } => {
            children.iter().any(holds_virtual)
        }
        _ => false,
    }
}

pub(super) fn node(
    v: &mut WasmView,
    b: &mut Build,
    n: &Node,
    ctx: Ctx,
    window: &mut Window,
    cx: &mut Context<WasmView>,
) -> AnyElement {
    match n {
        Node::Empty => div().into_any_element(),
        Node::Column { .. } | Node::Row { .. } => container(v, b, n, ctx, window, cx),
        Node::Text {
            text: s,
            style,
            spans,
            color,
            size,
            bold,
            font,
            wrap,
        } => {
            let look = Look {
                style: *style,
                color: color.as_ref(),
                size: *size,
                bold: *bold,
                font: *font,
                wrap: *wrap,
            };
            text(b, s, spans, look, ctx, cx)
        }
        Node::TextField {
            id,
            layout,
            monospace,
            ..
        } => field(v, b, id, layout, *monospace, ctx.in_row),
        Node::List {
            window: Some(range),
            ..
        } => list::virtual_list(v, b, n, range, ctx, cx),
        Node::List { .. } => list::list(v, b, n, ctx, window, cx),
        Node::Canvas { .. } => canvas::canvas(v, b, n, ctx.in_row, cx),
        Node::Popup {
            id,
            modal,
            child,
            style,
            ..
        } => {
            popup(v, b, id, *modal, child, *style, window, cx);
            div().into_any_element()
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn popup(
    v: &mut WasmView,
    b: &mut Build,
    id: &str,
    modal: bool,
    child: &Node,
    style: wasm_mods_protocol::Style,
    window: &mut Window,
    cx: &mut Context<WasmView>,
) {
    v.open_popup = Some(id.to_owned());
    let layout = Layout {
        padding: Pad::M,
        ..Default::default()
    };
    let panel = Node::Column {
        children: vec![child.clone()],
        gap: 0.,
        layout,
        style,
        id: None,
        tooltip: None,
    };
    let t = b.t;
    let (bg, border, scrim) = (t.popover, t.border, t.bg.opacity(0.6));
    let panel = container(v, b, &panel, Ctx::default(), window, cx);
    let dismiss = id.to_owned();
    let overlay = div()
        .id(SharedString::from(format!("popup-{id}")))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .when(modal, |overlay| overlay.bg(scrim))
        .on_click(cx.listener(move |v, _: &ClickEvent, _, _| {
            if !modal {
                v.send_events(
                    vec![Event::Dismissed {
                        id: dismiss.clone(),
                    }],
                    None,
                );
            }
        }))
        .child(
            div()
                .id(SharedString::from(format!("popup-panel-{id}")))
                .bg(bg)
                .border_1()
                .border_color(border)
                .rounded(z(10.))
                .shadow_lg()
                .max_w(relative(0.9))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(panel),
        );
    b.popups.push(overlay.into_any_element());
}

fn container(
    v: &mut WasmView,
    b: &mut Build,
    n: &Node,
    ctx: Ctx,
    window: &mut Window,
    cx: &mut Context<WasmView>,
) -> AnyElement {
    let (row, children, gap, layout, style, id) = match n {
        Node::Column {
            children,
            gap,
            layout,
            style,
            id,
            ..
        } => (false, children, *gap, layout, style, id.as_deref()),
        Node::Row {
            children,
            gap,
            layout,
            style,
            id,
            ..
        } => (true, children, *gap, layout, style, id.as_deref()),
        _ => return div().into_any_element(),
    };
    let t = b.t;
    let mut el = div().flex().gap(z(gap));
    el = if row { el.flex_row() } else { el.flex_col() };
    let (el, auto_pad, hover) = variant(el, style.variant, t);
    let mut el = match pad(layout.padding) {
        Some(p) => el.p(z(p)),
        None => el.py(z(auto_pad.0)).px(z(auto_pad.1)),
    };
    if let Some(bg) = &style.bg {
        el = el.bg(paint(bg, t));
    }
    if let Some(fg) = &style.fg {
        el = el.text_color(paint(fg, t));
    }
    if let Some(border) = &style.border {
        el = el.border_1().border_color(paint(border, t));
    }
    if let Some(rounding) = style.rounding {
        el = el.rounded(z(match rounding {
            Rounding::None => 0.,
            Rounding::S => 4.,
            Rounding::M => 6.,
            Rounding::L => 10.,
            Rounding::Full => 999.,
        }));
    }
    if let Some(opacity) = style.opacity {
        el = el.opacity(opacity);
    }
    el = match layout.align {
        Align::Center => el.items_center(),
        Align::Auto if row => el.items_center(),
        Align::End => el.items_end(),
        Align::Start => el.items_start(),
        Align::Auto => el,
    };
    el = match layout.justify {
        Justify::Center => el.justify_center(),
        Justify::End => el.justify_end(),
        Justify::SpaceBetween => el.justify_between(),
        Justify::Start if row && layout.wrap != Some(false) => el.flex_wrap(),
        Justify::Start => el,
    };
    el = sized(el, layout, ctx.in_row);
    // Holding a virtual list, which scrolls itself: it takes the height it
    // is given.
    if layout.height == Size::Auto && holds_virtual(n) {
        el = if ctx.in_row {
            el.h_full()
        } else {
            el.flex_1().min_h_0()
        };
    }
    let inner = Ctx {
        in_row: row,
        virtual_row: ctx.virtual_row,
    };
    let kids: Vec<AnyElement> = children
        .iter()
        .map(|child| node(v, b, child, inner, window, cx))
        .collect();
    let el = el.children(kids);
    let scroll = layout.scroll;
    let Some(id) = id.filter(|_| !ctx.virtual_row) else {
        if scroll == Scroll::None {
            return el.into_any_element();
        }
        // Scrolling needs an id; this one is stable while the tree's shape is.
        let id = SharedString::from(format!("s-{:p}", children.as_ptr()));
        return scrolled(el.id(id), scroll).into_any_element();
    };
    let (click, secondary) = (id.to_owned(), id.to_owned());
    let el = el
        .id(SharedString::from(format!("n-{id}")))
        .cursor_pointer()
        .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
            v.send_events(vec![Event::Clicked { id: click.clone() }], None);
            cx.stop_propagation();
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |v, event: &MouseDownEvent, _, cx| {
                let (x, y) = (f32::from(event.position.x), f32::from(event.position.y));
                let id = secondary.clone();
                v.send_events(vec![Event::Secondary { id, x, y }], None);
                cx.stop_propagation();
            }),
        );
    let el = match hover {
        Hover::None => el,
        Hover::Fill(fill) => el.hover(move |style| style.bg(fill)),
        Hover::Dim => el.hover(|style| style.opacity(0.88)),
    };
    scrolled(el, scroll).into_any_element()
}

enum Hover {
    None,
    Fill(Hsla),
    Dim,
}

/// A container's look for its variant: its fill and border, its padding
/// when the layout leaves it (vertical, horizontal), and its hover.
fn variant(el: Div, variant: Variant, t: &Colors) -> (Div, (f32, f32), Hover) {
    match variant {
        Variant::Plain => (el, (0., 0.), Hover::None),
        Variant::Card => (
            el.bg(t.surface)
                .border_1()
                .border_color(t.border)
                .rounded(z(8.)),
            (10., 10.),
            Hover::None,
        ),
        Variant::Field => (
            el.bg(t.bg).border_1().border_color(t.input).rounded(z(6.)),
            (6., 6.),
            Hover::None,
        ),
        Variant::Pill => (
            el.bg(t.raised).rounded_full().text_size(z(11.5)),
            (2., 8.),
            Hover::None,
        ),
        Variant::Selected => (el.bg(t.selected).rounded(z(6.)), (6., 6.), Hover::None),
        Variant::Button(kind) => {
            let el = el
                .rounded(z(6.))
                .items_center()
                .justify_center()
                .text_size(z(12.5))
                .font_weight(FontWeight::MEDIUM);
            let (el, hover) = match kind {
                ButtonKind::Primary => (el.bg(t.solid).text_color(t.solid_fg), Hover::Dim),
                ButtonKind::Ghost => (el, Hover::Fill(t.hover)),
                ButtonKind::Danger => (
                    el.border_1().border_color(t.ansi[1]).text_color(t.ansi[1]),
                    Hover::Fill(t.hover),
                ),
                ButtonKind::Secondary => (
                    el.bg(t.raised).border_1().border_color(t.input),
                    Hover::Fill(t.hover),
                ),
            };
            (el, (5., 12.), hover)
        }
    }
}

fn scrolled(el: Stateful<Div>, scroll: Scroll) -> Stateful<Div> {
    match scroll {
        Scroll::None => el,
        Scroll::Vertical => el.overflow_y_scroll(),
        Scroll::Horizontal => el.overflow_x_scroll(),
        Scroll::Both => el.overflow_scroll(),
    }
}

/// How a text node asks to look.
struct Look<'a> {
    style: TextStyle,
    color: Option<&'a Paint>,
    size: Option<f32>,
    bold: bool,
    font: Font,
    wrap: Wrap,
}

fn text(
    b: &Build,
    s: &str,
    spans: &[Span],
    look: Look,
    ctx: Ctx,
    cx: &mut Context<WasmView>,
) -> AnyElement {
    let t = b.t;
    let mut el = div();
    let (mut size, mut color) = (13., None);
    match look.style {
        TextStyle::Heading => {
            size = 15.;
            el = el.font_weight(FontWeight::SEMIBOLD);
        }
        TextStyle::Title => {
            size = 18.;
            el = el.font_weight(FontWeight::SEMIBOLD);
        }
        TextStyle::Muted => color = Some(t.muted),
        TextStyle::Accent => color = Some(t.accent),
        TextStyle::Small => {
            size = 11.;
            color = Some(t.dim);
        }
        TextStyle::Mono => size = 12.5,
        TextStyle::Body => {}
    }
    if look.style == TextStyle::Mono || look.font == Font::Mono {
        el = el.font_family(b.mono.clone());
    }
    if let Some(asked) = look.size {
        size = asked;
    }
    if look.bold {
        el = el.font_weight(FontWeight::SEMIBOLD);
    }
    if let Some(asked) = look.color {
        color = Some(paint(asked, t));
    }
    el = el.text_size(z(size));
    if let Some(color) = color {
        el = el.text_color(color);
    }
    if spans.is_empty() {
        el = match look.wrap {
            Wrap::Extend => el.whitespace_nowrap().flex_shrink_0(),
            // One line cut with an ellipsis, in the room its row leaves.
            Wrap::Truncate => el.w_full().min_w_0().flex_1().truncate(),
            Wrap::Wrap => el,
        };
        return el.child(s.to_owned()).into_any_element();
    }
    // On one line when it extends (a diff line) or is cut, else wrapping
    // between runs.
    el = match look.wrap {
        Wrap::Extend => el.flex().flex_row().whitespace_nowrap().flex_shrink_0(),
        Wrap::Truncate => el
            .flex()
            .flex_row()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap(),
        Wrap::Wrap => el.flex().flex_row().flex_wrap(),
    };
    el.children(spans.iter().enumerate().map(|(index, span)| {
        let mut run = div().flex_shrink_0().child(span.text.clone());
        if let Some(color) = span.color {
            run = run.text_color(hsla(color));
        }
        if let Some(bg) = span.bg {
            run = run.bg(hsla(bg)).rounded(z(3.));
        }
        match &span.link {
            Some(target) if !ctx.virtual_row && is_web(target) => {
                let target = target.clone();
                run.id(SharedString::from(format!("link-{index}-{target}")))
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_url(&target)))
                    .into_any_element()
            }
            _ => run.into_any_element(),
        }
    }))
    .into_any_element()
}

/// Only web links open from a module's text; a file path is not followed.
fn is_web(target: &str) -> bool {
    target.starts_with("https://") || target.starts_with("http://")
}

fn field(
    v: &WasmView,
    b: &Build,
    id: &str,
    layout: &Layout,
    monospace: bool,
    in_row: bool,
) -> AnyElement {
    let Some(field) = v.fields.get(id) else {
        return div().into_any_element();
    };
    let input = div()
        .flex()
        .items_center()
        .w_full()
        .min_w(z(120.))
        .when(monospace, |input| input.font_family(b.mono.clone()))
        .child(field.input.clone());
    // The press is the field's: the view's own focus must not take it back.
    let wrap = div()
        .flex()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(input);
    sized(wrap, layout, in_row).into_any_element()
}
