//! Canvases: shapes painted straight into the window, panned and zoomed
//! here. The module draws in world coordinates and hears what shows through
//! `Event::View`. Pictures on a canvas are not drawn yet.

use super::{
    theme::hsla,
    tree::{Build, sized, z},
    view::WasmView,
};
use gpui::{
    AnyElement, App, Bounds, ClickEvent, ContentMask, Context, Hsla, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, ScrollWheelEvent, SharedString,
    TextAlign, TextRun, Window, div, fill, point, prelude::*, px, size,
};
use std::{cell::Cell, collections::HashMap, rc::Rc, sync::Arc};
use wasm_mods_protocol::{Coords, Event, Mods, Node, PanZoom, PointerKind, Shape, Size};

#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    x: f32,
    y: f32,
    zoom: f32,
}

const UNMOVED: View = View {
    x: 0.,
    y: 0.,
    zoom: 1.,
};

/// What a view was last fitted to: the canvas's size and its content's.
type Fitted = (f32, f32, Option<(f32, f32)>);

#[derive(Default)]
pub(crate) struct CanvasState {
    /// Its shapes, taken out of the frame when it arrives.
    shapes: Arc<Vec<Shape>>,
    w: f32,
    h: f32,
    view: Option<View>,
    moved: bool,
    reset: u64,
    fitted: Option<Fitted>,
    down: Option<Down>,
    sent_view: Option<View>,
    /// Where it was last painted, in window pixels.
    origin: Rc<Cell<Point<Pixels>>>,
}

#[derive(Clone, Copy)]
struct Down {
    pan: bool,
    x: f32,
    y: f32,
}

/// Takes the canvases' shapes out of a new frame into their states.
pub(super) fn take_shapes(v: &mut WasmView, ui: &mut Node) {
    let mut seen = Vec::new();
    let mut nodes = vec![ui];
    while let Some(node) = nodes.pop() {
        match node {
            Node::Canvas { id, shapes, .. } => {
                seen.push(id.clone());
                let canvas = v.canvases.entry(id.clone()).or_default();
                canvas.shapes = Arc::new(std::mem::take(shapes));
            }
            Node::Column { children, .. } | Node::Row { children, .. } => {
                nodes.extend(children.iter_mut());
            }
            Node::Popup { child, .. } => nodes.push(child),
            Node::List { items, .. } => {
                nodes.extend(
                    items
                        .iter_mut()
                        .filter_map(|item| item.child.as_deref_mut()),
                );
            }
            _ => {}
        }
    }
    // A canvas gone from the frame starts afresh if it comes back.
    v.canvases.retain(|id, _| seen.contains(id));
}

/// The zoom a view may take, as the module asked when that makes sense: a
/// bound that is not a positive number is ignored, and swapped bounds are
/// put in order, so clamping never panics on what a module sent.
pub(super) fn zoom_range(pz: &PanZoom) -> (f32, f32) {
    let sane = |zoom: f32| (zoom.is_finite() && zoom > 0.).then_some(zoom);
    let (lo, hi) = (
        sane(pz.min_zoom).unwrap_or(0.01),
        sane(pz.max_zoom).unwrap_or(100.),
    );
    (lo.min(hi), lo.max(hi))
}

fn fit(c: &CanvasState, pz: &PanZoom) -> View {
    let Some((cw, ch)) = pz.content.filter(|_| c.w > 0. && c.h > 0.) else {
        return UNMOVED;
    };
    let zoom = (c.w / cw)
        .min(c.h / ch)
        .clamp(zoom_range(pz).0, zoom_range(pz).1);
    View {
        x: -(c.w / zoom - cw) / 2.,
        y: -(c.h / zoom - ch) / 2.,
        zoom,
    }
}

fn zoom_at(c: &mut CanvasState, pz: &PanZoom, factor: f32, at_x: f32, at_y: f32) {
    let Some(view) = c.view else {
        return;
    };
    let zoom = (view.zoom * factor).clamp(zoom_range(pz).0, zoom_range(pz).1);
    let (world_x, world_y) = (view.x + at_x / view.zoom, view.y + at_y / view.zoom);
    c.view = Some(View {
        x: world_x - at_x / zoom,
        y: world_y - at_y / zoom,
        zoom,
    });
    c.moved = true;
}

