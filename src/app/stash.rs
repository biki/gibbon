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
        let theme = cx.theme().highlight_theme.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let d = git::stash_detail(&repo, &stash)?;
                    let styles: Vec<DiffStyles> =
                        d.files.iter().map(|f| highlight::compute(f, &theme)).collect();
                    anyhow::Ok((d, styles))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.view != View::Stash(index) {
                    return;
                }
                match result {
                    Ok((d, styles)) => {
                        let first = this.first_file("stash", &files::paths(&d.files), cx);
                        this.stash_file = first.unwrap_or(0);
                        this.stash_detail = Some(Rc::new(d));
                        this.stash_styles =
                            Some(Rc::new(styles.into_iter().map(Rc::new).collect()));
                    }
                    Err(e) => this.toast(Some(false), e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn stash_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.is_empty() {
            self.toast(None, "There are no changes to stash.");
            cx.notify();
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

    fn stash_op(&mut self, index: usize, pop: bool, cx: &mut Context<Self>) {
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

    fn drop_stash_dialog(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            "Drop stash?",
            "This deletes the stash and its changes. You cannot undo it.".into(),
            "Drop",
            move |this, cx| {
                this.view = View::Changes;
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
            .child(Icon::new(IconName::Archive).size(px(18.)).text_color(t.colors.primary))
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
                    .child(
                        div().text_size(px(12.)).text_color(muted).child(format!(
                            "{} · on {} · {}",
                            stash.refname(),
                            stash.branch().unwrap_or("?"),
                            fmt_time(stash.time)
                        )),
                    ),
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
        let Some(d) = self.stash_detail.clone() else {
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
        };
        let rows = Rc::new(self.file_rows("stash", &files::paths(&d.files), cx));
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
                            diff::file_row(&files.files[ix], selected, depth, ("stash-file", ix), cx)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.stash_file = ix;
                                    cx.notify();
                                }))
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .px_1p5()
        .py_1();
        let file = d.files.get(self.stash_file).cloned().map(Rc::new);
        let styles = self
            .stash_styles
            .as_ref()
            .and_then(|all| all.get(self.stash_file).cloned());
        let diff = self.render_diff(file, styles, DiffCtx::Commit, "stash-diff", cx);
        let border = cx.theme().colors.border;
        v_flex()
            .size_full()
            .child(header)
            .child(
                h_resizable("stash-split")
                    .child(
                        resizable_panel()
                            .size(px(340.))
                            .size_range(px(220.)..px(700.))
                            .child(
                                v_flex()
                                    .size_full()
                                    .border_r_1()
                                    .border_color(border)
                                    .child(
                                        // As tall as the diff header beside it.
                                        files::files_bar(&d.files, cx)
                                            .h(px(36.))
                                            .border_b_1()
                                            .border_color(border),
                                    )
                                    .child(list),
                            ),
                    )
                    .child(resizable_panel().child(diff)),
            )
            .into_any_element()
    }
}
