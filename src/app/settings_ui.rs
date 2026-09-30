//! The settings dialog (⌘,). Every change applies at once.

use crate::settings::{self, Appearance};

use super::*;

const UI_SIZES: [(&str, f32); 4] = [("12", 12.), ("13", 13.), ("14", 14.), ("15", 15.)];
const CODE_SIZES: [(&str, f32); 4] = [("11", 11.), ("12", 12.), ("13", 13.), ("14", 14.)];

impl GitApp {
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let s = settings::get(cx).clone();
            let muted = cx.theme().colors.muted_foreground;
            let row = |label: &'static str, hint: &'static str, control: AnyElement| {
                h_flex()
                    .w_full()
                    .py_2()
                    .gap_4()
                    .child(
                        v_flex()
                            .flex_1()
                            .child(div().font_weight(FontWeight::MEDIUM).child(label))
                            .child(div().text_size(px(12.)).text_color(muted).child(hint)),
                    )
                    .child(control)
            };
            let appearance = segmented(
                "set-appearance",
                &[
                    ("System", Appearance::System),
                    ("Light", Appearance::Light),
                    ("Dark", Appearance::Dark),
                ],
                s.appearance,
                |a, _, cx| settings::update(cx, |s| s.appearance = a),
                cx,
            );
            let ui = segmented(
                "set-ui",
                &UI_SIZES,
                s.ui_size,
                |n, _, cx| settings::update(cx, |s| s.ui_size = n),
                cx,
            );
            let code = segmented(
                "set-code",
                &CODE_SIZES,
                s.code_size,
                |n, _, cx| settings::update(cx, |s| s.code_size = n),
                cx,
            );
            let this = this.clone();
            let diff = segmented(
                "set-diff",
                &[("Unified", false), ("Split", true)],
                s.split_diff,
                move |split, _, cx| {
                    settings::update(cx, |s| s.split_diff = split);
                    this.update(cx, |app, _| {
                        app.diff_mode = if split {
                            DiffMode::Split
                        } else {
                            DiffMode::Unified
                        }
                    });
                },
                cx,
            );
            dialog.title("Settings").w(px(520.)).child(
                v_flex()
                    .child(row(
                        "Appearance",
                        "Follow macOS, or keep one look.",
                        appearance.into_any_element(),
                    ))
                    .child(row(
                        "Interface text",
                        "Size of the text around the app.",
                        ui.into_any_element(),
                    ))
                    .child(row(
                        "Code text",
                        "Size of the text in diffs.",
                        code.into_any_element(),
                    ))
                    .child(row(
                        "Diffs",
                        "How diffs open.",
                        diff.into_any_element(),
                    )),
            )
        });
    }
}
