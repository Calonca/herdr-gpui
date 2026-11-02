//! A virtual list's scrollbar (a diff's page): a thumb over the list's
//! right edge that shows where it sits and how much of it shows, and drags
//! it. The size and travel are the tab strip's thumb math, turned on its
//! side, as Settings' and the review's scrollbars are.

use super::{theme::Colors, tree::z, view::WasmView};
use crate::browser::Thumb;
use gpui::{prelude::*, *};

/// How wide the thumb draws, and its margin from the list's edge.
const WIDTH: f32 = 8.;
const INSET: f32 = 2.;

/// How opaque the thumb draws, then hovered or held.
const ALPHA: f32 = 0.35;
const HOVER_ALPHA: f32 = 0.55;
const HELD_ALPHA: f32 = 0.75;

/// What a thumb drags: the list it scrolls.
#[derive(Clone, Debug)]
pub(super) struct ListThumb(String);

/// The thumb for `handle`'s list as it last laid out, with the list's
/// bounds and how far it scrolls; none while every row fits.
fn thumb(handle: &ScrollHandle) -> Option<(Thumb, Bounds<Pixels>, f32)> {
    let bounds = handle.bounds();
    let max = f32::from(handle.max_offset().y);
    let thumb = Thumb::of(
        f32::from(bounds.size.height),
        max,
        -f32::from(handle.offset().y),
    )?;
    Some((thumb, bounds, max))
}

fn base(handle: &UniformListScrollHandle) -> ScrollHandle {
    handle.0.borrow().base_handle.clone()
}

/// The thumb over list `id`'s right edge; none while it all fits.
pub(super) fn thumb_for(
    id: &str,
    handle: &UniformListScrollHandle,
    t: &Colors,
    cx: &mut Context<WasmView>,
) -> Option<AnyElement> {
    let (thumb, _, _) = thumb(&base(handle))?;
    let color = t.muted;
    let grabbed = handle.clone();
    Some(
        div()
            .id(SharedString::from(format!("wasm-list-thumb-{id}")))
            .absolute()
            .right(z(INSET))
            .top(px(thumb.left))
            .w(z(WIDTH))
            .h(px(thumb.width))
            .rounded_full()
            .bg(color.opacity(ALPHA))
            // The wheel still scrolls the list over the thumb.
            .block_mouse_except_scroll()
            .hover(move |s| s.bg(color.opacity(HOVER_ALPHA)))
            .active(move |s| s.bg(color.opacity(HELD_ALPHA)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some((thumb, bounds, _)) = self::thumb(&base(&grabbed)) {
                        v.thumb_grab = f32::from(event.position.y - bounds.top()) - thumb.left;
                    }
                }),
            )
            .on_drag(ListThumb(id.to_owned()), |_, _, _, cx| {
                cx.new(|_| EmptyView)
            })
            .into_any_element(),
    )
}

/// Moves list `id`'s thumb with the pointer at `y`, on `list`, the element
/// around the list; it follows the pointer past the thumb.
pub(super) fn follow_drag(
    list: Div,
    id: &str,
    handle: &UniformListScrollHandle,
    cx: &mut Context<WasmView>,
) -> Div {
    let (id, handle) = (id.to_owned(), handle.clone());
    list.on_drag_move(
        cx.listener(move |v, event: &DragMoveEvent<ListThumb>, _, cx| {
            if event.drag(cx).0 != id {
                return;
            }
            let base = base(&handle);
            let Some((thumb, bounds, max)) = thumb(&base) else {
                return;
            };
            let top = f32::from(event.event.position.y - bounds.top()) - v.thumb_grab;
            let scrolled = thumb.scrolled_at(f32::from(bounds.size.height), max, top);
            let offset = base.offset();
            if (f32::from(offset.y) + scrolled).abs() < 0.5 {
                return;
            }
            base.set_offset(point(offset.x, px(-scrolled)));
            cx.notify();
        }),
    )
}
