//! Diff view: unified or split, with syntax colors, changed words, and —
//! for working-tree files — hunk buttons and line selection.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ops::Range;

use gpui_kit::component::scroll::{Scrollbar, ScrollbarHandle, ScrollbarMode};

use crate::git::{DiffLine, FileChange, LineKind, PatchOp};
use crate::highlight::DiffStyles;

use super::*;

/// Diff text size and row height, from the settings. `uniform_list` gives
/// every row the height of its first row, so every row must use this height:
/// a shorter row leaves a gap below it.
fn metrics(cx: &App) -> (f32, f32) {
    let size = crate::settings::get(cx).code_size;
    (size, (size * 1.6).round())
}
/// Longer lines are cut for display (minified files).
const MAX_COLS: usize = 1200;
/// Widths of a line number in the unified and in the split view, and of the
/// +/− sign before the code.
const NUM_W: f32 = 44.;
const SPLIT_NUM_W: f32 = 48.;
const SIGN_W: f32 = 18.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffMode {
    Unified,
    Split,
}

/// Where a diff comes from; decides which actions it offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffCtx {
    Commit,
    Unstaged,
    Staged,
}

/// The file that a diff shows, shared without a copy: a working-tree
/// file, or a file of a commit or a stash.
#[derive(Clone)]
pub(super) enum DiffFile {
    Own(Rc<FileDiff>),
    Of(Rc<CommitDetail>, usize),
}

impl std::ops::Deref for DiffFile {
    type Target = FileDiff;

    fn deref(&self) -> &FileDiff {
        match self {
            DiffFile::Own(file) => file,
            DiffFile::Of(detail, ix) => &detail.files[*ix],
        }
    }
}

/// One row of the split view.
#[derive(Clone, Copy)]
enum SplitRow {
    Hunk(usize),
    Pair(Option<usize>, Option<usize>),
    Note(usize),
    More,
}

fn split_rows(file: &FileDiff) -> Vec<SplitRow> {
    let lines = &file.lines;
    let mut rows = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            LineKind::Hunk => rows.push(SplitRow::Hunk(i)),
            LineKind::Context => rows.push(SplitRow::Pair(Some(i), Some(i))),
            LineKind::Note => rows.push(SplitRow::Note(i)),
            LineKind::Add => rows.push(SplitRow::Pair(None, Some(i))),
            LineKind::Del => {
                let dels = i;
                while i < lines.len() && lines[i].kind == LineKind::Del {
                    i += 1;
                }
                let adds = i;
                while i < lines.len() && lines[i].kind == LineKind::Add {
                    i += 1;
                }
                let (nd, na) = (adds - dels, i - adds);
                for k in 0..nd.max(na) {
                    rows.push(SplitRow::Pair(
                        (k < nd).then_some(dels + k),
                        (k < na).then_some(adds + k),
                    ));
                }
                continue;
            }
        }
        i += 1;
    }
    if file.truncated {
        rows.push(SplitRow::More);
    }
    rows
}

/// What the diff view derives from its file. The pane keeps it.
struct DiffLayout {
    /// The rows of the split view, empty in the unified view.
    rows: Vec<SplitRow>,
    /// Characters in the longest line that `code` shows.
    cols: usize,
}

impl DiffLayout {
    fn new(file: &FileDiff, mode: DiffMode) -> Self {
        DiffLayout {
            rows: match mode {
                DiffMode::Split => split_rows(file),
                DiffMode::Unified => Vec::new(),
            },
            cols: file
                .lines
                .iter()
                .filter(|l| l.kind != LineKind::Hunk)
                .map(|l| shown(&l.text).chars().count())
                .max()
                .unwrap_or(0),
        }
    }
}

/// The scroll state of a diff view. The list scrolls the rows up and down.
/// The code scrolls sideways by `x` under fixed line numbers, so both sides
/// of the split view scroll together.
#[derive(Clone, Default)]
pub(super) struct DiffScroll(Rc<ScrollState>);

