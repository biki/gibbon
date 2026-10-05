//! Worktrees: a sidebar row per worktree with what it holds now, and
//! removing a worktree. Agents often work in worktrees of their own, so the
//! rows show their work while it happens. The icon of a row shows the state
//! of its work (see `agents`): a robot where an agent runs.

use gpui_kit::component::menu::ContextMenuExt as _;

use super::agents::AgentState;
use super::pulls::lines_tooltip;
use super::sidebar::side_row;
use super::*;

impl GitApp {
    /// The sidebar shows the worktrees when there is more than one.
    pub(super) fn has_worktrees(&self) -> bool {
        self.worktrees.len() > 1
    }

    fn worktree_info_of(&self, w: &git::Worktree) -> Option<&git::WorktreeInfo> {
        self.worktree_info.get(&w.path).map(|(_, info)| info)
    }

    /// Show the worktree at `path`: this tab's Changes, or its own tab.
    pub(super) fn open_worktree(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(ix) = self.worktrees.iter().position(|w| w.path == path) else {
            return;
        };
        if Some(ix) == self.current_worktree {
            self.view = View::Changes;
            cx.notify();
        } else if !self.worktrees[ix].prunable {
            cx.emit(AppEvent::OpenWorktree(path));
        }
    }

    /// A new tab of a worktree opens on its Changes: that is the agent's
    /// work in progress. A tab that was open before reopens where it was.
    pub(super) fn show_changes_if_new(&mut self, cx: &mut Context<Self>) {
        if !self.restored {
            self.view = View::Changes;
            cx.notify();
        }
    }

