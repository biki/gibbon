//! History: the commit list with its graph, and the selected commit below.
//!
//! Browsing another branch turns the list into a pick list: commits the
//! checked-out branch already has are dimmed, earlier picks are marked, and
//! the rest can be picked into the current branch without leaving it.

use std::cell::RefCell;
use std::ops::Range;

use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{v_resizable, Colorize as _};

use super::*;

const ROW_H: f32 = 30.;

/// What the commit list keeps between renders (see `pane::Memo`).
struct LogMemo {
    /// "3m ago" by commit index, made when the row is first shown.
    times: RefCell<HashMap<usize, SharedString>>,
    /// Commits of the browsed branch that HEAD does not have, and that were
    /// picked before.
    available: usize,
    picked: usize,
}

impl LogMemo {
    fn time(&self, ix: usize, ts: i64) -> SharedString {
        self.times
            .borrow_mut()
            .entry(ix)
            .or_insert_with(|| fmt_time(ts).into())
            .clone()
    }
}

/// What the commit detail keeps between renders.
struct CommitMemo {
    subject: SharedString,
    /// The body, joined to the width of the pane.
    body: SharedString,
    rows: Rc<Vec<FileRow>>,
}

impl GitApp {
    /// The commit list above, the shown commit below: its files beside its
    /// diff. Each is a pane.
    pub(super) fn render_history(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (border, muted) = (cx.theme().colors.border, cx.theme().colors.muted_foreground);
        let detail = if self.detail.is_some() {
            h_resizable("detail-split")
                .child(
                    resizable_panel()
                        .size(px(380.))
                        .size_range(px(260.)..px(800.))
                        .child(
                            div()
                                .size_full()
                                .border_t_1()
                                .border_r_1()
                                .border_color(border)
                                .child(self.pane(Part::Commit, cx)),
                        ),
                )
                .child(
                    resizable_panel().child(
                        div()
                            .size_full()
                            .border_t_1()
                            .border_color(border)
                            .child(self.pane(Part::Diff, cx)),
                    ),
                )
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .border_t_1()
                .border_color(border)
                .text_color(muted)
                .child(if self.cursor.is_some() {
                    "Loading commit…"
                } else {
                    "Select a commit."
                })
                .into_any_element()
        };
        v_resizable("history-split")
            .child(
                resizable_panel()
                    .size(px(430.))
                    .size_range(px(160.)..px(4000.))
                    .child(self.pane(Part::Log, cx)),
            )
            .child(resizable_panel().child(detail))
    }