/// Tells the module what a canvas with a view shows, when that changed.
fn send_view(v: &mut WasmView, id: &str) {
    let Some(c) = v.canvases.get_mut(id) else {
        return;
    };
    let Some(view) = c.view.filter(|_| c.w > 0.) else {
        return;
    };
    if c.sent_view == Some(view) {
        return;
    }
    c.sent_view = Some(view);
    let event = Event::View {
        id: id.to_owned(),
        x: view.x,
        y: view.y,
        width: c.w / view.zoom,
        height: c.h / view.zoom,
        zoom: view.zoom,
    };
    v.send_events(vec![event], None);
}

/// Refits a view the user has not moved (a new size or content), or one the
/// module reset.
fn settle(v: &mut WasmView, id: &str, pz: Option<&PanZoom>) {
    let Some(c) = v.canvases.get_mut(id) else {
        return;
    };
    let Some(pz) = pz else {
        c.view = None;
        return;
    };
    let fitted = Some((c.w, c.h, pz.content));
    if c.view.is_none() || pz.reset > c.reset || (!c.moved && c.fitted != fitted) {
        c.view = Some(fit(c, pz));
        c.reset = pz.reset;
        c.moved = false;
        c.fitted = fitted;
        if c.w > 0. {
            send_view(v, id);
        }
    }
}

fn mods(modifiers: &gpui::Modifiers) -> Mods {
    Mods {
        ctrl: modifiers.control,
        shift: modifiers.shift,
        alt: modifiers.alt,
    }
}

fn local(p: Point<Pixels>, origin: Point<Pixels>) -> (f32, f32) {
    (f32::from(p.x - origin.x), f32::from(p.y - origin.y))
}