    pub(super) fn render_worktree_row(
        &self,
        wi: usize,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let w = &self.worktrees[wi];
        let current = self.current_worktree == Some(wi);
        let info = self.worktree_info_of(w);
        let now = now();
        let agent = !self.agents_in(w).is_empty();
        let folder = if agent {
            IconName::Bot
        } else {
            IconName::FolderGit2
        };
        let (icon, color) = match self.worktree_state(w, now) {
            Some(AgentState::Missing) => (IconName::FolderX, t.colors.red),
            Some(AgentState::Conflict) => (IconName::GitMergeConflict, t.colors.red),
            Some(s @ (AgentState::Working | AgentState::Quiet)) => (folder, s.color(cx)),
            _ if current => (folder, t.colors.primary),
            _ => (folder, muted),
        };
        let name = match w.branch_name() {
            Some(b) => b.to_string(),
            None => format!("{} (detached)", w.folder()),
        };
        let changed = info.map_or(0, |i| i.changed);
        let ahead = info.map_or(0, |i| i.ahead);
        let age = info.filter(|i| i.active > 0).map(|i| fmt_age(i.active));
        let tip = self.worktree_tip(w, info, now);
        let path = w.path.clone();
        let menu_wt = w.clone();
        let can_remove = !w.main && !current && !w.locked;
        let this = cx.entity();
        // Like the checked-out branch: bold, never filled. The Changes row
        // above is this worktree's own.
        side_row(ix, false, cx)
            .child(Icon::new(icon).size(px(14.)).text_color(color))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(current, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .when(w.prunable, |d| d.text_color(muted).line_through())
                    .child(name),
            )
            .when(w.locked, |d| {
                d.child(Icon::new(IconName::Lock).size(px(11.)).text_color(muted))
            })
            // The checks of the branch's pull request.
            .when_some(
                w.branch_name()
                    .and_then(|b| self.pr_of_branch(b))
                    .and_then(|p| self.pr_checks_icon(p.number, cx)),
                |d, icon| d.child(icon),
            )
            .when(changed > 0, |d| {
                d.child(
                    h_flex()
                        .flex_none()
                        .gap_0p5()
                        .text_size(px(11.))
                        .text_color(t.colors.yellow)
                        .child(Icon::new(IconName::FilePen).size(px(11.)))
                        .child(changed.to_string()),
                )
            })
            .when(ahead > 0, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(format!("↑{ahead}")),
                )
            })
            .when_some(age, |d, age| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(age),
                )
            })
            .tooltip(lines_tooltip(tip))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.open_worktree(path.clone(), cx)),
            )
            .context_menu(move |menu, _, _| {
                worktree_menu(menu, &menu_wt, current, can_remove, this.clone())
            })
            .into_any_element()
    }

    /// The lines of a worktree row's tooltip: its name, then its details.
    fn worktree_tip(
        &self,
        w: &git::Worktree,
        info: Option<&git::WorktreeInfo>,
        now: i64,
    ) -> Rc<Vec<String>> {
        let title = match w.branch_name() {
            Some(b) => format!("{b} · {}", w.folder()),
            None => w.folder(),
        };
        let mut lines = vec![title, w.path.display().to_string()];
        if w.prunable {
            lines.push("The folder is gone. Right-click to forget it.".into());
            return Rc::new(lines);
        }
        lines.extend(self.state_text(w, now));
        if let Some(i) = info {
            let base = self
                .base
                .as_deref()
                .map(|b| b.strip_prefix("refs/heads/").unwrap_or(b));
            let mut facts = vec![];
            if let Some(base) = base.filter(|_| w.head.is_some()) {
                facts.push(match (i.ahead, i.behind) {
                    (0, 0) => format!("Even with {base}"),
                    (a, 0) => format!("{a} commit{} ahead of {base}", history::plural(a as usize)),
                    (0, b) => format!("{b} commit{} behind {base}", history::plural(b as usize)),
                    (a, b) => format!("{a} ahead of {base}, {b} behind"),
                });
            }
            facts.push(match i.changed {
                0 => "No uncommitted changes".to_string(),
                n => format!("{n} changed file{}", history::plural(n)),
            });
            lines.push(facts.join(" · "));
            if i.active > 0 {
                lines.push(format!("Last change {}: {}", fmt_time(i.active), i.subject));
            }
        }
        if let Some(p) = w.branch_name().and_then(|b| self.pr_of_branch(b)) {
            lines.push(format!("Pull request #{} {}", p.number, p.title));
            lines.extend(self.pr_status_lines(p.number));
        }
        if w.locked {
            lines.push("Locked: Gibbon does not remove it.".into());
        }
        Rc::new(lines)
    }

    /// Confirm, then remove the worktree, and with `with_branch` its
    /// branch. A worktree whose folder is gone is only forgotten.
    pub(super) fn remove_worktree_dialog(
        &mut self,
        wt: git::Worktree,
        with_branch: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = wt.path.display().to_string();
        if wt.prunable {
            self.confirm(
                "Forget worktree?",
                format!(
                    "The folder {path} is gone. This removes it from the worktree list, \
                     with the other worktrees whose folders are gone."
                ),
                "Forget",
                |this, cx| {
                    this.run_op(
                        "Pruning…",
                        Some("Forgot the missing worktrees".into()),
                        |repo| git::prune_worktrees(repo).map(|_| String::new()),
                        cx,
                    )
                },
                window,
                cx,
            );
            return;
        }
        let changed = self.worktree_info_of(&wt).map_or(0, |i| i.changed);
        let mut body = if changed > 0 {
            format!(
                "This deletes the folder {path} and its {changed} changed file{}. \
                 You cannot undo it.",
                history::plural(changed)
            )
        } else {
            format!("This deletes the folder {path}.")
        };
        match (wt.branch_name(), with_branch) {
            (Some(b), true) => body.push_str(&format!(" Then it deletes the branch {b}.")),
            (Some(b), false) => body.push_str(&format!(" The branch {b} stays.")),
            (None, _) => {}
        }
        self.confirm(
            "Remove worktree?",
            body,
            "Remove",
            move |this, cx| this.remove_worktree(wt.clone(), changed > 0, with_branch, cx),
            window,
            cx,
        );
    }

    /// `force` removes a worktree with changes: the user saw their count.
    /// A worktree that got changes since then makes git refuse.
    fn remove_worktree(
        &mut self,
        wt: git::Worktree,
        force: bool,
        with_branch: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.set_busy(Some("Removing worktree…".into()), cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let path = wt.path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { git::remove_worktree(&repo, &path, force) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.set_busy(None, cx);
                match result {
                    Ok(()) => {
                        cx.emit(AppEvent::Forget(wt.path.clone()));
                        match wt.branch_name().filter(|_| with_branch) {
                            // It reloads when it is done.
                            Some(b) => this.delete_branch(b.to_string(), cx),
                            None => {
                                let msg = format!("Removed the worktree {}", wt.folder());
                                this.toast(Some(true), msg, cx);
                                this.reload(cx);
                            }
                        }
                    }
                    Err(e) => {
                        this.toast(Some(false), e.to_string(), cx);
                        this.reload(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// The menu of a worktree's row and card.
pub(super) fn worktree_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    wt: &git::Worktree,
    current: bool,
    can_remove: bool,
    this: Entity<GitApp>,
) -> gpui_kit::component::menu::PopupMenu {
    let mut menu = menu;
    if !wt.prunable {
        let (app, path) = (this.clone(), wt.path.clone());
        let label = if current {
            "Show Changes"
        } else {
            "Open in Tab"
        };
        menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
            let path = path.clone();
            app.update(cx, |app, cx| app.open_worktree(path, cx));
        }));
    }
    if let Some(branch) = wt.branch.clone() {
        let app = this.clone();
        menu = menu.item(
            PopupMenuItem::new("Browse Commits").on_click(move |_, _, cx| {
                let r = branch.clone();
                app.update(cx, |app, cx| app.show_target(LogTarget::Ref(r), cx));
            }),
        );
    }
    if !wt.prunable && wt.head.is_some() {
        let (app, w) = (this.clone(), wt.clone());
        menu = menu.item(
            PopupMenuItem::new("Review Changes").on_click(move |_, _, cx| {
                app.update(cx, |app, cx| app.start_worktree_review(&w, cx));
            }),
        );
    }
    menu = menu.separator();
    if !wt.prunable {
        let dir = wt.path.clone();
        menu = menu.item(
            PopupMenuItem::new("Open in Finder").on_click(move |_, _, _| {
                let _ = std::process::Command::new("open").arg(&dir).spawn();
            }),
        );
    }
    let copy = wt.path.display().to_string();
    menu = menu
        .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
        }))
        .separator();
    if wt.prunable {
        let (app, w) = (this.clone(), wt.clone());
        return menu.item(
            PopupMenuItem::new("Forget…").on_click(move |_, window, cx| {
                let w = w.clone();
                app.update(cx, |app, cx| {
                    app.remove_worktree_dialog(w, false, window, cx)
                });
            }),
        );
    }
    let (app, w) = (this.clone(), wt.clone());
    menu = menu.item(
        PopupMenuItem::new("Remove…")
            .disabled(!can_remove)
            .on_click(move |_, window, cx| {
                let w = w.clone();
                app.update(cx, |app, cx| {
                    app.remove_worktree_dialog(w, false, window, cx)
                });
            }),
    );
    if let Some(b) = wt.branch_name() {
        let (app, w) = (this, wt.clone());
        menu = menu.item(
            PopupMenuItem::new(format!("Remove with Branch {b}…"))
                .disabled(!can_remove)
                .on_click(move |_, window, cx| {
                    let w = w.clone();
                    app.update(cx, |app, cx| {
                        app.remove_worktree_dialog(w, true, window, cx)
                    });
                }),
        );
    }
    menu
}