    pub(super) fn render_log(&mut self, memo: &mut Memo, cx: &mut Context<Self>) -> impl IntoElement {
        let memo = keep(memo, || self.log_memo());
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let n = self.commits.len();
        let graph_w = graph::LANE_W * self.graph.width.min(graph::MAX_LANES) as f32 + 10.;
        let header = h_flex()
            .h(px(28.))
            .flex_none()
            .pr_3()
            .gap_3()
            .border_b_1()
            .border_color(t.colors.border)
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(muted)
            .child(div().w(px(graph_w.max(56.))).pl_2().child("Graph"))
            .child(div().flex_1().child("Description"))
            .child(div().w(px(150.)).child("Author"))
            .child(div().w(px(90.)).child("Date"))
            .child(div().w(px(64.)).child("Commit"));
        v_flex()
            .size_full()
            .when_some(self.render_pick_banner(&memo, cx), |d, b| d.child(b))
            .child(header)
            .child(
                div()
                    .id("commit-list")
                    .key_context("CommitList")
                    .track_focus(&self.list_focus)
                    .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.step(-1, cx)))
                    .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
                    .flex_1()
                    .min_h_0()
                    .when(n == 0, |d| {
                        d.flex().items_center().justify_center().text_color(muted).child(
                            if self.log_loading {
                                "Loading history…"
                            } else {
                                "No commits yet."
                            },
                        )
                    })
                    .when(n > 0, |d| {
                        d.child(
                            uniform_list(
                                "commits",
                                n,
                                cx.processor(move |this, range: Range<usize>, _window, cx| {
                                    range
                                        .map(|i| this.render_commit_row(i, graph_w, &memo, cx))
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(&self.log_scroll)
                            .size_full(),
                        )
                    }),
            )
    }

    fn log_memo(&self) -> LogMemo {
        let count = |state| self.picks.values().filter(|s| **s == state).count();
        LogMemo {
            times: RefCell::default(),
            available: count(PickState::Pickable),
            picked: count(PickState::AlreadyPicked),
        }
    }

    /// Shown while browsing a branch that is not checked out.
    fn render_pick_banner(&self, memo: &LogMemo, cx: &mut Context<Self>) -> Option<AnyElement> {
        let target = self.foreign_target()?;
        let t = cx.theme();
        let pr = crate::github::number_of(target)
            .and_then(|n| self.prs.iter().find(|p| p.number == n));
        let short = pr
            .map(|p| format!("#{} {}", p.number, p.title))
            .unwrap_or_else(|| target.to_string());
        let short = short
            .strip_prefix("refs/heads/")
            .or_else(|| short.strip_prefix("refs/remotes/"))
            .or_else(|| short.strip_prefix("refs/tags/"))
            .unwrap_or(&short)
            .to_string();
        let head = self.head_name();
        let (available, picked) = (memo.available, memo.picked);
        let chosen = self.pickable_selection().len();
        let accent = t.colors.primary;
        let text = if !self.refs_loaded || self.log_loading {
            format!("Comparing with {head}…")
        } else if available == 0 && picked == 0 {
            format!("{head} already contains every commit of {short}.")
        } else {
            let mut s = format!("{available} commit{} not in {head}", plural(available));
            if picked > 0 {
                s.push_str(&format!(" · {picked} picked before"));
            }
            s
        };
        Some(
            h_flex()
                .flex_none()
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(t.colors.border)
                .bg(accent.opacity(0.08))
                .child(Icon::new(IconName::Cherry).size(px(16.)).text_color(accent))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            h_flex()
                                .gap_1()
                                .child("Browsing")
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(short))
                                .child(div().text_color(t.colors.muted_foreground).child("·"))
                                .child("you stay on")
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(head.clone())),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.colors.muted_foreground)
                                .child(text),
                        ),
                )
                .child(
                    Button::new("back-to-head")
                        .ghost()
                        .small()
                        .label(format!("Back to {head}"))
                        .on_click(cx.listener(|this, _, _, cx| this.show_target(LogTarget::Head, cx))),
                )
                .child(
                    Button::new("pick")
                        .primary()
                        .small()
                        .disabled(chosen == 0 || self.busy.is_some())
                        .child(
                            h_flex()
                                .gap_1p5()
                                .child(Icon::new(IconName::Cherry).size(px(14.)))
                                .child(if chosen == 0 {
                                    format!("Pick into {head}")
                                } else {
                                    format!("Pick {chosen} into {head}")
                                }),
                        )
                        .tooltip("Select commits with click, ⌘-click or ⇧-click")
                        .on_click(cx.listener(|this, _, _, cx| this.pick_selected(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_commit_row(
        &self,
        ix: usize,
        graph_w: f32,
        memo: &LogMemo,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let c = &self.commits[ix];
        let selected = self.selected.contains(&ix);
        let foreign = self.foreign_target().is_some();
        let pick = self.picks.get(&c.sha).copied();
        // Browsing another branch: commits HEAD already has step back.
        let in_head = foreign && pick.is_none();
        let row_bg = if selected {
            t.colors.list_active
        } else {
            t.colors.background
        };
        let g = self.graph.clone();
        let ring = row_bg;
        let graph_cell = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if let Some(row) = g.rows.get(ix) {
                    window.with_content_mask(
                        Some(ContentMask { bounds }),
                        |window| graph::paint_row(row, bounds, ring, window),
                    );
                }
            },
        )
        .w(px(graph_w.max(56.)))
        .h_full()
        .flex_none();

        let head_name = self.head_name();
        let sha = c.sha.clone();
        let subject = c.subject.clone();
        let pickable = pick == Some(PickState::Pickable);
        let on_head = self.target == LogTarget::Head;
        let this = cx.entity();
        h_flex()
            .id(("commit", ix))
            .h(px(ROW_H))
            .w_full()
            .pr_3()
            .gap_3()
            .bg(row_bg)
            .when(!selected, |d| d.child(hover_fill(t.colors.list_hover, px(0.))))
            .child(graph_cell)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .overflow_hidden()
                    .when(in_head, |d| d.opacity(0.45))
                    .children(c.refs.iter().take(4).map(|r| ref_badge(r, cx)))
                    .when(pick == Some(PickState::AlreadyPicked), |d| {
                        d.child(
                            h_flex()
                                .flex_none()
                                .h(px(18.))
                                .px_1p5()
                                .gap_1()
                                .rounded(px(4.))
                                .bg(t.colors.green.opacity(0.15))
                                .text_color(t.colors.green)
                                .text_size(px(11.))
                                .child(Icon::new(IconName::Check).size(px(11.)))
                                .child("picked"),
                        )
                    })
                    .child(div().flex_1().min_w_0().truncate().child(c.subject.clone())),
            )
            .child(
                h_flex()
                    .w(px(150.))
                    .flex_none()
                    .gap_1p5()
                    .when(in_head, |d| d.opacity(0.45))
                    .child(avatar(&c.author, 16.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(muted)
                            .child(c.author.clone()),
                    ),
            )
            .child(
                div()
                    .w(px(90.))
                    .flex_none()
                    .truncate()
                    .text_color(muted)
                    .when(in_head, |d| d.opacity(0.45))
                    .child(memo.time(ix, c.time)),
            )
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .font_family(crate::theme::mono_font(cx))
                    .text_size(px(11.5))
                    .text_color(muted)
                    .when(in_head, |d| d.opacity(0.45))
                    .child(c.short().to_string()),
            )
            // Lists select on mouse down, as macOS lists do.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.click_commit(ix, e.modifiers, window, cx)
                }),
            )
            .context_menu(move |menu, _, _| {
                let mut menu = menu;
                if foreign {
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("Pick into {head_name}"))
                            .disabled(!pickable)
                            .on_click(move |_, _, cx| {
                                this.update(cx, |app, cx| {
                                    if !app.selected.contains(&ix) {
                                        app.selected = HashSet::from([ix]);
                                        app.cursor = Some(ix);
                                    }
                                    app.pick_selected(cx)
                                });
                            }),
                    );
                    menu = menu.separator();
                }
                let (sha, subject) = (sha.clone(), subject.clone());
                let (this2, start) = (this.clone(), sha.clone());
                let menu = if on_head {
                    let (this3, from) = (this.clone(), sha.clone());
                    menu.item(PopupMenuItem::new("Interactive Rebase from Here…").on_click(
                        move |_, _, cx| {
                            let from = from.clone();
                            this3.update(cx, |app, cx| app.start_rebase(from, cx));
                        },
                    ))
                } else {
                    menu
                };
                let menu = menu
                    .item(PopupMenuItem::new("New Branch Here…").on_click(
                        move |_, window, cx| {
                            let s = (start.clone(), start[..7].to_string());
                            this2.update(cx, |app, cx| app.new_branch_dialog(Some(s), window, cx));
                        },
                    ))
                    .separator();
                menu.item(PopupMenuItem::new("Copy SHA").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()))
                }))
                .item(PopupMenuItem::new("Copy Subject").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(subject.clone()))
                }))
            })
            .into_any_element()
    }

    fn click_commit(
        &mut self,
        ix: usize,
        mods: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.list_focus.focus(window, cx);
        if mods.platform {
            if !self.selected.remove(&ix) {
                self.selected.insert(ix);
            }
            self.anchor = Some(ix);
        } else if mods.shift
            && let Some(a) = self.anchor
        {
            let (lo, hi) = (a.min(ix), a.max(ix));
            self.selected = (lo..=hi).collect();
        } else {
            self.selected = HashSet::from([ix]);
            self.anchor = Some(ix);
        }
        self.cursor = Some(ix);
        self.load_detail(cx);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.commits.len();
        if n == 0 {
            return;
        }
        let ix = match self.cursor {
            Some(c) => (c as isize + delta).clamp(0, n as isize - 1) as usize,
            None => 0,
        };
        self.cursor = Some(ix);
        self.anchor = Some(ix);
        self.selected = HashSet::from([ix]);
        self.log_scroll.scroll_to_item(ix, ScrollStrategy::Center);
        self.load_detail(cx);
        cx.notify();
    }

    /// The shown commit: message, author, parents and files.
    pub(super) fn render_commit(&mut self, memo: &mut Memo, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.detail.clone() else {
            return div().into_any_element();
        };
        let memo = keep(memo, || {
            let (subject, body) = match d.message.split_once('\n') {
                Some((s, b)) => (s.to_string(), reflow(b).trim().to_string()),
                None => (d.message.clone(), String::new()),
            };
            CommitMemo {
                subject: subject.into(),
                body: body.into(),
                rows: Rc::new(self.file_rows("commit", &files::paths(&d.files), cx)),
            }
        });
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let multi = self.selected.len() > 1;
        let info = v_flex()
            .flex_none()
            .p_3()
            .gap_2()
            .border_b_1()
            .border_color(t.colors.border)
            .when(multi, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.colors.primary)
                        .child(format!("{} commits selected", self.selected.len())),
                )
            })
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(memo.subject.clone()),
            )
            .when(!memo.body.is_empty(), |el| {
                el.child(
                    div()
                        .id("commit-body")
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .text_color(muted)
                        .child(memo.body.clone()),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(avatar(&d.author, 22.))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(d.author.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .truncate()
                                    .child(format!("{} · {}", d.email, fmt_full_time(d.time))),
                            ),
                    ),
            )
            // Picks and rebases keep the author but change the committer.
            .when(d.committer != d.author || d.commit_time != d.time, |el| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(format!(
                            "Committed by {} · {}",
                            d.committer,
                            fmt_full_time(d.commit_time)
                        )),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .text_size(px(11.5))
                    .text_color(muted)
                    .child(sha_chip(&d.sha, cx))
                    .when(!d.parents.is_empty(), |el| {
                        el.child(if d.parents.len() > 1 { "parents" } else { "parent" })
                            .children(
                                d.parents
                                    .iter()
                                    .map(|p| div().font_family(crate::theme::mono_font(cx)).child(p[..7].to_string())),
                            )
                    }),
            );
        let files_header = files::files_bar(&d.files, cx);
        let rows = memo.rows.clone();
        let files = d.clone();
        let file_list = uniform_list(
            "detail-files",
            rows.len(),
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                range
                    .map(|i| match rows[i] {
                        FileRow::Dir(ref dir) => this.dir_row("commit", dir, ("detail-dir", i), cx),
                        FileRow::File { ix, depth } => {
                            let selected = this.detail_file == ix;
                            diff::file_row(&files.files[ix], selected, depth, ("detail-file", ix), cx)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.detail_file = ix;
                                        cx.notify();
                                    }),
                                )
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .px_1p5();
        v_flex()
            .size_full()
            .child(info)
            .child(files_header)
            .child(file_list)
            .into_any_element()
    }
}

