//! Branch cleanup: delete the local branches that are merged into the base
//! branch or whose remote branch is gone, and their worktrees, in one
//! action. Agents leave a branch, and often a worktree, for each task.

use std::sync::Arc;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::spinner::Spinner;

use crate::github::{self, ClosedPr};

use super::review::short_ref;
use super::sidebar::FIXED;
use super::*;

/// The closed pull requests that one call asks GitHub for. A gone branch
/// whose pull request is older asks for its own.
const CLOSED_PRS: usize = 500;

/// Why a branch is in the cleanup list.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Stale {
    /// The base branch contains its tip.
    Merged,
    /// GitHub merged a pull request with its commits, often as one squashed
    /// commit. `into` is the branch it went into when that is not the base
    /// branch: a stacked pull request.
    PrMerged { number: u64, into: Option<String> },
    /// Its remote branch is gone, and a merge of it into the base branch
    /// changes nothing: the base branch has its changes in other commits.
    InBase,
    /// Its remote branch is gone. `closed` is a pull request with its
    /// commits that GitHub closed without a merge.
    Gone { closed: Option<u64> },
}

/// How the tip of a branch fits a pull request of that branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    /// The tip is the head of the pull request.
    Head,
    /// The tip is one of its commits: the pull request got more after it.
    Inside,
    /// The tip has this many commits after the head of the pull request.
    After(u32),
    /// Neither has the other.
    Apart,
}

/// What became of a branch, from its closed pull requests (newest first).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Fate {
    /// Merged, with `after` commits of the branch that came after it.
    Merged { pr: ClosedPr, after: u32 },
    /// Closed without a merge, with the branch's commits.
    Closed(u64),
}

/// The merged pull request that has the branch's tip, else the newest one
/// that the tip came after, else a closed one with the tip. `fit` can ask
/// git or GitHub, so it runs only as far as needed.
fn pr_fate(prs: &[ClosedPr], mut fit: impl FnMut(&ClosedPr) -> Fit) -> Option<Fate> {
    let mut closed = None;
    for pr in prs {
        match (fit(pr), pr.merged) {
            (Fit::Apart, _) => {}
            (Fit::Head | Fit::Inside, true) => return Some(Fate::Merged { pr: pr.clone(), after: 0 }),
            (Fit::After(after), true) => return Some(Fate::Merged { pr: pr.clone(), after }),
            (_, false) => {
                closed.get_or_insert(pr.number);
            }
        }
    }
    closed.map(Fate::Closed)
}

/// How the tip `sha` fits `pr`. When the repository does not have the
/// head of the pull request, it got commits after the last fetch: GitHub
/// lists them.
fn fit(repo: &Repo, pr: &ClosedPr, sha: &str) -> Fit {
    if pr.head_oid == sha {
        return Fit::Head;
    }
    if !git::has_commit(repo, &pr.head_oid) {
        let inside = github::commit_oids(repo, pr.number).is_ok_and(|oids| oids.iter().any(|o| o == sha));
        return if inside { Fit::Inside } else { Fit::Apart };
    }
    if git::is_ancestor(repo, sha, &pr.head_oid) {
        Fit::Inside
    } else if git::is_ancestor(repo, &pr.head_oid, sha) {
        git::commits_not_in(repo, sha, &pr.head_oid).map_or(Fit::Apart, Fit::After)
    } else {
        Fit::Apart
    }
}

/// `dev` for `refs/heads/dev` and for `refs/remotes/origin/dev`.
fn branch_name(base: &str) -> &str {
    if let Some(b) = base.strip_prefix("refs/heads/") {
        return b;
    }
    base.strip_prefix("refs/remotes/")
        .and_then(|r| r.split_once('/'))
        .map_or(base, |(_, b)| b)
}

/// A branch in the cleanup dialog.
#[derive(Clone, Debug)]
pub(super) struct CleanupRow {
    /// `feature/x`.
    name: String,
    /// The commit it pointed at when the dialog opened. A branch that moved
    /// since then stays.
    sha: String,
    stale: Stale,
    /// Its commits that the base branch does not have: they go with it.
    /// None when there is no base branch to compare with.
    lost: Option<u32>,
    /// The worktree that has it checked out. It goes first.
    worktree: Option<git::Worktree>,
    /// Changed files in that worktree: they go with it.
    changed: usize,
    /// Git still checks what it loses (see `check_row`). Until then it
    /// cannot be selected.
    checking: bool,
    selected: bool,
}