#[derive(Default)]
struct ScrollState {
    list: UniformListScrollHandle,
    /// Can be more than the maximum after the view gets wider: read it with
    /// `x()`.
    x: Cell<Pixels>,
    /// Width of the longest line's text.
    text_w: Cell<Pixels>,
    /// Height of all rows.
    rows_h: Cell<Pixels>,
    split: Cell<bool>,
    /// The list has room below the rows for the horizontal scrollbar.
    room_below: Cell<bool>,
    /// The path of the shown file: another file starts at the left.
    path: RefCell<String>,
    /// The axis of the current scroll gesture, locked as the list locks it.
    gesture: RefCell<OngoingScroll>,
}

impl DiffScroll {
    fn set_file(&self, path: &str, text_w: Pixels, rows_h: Pixels, split: bool) {
        let s = &self.0;
        if *s.path.borrow() != path {
            *s.path.borrow_mut() = path.to_string();
            s.x.set(px(0.));
        }
        s.text_w.set(text_w);
        s.rows_h.set(rows_h);
        s.split.set(split);
    }

    /// Width of one code column, beside the line numbers.
    fn code_w(&self, view_w: Pixels) -> Pixels {
        if self.0.split.get() {
            (view_w - px(1.)) / 2. - px(SPLIT_NUM_W + SIGN_W)
        } else {
            view_w - px(2. * NUM_W + SIGN_W)
        }
    }

    /// Whether the view needs its vertical and its horizontal scrollbar.
    /// Each takes room from the content, so each can make the other needed.
    fn bars(&self) -> (bool, bool) {
        let s = &self.0;
        let view = s.list.viewport_bounds().size;
        let bar = Scrollbar::width();
        let (text, rows) = (s.text_w.get(), s.rows_h.get());
        let mut vertical = rows > view.height;
        let end = if vertical { bar } else { px(0.) };
        let horizontal = text + end > self.code_w(view.width);
        if horizontal {
            vertical = vertical || rows + bar > view.height;
        }
        (vertical, horizontal)
    }

    /// The room below the rows comes from the size of the last frame. If
    /// the view changed size, render again with the right room.
    fn check_room(&self, window: &Window) {
        if self.bars().1 != self.0.room_below.get() {
            window.request_animation_frame();
        }
    }

    fn max_x(&self) -> Pixels {
        let s = &self.0;
        let (vertical, _) = self.bars();
        // The line ends clear the vertical scrollbar.
        let end = if vertical { Scrollbar::width() } else { px(0.) };
        let view = s.list.viewport_bounds().size.width;
        (s.text_w.get() + end - self.code_w(view)).max(px(0.))
    }

    fn x(&self) -> Pixels {
        self.0.x.get().min(self.max_x())
    }

    /// Whether the offset changed.
    fn set_x(&self, x: Pixels) -> bool {
        let x = x.clamp(px(0.), self.max_x());
        self.0.x.replace(x) != x
    }

    /// Scroll sideways by the horizontal part of a wheel event. The list
    /// takes the vertical part.
    fn wheel(&self, e: &ScrollWheelEvent, window: &Window) -> bool {
        let mut delta = e.delta.pixel_delta(window.line_height());
        if e.delta.precise() {
            self.0
                .gesture
                .borrow_mut()
                .filter(&mut delta, e.touch_phase);
        }
        !delta.x.is_zero() && self.set_x(self.x() - delta.x)
    }
}

impl ScrollbarHandle for DiffScroll {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.list.viewport_bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        point(-self.x(), self.0.list.offset().y)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_x(-offset.x);
        self.0.list.set_offset(point(px(0.), offset.y));
    }

    fn content_size(&self) -> Size<Pixels> {
        let list = self.0.list.content_size();
        size(list.width + self.max_x(), list.height)
    }
}

/// Add and Del lines of the hunk that starts at `hunk`.
pub(super) fn hunk_lines(file: &FileDiff, hunk: usize) -> HashSet<usize> {
    (hunk + 1..file.lines.len())
        .take_while(|&i| file.lines[i].kind != LineKind::Hunk)
        .filter(|&i| matches!(file.lines[i].kind, LineKind::Add | LineKind::Del))
        .collect()
}

