//! What the pointer shows: hover highlights that show at once and fade
//! out, and the pointing hand over controls.
//!
//! A `.hover()` style in GPUI switches at once both ways, so rows paint
//! their highlight with `hover_fill`.
//!
//! The kit gives its buttons the arrow, and its checkboxes the cursor of the
//! element under them. Make them with `button` and `checkbox` to show the
//! pointing hand, and turn them off with `off`, not `disabled`.

use std::time::Duration;

use gpui_kit::base::motion::{self, Transition};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::*;

/// How long a highlight takes to fade out. It shows without a fade: a fade
/// in makes the pointer feel slow, and each frame of a fade renders the
/// view again.
const FADE_OUT: Duration = Duration::from_millis(120);

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
            let (target, fade) = if hovered {
                (1., Duration::ZERO)
            } else {
                (0., FADE_OUT)
            };
            // While it fades, the transition asks for the next frame.
            let value = motion::transition_with_status(
                "hover-fill",
                target,
                Transition::new(fade),
                window,
                cx,
            )
            .value;
            if value > 0. {
                window.paint_quad(fill(bounds, color.opacity(value)).corner_radii(radius));
            }
        },
    )
    .absolute()
    .inset_0()
}

/// A kit button that shows the pointing hand.
pub(super) fn button(id: impl Into<ElementId>) -> Button {
    Button::new(id).cursor_pointer()
}

/// A kit checkbox that shows the pointing hand.
pub(super) fn checkbox(id: impl Into<ElementId>) -> Checkbox {
    Checkbox::new(id).cursor_pointer()
}

pub(super) trait Off: Disableable + Styled + Sized {
    /// Disable the control while `off`, and show the arrow on it. The kit
    /// keeps our cursor in its disabled style, so `disabled` alone keeps the
    /// pointing hand.
    fn off(self, off: bool) -> Self {
        let this = self.disabled(off);
        if off { this.cursor_default() } else { this }
    }
}

impl<T: Disableable + Styled> Off for T {}
