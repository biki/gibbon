//! Agents: a card per worktree with what happens in it now. Parallel agents
//! each work in a worktree of their own, so the cards show all their work
//! side by side: the state, the files that changed last, the last commit,
//! and the buttons to review it or to pick its commits.
//!
//! The state comes from the worktree's files and commits, and from the
//! agent processes that run in its folder (see `crate::agents`).

use gpui_kit::component::menu::ContextMenuExt as _;

use crate::agents::Agent;
use crate::git::{Worktree, WorktreeFile, WorktreeInfo};

use super::*;

/// A worktree whose files or commits changed this recently is Working.
pub(super) const WORKING_SECS: i64 = 120;
/// Changed files that a card lists at most.
const CARD_FILES: usize = 4;
const FILE_ROW_H: f32 = 22.;
/// The narrowest card. A row holds as many cards as fit, up to `MAX_COLS`.
const CARD_MIN_W: f32 = 380.;
const MAX_COLS: f32 = 3.;
const GAP: f32 = 12.;
const PAD: f32 = 16.;

/// What happens in a worktree now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentState {
    /// The worktree's folder is gone.
    Missing,
    /// A cherry-pick, rebase or merge stopped on a conflict.
    Conflict,
    /// Its files or commits changed in the last `WORKING_SECS`.
    Working,
    /// An agent runs there, but nothing changed for a while. It may wait
    /// for an answer, or run a long command.
    Quiet,
    /// Uncommitted changes, and no agent runs there.
    Idle,
    /// Commits ahead of the base branch, no uncommitted changes, and no
    /// agent runs there.
    Ready,
    /// No commits ahead of the base branch and no uncommitted changes.
    Clean,
}

impl AgentState {
    fn label(self) -> &'static str {
        match self {
            AgentState::Missing => "Folder gone",
            AgentState::Conflict => "Conflict",
            AgentState::Working => "Working",
            AgentState::Quiet => "Quiet",
            AgentState::Idle => "Idle",
            AgentState::Ready => "Ready to review",
            AgentState::Clean => "No changes",
        }
    }

    pub(super) fn color(self, cx: &App) -> Hsla {
        let t = cx.theme();
        match self {
            AgentState::Missing | AgentState::Conflict => t.colors.red,
            AgentState::Working => t.colors.green,
            AgentState::Quiet => t.colors.yellow,
            AgentState::Ready => t.colors.primary,
            AgentState::Idle | AgentState::Clean => t.colors.muted_foreground,
        }
    }

    /// The state in a pill: "Quiet · 6m" for the states that last.
    fn pill_text(self, info: Option<&WorktreeInfo>) -> String {
        match (self, info) {
            (AgentState::Quiet | AgentState::Idle, Some(i)) if i.active > 0 => {
                format!("{} · {}", self.label(), fmt_age(i.active))
            }
            _ => self.label().to_string(),
        }
    }
}

/// The state of the worktree `w`. None until its info loads.
pub(super) fn agent_state(
    w: &Worktree,
    info: Option<&WorktreeInfo>,
    agent: bool,
    now: i64,
) -> Option<AgentState> {
    if w.prunable {
        return Some(AgentState::Missing);
    }
    let i = info?;
    let work = i.changed > 0 || i.ahead > 0;
    Some(if i.paused.is_some() {
        AgentState::Conflict
    } else if now - i.active < WORKING_SECS && (work || agent) {
        AgentState::Working
    } else if agent {
        AgentState::Quiet
    } else if i.changed > 0 {
        AgentState::Idle
    } else if i.ahead > 0 {
        AgentState::Ready
    } else {
        AgentState::Clean
    })
}

/// What the tab strip shows for the tab of a worktree (see `workspace`).
/// The tab that loads the worktrees sends it for each of them, so a hidden
/// tab shows the work that goes on in its worktree too.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabLabel {
    /// The branch of a linked worktree: it names the tab. A main worktree
    /// keeps the name of its folder.
    pub(super) branch: Option<String>,
    pub(super) changed: usize,
    /// An agent runs in the worktree.
    pub(super) agent: bool,
    pub(super) state: Option<AgentState>,
    /// The lines of the tab's tooltip.
    pub(super) tip: Vec<String>,
}

