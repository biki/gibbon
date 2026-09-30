//! Hover highlights that fade in and out. A `.hover()` style in GPUI
//! switches at once, so rows paint their highlight with `hover_fill`.

use std::time::Duration;

use gpui_kit::base::motion::{self, MotionStatus, Transition};
use gpui_kit::*;

/// How long a highlight takes to fade in or out.
const FADE: Duration = Duration::from_millis(150);

/// A highlight in `color` under the content of a row, shown while the
/// pointer is on the row. Add it as the first child of a row with an id:
/// the fade keeps its state under that id.
pub(super) fn hover_fill(color: Hsla, radius: Pixels) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let hovered = !cx.has_active_drag() && hitbox.is_hovered(window);
            // Draw the row again when the pointer comes on or goes off it.
            let view = window.current_view();
            window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture && hitbox.is_hovered(window) != hovered {
                    cx.notify(view);
                }
            });
            let target = if hovered { 1. } else { 0. };
            let fade = motion::transition_with_status(
                "hover-fill",
                target,
                Transition::new(FADE),
                window,
                cx,
            );
            if matches!(fade.status, MotionStatus::Delayed | MotionStatus::Running) {
                window.request_animation_frame();
            }
            if fade.value > 0. {
                window.paint_quad(fill(bounds, color.opacity(fade.value)).corner_radii(radius));
            }
        },
    )
    .absolute()
    .inset_0()
}
