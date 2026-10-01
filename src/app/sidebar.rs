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
    Activity,
    Header {
        key: &'static str,
        label: &'static str,
        count: usize,
        /// Branches that a cleanup lists, for the local branches.
        stale: usize,
    },
    Branch(usize),
    /// Index into `stashes`.
    Stash(usize),
    /// Index into `prs`.
    Pr(usize),
    /// Index into `worktrees`.
    Worktree(usize),
    /// Opens a short branch section (the number of branches it hides) or
    /// makes an open one short again (None).
    Fold {
        key: &'static str,
        hidden: Option<usize>,
    },
}

const ROW_H: f32 = 28.;

/// Branches that a branch section always shows, first, in this order. The
/// base branch comes before them.
pub(super) const FIXED: [&str; 8] = [
    "main",
    "master",
    "trunk",
    "develop",
    "dev",
    "development",
    "staging",
    "production",
];
/// The most recent other branches that a short section shows.
const RECENT: usize = 5;
/// A section with fewer hidden branches than this shows all: a row that
/// shows one more branch is as long as that branch.
const MIN_HIDDEN: usize = 2;

/// Whether a section shows all its branches, or which row changes that.
#[derive(Debug, PartialEq, Eq)]
enum Fold {
    /// Nothing to hide.
    None,
    /// Short: this many branches are hidden.
    More(usize),
    /// Open, and it can be short again.
    Less,
}

/// The place of a branch among the fixed ones, if it is one: the base
/// branch first, then the order of `FIXED`. A remote branch counts by its
/// name on the remote.
fn fixed_rank(b: &Branch, base: Option<&str>) -> Option<usize> {
    if Some(b.refname.as_str()) == base {
        return Some(0);
    }
    let name = match b.kind {
        RefKind::Remote => b.name.split_once('/').map_or(b.name.as_str(), |(_, n)| n),
        _ => b.name.as_str(),
    };
    FIXED.iter().position(|f| *f == name).map(|p| p + 1)
}

/// The branches of a section, from `matches` (newest first): the fixed
/// ones first, then the others. A short section shows the `RECENT` newest
/// others and the checked-out branch; `all` shows every branch.
fn section_branches(
    branches: &[Branch],
    matches: &[usize],
    base: Option<&str>,
    all: bool,
) -> (Vec<usize>, Fold) {
    let mut fixed = vec![];
    let mut rest = vec![];
    for &i in matches {
        match fixed_rank(&branches[i], base) {
            Some(rank) => fixed.push((rank, i)),
            None => rest.push(i),
        }
    }
    // Stable: remote branches with the same name keep their order.
    fixed.sort_by_key(|&(rank, _)| rank);
    let mut out: Vec<usize> = fixed.into_iter().map(|(_, i)| i).collect();
    let shown = |pos: usize, i: usize| pos < RECENT || branches[i].is_head;
    let hidden = rest
        .iter()
        .enumerate()
        .filter(|&(pos, &i)| !shown(pos, i))
        .count();
    if hidden < MIN_HIDDEN {
        out.extend(rest);
        return (out, Fold::None);
    }
    if all {
        out.extend(rest);
        return (out, Fold::Less);
    }
    out.extend(
        rest.iter()
            .enumerate()
            .filter(|&(pos, &i)| shown(pos, i))
            .map(|(_, &i)| i),
    );
    (out, Fold::More(hidden))
}

/// The frame of a sidebar row: filled while `active`, highlighted on hover.
pub(super) fn side_row(ix: usize, active: bool, cx: &App) -> Stateful<Div> {
    let t = cx.theme();
    h_flex()
        .id(("side", ix))
        .w_full()
        .h(px(ROW_H))
        .px_2()
        .gap_2()
        .rounded(t.radius)
        .cursor_pointer()
        .when(active, |d| d.bg(t.colors.sidebar_accent))
        .when(!active, |d| {
            d.child(hover_fill(t.colors.list_hover, t.radius))
        })
}