pub(super) fn canvas(
    v: &mut WasmView,
    b: &Build,
    n: &Node,
    in_row: bool,
    cx: &mut Context<WasmView>,
) -> AnyElement {
    let Node::Canvas {
        id,
        width,
        height,
        layout,
        coords,
        view: pz,
        sense,
        ..
    } = n
    else {
        return div().into_any_element();
    };
    let (id, unit, pz, sense) = (id.clone(), *coords == Coords::Unit, *pz, *sense);
    v.canvases.entry(id.clone()).or_default();
    settle(v, &id, pz.as_ref());
    let Some(c) = v.canvases.get(&id) else {
        return div().into_any_element();
    };
    let (w, h, view) = (c.w, c.h, c.view.unwrap_or(UNMOVED));
    let (origin, shapes) = (c.origin.clone(), c.shapes.clone());

    let mut el = div()
        .id(SharedString::from(format!("cv-{id}")))
        .relative()
        .flex_shrink_0()
        .overflow_hidden()
        .h(z(*height));
    el = if layout.width == Size::Fill || (*width == 0. && unit) {
        if in_row { el.flex_1() } else { el.w_full() }
    } else {
        el.w(z(*width))
    };
    el = sized(el, layout, in_row);

    // This element's pixels to the world, as events carry them.
    let to_world = move |p: Point<Pixels>, c: &CanvasState| {
        let view = c.view.unwrap_or(UNMOVED);
        let (lx, ly) = local(p, c.origin.get());
        let (x, y) = (lx / view.zoom + view.x, ly / view.zoom + view.y);
        if unit {
            (x / c.w.max(1.), y / c.h.max(1.))
        } else {
            (x, y)
        }
    };
    let pointer = {
        let id = id.clone();
        move |v: &mut WasmView, kind: PointerKind, p: Point<Pixels>, m: &gpui::Modifiers| {
            let Some(c) = v.canvases.get(&id) else {
                return;
            };
            let (x, y) = to_world(p, c);
            let event = Event::Pointer {
                id: id.clone(),
                x,
                y,
                kind,
            };
            v.send_events(vec![event], Some(mods(m)));
        }
    };

    let (cid, press) = (id.clone(), pointer.clone());
    el = el.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |v, e: &MouseDownEvent, window, cx| {
            let Some(c) = v.canvases.get_mut(&cid) else {
                return;
            };
            let (x, y) = local(e.position, c.origin.get());
            let pan = pz.is_some_and(|pz| pz.drag_pans);
            c.down = Some(Down { pan, x, y });
            if !pan {
                press(v, PointerKind::Down, e.position, &e.modifiers);
            }
            window.focus(&v.focus, cx);
            cx.stop_propagation();
        }),
    );
    let cid = id.clone();
    el = el.on_mouse_down(
        MouseButton::Middle,
        cx.listener(move |v, e: &MouseDownEvent, _, cx| {
            if pz.is_none() {
                return;
            }
            let Some(c) = v.canvases.get_mut(&cid) else {
                return;
            };
            let (x, y) = local(e.position, c.origin.get());
            c.down = Some(Down { pan: true, x, y });
            cx.stop_propagation();
        }),
    );
    let (cid, moved) = (id.clone(), pointer.clone());
    el = el.on_mouse_move(cx.listener(move |v, e: &MouseMoveEvent, _, cx| {
        let Some(c) = v.canvases.get_mut(&cid) else {
            return;
        };
        let (x, y) = local(e.position, c.origin.get());
        match (c.down, c.view) {
            (Some(down), Some(view)) if down.pan => {
                c.view = Some(View {
                    x: view.x - (x - down.x) / view.zoom,
                    y: view.y - (y - down.y) / view.zoom,
                    ..view
                });
                c.down = Some(Down { x, y, ..down });
                c.moved = true;
                cx.notify();
            }
            (Some(_), _) if e.dragging() => moved(v, PointerKind::Drag, e.position, &e.modifiers),
            (None, _) if sense.hover => moved(v, PointerKind::Move, e.position, &e.modifiers),
            _ => {}
        }
    }));
    for button in [MouseButton::Left, MouseButton::Middle] {
        let (cid, release) = (id.clone(), pointer.clone());
        el = el.on_mouse_up(
            button,
            cx.listener(move |v, e: &MouseUpEvent, _, _| {
                let Some(c) = v.canvases.get_mut(&cid) else {
                    return;
                };
                let Some(down) = c.down.take() else {
                    return;
                };
                if down.pan {
                    send_view(v, &cid);
                } else {
                    release(v, PointerKind::Up, e.position, &e.modifiers);
                }
            }),
        );
    }
    if sense.hover {
        let cid = id.clone();
        el = el.on_hover(cx.listener(move |v, hovered: &bool, _, _| {
            let kind = if *hovered {
                PointerKind::Enter
            } else {
                PointerKind::Leave
            };
            let event = Event::Pointer {
                id: cid.clone(),
                x: 0.,
                y: 0.,
                kind,
            };
            v.send_events(vec![event], None);
        }));
    }
    el = wheel(el, &id, pz, sense.wheel, to_world, cx);

    // Its size and place come from layout: a new size refits the view and
    // tells the module.
    let weak = cx.entity().downgrade();
    let mono = b.mono.clone();
    let cid = id.clone();
    let painter = gpui::canvas(
        move |bounds, _, cx| {
            origin.set(bounds.origin);
            let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            if size != (w, h) {
                let _ = weak.update(cx, |v, cx| {
                    if let Some(c) = v.canvases.get_mut(&cid) {
                        (c.w, c.h) = size;
                    }
                    settle(v, &cid, pz.as_ref());
                    if pz.is_some() {
                        send_view(v, &cid);
                    }
                    cx.notify();
                });
            }
            bounds
        },
        move |_, bounds: Bounds<Pixels>, window, cx| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                paint(&shapes, bounds, view, unit, &mono, window, cx);
            });
        },
    )
    .absolute()
    .size_full();
    el.child(painter).into_any_element()
}