impl GitApp {
    /// The labels of the tabs of this repository's worktrees, by folder.
    fn tab_labels(&self, now: i64) -> Vec<(PathBuf, TabLabel)> {
        let Some(repo) = &self.repo else {
            return vec![];
        };
        let branch_tip = |folder: String, branch: Option<&str>| match branch {
            Some(b) => format!("{folder} · {b}"),
            None => folder,
        };
        if !self.has_worktrees() {
            let root = repo
                .root
                .canonicalize()
                .unwrap_or_else(|_| repo.root.clone());
            let tip = vec![
                branch_tip(repo.name.clone(), self.head.branch.as_deref()),
                root.display().to_string(),
            ];
            let label = TabLabel {
                branch: None,
                changed: self.status.len(),
                agent: false,
                state: None,
                tip,
            };
            return vec![(root, label)];
        }
        let main = self.worktrees[0].folder();
        self.worktrees
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.prunable)
            .map(|(i, w)| {
                // This tab's own status is fresher than its row's.
                let changed = if Some(i) == self.current_worktree {
                    self.status.len()
                } else {
                    self.worktree_info
                        .get(&w.path)
                        .map_or(0, |(_, i)| i.changed)
                };
                let branch = w.branch_name().filter(|_| !w.main).map(str::to_string);
                let title = if w.main {
                    branch_tip(w.folder(), w.branch_name())
                } else {
                    let name = branch
                        .clone()
                        .unwrap_or_else(|| format!("{} (detached)", w.folder()));
                    format!("{name} · a worktree of {main}")
                };
                let mut tip = vec![title, w.path.display().to_string()];
                tip.extend(self.state_text(w, now));
                if changed > 0 {
                    tip.push(format!(
                        "{changed} changed file{}",
                        history::plural(changed)
                    ));
                }
                let label = TabLabel {
                    branch,
                    changed,
                    agent: !self.agents_in(w).is_empty(),
                    state: self.worktree_state(w, now),
                    tip,
                };
                (w.path.clone(), label)
            })
            .collect()
    }

    /// Send the labels of the tabs to the window when they changed.
    pub(super) fn send_tab_labels(&mut self, cx: &mut Context<Self>) {
        let labels = self.tab_labels(now());
        if labels != self.sent_labels {
            self.sent_labels = labels.clone();
            cx.emit(AppEvent::Labels(labels));
        }
    }

    /// The folders of this repository's worktrees.
    pub(super) fn worktree_paths(&self) -> Vec<PathBuf> {
        self.worktrees.iter().map(|w| w.path.clone()).collect()
    }

    /// Show the cards, with fresh counts.
    pub(super) fn show_agents(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Agents {
            self.view = View::Agents;
            self.load_worktree_info(false, cx);
        }
        cx.notify();
    }

    /// The agents that run in the worktree `w`.
    pub(super) fn agents_in(&self, w: &Worktree) -> &[Agent] {
        self.agents.get(&w.path).map_or(&[], Vec::as_slice)
    }

    pub(super) fn worktree_state(&self, w: &Worktree, now: i64) -> Option<AgentState> {
        let info = self.worktree_info.get(&w.path).map(|(_, i)| i);
        agent_state(w, info, !self.agents_in(w).is_empty(), now)
    }

    /// How many worktrees are in each state, in the order of `ORDER`.
    fn state_counts(&self, now: i64) -> Vec<(AgentState, usize)> {
        const ORDER: [AgentState; 5] = [
            AgentState::Working,
            AgentState::Quiet,
            AgentState::Conflict,
            AgentState::Ready,
            AgentState::Idle,
        ];
        let states: Vec<AgentState> = self
            .worktrees
            .iter()
            .filter_map(|w| self.worktree_state(w, now))
            .collect();
        ORDER
            .into_iter()
            .map(|s| (s, states.iter().filter(|&&o| o == s).count()))
            .filter(|&(_, n)| n > 0)
            .collect()
    }

    /// The tooltip of the Agents row in the sidebar.
    pub(super) fn agents_tip(&self, now: i64) -> String {
        let counts = self.state_counts(now);
        if counts.is_empty() {
            return "Agents in the worktrees  ⌘5".to_string();
        }
        let words: Vec<String> = counts
            .iter()
            .map(|(s, n)| format!("{n} {}", s.label().to_lowercase()))
            .collect();
        format!("{}  ⌘5", words.join(" · "))
    }

    /// Worktrees whose files or commits change now: the badge of the
    /// Agents row.
    pub(super) fn working_count(&self, now: i64) -> usize {
        self.worktrees
            .iter()
            .filter(|w| self.worktree_state(w, now) == Some(AgentState::Working))
            .count()
    }

    /// The state of `w` in words, for tooltips.
    pub(super) fn state_text(&self, w: &Worktree, now: i64) -> Option<String> {
        let state = self.worktree_state(w, now)?;
        let info = self.worktree_info.get(&w.path).map(|(_, i)| i);
        let agents = crate::agents::names(self.agents_in(w));
        let quiet = info.map(|i| fmt_time(i.active)).unwrap_or_default();
        let base = self.base_name_of(w);
        Some(match state {
            AgentState::Missing => "The folder is gone.".to_string(),
            AgentState::Conflict => match info.and_then(|i| i.paused) {
                Some(p) => format!("{} stopped on a conflict.", p.name()),
                None => "Stopped on a conflict.".to_string(),
            },
            AgentState::Working if agents.is_empty() => {
                format!("Working: the last change was {quiet}.")
            }
            AgentState::Working => format!("Working: {agents} runs here."),
            AgentState::Quiet => format!(
                "Quiet: {agents} runs here, but the last change was {quiet}. \
                 It may wait for your answer."
            ),
            AgentState::Idle => format!(
                "Idle: uncommitted changes, and no agent runs here. \
                 The last change was {quiet}."
            ),
            AgentState::Ready => {
                format!("Ready to review: commits ahead of {base}, and no uncommitted changes.")
            }
            AgentState::Clean => {
                format!("No changes: no commits ahead of {base}, and no uncommitted changes.")
            }
        })
    }

    pub(super) fn render_agents(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let n = self.worktrees.len();
        let mut parts = vec![format!("{n} worktree{}", history::plural(n))];
        let running: Vec<Agent> = self.agents.values().flatten().copied().collect();
        if !running.is_empty() {
            parts.push(format!(
                "{} run{}",
                crate::agents::names(&running),
                if running.len() == 1 { "s" } else { "" }
            ));
        }
        parts.extend(
            self.state_counts(now())
                .into_iter()
                .map(|(s, n)| format!("{n} {}", s.label().to_lowercase())),
        );
        let header = h_flex()
            .flex_none()
            .px_3()
            .py_2()
            .gap_3()
            .border_b_1()
            .border_color(t.colors.border)
            .child(
                Icon::new(IconName::Bot)
                    .size(px(18.))
                    .text_color(t.colors.primary),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Agents"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(muted)
                            .truncate()
                            .child(parts.join(" · ")),
                    ),
            );
        v_flex()
            .size_full()
            .child(header)
            .child(div().flex_1().min_h_0().child(self.pane(Part::Agents, cx)))
            .into_any_element()
    }

    /// The cards, in their own pane. A row holds as many cards as the
    /// width of the view fits.
    pub(super) fn render_agent_cards(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().colors.muted_foreground;
        if !self.has_worktrees() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(muted)
                .child(Icon::new(IconName::Bot).size(px(28.)))
                .child("This repository has one worktree.")
                .child(
                    div()
                        .text_size(px(12.))
                        .child("Agents that work in worktrees of their own show here."),
                )
                .into_any_element();
        }
        let sidebar = split_size("main-split", 250., 190.0..420., cx);
        let room = f32::from(window.viewport_size().width) - sidebar - 2. * PAD;
        let cols = ((room + GAP) / (CARD_MIN_W + GAP))
            .floor()
            .clamp(1., MAX_COLS) as u16;
        let now = now();
        let cards: Vec<AnyElement> = (0..self.worktrees.len())
            .map(|wi| self.render_agent_card(wi, now, cx))
            .collect();
        div()
            .id("agent-cards")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.agents_scroll)
            .child(
                div()
                    .grid()
                    .grid_cols(cols)
                    .gap(px(GAP))
                    .p(px(PAD))
                    .children(cards),
            )
            .into_any_element()
    }

    fn render_agent_card(&self, wi: usize, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let w = &self.worktrees[wi];
        let info = self.worktree_info.get(&w.path).map(|(_, i)| i);
        let agents = self.agents_in(w);
        let state = agent_state(w, info, !agents.is_empty(), now);
        let current = self.current_worktree == Some(wi);
        let color = state.map_or(muted, |s| s.color(cx));
        let icon = match state {
            Some(AgentState::Missing) => IconName::FolderX,
            Some(AgentState::Conflict) => IconName::GitMergeConflict,
            _ if !agents.is_empty() => IconName::Bot,
            _ => IconName::FolderGit2,
        };
        let name = match w.branch_name() {
            Some(b) => b.to_string(),
            None => format!("{} (detached)", w.folder()),
        };
        let mut about = vec![w.folder()];
        if current {
            about.push("this tab".to_string());
        }
        if !agents.is_empty() {
            about.push(crate::agents::names(agents));
        }
        let pill_text = state.map_or("Loading…".to_string(), |s| s.pill_text(info));
        let tip = self.state_text(w, now).unwrap_or_default();
        let header = h_flex()
            .gap_2p5()
            .child(
                div()
                    .flex_none()
                    .size(px(32.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(color.opacity(0.14))
                    .child(Icon::new(icon).size(px(17.)).text_color(color)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(w.prunable, |d| d.line_through().text_color(muted))
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(muted)
                            .child(about.join(" · ")),
                    ),
            )
            .child(
                h_flex()
                    .id(("agent-state", wi))
                    .flex_none()
                    .h(px(22.))
                    .px_2()
                    .gap_1p5()
                    .rounded_full()
                    .bg(color.opacity(0.14))
                    .text_color(color)
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().size(px(6.)).rounded_full().bg(color))
                    .child(pill_text)
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    }),
            );
        let body = match (w.prunable, info) {
            (true, _) => v_flex()
                .flex_1()
                .gap_1()
                .text_size(px(12.))
                .text_color(muted)
                .child(div().child(w.path.display().to_string()))
                .child("The folder is gone. Forget the worktree to remove it from the list.")
                .into_any_element(),
            (false, None) => v_flex()
                .flex_1()
                .text_size(px(12.))
                .text_color(muted)
                .child("Loading…")
                .into_any_element(),
            (false, Some(i)) => v_flex()
                .flex_1()
                .gap_2()
                .child(self.commit_line(w, i, cx))
                .child(self.file_lines(i, now, cx))
                .child(self.stats_line(wi, i, cx))
                .into_any_element(),
        };
        v_flex()
            .id(("agent-card", wi))
            .min_w_0()
            .p_3()
            .gap_3()
            .rounded(px(10.))
            .border_1()
            .border_color(if state == Some(AgentState::Working) {
                t.colors.green.opacity(0.5)
            } else {
                t.colors.border
            })
            .bg(t.colors.popover)
            .child(header)
            .child(body)
            .child(self.card_buttons(wi, state, cx))
            .context_menu({
                let (wt, this) = (w.clone(), cx.entity());
                let can_remove = !w.main && !current && !w.locked;
                move |menu, _, _| {
                    worktrees::worktree_menu(menu, &wt, current, can_remove, this.clone())
                }
            })
            .into_any_element()
    }

    /// The last commit of the branch, or that it has none of its own.
    fn commit_line(&self, w: &Worktree, i: &WorktreeInfo, cx: &App) -> impl IntoElement {
        let t = cx.theme();
        let on_base = w.branch.is_some() && w.branch == self.base;
        let line = h_flex()
            .h(px(20.))
            .gap_1p5()
            .text_size(px(12.))
            .text_color(t.colors.muted_foreground)
            .child(Icon::new(IconName::GitCommitHorizontal).size(px(13.)));
        if i.ahead == 0 && !on_base && w.head.is_some() {
            return line.child(format!("No commits ahead of {} yet", self.base_name_of(w)));
        }
        line.child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.colors.foreground)
                .child(i.subject.clone()),
        )
        .when(i.committed > 0, |d| {
            d.child(div().flex_none().child(fmt_time(i.committed)))
        })
    }

    /// The changed files that changed last, with how long ago.
    fn file_lines(&self, i: &WorktreeInfo, now: i64, cx: &App) -> impl IntoElement {
        let muted = cx.theme().colors.muted_foreground;
        let area = v_flex().h(px(FILE_ROW_H * CARD_FILES as f32));
        if i.changed == 0 {
            return area
                .justify_center()
                .items_center()
                .text_size(px(12.))
                .text_color(muted)
                .child("No uncommitted changes");
        }
        // The last row says how many more there are.
        let shown = if i.changed > CARD_FILES {
            CARD_FILES - 1
        } else {
            CARD_FILES
        };
        area.children(
            i.recent
                .iter()
                .take(shown)
                .map(|f| file_line(f, now, cx).into_any_element()),
        )
        .when(i.changed > shown, |d| {
            let more = i.changed - shown;
            d.child(
                div()
                    .h(px(FILE_ROW_H))
                    .flex()
                    .items_center()
                    .pl(px(24.))
                    .text_size(px(11.5))
                    .text_color(muted)
                    .child(format!("and {more} more file{}", history::plural(more))),
            )
        })
    }

    /// Commits ahead and behind, changed files and lines, and the pull
    /// request.
    fn stats_line(&self, wi: usize, i: &WorktreeInfo, cx: &App) -> impl IntoElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let w = &self.worktrees[wi];
        let pr = w.branch_name().and_then(|b| self.pr_of_branch(b));
        let track = match (i.ahead, i.behind) {
            (0, 0) => None,
            (a, 0) => Some(format!("↑{a}")),
            (0, b) => Some(format!("↓{b}")),
            (a, b) => Some(format!("↑{a} ↓{b}")),
        };
        let base = self.base_name_of(w);
        let track_tip = format!(
            "{} commit{} ahead of {base}, {} behind",
            i.ahead,
            history::plural(i.ahead as usize),
            i.behind
        );
        h_flex()
            .h(px(20.))
            .gap_3()
            .text_size(px(11.5))
            .text_color(muted)
            .when_some(track, |d, track| {
                d.child(
                    div()
                        .id(("agent-track", wi))
                        .flex_none()
                        .child(track)
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(track_tip.clone())
                                .build(window, cx)
                        }),
                )
            })
            .when(i.changed > 0, |d| {
                d.child(
                    h_flex()
                        .flex_none()
                        .gap_0p5()
                        .text_color(t.colors.yellow)
                        .child(Icon::new(IconName::FilePen).size(px(11.)))
                        .child(i.changed.to_string()),
                )
            })
            .when_some(i.lines.filter(|&l| l != (0, 0)), |d, (adds, dels)| {
                d.child(
                    h_flex()
                        .flex_none()
                        .gap_1()
                        .child(div().text_color(t.colors.green).child(format!("+{adds}")))
                        .child(div().text_color(t.colors.red).child(format!("−{dels}"))),
                )
            })
            .child(div().flex_1())
            .when_some(pr, |d, p| {
                d.child(
                    h_flex()
                        .flex_none()
                        .gap_1()
                        .child(
                            Icon::new(IconName::GitPullRequest)
                                .size(px(12.))
                                .text_color(t.colors.green),
                        )
                        .child(format!("#{}", p.number))
                        .children(self.pr_badges(p.number, cx)),
                )
            })
    }

    fn card_buttons(
        &self,
        wi: usize,
        state: Option<AgentState>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme();
        let w = &self.worktrees[wi];
        let current = self.current_worktree == Some(wi);
        let can_remove = !w.main && !current && !w.locked;
        let this = cx.entity();
        let label = |icon: IconName, text: &'static str| {
            h_flex()
                .gap_1()
                .child(Icon::new(icon).size(px(12.)))
                .child(text)
        };
        let mut row = h_flex()
            .gap_1p5()
            .pt_3()
            .border_t_1()
            .border_color(t.colors.border);
        if w.prunable {
            let (app, wt) = (this.clone(), w.clone());
            row = row.child(
                button(("agent-forget", wi))
                    .xsmall()
                    .child(label(IconName::FolderX, "Forget…"))
                    .on_click(move |_, window, cx| {
                        let wt = wt.clone();
                        app.update(cx, |app, cx| {
                            app.remove_worktree_dialog(wt, false, window, cx)
                        });
                    }),
            );
        } else {
            let has_work = self
                .worktree_info
                .get(&w.path)
                .is_some_and(|(_, i)| i.changed > 0 || i.ahead > 0);
            let (app, wt) = (this.clone(), w.clone());
            let review = button(("agent-review", wi))
                .xsmall()
                .when(state == Some(AgentState::Ready), |b| b.primary())
                .off(w.head.is_none() || !has_work)
                .child(label(IconName::GitCompare, "Review"))
                .tooltip("Review the branch and its uncommitted changes as one diff")
                .on_click(move |_, _, cx| {
                    app.update(cx, |app, cx| app.start_worktree_review(&wt, cx));
                });
            let (app, path) = (this.clone(), w.path.clone());
            let open = button(("agent-open", wi))
                .xsmall()
                .child(if current {
                    label(IconName::FilePen, "Changes")
                } else {
                    label(IconName::AppWindow, "Open Tab")
                })
                .tooltip(if current {
                    "Show the uncommitted changes of this tab"
                } else {
                    "Open the worktree in a tab, on its Changes"
                })
                .on_click(move |_, _, cx| {
                    let path = path.clone();
                    app.update(cx, |app, cx| app.open_worktree(path, cx));
                });
            row = row.child(review).child(open);
            if let Some(branch) = w.branch.clone() {
                let app = this.clone();
                let tip = format!(
                    "Browse the commits, and pick them into {}",
                    self.head_name()
                );
                row = row.child(
                    button(("agent-commits", wi))
                        .xsmall()
                        .child(label(IconName::GitCommitVertical, "Commits"))
                        .tooltip(tip)
                        .on_click(move |_, _, cx| {
                            let r = branch.clone();
                            app.update(cx, |app, cx| app.show_target(LogTarget::Ref(r), cx));
                        }),
                );
            }
        }
        let wt = w.clone();
        row.child(div().flex_1()).child(
            button(("agent-more", wi))
                .ghost()
                .xsmall()
                .icon(IconName::Ellipsis)
                .tooltip("More")
                .dropdown_menu(move |menu, _, _| {
                    worktrees::worktree_menu(menu, &wt, current, can_remove, this.clone())
                }),
        )
    }
}