impl CleanupRow {
    /// A row from what the tab knows. A merged branch without a worktree
    /// loses nothing. Git must check the others.
    fn new(branch: &Branch, merged: bool, worktree: Option<&git::Worktree>) -> Self {
        let checking = !merged || worktree.is_some_and(|w| !w.prunable);
        CleanupRow {
            name: branch.name.clone(),
            sha: branch.sha.clone(),
            stale: if merged { Stale::Merged } else { Stale::Gone { closed: None } },
            lost: merged.then_some(0),
            worktree: worktree.cloned(),
            changed: 0,
            checking,
            selected: !checking,
        }
    }

    /// Deleting it loses nothing: such a row starts selected.
    fn safe(&self) -> bool {
        self.lost == Some(0) && self.changed == 0
    }
}

/// Check what a cleanup of `row` loses: the commits that `base` does not
/// have, and the changes in its worktree. For a background task: on a large
/// repository each step can take seconds.
///
/// A gone branch was often squash-merged on GitHub: its own commits never
/// reach the base branch. `prs` are the newest closed pull requests, None
/// without GitHub. A pull request that merged the branch's commits says
/// that nothing is lost, and which commits came after it. Without one, a
/// merge in memory finds a squash merge, but only while the base branch did
/// not change the same lines since.
fn check_row(
    repo: &Repo,
    mut row: CleanupRow,
    base: Option<&str>,
    prs: Option<&[ClosedPr]>,
) -> CleanupRow {
    if row.stale != Stale::Merged {
        let fate = prs.and_then(|prs| {
            let mut own: Vec<ClosedPr> = prs.iter().filter(|p| p.head == row.name).cloned().collect();
            if own.is_empty() {
                own = github::closed(repo, Some(&row.name), 20).unwrap_or_default();
            }
            pr_fate(&own, |pr| fit(repo, pr, &row.sha))
        });
        match fate {
            Some(Fate::Merged { pr, after }) => {
                let into = base.map(branch_name);
                row.stale = Stale::PrMerged {
                    number: pr.number,
                    into: (into != Some(pr.base.as_str())).then_some(pr.base),
                };
                row.lost = Some(after);
            }
            _ if base.is_some_and(|base| git::changes_in(repo, &row.sha, base)) => {
                row.stale = Stale::InBase;
                row.lost = Some(0);
            }
            fate => {
                let closed = match fate {
                    Some(Fate::Closed(n)) => Some(n),
                    _ => None,
                };
                row.stale = Stale::Gone { closed };
                row.lost = base.and_then(|base| git::commits_not_in(repo, &row.sha, base).ok());
            }
        }
    }
    row.changed = row
        .worktree
        .as_ref()
        .filter(|w| !w.prunable)
        .and_then(|w| git::status(&w.repo()).ok())
        .map_or(0, |s| s.len());
    row.checking = false;
    row.selected = row.safe();
    row
}

fn branches_word(n: usize) -> &'static str {
    if n == 1 { "branch" } else { "branches" }
}

impl GitApp {
    /// The short name of the base branch, or a description without one.
    fn base_label(&self) -> String {
        self.base
            .as_deref()
            .map_or_else(|| "the base branch".to_string(), short_ref)
    }

    /// The local branches that a cleanup lists: merged into the base branch
    /// or gone from their remote, each with the worktree that has it checked
    /// out. Never the base branch or a fixed one (`develop` is often even
    /// with `main`), and never a branch that this tab's worktree, the main
    /// worktree or a locked worktree has checked out: Gibbon does not remove
    /// those worktrees.
    pub(super) fn stale_branches(&self) -> Vec<(&Branch, Option<&git::Worktree>)> {
        let own = self
            .current_worktree
            .and_then(|i| self.worktrees.get(i))
            .map(|w| &w.path);
        self.branches
            .iter()
            .filter(|b| b.kind == RefKind::Local && !b.is_head)
            .filter(|b| b.gone || self.merged.contains(&b.refname))
            .filter(|b| Some(&b.refname) != self.base.as_ref() && !FIXED.contains(&b.name.as_str()))
            .filter_map(|b| match self.checkout_of(&b.refname) {
                None => Some((b, None)),
                Some(w) if w.main || w.locked || Some(&w.path) == own => None,
                Some(w) => Some((b, Some(w))),
            })
            .collect()
    }

