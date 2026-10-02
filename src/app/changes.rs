//! Changes: staged and unstaged files, the commit box, and the file's diff.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::ContextMenuExt as _;

use super::*;

enum Row {
    Header {
        staged: bool,
        count: usize,
    },
    Dir {
        staged: bool,
        dir: files::Dir,
    },
    File {
        entry: usize,
        staged: bool,
        depth: Option<usize>,
    },
}

/// How long the rows of a file that changed on disk stay highlighted.
pub(super) const FLASH: Duration = Duration::from_millis(2500);

/// A highlight in `color` under the content of a row whose file changed at
/// `at`: an outline and a faint fill, so it does not look like the
/// selection, which is a fill in the accent color. It fades out over
/// `FLASH`, slowly at first. Add it before the content of a row.
pub(super) fn flash_fill(at: Instant, color: Hsla, radius: Pixels) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let t = at.elapsed().as_secs_f32() / FLASH.as_secs_f32();
            if t < 1. {
                // While it fades, draw the pane again on the next frame.
                window.request_animation_frame();
                let fade = 1. - t * t * t;
                window.paint_quad(
                    fill(bounds, color.opacity(0.10 * fade))
                        .corner_radii(radius)
                        .border_widths(px(1.))
                        .border_color(color.opacity(0.85 * fade)),
                );
            }
        },
    )
    .absolute()
    .inset_0()
}

/// The file tree of each side keeps its own closed folders.
fn scope(staged: bool) -> &'static str {
    if staged { "staged" } else { "unstaged" }
}

impl GitApp {
    /// Staged files, then unstaged files, each in the view of the settings.
    fn change_rows(&self, cx: &App) -> Vec<Row> {
        let s = crate::settings::get(cx);
        let mut rows = Vec::new();
        for staged in [true, false] {
            let entries: Vec<usize> = (0..self.status.len())
                .filter(|&i| {
                    let e = &self.status[i];
                    if staged {
                        e.staged.is_some()
                    } else {
                        e.unstaged.is_some()
                    }
                })
                .collect();
            if entries.is_empty() {
                continue;
            }
            rows.push(Row::Header {
                staged,
                count: entries.len(),
            });
            let paths: Vec<&str> = entries
                .iter()
                .map(|&i| self.status[i].path.as_str())
                .collect();
            let times: Vec<_> = entries
                .iter()
                .map(|&i| {
                    let e = &self.status[i];
                    e.modified.or_else(|| self.deleted_at.get(&e.path).copied())
                })
                .collect();
            let sort = if s.changes_recent {
                files::Sort::Recent(&times)
            } else {
                files::Sort::Name {
                    desc: s.file_sort_desc,
                }
            };
            rows.extend(
                self.sorted_file_rows(scope(staged), &paths, sort, cx)
                    .into_iter()
                    .map(|r| match r {
                        FileRow::Dir(dir) => Row::Dir { staged, dir },
                        FileRow::File { ix, depth } => Row::File {
                            entry: entries[ix],
                            staged,
                            depth,
                        },
                    }),
            );
        }
        rows
    }

    /// The first file that the Changes list shows: (path, staged side).
    pub(super) fn first_change(&self, cx: &App) -> Option<(String, bool)> {
        self.change_rows(cx).into_iter().find_map(|r| match r {
            Row::File { entry, staged, .. } => Some((self.status[entry].path.clone(), staged)),
            _ => None,
        })
    }

    /// The changed files and the commit box beside the diff. Each is a pane.
    pub(super) fn render_changes(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        split("changes-split", false)
            .child(
                split_panel("changes-split", 380., 260.0..700., cx)
                    .child(self.pane(Part::Changes, cx)),
            )
            .child(resizable_panel().child(self.pane(Part::Diff, cx)))
    }