/// The wheel: pans and zooms a canvas with a view here, else is sent to the
/// module when it asks for it.
fn wheel(
    el: gpui::Stateful<gpui::Div>,
    id: &str,
    pz: Option<PanZoom>,
    wants: bool,
    to_world: impl Fn(Point<Pixels>, &CanvasState) -> (f32, f32) + 'static,
    cx: &mut Context<WasmView>,
) -> gpui::Stateful<gpui::Div> {
    let cid = id.to_owned();
    if let Some(pz) = pz {
        let el = el.on_scroll_wheel(cx.listener(move |v, e: &ScrollWheelEvent, window, cx| {
            let delta = e.delta.pixel_delta(window.line_height());
            let Some(c) = v.canvases.get_mut(&cid) else {
                return;
            };
            if e.modifiers.control {
                let (x, y) = local(e.position, c.origin.get());
                zoom_at(c, &pz, (f32::from(delta.y) * 0.003).exp(), x, y);
            } else if let Some(view) = c.view {
                let (dx, dy) = if e.modifiers.shift && delta.x == px(0.) {
                    (delta.y, px(0.))
                } else {
                    (delta.x, delta.y)
                };
                c.view = Some(View {
                    x: view.x - f32::from(dx) / view.zoom,
                    y: view.y - f32::from(dy) / view.zoom,
                    ..view
                });
                c.moved = true;
            }
            send_view(v, &cid);
            cx.stop_propagation();
            cx.notify();
        }));
        let cid = id.to_owned();
        return el.on_click(cx.listener(move |v, e: &ClickEvent, _, cx| {
            if e.click_count() < 2 {
                return;
            }
            let Some(c) = v.canvases.get_mut(&cid) else {
                return;
            };
            c.view = Some(fit(c, &pz));
            c.moved = false;
            send_view(v, &cid);
            cx.notify();
        }));
    }
    if !wants {
        return el;
    }
    el.on_scroll_wheel(cx.listener(move |v, e: &ScrollWheelEvent, window, cx| {
        let delta = e.delta.pixel_delta(window.line_height());
        let Some(c) = v.canvases.get(&cid) else {
            return;
        };
        let (x, y) = to_world(e.position, c);
        let id = cid.clone();
        let event = if e.modifiers.control {
            let factor = (f32::from(delta.y) * 0.003).exp();
            Event::Zoom { id, factor, x, y }
        } else {
            let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
            Event::Wheel { id, dx, dy, x, y }
        };
        v.send_events(vec![event], Some(mods(&e.modifiers)));
        cx.stop_propagation();
    }))
}

/// Strokes and fills batched by color, drawn in the order each first
/// appeared; anything else ends the run.
#[derive(Default)]
struct Batch {
    order: Vec<(u64, Hsla)>,
    paths: HashMap<u64, PathBuilder>,
}

impl Batch {
    fn flush(&mut self, window: &mut Window) {
        for (key, color) in self.order.drain(..) {
            if let Some(path) = self.paths.remove(&key).and_then(|b| b.build().ok()) {
                window.paint_path(path, color);
            }
        }
    }

    /// The builder for a fill, or a stroke `width` wide, of `color`.
    fn builder(&mut self, stroke: Option<f32>, color: u32) -> Option<&mut PathBuilder> {
        let key = match stroke {
            Some(width) => (u64::from(color) << 32) | u64::from(width.to_bits() | 1),
            None => u64::from(color) << 32,
        };
        let order = &mut self.order;
        Some(self.paths.entry(key).or_insert_with(|| {
            order.push((key, hsla(color)));
            match stroke {
                Some(width) => PathBuilder::stroke(px(width.max(0.5))),
                None => PathBuilder::fill(),
            }
        }))
    }
}