impl GitApp {
    /// The diff of the shown view: the selected file of the commit, of the
    /// stash, of the review, or of the working tree.
    pub(super) fn render_shown_diff(
        &mut self,
        memo: &mut Memo,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (file, styles, ctx, id) = match self.view {
            View::History => (
                self.detail
                    .clone()
                    .filter(|d| self.detail_file < d.files.len())
                    .map(|d| DiffFile::Of(d, self.detail_file)),
                self.detail
                    .as_ref()
                    .and_then(|d| self.detail_styles.get(&d.sha, self.detail_file)),
                DiffCtx::Commit,
                "commit-diff",
            ),
            View::Stash(_) => (
                self.stash_detail
                    .clone()
                    .filter(|d| self.stash_file < d.files.len())
                    .map(|d| DiffFile::Of(d, self.stash_file)),
                self.stash_detail
                    .as_ref()
                    .and_then(|d| self.stash_styles.get(&d.sha, self.stash_file)),
                DiffCtx::Commit,
                "stash-diff",
            ),
            View::Changes => (
                self.change_diff.clone().map(DiffFile::Own),
                self.change_styles.clone(),
                match &self.change_sel {
                    Some((_, true)) => DiffCtx::Staged,
                    _ => DiffCtx::Unstaged,
                },
                "change-diff",
            ),
            View::Review => {
                let ui = self.review.as_ref();
                let d = ui.and_then(|r| Some((r.diff.clone()?, r.file)));
                let styles = d
                    .as_ref()
                    .and_then(|(d, ix)| self.review_styles.get(&d.sha, *ix));
                let file = d
                    .filter(|(d, ix)| *ix < d.files.len())
                    .map(|(d, ix)| DiffFile::Of(d, ix));
                let viewed = file.as_ref().and_then(|_| self.viewed_check(cx));
                return self.render_diff(
                    file,
                    styles,
                    DiffCtx::Commit,
                    "review-diff",
                    viewed,
                    memo,
                    cx,
                );
            }
            View::Rebase | View::Activity => return div().into_any_element(),
        };
        self.render_diff(file, styles, ctx, id, None, memo, cx)
    }

