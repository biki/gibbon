//! Diff view: unified or split, with syntax colors, changed words, and —
//! for working-tree files — hunk buttons and line selection.

use std::collections::HashSet;
use std::ops::Range;

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
        &self,
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
            .child(
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
        } else if mode == DiffMode::Split {
            let rows = keep(memo, || split_rows(&file));
            let (f, st) = (file.clone(), styles.clone());
            uniform_list(
                id,
                rows.len(),
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| this.split_row(&f, st.as_deref(), rows[i], i, ctx, partial, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .into_any_element()
        } else {
            let n = file.lines.len() + usize::from(file.truncated);
            let (f, st) = (file.clone(), styles.clone());
            uniform_list(
                id,
                n,
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| this.unified_row(&f, st.as_deref(), i, ctx, partial, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
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
                Button::new("sel-clear")
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
                    Button::new("sel-discard")
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
                Button::new("sel-stage")
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
            .pl(px(106.))
            .pr_2()
            .gap_1()
            .bg(t.colors.primary.opacity(0.07))
            .font_family(crate::theme::mono_font(cx))
            .text_size(px(11.5))
            .text_color(t.colors.muted_foreground)
            .child(div().flex_1().min_w_0().truncate().child(line.text.clone()))
            .when(partial, |d| {
                d.when(!staged, |d| {
                    d.child(
                        Button::new(("hunk-discard", i))
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
                    Button::new(("hunk-stage", i))
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

    fn unified_row(
        &self,
        file: &FileDiff,
        styles: Option<&DiffStyles>,
        i: usize,
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
                .w(px(44.))
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
            .child(code(line, i, styles, cx))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn split_row(
        &self,
        file: &FileDiff,
        styles: Option<&DiffStyles>,
        row: SplitRow,
        ix: usize,
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
                    .pl(px(106.))
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
                        .w(px(48.))
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
                .child(code(line, i, styles, cx))
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
        .w(px(18.))
        .flex_none()
        .text_center()
        .text_color(color)
        .child(text)
}

/// The line's text with syntax colors and changed-word backgrounds.
fn code(line: &DiffLine, i: usize, styles: Option<&DiffStyles>, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let mut text = line.text.as_str();
    if text.len() > MAX_COLS {
        let mut end = MAX_COLS;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text = &text[..end];
    }
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
        .child(StyledText::new(text.to_string()).with_highlights(spans))
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
    let counts = h_flex()
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
        );
    path_row(&f.path, letter, selected, depth, id, cx).child(counts)
}

/// Name first, folder after it in muted text. At a `depth` of a tree, the
/// row is indented and the folder rows above it show the folder.
pub(super) fn path_row(
    path: &str,
    change: git::Change,
    selected: bool,
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