fn paint(
    shapes: &[Shape],
    bounds: Bounds<Pixels>,
    view: View,
    unit: bool,
    mono: &SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let zoom = view.zoom;
    let sx = |n: f32| ((if unit { n * bw } else { n }) - view.x) * zoom + ox;
    let sy = |n: f32| ((if unit { n * bh } else { n }) - view.y) * zoom + oy;
    let sw = |n: f32| (if unit { n * bw } else { n }) * zoom;
    let sh = |n: f32| (if unit { n * bh } else { n }) * zoom;
    let visible =
        |x: f32, y: f32, w: f32, h: f32| x + w >= ox && y + h >= oy && x <= ox + bw && y <= oy + bh;
    let p = |x: f32, y: f32| point(px(x), px(y));
    let mut batch = Batch::default();
    for shape in shapes {
        match shape {
            Shape::Rect {
                x,
                y,
                w,
                h,
                radius,
                fill: color,
                stroke,
            } => {
                let (x, y, w, h) = (sx(*x), sy(*y), sw(*w), sh(*h));
                if !visible(x, y, w, h) {
                    continue;
                }
                if stroke.is_none() && *radius == 0. {
                    if let Some(path) = batch.builder(None, *color) {
                        path.add_polygon(
                            &[p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h)],
                            true,
                        );
                    }
                    continue;
                }
                batch.flush(window);
                let mut quad = fill(Bounds::new(p(x, y), size(px(w), px(h))), hsla(*color))
                    .corner_radii(px(radius * zoom));
                if let Some((width, color)) = stroke {
                    quad = quad.border_widths(px(*width)).border_color(hsla(*color));
                }
                window.paint_quad(quad);
            }
            Shape::Circle {
                x,
                y,
                r,
                fill: color,
            } => {
                let (r, x, y) = (r * zoom, sx(*x), sy(*y));
                if r < 0.2 || !visible(x - r, y - r, 2. * r, 2. * r) {
                    continue;
                }
                batch.flush(window);
                let bounds = Bounds::new(p(x - r, y - r), size(px(2. * r), px(2. * r)));
                window.paint_quad(fill(bounds, hsla(*color)).corner_radii(px(r)));
            }
            Shape::Line {
                x1,
                y1,
                x2,
                y2,
                width,
                color,
            } => {
                if let Some(path) = batch.builder(Some(width * zoom), *color) {
                    path.move_to(p(sx(*x1), sy(*y1)));
                    path.line_to(p(sx(*x2), sy(*y2)));
                }
            }
            Shape::Path {
                points,
                closed,
                fill: color,
                stroke,
            } => {
                let points: Vec<Point<Pixels>> =
                    points.iter().map(|(x, y)| p(sx(*x), sy(*y))).collect();
                if let Some(color) = color
                    && points.len() > 2
                    && let Some(path) = batch.builder(None, *color)
                {
                    path.add_polygon(&points, true);
                }
                if let Some((width, color)) = stroke
                    && let [first, rest @ ..] = points.as_slice()
                    && !rest.is_empty()
                    && let Some(path) = batch.builder(Some(width * zoom), *color)
                {
                    path.move_to(*first);
                    for point in rest {
                        path.line_to(*point);
                    }
                    if *closed {
                        path.line_to(*first);
                    }
                }
            }
            Shape::Text {
                x,
                y,
                text,
                size: s,
                color,
            } => {
                let (s, x, y) = (s * zoom, sx(*x), sy(*y));
                let wide = 0.6 * s * text.chars().count() as f32;
                if s < 2. || !visible(x, y, wide, s * 1.4) || text.contains('\n') {
                    continue;
                }
                batch.flush(window);
                text_at(text, hsla(*color), s, s * 1.25, p(x, y), mono, window, cx);
            }
            // Pictures are not drawn yet.
            Shape::Image { .. } => {}
        }
    }
    batch.flush(window);
}

#[allow(clippy::too_many_arguments)]
fn text_at(
    text: &str,
    color: Hsla,
    font_size: f32,
    line_height: f32,
    at: Point<Pixels>,
    mono: &SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    let run = TextRun {
        len: text.len(),
        font: gpui::font(mono.clone()),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from(text.to_owned()),
        px(font_size),
        &[run],
        None,
    );
    let _ = line.paint(at, px(line_height), TextAlign::Left, None, window, cx);
}