    /// `extra` goes in the header, before the view switch.
    #[allow(clippy::too_many_arguments)]
    fn render_diff(
        &mut self,
        file: Option<DiffFile>,
        styles: Option<Rc<DiffStyles>>,
        ctx: DiffCtx,
        id: &'static str,
        extra: Option<AnyElement>,
        memo: &mut Memo,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let Some(file) = file else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child("No file selected.")
                .into_any_element();
        };
        let title = match &file.old_path {
            Some(old) if file.change == FileChange::Renamed => format!("{old} → {}", file.path),
            _ => file.path.clone(),
        };
        let mode = if crate::settings::get(cx).split_diff {
            DiffMode::Split
        } else {
            DiffMode::Unified
        };
        let header = h_flex()
            .flex_none()
            .h(px(36.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(t.colors.border)
            .child(
                Icon::new(IconName::FileDiff)
                    .size(px(14.))
                    .text_color(muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            // Git counts no lines in a binary file.
            .when(!file.binary, |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.colors.green)
                        .child(format!("+{}", file.additions)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.colors.red)
                        .child(format!("−{}", file.deletions)),
                )
            })
            .when_some(extra, |d, e| d.child(div().ml_2().child(e)))
            .child(
                segmented(
                    "diff-mode",
                    &[("Unified", DiffMode::Unified), ("Split", DiffMode::Split)],
                    mode,
                    |m, _, cx| crate::settings::update(cx, |s| s.split_diff = m == DiffMode::Split),
                    cx,
                )
                .ml_2(),
            );
        let note = |text: &'static str| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(text)
        };
        let partial = ctx != DiffCtx::Commit && git::supports_partial(&file);
        let body = if file.binary {
            note("Binary file, no text diff.").into_any_element()
        } else if file.lines.is_empty() {
            note("No content changes.").into_any_element()
        } else {
            let layout = keep(memo, || DiffLayout::new(&file, mode));
            let scroll = self.diff_scroll.entry(id).or_default().clone();
            let n = match mode {
                DiffMode::Split => layout.rows.len(),
                DiffMode::Unified => file.lines.len() + usize::from(file.truncated),
            };
            let (size, line_h) = metrics(cx);
            let text = cx.text_system();
            let font_id = text.resolve_font(&font(crate::theme::mono_font(cx)));
            let advance = text
                .advance(font_id, px(size), 'm')
                .map_or(px(size * 0.6), |a| a.width);
            scroll.set_file(
                &file.path,
                advance * layout.cols as f32,
                px(line_h) * n as f32,
                mode == DiffMode::Split,
            );
            let (_, room_below) = scroll.bars();
            scroll.0.room_below.set(room_below);
            let (f, st, s) = (file.clone(), styles.clone(), scroll.clone());
            let mut list = if mode == DiffMode::Split {
                uniform_list(
                    id,
                    n,
                    cx.processor(move |this, range: Range<usize>, window, cx| {
                        s.check_room(window);
                        let x = s.x();
                        range
                            .map(|i| {
                                let row = layout.rows[i];
                                this.split_row(&f, st.as_deref(), row, i, x, ctx, partial, cx)
                            })
                            .collect::<Vec<_>>()
                    }),
                )
            } else {
                uniform_list(
                    id,
                    n,
                    cx.processor(move |this, range: Range<usize>, window, cx| {
                        s.check_room(window);
                        let x = s.x();
                        range
                            .map(|i| this.unified_row(&f, st.as_deref(), i, x, ctx, partial, cx))
                            .collect::<Vec<_>>()
                    }),
                )
            };
            // Without this, a sideways swipe scrolls the rows up and down.
            list.style().restrict_scroll_to_axis = Some(true);
            // GPUI notifies the pane for a scroll of the list. The sideways
            // scroll is not the list's, so it notifies the pane itself.
            let pane = self.panes.get(&Part::Diff).map(|p| p.entity_id());
            let s = scroll.clone();
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .on_scroll_wheel(move |e, window, cx| {
                    if s.wheel(e, window)
                        && let Some(pane) = pane
                    {
                        cx.notify(pane);
                    }
                })
                .child(
                    list.track_scroll(&scroll.0.list)
                        .size_full()
                        // The last row scrolls clear of the horizontal scrollbar.
                        .when(room_below, |l| l.pb(Scrollbar::width())),
                )
                .child(
                    div().absolute().inset_0().child(
                        Scrollbar::new(&scroll)
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
                )
                .into_any_element()
        };
        v_flex()
            .size_full()
            .child(header)
            .when(partial && !self.line_sel.is_empty(), |d| {
                d.child(self.render_selection_bar(ctx, cx))
            })
            .child(body)
            .into_any_element()
    }