/// Join the lines that a commit body is wrapped at (often 72 columns), so
/// the text wraps to the width of the pane. Blank lines, list items,
/// indented lines, code fences and trailers (`Refs: #12`) keep their breaks.
fn reflow(body: &str) -> String {
    let mut out = vec![];
    let mut para = vec![];
    let mut fence = false;
    for line in body.lines().map(str::trim_end) {
        let fence_line = line.trim_start().starts_with("```");
        if fence || fence_line || line.is_empty() {
            join_lines(&mut para, &mut out);
            out.push(line.to_string());
            fence ^= fence_line;
        } else {
            para.push(line);
        }
    }
    join_lines(&mut para, &mut out);
    out.join("\n")
}

/// Move the lines of one paragraph to `out`, joined where they are prose.
fn join_lines(para: &mut Vec<&str>, out: &mut Vec<String>) {
    if para.iter().all(|l| is_trailer(l)) {
        out.extend(para.drain(..).map(String::from));
        return;
    }
    // The last line in `out` takes the next line of prose.
    let mut open = false;
    let mut in_item = false;
    for line in para.drain(..) {
        let text = line.trim_start();
        let item = is_list_item(text);
        let indented = text.len() < line.len();
        match out.last_mut() {
            Some(last) if open && !item && (!indented || in_item) => {
                last.push(' ');
                last.push_str(text);
            }
            _ => {
                out.push(line.to_string());
                in_item = item;
                open = item || !indented;
            }
        }
    }
}

