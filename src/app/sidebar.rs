//! Sidebar: the two views, then local branches, remotes and tags.
//! One virtualized list, so a thousand remote branches cost nothing.

use std::ops::Range;

use gpui_kit::component::input::Input;
use gpui_kit::component::menu::ContextMenuExt as _;

use super::*;

enum Row {
    Changes,
    History,
    All,
    Header {
        key: &'static str,
        label: &'static str,
        count: usize,
    },
    Branch(usize),
    /// Index into `stashes`.
    Stash(usize),
    /// Index into `prs`.
    Pr(usize),
}

const ROW_H: f32 = 28.;

impl GitApp {
    fn sidebar_rows(&self, cx: &App) -> Vec<Row> {
        let needle = self.filter.read(cx).value().to_lowercase();
        let mut rows = vec![Row::Changes, Row::History, Row::All];
        for (key, label, kind) in [
            ("local", "Branches", Some(RefKind::Local)),
            ("prs", "Pull requests", None),
            ("stashes", "Stashes", None),
            ("remote", "Remote", Some(RefKind::Remote)),
            ("tags", "Tags", Some(RefKind::Tag)),
        ] {
            let Some(kind) = kind else {
                let matches: Vec<usize> = if key == "prs" {
                    self.prs
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| {
                            needle.is_empty()
                                || p.title.to_lowercase().contains(&needle)
                                || p.head.to_lowercase().contains(&needle)
                        })
                        .map(|(i, _)| i)
                        .collect()
                } else {
                    self.stashes
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| {
                            needle.is_empty() || s.message.to_lowercase().contains(&needle)
                        })
                        .map(|(i, _)| i)
                        .collect()
                };
                if matches.is_empty() {
                    continue;
                }
                rows.push(Row::Header {
                    key,
                    label,
                    count: matches.len(),
                });
                if !self.collapsed.contains(key) {
                    let row = if key == "prs" { Row::Pr } else { Row::Stash };
                    rows.extend(matches.into_iter().map(row));
                }
                continue;
            };
            let matches: Vec<usize> = self
                .branches
                .iter()
                .enumerate()
                .filter(|(_, b)| b.kind == kind)
                .filter(|(_, b)| needle.is_empty() || b.name.to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect();
            if matches.is_empty() {
                continue;
            }
            rows.push(Row::Header {
                key,
                label,
                count: matches.len(),
            });
            if !self.collapsed.contains(key) {
                rows.extend(matches.into_iter().map(Row::Branch));
            }
        }
        rows
    }

    pub(super) fn render_sidebar(&mut self, memo: &mut Memo, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = keep(memo, || self.sidebar_rows(cx));
        let t = cx.theme();
        let n = rows.len();
        v_flex()
            .size_full()
            .bg(t.colors.sidebar)
            .border_r_1()
            .border_color(t.colors.border)
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .child(Input::new(&self.filter).small().cleanable(true)),
            )
            .child(
                uniform_list(
                    "sidebar",
                    n,
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        range
                            .map(|i| this.render_side_row(&rows[i], i, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.side_scroll)
                .flex_1()
                .px_1p5()
                .py_1(),
            )
    }

    fn render_side_row(&self, row: &Row, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let base = |active: bool| {
            h_flex()
                .id(("side", ix))
                .w_full()
                .h(px(ROW_H))
                .px_2()
                .gap_2()
                .rounded(t.radius)
                .cursor_pointer()
                .when(active, |d| d.bg(t.colors.sidebar_accent))
                .when(!active, |d| d.child(hover_fill(t.colors.list_hover, t.radius)))
        };
        let label = |text: String, strong: bool| {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(strong, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(text)
        };
        let badge = |text: String| {
            div()
                .px_1p5()
                .rounded(px(9.))
                .bg(t.colors.muted)
                .text_size(px(11.))
                .text_color(muted)
                .child(text)
        };
        match row {
            Row::Changes => {
                let active = self.view == View::Changes;
                let n = self.status.len();
                base(active)
                    .child(Icon::new(IconName::FilePen).size(px(15.)).text_color(muted))
                    .child(label("Changes".into(), active))
                    .when(n > 0, |d| d.child(badge(n.to_string())))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.view = View::Changes;
                            cx.notify();
                        }),
                    )
                    .into_any_element()
            }
            Row::History => {
                let active = self.view == View::History && self.target == LogTarget::Head;
                base(active)
                    .child(Icon::new(IconName::GitCommitVertical).size(px(15.)).text_color(muted))
                    .child(label("History".into(), active))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.show_target(LogTarget::Head, cx)),
                    )
                    .into_any_element()
            }
            Row::All => {
                let active = self.view == View::History && self.target == LogTarget::All;
                base(active)
                    .child(Icon::new(IconName::GitGraph).size(px(15.)).text_color(muted))
                    .child(label("All branches".into(), active))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.show_target(LogTarget::All, cx)),
                    )
                    .into_any_element()
            }
            &Row::Header { key, label, count } => {
                let collapsed = self.collapsed.contains(key);
                h_flex()
                    .id(("side-head", ix))
                    .w_full()
                    .h(px(ROW_H))
                    .mt_2()
                    .px_2()
                    .gap_1()
                    .cursor_pointer()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(12.)),
                    )
                    .child(div().flex_1().child(label.to_uppercase()))
                    .child(count.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed.remove(key) {
                            this.collapsed.insert(key);
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            }
            &Row::Pr(pi) => {
                let p = self.prs[pi].clone();
                let active = self.view == View::History
                    && matches!(&self.target, LogTarget::Ref(r) if *r == p.refname());
                let (menu_pr, this) = (p.clone(), cx.entity());
                base(active)
                    .child(
                        Icon::new(if p.draft {
                            IconName::GitPullRequestDraft
                        } else {
                            IconName::GitPullRequest
                        })
                        .size(px(14.))
                        .text_color(if p.draft { muted } else { t.colors.green }),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(muted)
                            .flex_none()
                            .child(format!("#{}", p.number)),
                    )
                    .child(label(p.title.clone(), false))
                    .tooltip({
                        let tip = format!("{} → {} · {}", p.head, p.base, p.author);
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                .build(window, cx)
                        }
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| this.browse_pr(p.clone(), cx)),
                    )
                    .context_menu(move |menu, _, _| {
                        let (url, url2, n) = (
                            menu_pr.url.clone(),
                            menu_pr.url.clone(),
                            menu_pr.number,
                        );
                        let (this, this2) = (this.clone(), this.clone());
                        let browse = menu_pr.clone();
                        menu.item(PopupMenuItem::new("Browse Commits").on_click(
                            move |_, _, cx| {
                                let pr = browse.clone();
                                this.update(cx, |app, cx| app.browse_pr(pr, cx));
                            },
                        ))
                        .item(PopupMenuItem::new("Check Out").on_click(move |_, _, cx| {
                            this2.update(cx, |app, cx| {
                                app.target = LogTarget::Head;
                                app.run_op(
                                    "Checking out…",
                                    Some(format!("Checked out #{n}")),
                                    move |repo| crate::github::checkout(repo, n),
                                    cx,
                                )
                            });
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Open on GitHub").on_click(move |_, _, _| {
                            crate::github::open_url(&url)
                        }))
                        .item(PopupMenuItem::new("Copy URL").on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(url2.clone()))
                        }))
                    })
                    .into_any_element()
            }
            &Row::Stash(si) => {
                let s = &self.stashes[si];
                let index = s.index;
                let active = self.view == View::Stash(index);
                let this = cx.entity();
                base(active)
                    .child(Icon::new(IconName::Archive).size(px(14.)).text_color(muted))
                    .child(label(s.title().to_string(), false))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(fmt_time(s.time)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| this.show_stash(index, cx)),
                    )
                    .context_menu(move |menu, _, _| {
                        let (this, this2, this3) = (this.clone(), this.clone(), this.clone());
                        menu.item(PopupMenuItem::new("Apply").on_click(move |_, _, cx| {
                            this.update(cx, |app, cx| app.stash_op(index, false, cx));
                        }))
                        .item(PopupMenuItem::new("Pop").on_click(move |_, _, cx| {
                            this2.update(cx, |app, cx| app.stash_op(index, true, cx));
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Drop…").on_click(move |_, window, cx| {
                            this3.update(cx, |app, cx| app.drop_stash_dialog(index, window, cx));
                        }))
                    })
                    .into_any_element()
            }
            &Row::Branch(bi) => {
                let b = &self.branches[bi];
                let active = self.view == View::History
                    && matches!(&self.target, LogTarget::Ref(r) if *r == b.refname);
                let icon = match b.kind {
                    RefKind::Local => IconName::GitBranch,
                    RefKind::Remote => IconName::Cloud,
                    RefKind::Tag => IconName::Tag,
                };
                let name = b.name.clone();
                let track = match (b.ahead, b.behind) {
                    (0, 0) => None,
                    (a, 0) => Some(format!("↑{a}")),
                    (0, z) => Some(format!("↓{z}")),
                    (a, z) => Some(format!("↑{a} ↓{z}")),
                };
                let (refname, branch) = (b.refname.clone(), b.clone());
                let menu_branch = b.clone();
                let head = self.head_name();
                let this = cx.entity();
                base(active)
                    .child(
                        Icon::new(icon)
                            .size(px(14.))
                            .text_color(if b.is_head { t.colors.primary } else { muted }),
                    )
                    .child(label(name, b.is_head))
                    .when_some(track, |d, s| {
                        d.child(div().text_size(px(11.)).text_color(muted).child(s))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                            if e.click_count >= 2 && branch.kind != RefKind::Tag {
                                this.switch_branch(branch.clone(), cx);
                            } else {
                                this.show_target(LogTarget::Ref(refname.clone()), cx);
                            }
                        }),
                    )
                    .context_menu(move |menu, _, _| {
                        let (a, b, c) = (menu_branch.clone(), menu_branch.clone(), this.clone());
                        let this2 = this.clone();
                        let copy = menu_branch.name.clone();
                        let mut menu = menu.item(
                            PopupMenuItem::new(format!("Browse {}", a.name)).on_click(
                                move |_, _, cx| {
                                    let r = a.refname.clone();
                                    c.update(cx, |app, cx| app.show_target(LogTarget::Ref(r), cx));
                                },
                            ),
                        );
                        if !menu_branch.is_head {
                            menu = menu.label(format!("Pick commits from here into {head}"));
                        }
                        if menu_branch.kind != RefKind::Tag && !menu_branch.is_head {
                            menu = menu.separator().item(
                                PopupMenuItem::new(format!("Switch to {}", b.name)).on_click(
                                    move |_, _, cx| {
                                        let b = b.clone();
                                        this2.update(cx, |app, cx| app.switch_branch(b, cx));
                                    },
                                ),
                            );
                        }
                        let (this3, from) = (this.clone(), menu_branch.clone());
                        menu = menu.separator().item(
                            PopupMenuItem::new(format!("New Branch from {}…", from.name))
                                .on_click(move |_, window, cx| {
                                    let start = (from.refname.clone(), from.name.clone());
                                    this3.update(cx, |app, cx| {
                                        app.new_branch_dialog(Some(start), window, cx)
                                    });
                                }),
                        );
                        if menu_branch.kind == RefKind::Local {
                            let (this6, pr_branch) = (this.clone(), menu_branch.name.clone());
                            menu = menu.item(PopupMenuItem::new("Create Pull Request…").on_click(
                                move |_, _, cx| {
                                    let b = pr_branch.clone();
                                    this6.update(cx, |app, cx| {
                                        app.run_op(
                                            "Pushing…",
                                            Some("Opened the pull request form on GitHub".into()),
                                            move |repo| crate::github::create_in_browser(repo, &b),
                                            cx,
                                        )
                                    });
                                },
                            ));
                            let (this4, this5) = (this.clone(), this.clone());
                            let (ren, del) = (menu_branch.clone(), menu_branch.clone());
                            menu = menu.item(PopupMenuItem::new("Rename…").on_click(
                                move |_, window, cx| {
                                    let b = ren.clone();
                                    this4.update(cx, |app, cx| {
                                        app.rename_branch_dialog(b, window, cx)
                                    });
                                },
                            ));
                            menu = menu.item(
                                PopupMenuItem::new("Delete…")
                                    .disabled(del.is_head)
                                    .on_click(move |_, window, cx| {
                                        let b = del.clone();
                                        this5.update(cx, |app, cx| {
                                            app.delete_branch_dialog(b, window, cx)
                                        });
                                    }),
                            );
                        }
                        if menu_branch.kind == RefKind::Remote {
                            let (this4, del) = (this.clone(), menu_branch.clone());
                            menu = menu.item(PopupMenuItem::new("Delete…").on_click(
                                move |_, window, cx| {
                                    let b = del.clone();
                                    this4.update(cx, |app, cx| {
                                        app.delete_remote_branch_dialog(b, window, cx)
                                    });
                                },
                            ));
                        }
                        menu.separator().item(PopupMenuItem::new("Copy Name").on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            },
                        ))
                    })
                    .into_any_element()
            }
        }
    }
}