    fn render_selection_bar(&self, ctx: DiffCtx, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let n = self.line_sel.len();
        let staged = ctx == DiffCtx::Staged;
        h_flex()
            .flex_none()
            .h(px(36.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(t.colors.border)
            .bg(t.colors.primary.opacity(0.08))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .child(format!("{n} line{} selected", history::plural(n))),
            )
            .child(
                button("sel-clear")
                    .ghost()
                    .xsmall()
                    .label("Clear")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.line_sel.clear();
                        cx.notify();
                    })),
            )
            .when(!staged, |d| {
                d.child(
                    button("sel-discard")
                        .xsmall()
                        .danger()
                        .label("Discard lines")
                        .on_click(cx.listener(|this, _, window, cx| {
                            let chosen = this.line_sel.clone();
                            this.confirm_discard_lines(chosen, window, cx)
                        })),
                )
            })
            .child(
                button("sel-stage")
                    .xsmall()
                    .primary()
                    .label(if staged {
                        "Unstage lines"
                    } else {
                        "Stage lines"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let chosen = this.line_sel.clone();
                        let op = if staged {
                            PatchOp::Unstage
                        } else {
                            PatchOp::Stage
                        };
                        this.apply_lines(chosen, op, cx)
                    })),
            )
    }

    fn hunk_row(
        &self,
        file: &FileDiff,
        i: usize,
        ctx: DiffCtx,
        partial: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let line = &file.lines[i];
        let staged = ctx == DiffCtx::Staged;
        let (_, line_h) = metrics(cx);
        // Buttons fit inside the row: xsmall is 20px, rows can be 18px.
        let button_h = px(line_h - 2.);
        h_flex()
            .id(("hunk", i))
            .h(px(line_h))
            .w_full()
            .pl(px(2. * NUM_W + SIGN_W))
            // The buttons clear the vertical scrollbar.
            .pr(px(8.) + Scrollbar::width())
            .gap_1()
            .bg(t.colors.primary.opacity(0.07))
            .font_family(crate::theme::mono_font(cx))
            .text_size(px(11.5))
            .text_color(t.colors.muted_foreground)
            .child(div().flex_1().min_w_0().truncate().child(line.text.clone()))
            .when(partial, |d| {
                d.when(!staged, |d| {
                    d.child(
                        button(("hunk-discard", i))
                            .font_family(crate::theme::ui_font(cx))
                            .ghost()
                            .xsmall()
                            .h(button_h)
                            .label("Discard")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(f) = this.change_diff.clone() {
                                    this.confirm_discard_lines(hunk_lines(&f, i), window, cx)
                                }
                            })),
                    )
                })
                .child(
                    button(("hunk-stage", i))
                        .font_family(crate::theme::ui_font(cx))
                        .ghost()
                        .xsmall()
                        .h(button_h)
                        .label(if staged { "Unstage hunk" } else { "Stage hunk" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(f) = this.change_diff.clone() {
                                let op = if staged {
                                    PatchOp::Unstage
                                } else {
                                    PatchOp::Stage
                                };
                                this.apply_lines(hunk_lines(&f, i), op, cx)
                            }
                        })),
                )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn unified_row(
        &self,
        file: &FileDiff,
        styles: Option<&DiffStyles>,
        i: usize,
        x: Pixels,
        ctx: DiffCtx,
        partial: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let Some(line) = file.lines.get(i) else {
            return more_row(cx);
        };
        if line.kind == LineKind::Hunk {
            return self.hunk_row(file, i, ctx, partial, cx);
        }
        let selectable = partial && matches!(line.kind, LineKind::Add | LineKind::Del);
        let selected = selectable && self.line_sel.contains(&i);
        let (bg, gutter_bg) = line_colors(line.kind, selected, cx);
        let (size, line_h) = metrics(cx);
        let num = |n: Option<u32>| {
            div()
                .w(px(NUM_W))
                .flex_none()
                .pr_2()
                .text_right()
                .text_color(muted.opacity(0.7))
                .child(n.map(|n| n.to_string()).unwrap_or_default())
        };
        h_flex()
            .h(px(line_h))
            .w_full()
            .bg(bg)
            .font_family(crate::theme::mono_font(cx))
            .text_size(px(size))
            .child(
                h_flex()
                    .id(("gutter", i))
                    .h_full()
                    .flex_none()
                    .bg(gutter_bg)
                    .when(selected, |d| d.border_l_2().border_color(t.colors.primary))
                    .when(selectable, |d| {
                        d.cursor_pointer().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                this.toggle_line(i, e.modifiers.shift, cx)
                            }),
                        )
                    })
                    .child(num(line.old_no))
                    .child(num(line.new_no)),
            )
            .child(sign(line.kind, cx))
            .child(code(line, i, styles, x, cx))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn split_row(
        &self,
        file: &FileDiff,
        styles: Option<&DiffStyles>,
        row: SplitRow,
        ix: usize,
        x: Pixels,
        ctx: DiffCtx,
        partial: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().colors.muted_foreground;
        let border = cx.theme().colors.border;
        let (size, line_h) = metrics(cx);
        let (left, right) = match row {
            SplitRow::Hunk(i) => return self.hunk_row(file, i, ctx, partial, cx),
            SplitRow::More => return more_row(cx),
            SplitRow::Note(i) => {
                return div()
                    .h(px(line_h))
                    .pl(px(2. * NUM_W + SIGN_W))
                    .font_family(crate::theme::mono_font(cx))
                    .text_size(px(11.5))
                    .text_color(muted)
                    .child(file.lines[i].text.clone())
                    .into_any_element();
            }
            SplitRow::Pair(l, r) => (l, r),
        };
        let half = |side: Option<usize>, old: bool, cx: &mut Context<Self>| {
            let t = cx.theme();
            let Some(i) = side else {
                // Nothing on this side: a hatched-looking empty cell.
                return h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(t.colors.muted.opacity(0.35))
                    .into_any_element();
            };
            let line = &file.lines[i];
            let kind = line.kind;
            let selectable = partial && matches!(kind, LineKind::Add | LineKind::Del);
            let selected = selectable && self.line_sel.contains(&i);
            let (bg, gutter_bg) = line_colors(kind, selected, cx);
            let n = if old { line.old_no } else { line.new_no };
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .bg(bg)
                .child(
                    div()
                        .id(("split-gutter", i * 2 + usize::from(old)))
                        .w(px(SPLIT_NUM_W))
                        .h_full()
                        .flex_none()
                        .pr_2()
                        .text_right()
                        .bg(gutter_bg)
                        .text_color(t.colors.muted_foreground.opacity(0.7))
                        .when(selected, |d| d.border_l_2().border_color(t.colors.primary))
                        .when(selectable, |d| {
                            d.cursor_pointer().on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                    this.toggle_line(i, e.modifiers.shift, cx)
                                }),
                            )
                        })
                        .child(n.map(|n| n.to_string()).unwrap_or_default()),
                )
                .child(sign(kind, cx))
                .child(code(line, i, styles, x, cx))
                .into_any_element()
        };
        let l = half(left, true, cx);
        let r = half(right, false, cx);
        h_flex()
            .id(("split", ix))
            .h(px(line_h))
            .w_full()
            .font_family(crate::theme::mono_font(cx))
            .text_size(px(size))
            .child(l)
            .child(div().w(px(1.)).h_full().bg(border))
            .child(r)
            .into_any_element()
    }

    fn toggle_line(&mut self, i: usize, shift: bool, cx: &mut Context<Self>) {
        let Some(file) = self.change_diff.clone() else {
            return;
        };
        match (shift, self.line_anchor) {
            (true, Some(a)) => {
                let (lo, hi) = (a.min(i), a.max(i));
                self.line_sel.extend(
                    (lo..=hi)
                        .filter(|&k| matches!(file.lines[k].kind, LineKind::Add | LineKind::Del)),
                );
            }
            _ => {
                if !self.line_sel.remove(&i) {
                    self.line_sel.insert(i);
                }
            }
        }
        self.line_anchor = Some(i);
        cx.notify();
    }
}