fn is_list_item(text: &str) -> bool {
    let rest = text.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() < text.len() {
        return rest.starts_with(". ") || rest.starts_with(") ");
    }
    ["- ", "* ", "+ ", "• "].iter().any(|m| text.starts_with(m))
}

/// `Signed-off-by: …`, `Refs: #12`, `BREAKING CHANGE: …`.
fn is_trailer(line: &str) -> bool {
    line.split_once(": ").is_some_and(|(key, _)| {
        key == "BREAKING CHANGE"
            || !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

pub(super) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn ref_badge(r: &git::RefLabel, cx: &App) -> AnyElement {
    let t = cx.theme();
    let (fg, bg, icon) = match r.kind {
        RefKind::Local if r.head => (t.colors.primary_foreground, t.colors.primary, IconName::GitBranch),
        RefKind::Local => (t.colors.primary, t.colors.primary.opacity(0.14), IconName::GitBranch),
        RefKind::Remote => (t.colors.muted_foreground, t.colors.muted, IconName::Cloud),
        RefKind::Tag => (t.colors.yellow, t.colors.yellow.opacity(0.14), IconName::Tag),
    };
    h_flex()
        .flex_none()
        .h(px(18.))
        .px_1p5()
        .gap_1()
        .rounded(px(4.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .child(Icon::new(icon).size(px(11.)))
        .child(div().max_w(px(180.)).truncate().child(r.name.clone()))
        .into_any_element()
}

/// A round badge with the author's initials, colored by name.
pub(super) fn avatar(name: &str, size: f32) -> impl IntoElement {
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let hash = name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let color = graph::lane_color(hash as usize);
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(color.opacity(0.22))
        .text_color(color.lighten(0.1))
        .text_size(px(size * 0.42))
        .font_weight(FontWeight::BOLD)
        .child(initials)
}

fn sha_chip(sha: &str, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let full = sha.to_string();
    h_flex()
        .id("sha-chip")
        .gap_1()
        .px_1p5()
        .h(px(20.))
        .rounded(px(4.))
        .bg(t.colors.muted)
        .cursor_pointer()
        .font_family(crate::theme::mono_font(cx))
        .child(sha[..sha.len().min(10)].to_string())
        .child(Icon::new(IconName::Copy).size(px(11.)))
        .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Copy SHA").build(window, cx))
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(full.clone())))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::reflow;

    #[test]
    fn reflow_joins_wrapped_prose() {
        let body = "The workspace embedded the tab. So each render of\nthe window also rendered the tab.\n\nThe tab is now cached.";
        assert_eq!(
            reflow(body),
            "The workspace embedded the tab. So each render of the window also rendered the tab.\n\nThe tab is now cached."
        );
    }

    #[test]
    fn reflow_keeps_lists_code_and_trailers() {
        let body = "Changes:\n- one item that is\n  wrapped\n- two\n1. three\n\n    let x = 1;\n    let y = 2;\n\n```\na\nb\n```\n\nRefs: #12\nSigned-off-by: A <a@b.c>";
        assert_eq!(
            reflow(body),
            "Changes:\n- one item that is wrapped\n- two\n1. three\n\n    let x = 1;\n    let y = 2;\n\n```\na\nb\n```\n\nRefs: #12\nSigned-off-by: A <a@b.c>"
        );
    }

    #[test]
    fn reflow_keeps_a_colon_in_prose() {
        assert_eq!(reflow("Note: the old\nkey stays."), "Note: the old key stays.");
    }
}
