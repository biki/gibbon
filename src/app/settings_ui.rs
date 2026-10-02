//! The settings dialog (⌘,). Every change applies at once.

use crate::fonts::{self, FontChoice};
use crate::settings::{self, Appearance};
use crate::theme::{self, Palette};

use super::*;

const UI_SIZES: [(&str, f32); 4] = [("12", 12.), ("13", 13.), ("14", 14.), ("15", 15.)];
const CODE_SIZES: [(&str, f32); 4] = [("11", 11.), ("12", 12.), ("13", 13.), ("14", 14.)];
/// Minutes between automatic fetches.
const FETCH_EVERY: [(&str, u32); 4] = [("Off", 0), ("5 min", 5), ("10 min", 10), ("30 min", 30)];

/// The settings dialog. It needs no repository.
pub(super) fn open_settings(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
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
        let diff = segmented(
            "set-diff",
            &[("Unified", false), ("Split", true)],
            s.split_diff,
            |split, _, cx| settings::update(cx, |s| s.split_diff = split),
            cx,
        );
        let wrap = segmented(
            "set-wrap",
            &[("Scroll", false), ("Wrap", true)],
            s.wrap_diff,
            |wrap, _, cx| settings::update_layout(cx, |s| s.wrap_diff = wrap),
            cx,
        );
        let fetch = segmented(
            "set-fetch",
            &FETCH_EVERY,
            s.auto_fetch,
            |n, _, cx| settings::update(cx, |s| s.auto_fetch = n),
            cx,
        );
        let updates = segmented(
            "set-updates",
            &[("Off", false), ("Automatic", true)],
            s.auto_update,
            |on, _, cx| settings::update(cx, |s| s.auto_update = on),
            cx,
        );
        let ui_font = fonts::ui(&s.ui_font);
        let code_font = fonts::code(&s.code_font);
        let section = |label: &'static str, grid: AnyElement, preview: Option<AnyElement>| {
            v_flex()
                .w_full()
                .py_2()
                .gap_2()
                .child(div().font_weight(FontWeight::MEDIUM).child(label))
                .child(grid)
                .children(preview)
        };
        let preview = |family: &'static str, size: f32, text: &'static str| {
            div()
                .px_2()
                .py_1p5()
                .rounded(px(6.))
                .bg(cx.theme().colors.muted)
                .font_family(family)
                .text_size(px(size))
                .truncate()
                .child(text)
                .into_any_element()
        };
        dialog.title("Settings").w(px(640.)).child(
            v_flex()
                .child(row(
                    "Appearance",
                    "Follow macOS, or keep one look.",
                    appearance.into_any_element(),
                ))
                .child(section(
                    "Theme",
                    scheme_grid(theme::current(cx).id, theme::palette(cx).light, cx),
                    None,
                ))
                .child(section(
                    "Interface font",
                    font_grid("ui-font", fonts::UI_FONTS, ui_font.id, false, cx),
                    Some(preview(
                        ui_font.family,
                        s.ui_size,
                        "Browsing feature/login · you stay on main · 3 commits to pick",
                    )),
                ))
                .child(section(
                    "Code font",
                    font_grid("code-font", fonts::CODE_FONTS, code_font.id, true, cx),
                    Some(preview(
                        code_font.family,
                        s.code_size,
                        ".unwrap_or(3000); // Il1| O0 {} => != <=",
                    )),
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
                .child(row("Diffs", "How diffs open.", diff.into_any_element()))
                .child(row(
                    "Long lines",
                    "Scroll sideways to see them, or wrap them.",
                    wrap.into_any_element(),
                ))
                .child(row(
                    "Fetch",
                    "Fetch all remotes on this timer, and when you switch tabs.",
                    fetch.into_any_element(),
                ))
                .child(row(
                    "Updates",
                    "Install new releases from GitHub. Restart Gibbon to use one.",
                    updates.into_any_element(),
                )),
        )
    });
}

/// The frame of a choice in a grid: outlined in the accent while `selected`,
/// highlighted on hover.
fn card(id: (&'static str, usize), selected: bool, cx: &App) -> Stateful<Div> {
    let t = cx.theme();
    div()
        .id(id)
        .min_w_0()
        .px_2()
        .py_1p5()
        .rounded(px(7.))
        .border_1()
        .cursor_pointer()
        .border_color(if selected {
            t.colors.primary
        } else {
            t.colors.border
        })
        .when(selected, |d| d.bg(t.colors.primary.opacity(0.14)))
        .when(!selected, |d| {
            d.child(hover_fill(t.colors.list_hover, px(6.)))
        })
}

/// A card for each scheme: a small window in its colors for the current
/// appearance, its name and its note.
fn scheme_grid(current: &'static str, light: bool, cx: &App) -> AnyElement {
    let muted = cx.theme().colors.muted_foreground;
    div()
        .grid()
        .grid_cols(5)
        .gap(px(6.))
        .children(theme::SCHEMES.iter().enumerate().map(|(i, s)| {
            card(("scheme", i), s.id == current, cx)
                .child(scheme_preview(if light { &s.light } else { &s.dark }))
                .child(div().text_size(px(13.)).truncate().child(s.name))
                .child(div().text_size(px(10.5)).text_color(muted).child(s.note))
                .on_click(move |_, _, cx| settings::update(cx, |set| set.theme = s.id.to_string()))
        }))
        .into_any_element()
}

/// A sidebar, then a pane with an accent pill and a selected row.
fn scheme_preview(p: &Palette) -> impl IntoElement {
    h_flex()
        .h(px(24.))
        .mb_1()
        .rounded(px(5.))
        .border_1()
        .border_color(rgb(p.border))
        .child(
            div()
                .w(px(16.))
                .h_full()
                .rounded_l(px(4.))
                .bg(rgb(p.sidebar)),
        )
        .child(
            v_flex()
                .flex_1()
                .h_full()
                .px_1()
                .gap(px(3.))
                .justify_center()
                .rounded_r(px(4.))
                .bg(rgb(p.bg))
                .child(div().w(px(18.)).h(px(5.)).rounded(px(2.)).bg(rgb(p.accent)))
                .child(
                    div()
                        .w_full()
                        .h(px(5.))
                        .rounded(px(2.))
                        .bg(rgb(p.selection)),
                ),
        )
}

/// Four columns of font cards; each name is set in its own font.
fn font_grid(
    id: &'static str,
    list: &'static [FontChoice],
    current: &'static str,
    code: bool,
    cx: &App,
) -> AnyElement {
    let t = cx.theme();
    div()
        .grid()
        .grid_cols(4)
        .gap(px(6.))
        .children(list.iter().enumerate().map(|(i, f)| {
            card((id, i), f.id == current, cx)
                .child(
                    div()
                        .font_family(f.family)
                        .text_size(px(13.))
                        .truncate()
                        .child(f.name),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(t.colors.muted_foreground)
                        .child(f.note),
                )
                .on_click(move |_, _, cx| {
                    settings::update(cx, |s| {
                        if code {
                            s.code_font = f.id.to_string();
                        } else {
                            s.ui_font = f.id.to_string();
                        }
                    })
                })
        }))
        .into_any_element()
}