    pub(super) fn render_change_list(
        &mut self,
        memo: &mut Memo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows = keep(memo, || self.change_rows(cx));
        let has_dirs = rows.iter().any(|r| matches!(r, Row::Dir { .. }));
        let (muted, border) = (cx.theme().colors.muted_foreground, cx.theme().colors.border);
        let n = rows.len();
        let staged = self.status.iter().filter(|e| e.staged.is_some()).count();
        let list = if n == 0 {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(muted)
                .child(Icon::new(IconName::CircleCheck).size(px(28.)))
                .child("No local changes.")
                .into_any_element()
        } else {
            uniform_list(
                "changes",
                n,
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| this.render_change_row(&rows[i], i, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .px_1p5()
            .py_1()
            .into_any_element()
        };
        let commit_box = v_flex()
            .flex_none()
            .p_3()
            .gap_2()
            .border_t_1()
            .border_color(border)
            .child(Textarea::new(&self.message).h(px(96.)))
            .child(
                button("commit")
                    .primary()
                    .w_full()
                    .off(staged == 0 || self.busy.is_some())
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(Icon::new(IconName::GitCommitHorizontal).size(px(15.)))
                            .child(format!("Commit to {}", self.head_name()))
                            .child(div().ml_1().text_size(px(11.)).opacity(0.7).child("⌘↵")),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
            );
        v_flex()
            .size_full()
            .border_r_1()
            .border_color(border)
            .when(n > 0, |d| d.child(self.render_changes_bar(has_dirs, cx)))
            .when_some(self.paused, |d, p| {
                d.child(self.render_paused_banner(p, cx))
            })
            .child(list)
            .child(commit_box)
    }

    /// As tall as the diff header beside it.
    fn render_changes_bar(&self, has_dirs: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let folders = self.folder_buttons(&["staged", "unstaged"], has_dirs, cx);
        let t = cx.theme();
        let n = self.status.len();
        h_flex()
            .flex_none()
            .h(px(36.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(t.colors.border)
            .text_size(px(12.))
            .text_color(t.colors.muted_foreground)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(format!("{n} changed file{}", history::plural(n))),
            )
            .child(files::change_view_buttons(folders, cx))
    }

    fn render_paused_banner(&self, p: git::Paused, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let conflicts = self
            .status
            .iter()
            .filter(|e| e.unstaged == Some(git::Change::Conflicted))
            .count();
        v_flex()
            .flex_none()
            .p_3()
            .gap_2()
            .border_b_1()
            .border_color(t.colors.border)
            .bg(t.colors.yellow.opacity(0.10))
            .child(
                h_flex()
                    .gap_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        Icon::new(IconName::GitMergeConflict)
                            .size(px(15.))
                            .text_color(t.colors.yellow),
                    )
                    .child(format!("{} paused", p.name())),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.colors.muted_foreground)
                    .child(if conflicts > 0 {
                        format!(
                            "{conflicts} file{} with conflicts. Fix them in your editor, \
                         stage them, then continue.",
                            history::plural(conflicts)
                        )
                    } else {
                        format!(
                            "All conflicts are staged. Continue to finish the {}.",
                            p.name().to_lowercase()
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        button("pick-continue")
                            .primary()
                            .small()
                            .label("Continue")
                            .off(conflicts > 0 || self.busy.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_op(
                                    "Continuing…",
                                    Some(format!("{} finished", p.name())),
                                    move |repo| git::continue_paused(repo, p),
                                    cx,
                                )
                            })),
                    )
                    .when(p.can_skip(), |d| {
                        d.child(
                            button("paused-skip")
                                .small()
                                .label("Skip")
                                .tooltip("Leave this commit out and go on")
                                .off(self.busy.is_some())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_op(
                                        "Skipping…",
                                        None,
                                        move |repo| git::skip_paused(repo, p),
                                        cx,
                                    )
                                })),
                        )
                    })
                    .child(
                        button("pick-abort")
                            .small()
                            .label("Abort")
                            .off(self.busy.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_op(
                                    "Aborting…",
                                    Some(format!("{} aborted", p.name())),
                                    move |repo| git::abort_paused(repo, p),
                                    cx,
                                )
                            })),
                    ),
            )
    }

    fn render_change_row(&self, row: &Row, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        match *row {
            Row::Dir { staged, ref dir } => {
                self.dir_row(scope(staged), dir, ("change-dir", ix), cx)
            }
            Row::Header { staged, count } => h_flex()
                .w_full()
                .h(px(30.))
                .px_2()
                .gap_2()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(div().flex_1().child(if staged {
                    format!("STAGED · {count}")
                } else {
                    format!("CHANGES · {count}")
                }))
                .when(!staged, |d| {
                    d.child(
                        button(("stash", ix))
                            .ghost()
                            .xsmall()
                            .label("Stash")
                            .tooltip("Stash all changes  ⌥⌘S")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.stash_dialog(window, cx)),
                            ),
                    )
                    .child(
                        button(("discard-all", ix))
                            .ghost()
                            .xsmall()
                            .label("Discard all")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let entries: Vec<StatusEntry> = this
                                    .status
                                    .iter()
                                    .filter(|e| e.unstaged.is_some())
                                    .cloned()
                                    .collect();
                                this.confirm_discard_files(entries, window, cx)
                            })),
                    )
                })
                .child(
                    button(("stage-all", ix))
                        .ghost()
                        .xsmall()
                        .label(if staged { "Unstage all" } else { "Stage all" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let label = if staged { "Unstaging…" } else { "Staging…" };
                            this.run_op(
                                label,
                                None,
                                move |repo| {
                                    if staged {
                                        git::unstage_all(repo)
                                    } else {
                                        git::stage_all(repo)
                                    }
                                    .map(|_| String::new())
                                },
                                cx,
                            )
                        })),
                )
                .into_any_element(),
            Row::File {
                entry,
                staged,
                depth,
            } => {
                let e = &self.status[entry];
                let change =
                    if staged { e.staged } else { e.unstaged }.unwrap_or(git::Change::Modified);
                let selected = self.change_sel.as_ref() == Some(&(e.path.clone(), staged));
                let flash = self
                    .edited
                    .get(&e.path)
                    .copied()
                    .filter(|at| at.elapsed() < FLASH);
                let (path, path2) = (e.path.clone(), e.path.clone());
                let entry_menu = e.clone();
                let root = self.repo.as_ref().map(|r| r.root.clone());
                let this = cx.entity();
                diff::path_row(&e.path, change, selected, flash, depth, ("change", ix), cx)
                    .child(
                        button(("stage", ix))
                            .ghost()
                            .xsmall()
                            .icon(if staged {
                                IconName::Minus
                            } else {
                                IconName::Plus
                            })
                            .tooltip(if staged { "Unstage" } else { "Stage" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let paths = vec![path.clone()];
                                this.run_op(
                                    if staged { "Unstaging…" } else { "Staging…" },
                                    None,
                                    move |repo| {
                                        if staged {
                                            git::unstage(repo, &paths)
                                        } else {
                                            git::stage(repo, &paths)
                                        }
                                        .map(|_| String::new())
                                    },
                                    cx,
                                )
                            })),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.change_sel = Some((path2.clone(), staged));
                            this.load_change_diff(cx);
                            cx.notify();
                        }),
                    )
                    .context_menu(move |menu, _, _| {
                        let e = entry_menu.clone();
                        let (a, b, c) = (this.clone(), this.clone(), e.clone());
                        let paths = vec![e.path.clone()];
                        let mut menu = menu.item(
                            PopupMenuItem::new(if staged { "Unstage" } else { "Stage" }).on_click(
                                move |_, _, cx| {
                                    let paths = paths.clone();
                                    a.update(cx, |app, cx| {
                                        app.run_op(
                                            "Staging…",
                                            None,
                                            move |repo| {
                                                if staged {
                                                    git::unstage(repo, &paths)
                                                } else {
                                                    git::stage(repo, &paths)
                                                }
                                                .map(|_| String::new())
                                            },
                                            cx,
                                        )
                                    });
                                },
                            ),
                        );
                        if !staged {
                            menu = menu.item(PopupMenuItem::new("Discard Changes…").on_click(
                                move |_, window, cx| {
                                    let e = c.clone();
                                    b.update(cx, |app, cx| {
                                        app.confirm_discard_files(vec![e], window, cx)
                                    });
                                },
                            ));
                        }
                        let copy = e.path.clone();
                        let full = root.as_ref().map(|r| r.join(&e.path));
                        menu.separator()
                            .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }))
                            .item(PopupMenuItem::new("Reveal in Finder").on_click(
                                move |_, _, _| {
                                    if let Some(p) = &full {
                                        let _ = std::process::Command::new("open")
                                            .arg("-R")
                                            .arg(p)
                                            .spawn();
                                    }
                                },
                            ))
                    })
                    .into_any_element()
            }
        }
    }
}