impl GitApp {
    fn sidebar_rows(&self, cx: &App) -> Vec<Row> {
        let needle = self.filter.read(cx).value().to_lowercase();
        let mut rows = vec![Row::Changes, Row::History, Row::All, Row::Activity];
        if self.has_worktrees() {
            let matches: Vec<usize> = (0..self.worktrees.len())
                .filter(|&i| {
                    let w = &self.worktrees[i];
                    needle.is_empty()
                        || w.branch_name()
                            .is_some_and(|b| b.to_lowercase().contains(&needle))
                        || w.folder().to_lowercase().contains(&needle)
                })
                .collect();
            if !matches.is_empty() {
                rows.push(Row::Header {
                    key: "worktrees",
                    label: "Worktrees",
                    count: matches.len(),
                    stale: 0,
                });
                if !self.collapsed.contains("worktrees") {
                    rows.extend(matches.into_iter().map(Row::Worktree));
                }
            }
        }
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
                    stale: 0,
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
                stale: if kind == RefKind::Local {
                    self.stale_branches().len()
                } else {
                    0
                },
            });
            if self.collapsed.contains(key) {
                continue;
            }
            if kind == RefKind::Tag {
                rows.extend(matches.into_iter().map(Row::Branch));
                continue;
            }
            // A filter shows every match.
            let filtering = !needle.is_empty();
            let all = filtering || self.expanded.contains(key);
            let (shown, fold) =
                section_branches(&self.branches, &matches, self.base.as_deref(), all);
            rows.extend(shown.into_iter().map(Row::Branch));
            match fold {
                Fold::More(n) => rows.push(Row::Fold {
                    key,
                    hidden: Some(n),
                }),
                Fold::Less if !filtering => rows.push(Row::Fold { key, hidden: None }),
                _ => {}
            }
        }
        rows
    }

    pub(super) fn render_sidebar(
        &mut self,
        memo: &mut Memo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
        let base = |active: bool| side_row(ix, active, cx);
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
            &Row::Worktree(wi) => self.render_worktree_row(wi, ix, cx),
            &Row::Fold { key, hidden } => {
                let (icon, text) = match hidden {
                    Some(n) => (IconName::ChevronDown, format!("Show {n} more")),
                    None => (IconName::ChevronUp, "Show fewer".to_string()),
                };
                base(false)
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(Icon::new(icon).size(px(14.)))
                    .child(text)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(key) {
                            this.expanded.insert(key);
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            }
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
                    .child(
                        Icon::new(IconName::GitCommitVertical)
                            .size(px(15.))
                            .text_color(muted),
                    )
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
                    .child(
                        Icon::new(IconName::GitGraph)
                            .size(px(15.))
                            .text_color(muted),
                    )
                    .child(label("All branches".into(), active))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.show_target(LogTarget::All, cx)),
                    )
                    .into_any_element()
            }
            Row::Activity => {
                let active = self.view == View::Activity;
                let new = if active { 0 } else { self.new_moves() };
                base(active)
                    .child(
                        Icon::new(IconName::Activity)
                            .size(px(15.))
                            .text_color(muted),
                    )
                    .child(label("Activity".into(), active))
                    .when(new > 0, |d| {
                        d.child(
                            div()
                                .px_1p5()
                                .rounded(px(9.))
                                .bg(t.colors.primary.opacity(0.16))
                                .text_size(px(11.))
                                .text_color(t.colors.primary)
                                .child(if new > 99 {
                                    "99+".to_string()
                                } else {
                                    new.to_string()
                                }),
                        )
                    })
                    .tooltip(move |window, cx| {
                        let tip = match new {
                            0 => "Moves of all branches  ⌘4".to_string(),
                            n => format!(
                                "{n} new move{} since your last look  ⌘4",
                                history::plural(n)
                            ),
                        };
                        gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.show_activity(cx)),
                    )
                    .into_any_element()
            }
            &Row::Header {
                key,
                label,
                count,
                stale,
            } => {
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
                    .when(stale > 0, |d| {
                        d.child(
                            div()
                                .id("clean-up")
                                .flex_none()
                                .p(px(3.))
                                .rounded(px(4.))
                                .hover(|d| {
                                    d.bg(t.colors.list_hover).text_color(t.colors.foreground)
                                })
                                .child(Icon::new(IconName::Broom).size(px(13.)))
                                .tooltip(move |window, cx| {
                                    let tip = format!(
                                        "Clean up {stale} merged or gone branch{}…",
                                        if stale == 1 { "" } else { "es" }
                                    );
                                    gpui_kit::component::tooltip::Tooltip::new(tip)
                                        .build(window, cx)
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    // Not a click on the header: that folds it.
                                    cx.stop_propagation();
                                    this.cleanup_dialog(window, cx);
                                })),
                        )
                    })
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
                        .text_color(if p.draft {
                            muted
                        } else {
                            t.colors.green
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(muted)
                            .flex_none()
                            .child(format!("#{}", p.number)),
                    )
                    .child(label(p.title.clone(), false))
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .children(self.pr_badges(p.number, cx)),
                    )
                    .tooltip(pulls::lines_tooltip(self.pr_tip(&p)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| this.browse_pr(p.clone(), false, cx)),
                    )
                    .context_menu(move |menu, _, _| {
                        let (url, url2, n) =
                            (menu_pr.url.clone(), menu_pr.url.clone(), menu_pr.number);
                        let (this, this2, this3) = (this.clone(), this.clone(), this.clone());
                        let (browse, review) = (menu_pr.clone(), menu_pr.clone());
                        menu.item(
                            PopupMenuItem::new("Browse Commits").on_click(move |_, _, cx| {
                                let pr = browse.clone();
                                this.update(cx, |app, cx| app.browse_pr(pr, false, cx));
                            }),
                        )
                        .item(
                            PopupMenuItem::new("Review Changes").on_click(move |_, _, cx| {
                                let pr = review.clone();
                                this3.update(cx, |app, cx| app.browse_pr(pr, true, cx));
                            }),
                        )
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
                        .item(
                            PopupMenuItem::new("Open on GitHub")
                                .on_click(move |_, _, _| crate::github::open_url(&url)),
                        )
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
                let pr = (b.kind == RefKind::Local)
                    .then(|| self.pr_of_branch(&b.name).map(|p| p.number))
                    .flatten();
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
                    .child(Icon::new(icon).size(px(14.)).text_color(if b.is_head {
                        t.colors.primary
                    } else {
                        muted
                    }))
                    .child(label(name, b.is_head))
                    // The checks of the branch's pull request.
                    .when_some(pr.and_then(|n| self.pr_checks_icon(n, cx)), |d, icon| {
                        d.child(icon)
                    })
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
                        let mut menu =
                            menu.item(PopupMenuItem::new(format!("Browse {}", a.name)).on_click(
                                move |_, _, cx| {
                                    let r = a.refname.clone();
                                    c.update(cx, |app, cx| app.show_target(LogTarget::Ref(r), cx));
                                },
                            ));
                        if menu_branch.kind != RefKind::Tag {
                            let (this, r) = (this.clone(), menu_branch.refname.clone());
                            menu = menu.item(PopupMenuItem::new("Review Changes").on_click(
                                move |_, _, cx| {
                                    let r = r.clone();
                                    this.update(cx, |app, cx| app.start_review(r, cx));
                                },
                            ));
                        }
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
                            PopupMenuItem::new(format!("New Branch from {}…", from.name)).on_click(
                                move |_, window, cx| {
                                    let start = (from.refname.clone(), from.name.clone());
                                    this3.update(cx, |app, cx| {
                                        app.new_branch_dialog(Some(start), window, cx)
                                    });
                                },
                            ),
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
                        menu.separator()
                            .item(PopupMenuItem::new("Copy Name").on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }))
                    })
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::{Fold, section_branches};
    use crate::git::{Branch, RefKind};

    fn branch(name: &str, kind: RefKind, is_head: bool) -> Branch {
        let refname = match kind {
            RefKind::Local => format!("refs/heads/{name}"),
            RefKind::Remote => format!("refs/remotes/{name}"),
            RefKind::Tag => format!("refs/tags/{name}"),
        };
        Branch {
            refname,
            name: name.to_string(),
            kind,
            sha: String::new(),
            ahead: 0,
            behind: 0,
            gone: false,
            is_head,
        }
    }

    fn names(branches: &[Branch], ixs: &[usize]) -> Vec<String> {
        ixs.iter().map(|&i| branches[i].name.clone()).collect()
    }

    #[test]
    fn a_short_section_pins_fixed_branches_and_the_head() {
        // Newest first, as git lists them.
        let list: Vec<Branch> = ["a", "b", "dev", "c", "d", "e", "f", "old", "main", "g"]
            .iter()
            .map(|n| branch(n, RefKind::Local, *n == "old"))
            .collect();
        let all: Vec<usize> = (0..list.len()).collect();
        let (shown, fold) = section_branches(&list, &all, Some("refs/heads/main"), false);
        assert_eq!(
            names(&list, &shown),
            ["main", "dev", "a", "b", "c", "d", "e", "old"]
        );
        assert_eq!(fold, Fold::More(2), "f and g are hidden");

        let (shown, fold) = section_branches(&list, &all, Some("refs/heads/main"), true);
        assert_eq!(shown.len(), list.len());
        assert_eq!(names(&list, &shown)[..2], ["main", "dev"]);
        assert_eq!(fold, Fold::Less);
    }

    #[test]
    fn a_small_section_shows_everything() {
        let list: Vec<Branch> = ["a", "b", "c", "d", "e", "f", "master"]
            .iter()
            .map(|n| branch(n, RefKind::Local, false))
            .collect();
        let all: Vec<usize> = (0..list.len()).collect();
        let (shown, fold) = section_branches(&list, &all, None, false);
        // One branch past the five recent ones: no row to show it.
        assert_eq!(
            names(&list, &shown),
            ["master", "a", "b", "c", "d", "e", "f"]
        );
        assert_eq!(fold, Fold::None);
    }

    #[test]
    fn remote_branches_count_by_their_name_on_the_remote() {
        let list: Vec<Branch> = [
            "origin/x1",
            "origin/x2",
            "upstream/main",
            "origin/x3",
            "origin/x4",
            "origin/x5",
            "origin/x6",
            "origin/x7",
            "origin/main",
            "origin/feature/main",
        ]
        .iter()
        .map(|n| branch(n, RefKind::Remote, false))
        .collect();
        let all: Vec<usize> = (0..list.len()).collect();
        let (shown, fold) = section_branches(&list, &all, Some("refs/heads/main"), false);
        assert_eq!(
            names(&list, &shown),
            [
                "upstream/main",
                "origin/main",
                "origin/x1",
                "origin/x2",
                "origin/x3",
                "origin/x4",
                "origin/x5"
            ]
        );
        assert_eq!(fold, Fold::More(3));
    }
}