fn more_row(cx: &App) -> AnyElement {
    div()
        .h(px(metrics(cx).1))
        .px_3()
        .text_color(cx.theme().colors.muted_foreground)
        .child("Diff too large, the rest is not shown.")
        .into_any_element()
}

fn line_colors(kind: LineKind, selected: bool, cx: &App) -> (Hsla, Hsla) {
    let t = cx.theme();
    let (green, red) = (t.colors.green, t.colors.red);
    let boost = if selected { 0.12 } else { 0. };
    match kind {
        LineKind::Add => (green.opacity(0.10 + boost), green.opacity(0.18 + boost)),
        LineKind::Del => (red.opacity(0.10 + boost), red.opacity(0.18 + boost)),
        _ => (transparent_black(), transparent_black()),
    }
}

fn sign(kind: LineKind, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let (text, color) = match kind {
        LineKind::Add => ("+", t.colors.green),
        LineKind::Del => ("−", t.colors.red),
        _ => ("", t.colors.muted_foreground),
    };
    div()
        .w(px(SIGN_W))
        .flex_none()
        .text_center()
        .text_color(color)
        .child(text)
}

/// The part of a line that the diff shows.
fn shown(text: &str) -> &str {
    if text.len() <= MAX_COLS {
        return text;
    }
    let mut end = MAX_COLS;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The line's text with syntax colors and changed-word backgrounds,
/// scrolled `x` to the left.
fn code(
    line: &DiffLine,
    i: usize,
    styles: Option<&DiffStyles>,
    x: Pixels,
    cx: &App,
) -> impl IntoElement {
    let t = cx.theme();
    let text = shown(&line.text);
    let len = text.len();
    let clip = |r: &Range<usize>| (r.start < len).then(|| r.start..r.end.min(len));
    let mut spans: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    if let Some(st) = styles {
        let syntax = st
            .syntax
            .get(i)
            .into_iter()
            .flatten()
            .filter_map(|(r, s)| clip(r).map(|r| (r, *s)));
        let word_bg = match line.kind {
            LineKind::Add => t.colors.green.opacity(0.30),
            _ => t.colors.red.opacity(0.30),
        };
        let words = st.words.get(i).into_iter().flatten().filter_map(|r| {
            clip(r).map(|r| {
                (
                    r,
                    HighlightStyle {
                        background_color: Some(word_bg),
                        ..Default::default()
                    },
                )
            })
        });
        spans = combine_highlights(syntax, words).collect();
    }
    let color = match line.kind {
        LineKind::Note => t.colors.muted_foreground,
        _ => t.colors.foreground,
    };
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(color)
        .child(
            div()
                .ml(-x)
                .child(StyledText::new(text.to_string()).with_highlights(spans)),
        )
}

/// A file row for lists: change badge, name, folder, counts.
pub(super) fn file_row(
    f: &FileDiff,
    selected: bool,
    depth: Option<usize>,
    id: impl Into<ElementId>,
    cx: &App,
) -> Stateful<Div> {
    let letter = match f.change {
        FileChange::Added => git::Change::Added,
        FileChange::Deleted => git::Change::Deleted,
        FileChange::Modified => git::Change::Modified,
        FileChange::Renamed => git::Change::Renamed,
    };
    // Git counts no lines in a binary file.
    let counts = (!f.binary).then(|| {
        h_flex()
            .gap_1p5()
            .text_size(px(11.))
            .child(
                div()
                    .text_color(cx.theme().colors.green)
                    .child(format!("+{}", f.additions)),
            )
            .child(
                div()
                    .text_color(cx.theme().colors.red)
                    .child(format!("−{}", f.deletions)),
            )
    });
    path_row(&f.path, letter, selected, None, depth, id, cx).children(counts)
}

/// Name first, folder after it in muted text. At a `depth` of a tree, the
/// row is indented and the folder rows above it show the folder. With
/// `flash`, the time the file changed, the row flashes (`changes::FLASH`).
pub(super) fn path_row(
    path: &str,
    change: git::Change,
    selected: bool,
    flash: Option<std::time::Instant>,
    depth: Option<usize>,
    id: impl Into<ElementId>,
    cx: &App,
) -> Stateful<Div> {
    let t = cx.theme();
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (d.to_string(), n.to_string()),
        None => (String::new(), path.to_string()),
    };
    h_flex()
        .id(id)
        .w_full()
        .h(px(28.))
        .px_2()
        .when_some(depth, |d, n| d.pl(files::file_indent(n)))
        .gap_2()
        .rounded(t.radius)
        .cursor_pointer()
        .when(selected, |d| d.bg(t.colors.list_active))
        .when(!selected, |d| {
            d.child(hover_fill(t.colors.list_hover, t.radius))
        })
        .when_some(flash, |d, at| {
            d.child(changes::flash_fill(at, t.colors.primary, t.radius))
        })
        .child(change_badge(change, cx))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_1p5()
                .overflow_hidden()
                // Without the folder beside it, a long name shrinks instead.
                .child(match depth {
                    Some(_) => div().min_w_0().truncate().child(name),
                    None => div().flex_none().child(name),
                })
                .when(depth.is_none(), |d| {
                    d.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(t.colors.muted_foreground)
                            .child(dir),
                    )
                }),
        )
}

pub(super) fn change_badge(change: git::Change, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let color = match change {
        git::Change::Added | git::Change::Untracked => t.colors.green,
        git::Change::Deleted => t.colors.red,
        git::Change::Modified => t.colors.yellow,
        git::Change::Renamed => t.colors.blue,
        git::Change::Conflicted => t.colors.red,
    };
    div()
        .flex_none()
        .size(px(16.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .bg(color.opacity(0.16))
        .text_color(color)
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .child(change.letter())
}