/// A changed file of a card: its change, name and folder, and its age.
fn file_line(f: &WorktreeFile, now: i64, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let muted = t.colors.muted_foreground;
    let (dir, name) = match f.path.rsplit_once('/') {
        Some((d, n)) => (d.to_string(), n.to_string()),
        None => (String::new(), f.path.clone()),
    };
    let fresh = f.modified.is_some_and(|m| now - m < WORKING_SECS);
    h_flex()
        .h(px(FILE_ROW_H))
        .gap_2()
        .text_size(px(12.5))
        .child(diff::change_badge(f.change, cx))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_1p5()
                .overflow_hidden()
                .child(div().flex_none().child(name))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(dir),
                ),
        )
        .when_some(f.modified, |d, m| {
            d.child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(if fresh { t.colors.green } else { muted })
                    .child(fmt_age(m)),
            )
        })
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::{AgentState, WORKING_SECS, agent_state};
    use crate::git::{Paused, Worktree, WorktreeInfo};

    fn worktree() -> Worktree {
        Worktree {
            path: "/r/wt".into(),
            head: Some("abc".into()),
            branch: Some("refs/heads/agent".into()),
            main: false,
            locked: false,
            prunable: false,
        }
    }

    fn info(changed: usize, ahead: u32, active: i64) -> WorktreeInfo {
        WorktreeInfo {
            changed,
            ahead,
            active,
            ..Default::default()
        }
    }

    #[test]
    fn the_state_follows_the_work_and_the_agent() {
        let w = worktree();
        let now = 10_000;
        let fresh = now - 10;
        let old = now - WORKING_SECS - 1;
        let state = |i: &WorktreeInfo, agent| agent_state(&w, Some(i), agent, now);
        assert_eq!(agent_state(&w, None, true, now), None, "not loaded yet");
        assert_eq!(state(&info(2, 0, fresh), false), Some(AgentState::Working));
        assert_eq!(state(&info(0, 0, fresh), true), Some(AgentState::Working));
        // A new commit on the base branch is no work of this worktree.
        assert_eq!(state(&info(0, 0, fresh), false), Some(AgentState::Clean));
        assert_eq!(state(&info(2, 1, old), true), Some(AgentState::Quiet));
        assert_eq!(state(&info(2, 1, old), false), Some(AgentState::Idle));
        assert_eq!(state(&info(0, 3, old), false), Some(AgentState::Ready));
        assert_eq!(state(&info(0, 0, old), false), Some(AgentState::Clean));
        let stopped = WorktreeInfo {
            paused: Some(Paused::Rebase),
            ..info(1, 1, fresh)
        };
        assert_eq!(state(&stopped, true), Some(AgentState::Conflict));
        let gone = Worktree {
            prunable: true,
            ..worktree()
        };
        assert_eq!(
            agent_state(&gone, None, false, now),
            Some(AgentState::Missing)
        );
    }
}
