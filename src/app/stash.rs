//! Stashes: save all changes, then look at, apply, pop or drop them.

use std::ops::Range;

use super::*;

impl GitApp {
    pub(super) fn show_stash(&mut self, index: usize, cx: &mut Context<Self>) {
        self.view = View::Stash(index);
        self.stash_file = 0;
        self.load_stash_detail(cx);
        cx.notify();
    }

    pub(super) fn load_stash_detail(&mut self, cx: &mut Context<Self>) {
        let View::Stash(index) = self.view else {
            return;
        };
        let (Some(repo), Some(stash)) = (
            self.repo.clone(),
            self.stashes.iter().find(|s| s.index == index).cloned(),
        ) else {
            self.stash_detail = None;
            return;
        };
        self.stash_styles.task = None;
        let shown = self.shown_file_rule("stash", cx);
        let theme = cx.theme().highlight_theme.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let d = git::stash_detail(&repo, &stash)?;
                    let ix = shown(&d.files).unwrap_or(0);
                    let styles = colors_with_text(&d, ix, &theme);
                    anyhow::Ok((d, ix, styles))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.view != View::Stash(index) {
                    return;
                }
                match result {
                    Ok((d, ix, styles)) => {
                        this.stash_file = ix;
                        this.stash_styles = FileStyles::new(d.sha.clone(), styles.map(|s| (ix, s)));
                        this.stash_detail = Some(Rc::new(d));
                        this.highlight_shown(cx);
                    }
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn stash_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.is_empty() {
            self.toast(None, "There are no changes to stash.", cx);
            return;
        }
        let n = self.status.len();
        self.prompt(
            "Stash changes",
            format!(
                "Put {n} changed file{} away, new files included. Message (optional):",
                history::plural(n)
            ),
            "",
            "Stash",
            |this, msg, cx| {
                this.run_op(
                    "Stashing…",
                    Some("Stashed your changes".into()),
                    move |repo| git::stash_push(repo, &msg),
                    cx,
                )
            },
            window,
            cx,
        );
    }

    pub(super) fn stash_op(&mut self, index: usize, pop: bool, cx: &mut Context<Self>) {
        let label = if pop { "Popping…" } else { "Applying…" };
        let done = if pop {
            "Applied and removed the stash"
        } else {
            "Applied the stash"
        };
        if pop {
            self.view = View::Changes;
        }
        self.run_op(
            label,
            Some(done.into()),
            move |repo| git::stash_apply(repo, index, pop),
            cx,
        );
    }

    pub(super) fn drop_stash_dialog(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(
            "Drop stash?",
            "This deletes the stash and its changes. You cannot undo it.".into(),
            "Drop",
            move |this, cx| {
                // The later stashes move up one index: the view would
                // show the next stash under the old number.
                if matches!(this.view, View::Stash(i) if i >= index) {
                    this.view = View::Changes;
                }
                this.run_op(
                    "Dropping…",
                    Some("Dropped the stash".into()),
                    move |repo| git::stash_drop(repo, index),
                    cx,
                )
            },
            window,
            cx,
        );
    }

    pub(super) fn render_stash(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let Some(stash) = self.stashes.iter().find(|s| s.index == index).cloned() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child("This stash no longer exists.")
                .into_any_element();
        };
        let busy = self.busy.is_some();
        let header = h_flex()
            .flex_none()
            .px_3()
            .py_2()
            .gap_3()
            .border_b_1()
            .border_color(t.colors.border)
            .child(
                Icon::new(IconName::Archive)
                    .size(px(18.))
                    .text_color(t.colors.primary),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(stash.title().to_string()),
                    )
                    .child(div().text_size(px(12.)).text_color(muted).child(format!(
                        "{} · on {} · {}",
                        stash.refname(),
                        stash.branch().unwrap_or("?"),
                        fmt_time(stash.time)
                    ))),
            )
            .child(
                Button::new("stash-drop")
                    .ghost()
                    .small()
                    .label("Drop…")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.drop_stash_dialog(index, window, cx)
                    })),
            )
            .child(
                Button::new("stash-apply")
                    .small()
                    .label("Apply")
                    .tooltip("Apply the changes and keep the stash")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.stash_op(index, false, cx))),
            )
            .child(
                Button::new("stash-pop")
                    .primary()
                    .small()
                    .label("Pop")
                    .tooltip("Apply the changes and delete the stash")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.stash_op(index, true, cx))),
            );
        if self.stash_detail.is_none() {
            return v_flex()
                .size_full()
                .child(header)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .child("Loading stash…"),
                )
                .into_any_element();
        }
        let border = cx.theme().colors.border;
        v_flex()
            .size_full()
            .child(header)
            .child(
                split("stash-split", false)
                    .child(
                        split_panel("stash-split", 340., 220.0..700., cx).child(
                            div()
                                .size_full()
                                .border_r_1()
                                .border_color(border)
                                .child(self.pane(Part::StashFiles, cx)),
                        ),
                    )
                    .child(resizable_panel().child(self.pane(Part::Diff, cx))),
            )
            .into_any_element()
    }

    /// The files of the shown stash.
    pub(super) fn render_stash_files(
        &mut self,
        memo: &mut Memo,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(d) = self.stash_detail.clone() else {
            return div().into_any_element();
        };
        let rows = keep(memo, || {
            self.file_rows("stash", &files::paths(&d.files), cx)
        });
        let files = d.clone();
        let list = uniform_list(
            "stash-files",
            rows.len(),
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                range
                    .map(|i| match rows[i] {
                        FileRow::Dir(ref dir) => this.dir_row("stash", dir, ("stash-dir", i), cx),
                        FileRow::File { ix, depth } => {
                            let selected = this.stash_file == ix;
                            diff::file_row(
                                &files.files[ix],
                                selected,
                                depth,
                                ("stash-file", ix),
                                cx,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.stash_file = ix;
                                    this.highlight_shown(cx);
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
        .px_1p5()
        .py_1();
        let border = cx.theme().colors.border;
        v_flex()
            .size_full()
            .child(
                // As tall as the diff header beside it.
                files::files_bar(&d.files, cx)
                    .h(px(36.))
                    .border_b_1()
                    .border_color(border),
            )
            .child(list)
            .into_any_element()
    }
}