    /// List the stale branches to choose from. The dialog opens at once, and
    /// each row that git must check fills in when its check ends.
    pub(super) fn cleanup_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let rows: Vec<CleanupRow> = self
            .stale_branches()
            .into_iter()
            .map(|(b, w)| CleanupRow::new(b, self.merged.contains(&b.refname), w))
            .collect();
        if rows.is_empty() {
            let msg = format!(
                "No branch to clean up: none is merged into {}, and none lost its remote branch.",
                self.base_label()
            );
            self.toast(None, msg, cx);
            return;
        }
        self.cleanup_epoch += 1;
        let epoch = self.cleanup_epoch;
        let (merged, gone): (Vec<_>, Vec<_>) = rows
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, r)| r.checking)
            .partition(|(_, r)| r.stale == Stale::Merged);
        self.check_rows(merged, None, epoch, cx);
        // Gone branches first ask GitHub which pull requests they went into.
        if !gone.is_empty() {
            cx.spawn(async move |this, cx| {
                let prs = cx
                    .background_executor()
                    .spawn(async move { github::closed(&repo, None, CLOSED_PRS).ok() })
                    .await;
                let _ = this.update(cx, |this, cx| this.check_rows(gone, prs.map(Arc::new), epoch, cx));
            })
            .detach();
        }
        self.cleanup = rows;
        self.open_cleanup(window, cx);
    }

    /// Check each of `rows` (with its place in the dialog) in its own task,
    /// so a slow one does not hold up the others.
    fn check_rows(
        &mut self,
        rows: Vec<(usize, CleanupRow)>,
        prs: Option<Arc<Vec<ClosedPr>>>,
        epoch: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        // A dialog that opened again has rows of its own.
        if epoch != self.cleanup_epoch {
            return;
        }
        for (i, row) in rows {
            let (repo, base, prs) = (repo.clone(), self.base.clone(), prs.clone());
            cx.spawn(async move |this, cx| {
                let row = cx
                    .background_executor()
                    .spawn(async move { check_row(&repo, row, base.as_deref(), prs.as_deref().map(Vec::as_slice)) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.cleanup_epoch == epoch
                        && let Some(slot) = this.cleanup.get_mut(i)
                    {
                        *slot = row;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    fn set_cleanup_selected(&mut self, row: Option<usize>, on: bool, cx: &mut Context<Self>) {
        for (i, r) in self.cleanup.iter_mut().enumerate() {
            if !r.checking && row.is_none_or(|row| row == i) {
                r.selected = on;
            }
        }
        cx.notify();
    }

    /// The dialog reads the rows on each frame, so a click shows at once.
    fn open_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        let base = self.base_label();
        window.open_dialog(cx, move |dialog, _, cx| {
            let t = cx.theme();
            let rows = this.read(cx).cleanup.clone();
            let chosen: Vec<&CleanupRow> = rows.iter().filter(|r| r.selected).collect();
            let n = chosen.len();
            let all = n == rows.len();
            let checking = rows.iter().filter(|r| r.checking).count();

            let toggle_all = this.clone();
            let mut list = v_flex()
                .id("cleanup-list")
                .max_h(px(320.))
                .overflow_y_scroll()
                .rounded(t.radius)
                .border_1()
                .border_color(t.colors.border)
                .child(
                    div().px_3().py_2().bg(t.colors.muted).child(
                        Checkbox::new("cleanup-all")
                            .small()
                            .checked(all)
                            .disabled(checking > 0)
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(t.colors.muted_foreground)
                                    .child(format!("All {} {}", rows.len(), branches_word(rows.len()))),
                            )
                            .on_click(move |on, _, cx| {
                                toggle_all.update(cx, |app, cx| app.set_cleanup_selected(None, *on, cx))
                            }),
                    ),
                );
            for (i, r) in rows.iter().enumerate() {
                let toggle = this.clone();
                list = list.child(
                    div().px_3().py_2().border_t_1().border_color(t.colors.border).child(
                        Checkbox::new(("cleanup", i))
                            .small()
                            .accessibility_label(r.name.clone())
                            .checked(r.selected)
                            .disabled(r.checking)
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(r.name.clone()),
                            )
                            .child(row_facts(r, &base, cx))
                            .on_click(move |on, _, cx| {
                                toggle.update(cx, |app, cx| app.set_cleanup_selected(Some(i), *on, cx))
                            }),
                    ),
                );
            }

            let run = this.clone();
            dialog
                .title("Clean up branches")
                .w(px(500.))
                .child(
                    v_flex()
                        .gap_3()
                        .text_size(px(13.))
                        .child(format!(
                            "These branches are merged into {base}, or their remote branch is gone. \
                             Their remote branches stay."
                        ))
                        .child(list)
                        .child(summary(&chosen, checking, &base, cx)),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("dialog-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("dialog-ok")
                                .with_variant(ButtonVariant::Danger)
                                .label(match n {
                                    0 => "Delete".to_string(),
                                    n => format!("Delete {n} {}", branches_word(n)),
                                })
                                .disabled(n == 0 || checking > 0)
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    run.update(cx, |app, cx| app.run_cleanup(cx));
                                }),
                        ),
                )
        });
    }

    /// Delete the chosen branches and their worktrees, one after the other.
    /// A branch that fails stays, and the others go on.
    fn run_cleanup(&mut self, cx: &mut Context<Self>) {
        let rows: Vec<CleanupRow> = std::mem::take(&mut self.cleanup)
            .into_iter()
            .filter(|r| r.selected && !r.checking)
            .collect();
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if rows.is_empty() || self.busy.is_some() {
            return;
        }
        let n = rows.len();
        self.set_busy(Some(format!("Deleting {n} {}…", branches_word(n)).into()), cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    // Git keeps the branch of a worktree whose folder is
                    // gone until it forgets that worktree.
                    if rows.iter().any(|r| r.worktree.as_ref().is_some_and(|w| w.prunable)) {
                        let _ = git::prune_worktrees(&repo);
                    }
                    rows.into_iter()
                        .map(|r| {
                            // `force` only when the user saw the changes.
                            let wt = r
                                .worktree
                                .as_ref()
                                .filter(|w| !w.prunable)
                                .map(|w| (w.path.as_path(), r.changed > 0));
                            let result = git::delete_stale_branch(&repo, &r.name, &r.sha, wt);
                            (r, result)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.set_busy(None, cx);
                let (mut branches, mut worktrees, mut failed) = (0, 0, vec![]);
                for (r, result) in results {
                    match result {
                        Ok(()) => {
                            branches += 1;
                            if let Some(w) = r.worktree {
                                worktrees += 1;
                                cx.emit(AppEvent::Forget(w.path));
                            }
                        }
                        Err(e) => failed.push(format!("{}: {e}", r.name)),
                    }
                }
                let mut msg = match (branches, worktrees) {
                    (0, _) => String::new(),
                    (b, 0) => format!("Deleted {b} {}.", branches_word(b)),
                    (b, w) => format!(
                        "Deleted {b} {} and {w} worktree{}.",
                        branches_word(b),
                        history::plural(w)
                    ),
                };
                if !failed.is_empty() {
                    let kept = format!("Kept {}", failed.join("; "));
                    msg = if msg.is_empty() { kept } else { format!("{msg} {kept}") };
                }
                this.toast(Some(failed.is_empty()), msg, cx);
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

/// The second line of a row: why it is listed, its worktree, and what goes
/// with it in yellow, or a spinner while git checks that.
fn row_facts(r: &CleanupRow, base: &str, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let warn = t.colors.yellow;
    let mut facts: Vec<(String, bool)> = vec![];
    match &r.stale {
        Stale::Merged => facts.push((format!("Merged into {base}"), false)),
        Stale::PrMerged { number, into: None } => facts.push((format!("Merged as #{number}"), false)),
        Stale::PrMerged { number, into: Some(into) } => {
            facts.push((format!("Merged as #{number} into {into}"), false))
        }
        Stale::InBase => facts.push((format!("Remote branch gone, its changes are in {base}"), false)),
        Stale::Gone { closed } => {
            facts.push(("Remote branch gone".to_string(), false));
            if let Some(n) = closed {
                facts.push((format!("#{n} closed without a merge"), false));
            }
        }
    }
    let plural = |n: u32| history::plural(n as usize);
    match (r.lost, &r.stale) {
        _ if r.checking => {}
        (Some(0), _) => {}
        (Some(n), Stale::PrMerged { number, .. }) => {
            facts.push((format!("{n} commit{} after #{number}", plural(n)), true))
        }
        (Some(n), _) => facts.push((format!("{n} commit{} not in {base}", plural(n)), true)),
        (None, _) => facts.push(("Its commits may be only here".to_string(), true)),
    }
    if let Some(w) = &r.worktree {
        facts.push(if w.prunable {
            (format!("Worktree {} (folder gone)", w.folder()), false)
        } else {
            (format!("Worktree {}", w.folder()), false)
        });
    }
    if r.changed > 0 {
        facts.push((format!("{} changed file{}", r.changed, history::plural(r.changed)), true));
    }
    let muted = t.colors.muted_foreground;
    let mut line = h_flex().flex_wrap().text_size(px(12.)).text_color(muted);
    for (i, (text, loss)) in facts.into_iter().enumerate() {
        if i > 0 {
            line = line.child(div().px_1().child("·"));
        }
        line = line.child(div().when(loss, |d| d.text_color(warn)).child(text));
    }
    line.when(r.checking, |d| {
        d.child(div().px_1().child("·")).child(
            h_flex()
                .gap_1()
                .child(Spinner::new().xsmall().color(muted))
                .child("Checking…"),
        )
    })
}

/// What the cleanup of the `chosen` rows deletes, red when it loses work.
/// While git checks rows, it says so.
fn summary(chosen: &[&CleanupRow], checking: usize, base: &str, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let muted = t.colors.muted_foreground;
    if checking > 0 {
        return h_flex()
            .gap_1p5()
            .text_size(px(12.))
            .text_color(muted)
            .child(Spinner::new().xsmall().color(muted))
            .child(format!("Checking {checking} {}…", branches_word(checking)));
    }
    let (text, loses) = summary_text(chosen, base);
    div()
        .text_size(px(12.))
        .text_color(if loses { t.colors.red } else { muted })
        .child(text)
}

/// What the cleanup of the `chosen` rows deletes, and whether it loses
/// commits or changes.
fn summary_text(chosen: &[&CleanupRow], base: &str) -> (String, bool) {
    let n = chosen.len();
    let worktrees = chosen.iter().filter(|r| r.worktree.is_some()).count();
    let commits: u32 = chosen.iter().filter_map(|r| r.lost).sum();
    let unknown = chosen.iter().any(|r| r.lost.is_none());
    let files: usize = chosen.iter().map(|r| r.changed).sum();
    let mut text = match (n, worktrees) {
        (0, _) => "Choose the branches to delete.".to_string(),
        (n, 0) => format!("This deletes {n} local {}.", branches_word(n)),
        (n, w) => format!(
            "This deletes {n} local {} and removes {w} worktree{}.",
            branches_word(n),
            history::plural(w)
        ),
    };
    let mut lose = vec![];
    if commits > 0 {
        lose.push(format!(
            "{commits} commit{} that {base} does not have",
            history::plural(commits as usize)
        ));
    }
    if unknown {
        lose.push("the commits of branches that Gibbon cannot compare".to_string());
    }
    if files > 0 {
        lose.push(format!("{files} changed file{}", history::plural(files)));
    }
    let loses = !lose.is_empty();
    if loses {
        text.push_str(&format!(" You lose {}. You cannot undo it.", lose.join(" and ")));
    }
    (text, loses)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::{CleanupRow, Fate, Fit, Stale, branch_name, pr_fate, summary_text};
    use crate::git::{Branch, RefKind, Worktree};
    use crate::github::ClosedPr;

    fn pr(number: u64, merged: bool) -> ClosedPr {
        ClosedPr {
            number,
            head: "agent/x".into(),
            head_oid: format!("oid{number}"),
            base: "dev".into(),
            merged,
        }
    }

    #[test]
    fn the_pull_request_that_merged_the_tip_wins() {
        // Newest first: #3 was closed, #2 merged the tip, #1 is older.
        let prs = [pr(3, false), pr(2, true), pr(1, true)];
        let mut asked = vec![];
        let fate = pr_fate(&prs, |p| {
            asked.push(p.number);
            match p.number {
                3 => Fit::Head,
                2 => Fit::Inside,
                _ => Fit::Head,
            }
        });
        assert_eq!(fate, Some(Fate::Merged { pr: prs[1].clone(), after: 0 }));
        assert_eq!(asked, [3, 2], "it stops at the first merged one");

        let fate = pr_fate(&prs, |p| if p.number == 1 { Fit::After(2) } else { Fit::Apart });
        assert_eq!(fate, Some(Fate::Merged { pr: prs[2].clone(), after: 2 }));
    }

    #[test]
    fn a_closed_pull_request_or_none() {
        let prs = [pr(5, true), pr(4, false)];
        let fate = pr_fate(&prs, |p| if p.number == 4 { Fit::Head } else { Fit::Apart });
        assert_eq!(fate, Some(Fate::Closed(4)), "the merged one has other commits");
        assert_eq!(pr_fate(&prs, |_| Fit::Apart), None);
        assert_eq!(pr_fate(&[], |_| Fit::Head), None);
    }

    #[test]
    fn base_branch_names() {
        assert_eq!(branch_name("refs/heads/dev"), "dev");
        assert_eq!(branch_name("refs/remotes/origin/release/2.0"), "release/2.0");
        assert_eq!(branch_name("dev"), "dev");
    }

    fn worktree(prunable: bool) -> Worktree {
        Worktree {
            path: "/r/agent".into(),
            head: None,
            branch: None,
            main: false,
            locked: false,
            prunable,
        }
    }

    #[test]
    fn rows_that_git_must_check_wait_unselected() {
        let b = Branch {
            refname: "refs/heads/x".into(),
            name: "x".into(),
            kind: RefKind::Local,
            sha: "abc".into(),
            ahead: 0,
            behind: 0,
            gone: true,
            is_head: false,
        };
        let merged = CleanupRow::new(&b, true, None);
        assert!(!merged.checking && merged.selected, "nothing to check, nothing lost");
        let gone = CleanupRow::new(&b, false, None);
        assert!(gone.checking && !gone.selected);
        assert_eq!(gone.lost, None);
        let with_files = CleanupRow::new(&b, true, Some(&worktree(false)));
        assert!(with_files.checking && !with_files.selected, "its worktree may have changes");
        let folder_gone = CleanupRow::new(&b, true, Some(&worktree(true)));
        assert!(!folder_gone.checking && folder_gone.selected);
    }

    fn row(lost: Option<u32>, changed: usize) -> CleanupRow {
        CleanupRow {
            name: "x".into(),
            sha: "abc".into(),
            stale: Stale::Gone { closed: None },
            lost,
            worktree: None,
            changed,
            checking: false,
            selected: false,
        }
    }

    #[test]
    fn only_rows_that_lose_nothing_start_selected() {
        assert!(row(Some(0), 0).safe());
        assert!(!row(Some(2), 0).safe(), "commits that the base does not have");
        assert!(!row(Some(0), 3).safe(), "changes in its worktree");
        assert!(!row(None, 0).safe(), "no base branch to compare with");
    }

    #[test]
    fn the_summary_names_what_is_lost() {
        let safe = row(Some(0), 0);
        assert_eq!(
            summary_text(&[&safe, &safe], "main"),
            ("This deletes 2 local branches.".to_string(), false)
        );
        let mut wt = row(Some(2), 1);
        wt.worktree = Some(worktree(false));
        assert_eq!(
            summary_text(&[&safe, &wt], "main"),
            (
                "This deletes 2 local branches and removes 1 worktree. You lose 2 commits \
                 that main does not have and 1 changed file. You cannot undo it."
                    .to_string(),
                true
            )
        );
        assert_eq!(summary_text(&[], "main").0, "Choose the branches to delete.");
    }
}
