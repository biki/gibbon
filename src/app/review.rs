//! Review: what a branch changed since it left its base, as one diff
//! (`git diff base...branch`), with a Viewed check per file as on GitHub.
//! A branch that a worktree has checked out can include that worktree's
//! uncommitted changes: the work of an agent that has not committed yet.
//!
//! The base is the branch that the review should show the work against:
//! the one you chose for the branch before, the base of its pull request,
//! the branch it was made from (an agent's branch is often made from
//! another agent's), or else the base branch of the repository.

use std::ops::Range;

use super::*;

pub(super) struct ReviewUi {
    /// What is under review, and what it is compared with: full ref names
    /// or commits.
    pub(super) target: String,
    pub(super) base: String,
    /// With uncommitted changes: the worktree whose files the review shows.
    pub(super) worktree: Option<PathBuf>,
    pub(super) diff: Option<Rc<CommitDetail>>,
    /// The viewed key of each file of `diff` (see `viewed_key`).
    keys: Vec<String>,
    /// Commits of `target` that `base` does not have.
    commits: usize,
    /// New files that the diff leaves out.
    more_new_files: usize,
    /// The shown file, an index into the files of `diff`.
    pub(super) file: usize,
    error: Option<String>,
}

impl ReviewUi {
    pub(super) fn new(target: String, base: String, worktree: Option<PathBuf>) -> Self {
        ReviewUi {
            target,
            base,
            worktree,
            diff: None,
            keys: vec![],
            commits: 0,
            more_new_files: 0,
            file: 0,
            error: None,
        }
    }
}

/// A file's path and a hash of its diff: a file that changes after it was
/// marked as viewed is not viewed any more. FNV-1a, so the hash stays the
/// same from one run to the next.
fn viewed_key(file: &FileDiff) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for line in &file.lines {
        eat(&[line.kind as u8]);
        eat(line.text.as_bytes());
        eat(b"\n");
    }
    format!("{}\0{hash:016x}", file.path)
}

/// Why a review compares with its base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BaseWhy {
    /// You chose it for this branch before.
    Chosen,
    /// The base of the branch's pull request.
    PullRequest(u64),
    /// The branch was made from it (see `git::spawn_bases`).
    MadeFrom,
    BaseBranch,
    /// The checked-out branch, for a review of the base branch.
    Head,
}

impl GitApp {
    /// The worktree that has `target` checked out, if any.
    pub(super) fn checkout_of(&self, target: &str) -> Option<&git::Worktree> {
        self.worktrees.iter().find(|w| match &w.branch {
            Some(b) => b == target,
            None => w.head.as_deref() == Some(target),
        })
    }

    /// Changed files in the worktree `w`: this tab's status for its own.
    fn changed_in(&self, w: &git::Worktree) -> usize {
        let own = self
            .current_worktree
            .and_then(|i| self.worktrees.get(i))
            .is_some_and(|c| c.path == w.path);
        if own {
            self.status.len()
        } else {
            self.worktree_info
                .get(&w.path)
                .map_or(0, |(_, i)| i.changed)
        }
    }

    /// The worktree `w` has uncommitted changes, or its row has not loaded
    /// yet: a review that opens at start-up then shows them too. A worktree
    /// without changes shows the same diff either way.
    fn may_have_changes(&self, w: &git::Worktree) -> bool {
        let own = self
            .current_worktree
            .and_then(|i| self.worktrees.get(i))
            .is_some_and(|c| c.path == w.path);
        own && !self.status.is_empty()
            || !own
                && self
                    .worktree_info
                    .get(&w.path)
                    .is_none_or(|(_, i)| i.changed > 0)
    }

    /// The base to review `target` against, and why (see the module
    /// comment). A made-from branch that the base branch contains gives way
    /// to the base branch: the diff is the same, and the base branch stays.
    pub(super) fn review_base_for(&self, target: &str) -> Option<(String, BaseWhy)> {
        let usable = |r: &String| r != target && self.branches.iter().any(|b| &b.refname == r);
        if let Some(b) = self.review_bases.get(target).filter(|b| usable(b)) {
            return Some((b.clone(), BaseWhy::Chosen));
        }
        let pr = target
            .strip_prefix("refs/heads/")
            .and_then(|name| self.pr_of_branch(name));
        if let Some(p) = pr
            && let Some(b) = self.pr_base_ref(p).filter(usable)
        {
            return Some((b, BaseWhy::PullRequest(p.number)));
        }
        if let Some(b) = self
            .spawned
            .get(target)
            .filter(|b| usable(b) && !self.merged.contains(*b))
        {
            return Some((b.clone(), BaseWhy::MadeFrom));
        }
        let head = self.head.branch.as_ref().map(|b| format!("refs/heads/{b}"));
        self.base
            .clone()
            .filter(|b| b != target)
            .map(|b| (b, BaseWhy::BaseBranch))
            .or(head.filter(|h| h != target).map(|h| (h, BaseWhy::Head)))
    }

    /// The base that the counts of the worktree `w` use: its review's base,
    /// or the base branch for a worktree on the base branch.
    pub(super) fn worktree_base(&self, w: &git::Worktree) -> Option<String> {
        match &w.branch {
            Some(b) if Some(b) != self.base.as_ref() => {
                self.review_base_for(b).map(|(base, _)| base)
            }
            _ => self.base.clone(),
        }
    }

    /// The short name of the base that the counts of `w` use, for texts.
    pub(super) fn base_name_of(&self, w: &git::Worktree) -> String {
        self.worktree_base(w)
            .map(|b| short_ref(&b))
            .unwrap_or_else(|| "the base branch".to_string())
    }

    /// The local branch of a pull request's base, else its remote branch.
    fn pr_base_ref(&self, pr: &crate::github::PullRequest) -> Option<String> {
        let local = format!("refs/heads/{}", pr.base);
        self.branches
            .iter()
            .find(|b| b.refname == local)
            .or_else(|| {
                self.branches.iter().find(|b| {
                    b.kind == RefKind::Remote
                        && b.name.split_once('/').map(|(_, n)| n) == Some(pr.base.as_str())
                })
            })
            .map(|b| b.refname.clone())
    }

    /// Why the shown review compares with its base, for the tooltip of the
    /// base button.
    fn base_tip(&self, ui: &ReviewUi) -> String {
        let target = short_ref(&ui.target);
        let base = short_ref(&ui.base);
        let why = self
            .review_base_for(&ui.target)
            .filter(|(b, _)| *b == ui.base)
            .map(|(_, why)| why);
        match why {
            Some(BaseWhy::Chosen) => format!("You chose {base} for {target}."),
            Some(BaseWhy::PullRequest(n)) => format!("The base of pull request #{n}."),
            Some(BaseWhy::MadeFrom) => format!("{target} was made from {base}."),
            Some(BaseWhy::BaseBranch) => "The base branch of the repository.".to_string(),
            Some(BaseWhy::Head) | None => format!("Compare {target} with another branch."),
        }
    }

    /// Review `target` against the base branch. When a worktree has
    /// `target` checked out and has changes, the review includes them.
    pub(super) fn start_review(&mut self, target: String, cx: &mut Context<Self>) {
        let Some((base, _)) = self.review_base_for(&target) else {
            let msg = format!(
                "{} is the base branch. Review another branch against it.",
                short_ref(&target)
            );
            self.toast(None, msg, cx);
            return;
        };
        let worktree = self
            .checkout_of(&target)
            .filter(|w| self.may_have_changes(w))
            .map(|w| w.path.clone());
        self.open_review(target, base, worktree, cx);
    }

    /// Review the work in a worktree: its branch, and its uncommitted
    /// changes when it has some.
    pub(super) fn start_worktree_review(&mut self, wt: &git::Worktree, cx: &mut Context<Self>) {
        let Some(target) = wt.branch.clone().or_else(|| wt.head.clone()) else {
            return;
        };
        let Some((base, _)) = self.review_base_for(&target) else {
            return self.start_review(target, cx);
        };
        let worktree = self.may_have_changes(wt).then(|| wt.path.clone());
        self.open_review(target, base, worktree, cx);
    }

    /// Review a pull request against its base branch.
    pub(super) fn start_pr_review(
        &mut self,
        pr: &crate::github::PullRequest,
        cx: &mut Context<Self>,
    ) {
        match self.pr_base_ref(pr) {
            Some(base) => self.open_review(pr.refname(), base, None, cx),
            None => self.start_review(pr.refname(), cx),
        }
    }

    fn open_review(
        &mut self,
        target: String,
        base: String,
        worktree: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let same = self
            .review
            .as_ref()
            .is_some_and(|r| r.target == target && r.base == base && r.worktree == worktree);
        if !same {
            self.review = Some(ReviewUi::new(target, base, worktree));
        }
        self.view = View::Review;
        self.load_review(cx);
        cx.notify();
    }

    /// Load the review's diff again. The shown file stays when it is still
    /// part of the diff.
    pub(super) fn load_review(&mut self, cx: &mut Context<Self>) {
        let (Some(repo), Some(ui)) = (self.repo.clone(), self.review.as_ref()) else {
            return;
        };
        let (target, base, worktree) = (ui.target.clone(), ui.base.clone(), ui.worktree.clone());
        let keep = ui
            .diff
            .as_ref()
            .and_then(|d| d.files.get(ui.file))
            .map(|f| f.path.clone());
        self.review_epoch += 1;
        let epoch = self.review_epoch;
        self.review_styles.task = None;
        let shown = self.shown_file_rule("review", cx);
        let theme = cx.theme().highlight_theme.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let d = git::branch_diff(&repo, &base, &target, worktree.as_deref())?;
                    let detail = CommitDetail {
                        // The key of its syntax colors: one per load, since
                        // uncommitted files change without a new commit.
                        sha: format!("review-{epoch}"),
                        parents: vec![d.merge_base],
                        author: String::new(),
                        email: String::new(),
                        time: 0,
                        committer: String::new(),
                        commit_time: 0,
                        message: String::new(),
                        files: d.files,
                    };
                    let keys = detail.files.iter().map(viewed_key).collect();
                    let ix = keep
                        .and_then(|p| detail.files.iter().position(|f| f.path == p))
                        .or_else(|| shown(&detail.files))
                        .unwrap_or(0);
                    let styles = colors_with_text(&detail, ix, &theme);
                    anyhow::Ok((detail, keys, d.commits, d.more_new_files, ix, styles))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.review_epoch {
                    return;
                }
                let Some(ui) = this.review.as_mut() else {
                    return;
                };
                match result {
                    Ok((detail, keys, commits, more, ix, styles)) => {
                        ui.file = ix;
                        ui.keys = keys;
                        ui.commits = commits;
                        ui.more_new_files = more;
                        ui.error = None;
                        this.review_styles =
                            FileStyles::new(detail.sha.clone(), styles.map(|s| (ix, s)));
                        ui.diff = Some(Rc::new(detail));
                        this.highlight_shown(cx);
                    }
                    Err(e) => {
                        ui.error = Some(e.to_string());
                        ui.diff = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Compare the review with `base`, and keep that choice for its
    /// branch: the next review of it starts there, and its worktree counts
    /// against it.
    fn set_review_base(&mut self, base: String, cx: &mut Context<Self>) {
        if let Some(ui) = self.review.as_mut()
            && ui.base != base
        {
            self.review_bases.insert(ui.target.clone(), base.clone());
            ui.base = base;
            ui.diff = None;
            self.load_review(cx);
            self.load_worktree_info(false, cx);
            cx.notify();
        }
    }

    fn set_review_uncommitted(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(target) = self.review.as_ref().map(|r| r.target.clone()) else {
            return;
        };
        let worktree = self
            .checkout_of(&target)
            .map(|w| w.path.clone())
            .filter(|_| on);
        if let Some(ui) = self.review.as_mut() {
            ui.worktree = worktree;
        }
        self.load_review(cx);
        cx.notify();
    }

    fn is_viewed(&self, ix: usize) -> bool {
        let Some(ui) = &self.review else {
            return false;
        };
        let (Some(set), Some(key)) = (self.viewed.get(&ui.target), ui.keys.get(ix)) else {
            return false;
        };
        set.contains(key)
    }

    /// Mark file `ix` as viewed, or not. A shown file that gets its check
    /// makes way for the next file without one.
    fn toggle_viewed(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(ui) = &self.review else {
            return;
        };
        let Some(key) = ui.keys.get(ix).cloned() else {
            return;
        };
        let shown = ui.file == ix;
        let set = self.viewed.entry(ui.target.clone()).or_default();
        let now_viewed = set.insert(key.clone());
        if !now_viewed {
            set.remove(&key);
        }
        if now_viewed
            && shown
            && let Some(next) = self.next_unviewed(ix, cx)
        {
            self.show_review_file(next, cx);
        }
        cx.notify();
    }

    /// The first file after `ix`, in the order of the list, that has no
    /// Viewed check. It starts again at the top.
    fn next_unviewed(&self, ix: usize, cx: &App) -> Option<usize> {
        let d = self.review.as_ref()?.diff.as_ref()?;
        let order: Vec<usize> = self
            .file_rows("review", &files::paths(&d.files), cx)
            .into_iter()
            .filter_map(|r| match r {
                FileRow::File { ix, .. } => Some(ix),
                FileRow::Dir(_) => None,
            })
            .collect();
        let at = order.iter().position(|&i| i == ix)?;
        order[at + 1..]
            .iter()
            .chain(&order[..at])
            .copied()
            .find(|&i| !self.is_viewed(i))
    }

    fn show_review_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(ui) = self.review.as_mut() {
            ui.file = ix;
            self.highlight_shown(cx);
            cx.notify();
        }
    }

    /// The Viewed check of the shown file, for the diff header.
    pub(super) fn viewed_check(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ix = self.review.as_ref()?.file;
        Some(
            checkbox("diff-viewed")
                .label("Viewed")
                .small()
                .checked(self.is_viewed(ix))
                .on_click(cx.listener(move |this, _: &bool, _, cx| this.toggle_viewed(ix, cx)))
                .into_any_element(),
        )
    }

    /// The viewed files to save: those of refs that still exist.
    pub(super) fn viewed_to_save(&self) -> HashMap<String, Vec<String>> {
        self.viewed
            .iter()
            .filter(|(target, set)| {
                !set.is_empty()
                    && (crate::github::number_of(target).is_some()
                        || self.branches.iter().any(|b| &b.refname == *target))
            })
            .map(|(target, set)| {
                let mut keys: Vec<String> = set.iter().cloned().collect();
                keys.sort();
                (target.clone(), keys)
            })
            .collect()
    }

    pub(super) fn render_review(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let Some(ui) = self.review.as_ref() else {
            return div().into_any_element();
        };
        let target = short_ref(&ui.target);
        let n = ui.diff.as_ref().map_or(0, |d| d.files.len());
        let viewed = (0..n).filter(|&i| self.is_viewed(i)).count();
        let summary = match (&ui.error, &ui.diff) {
            (Some(e), _) => e.clone(),
            (None, None) => "Loading changes…".to_string(),
            (None, Some(d)) => {
                let adds: u32 = d.files.iter().map(|f| f.additions).sum();
                let dels: u32 = d.files.iter().map(|f| f.deletions).sum();
                let mut s = format!(
                    "{} commit{} · {n} file{}",
                    ui.commits,
                    history::plural(ui.commits),
                    history::plural(n)
                );
                // Git counts no lines in a binary file.
                if d.files.iter().any(|f| !f.binary) {
                    s.push_str(&format!(" · +{adds} −{dels}"));
                }
                if ui.worktree.is_some() {
                    s.push_str(" · with uncommitted changes");
                }
                if n > 0 {
                    s.push_str(&format!(" · {viewed} of {n} viewed"));
                }
                if ui.more_new_files > 0 {
                    s.push_str(&format!(
                        " · {} more new file{} not shown",
                        ui.more_new_files,
                        history::plural(ui.more_new_files)
                    ));
                }
                s
            }
        };
        let checkout = self.checkout_of(&ui.target).cloned();
        // The worktree that has the branch checked out, when it is not this
        // tab's: its own tab is one click away.
        let own = self
            .current_worktree
            .and_then(|i| self.worktrees.get(i))
            .map(|w| w.path.clone());
        let open_tab = checkout
            .as_ref()
            .filter(|w| !w.prunable && Some(&w.path) != own.as_ref())
            .map(|w| w.path.clone());
        let uncommitted = checkout
            .as_ref()
            .map(|w| (self.changed_in(w), ui.worktree.is_some()));
        let target_ref = ui.target.clone();
        let header = h_flex()
            .flex_none()
            .px_3()
            .py_2()
            .gap_3()
            .border_b_1()
            .border_color(t.colors.border)
            .child(
                Icon::new(IconName::GitCompare)
                    .size(px(18.))
                    .text_color(t.colors.primary),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_1()
                            .child("Review")
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(target),
                            )
                            .child(div().text_color(muted).child("against"))
                            .child(self.review_base_button(cx)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if ui.error.is_some() {
                                t.colors.red
                            } else {
                                muted
                            })
                            .truncate()
                            .child(summary),
                    ),
            )
            .when_some(uncommitted, |d, (changed, on)| {
                d.child(
                    checkbox("review-uncommitted")
                        .small()
                        .label(format!("Uncommitted changes ({changed})"))
                        .checked(on)
                        .tooltip("Show the files of the worktree that has this branch checked out")
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            this.set_review_uncommitted(*on, cx)
                        })),
                )
            })
            .when_some(open_tab, |d, path| {
                d.child(
                    button("review-open-tab")
                        .ghost()
                        .small()
                        .label("Open Tab")
                        .tooltip("Open the worktree in a tab, on its Changes")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_worktree(path.clone(), cx)),
                        ),
                )
            })
            .child(
                button("review-browse")
                    .ghost()
                    .small()
                    .label("Browse Commits")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_target(LogTarget::Ref(target_ref.clone()), cx)
                    })),
            );
        let body = match &ui.diff {
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(if ui.error.is_some() {
                    "Choose another base branch."
                } else {
                    "Loading changes…"
                })
                .into_any_element(),
            Some(d) if d.files.is_empty() => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(muted)
                .child(Icon::new(IconName::CircleCheck).size(px(28.)))
                .child(format!("No changes against {}.", short_ref(&ui.base)))
                .into_any_element(),
            Some(_) => {
                let border = t.colors.border;
                split("review-split", false)
                    .child(
                        split_panel("review-split", 340., 220.0..700., cx).child(
                            div()
                                .size_full()
                                .border_r_1()
                                .border_color(border)
                                .child(self.pane(Part::ReviewFiles, cx)),
                        ),
                    )
                    .child(resizable_panel().child(self.pane(Part::Diff, cx)))
                    .into_any_element()
            }
        };
        v_flex()
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// The base of the review, with a menu to choose another.
    fn review_base_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let muted = cx.theme().colors.muted_foreground;
        let Some(ui) = &self.review else {
            return div().into_any_element();
        };
        let current = ui.base.clone();
        let target = ui.target.clone();
        let tip = self.base_tip(ui);
        let choices: Vec<(String, String)> = self
            .branches
            .iter()
            .filter(|b| b.kind == RefKind::Local)
            .chain(
                self.branches
                    .iter()
                    .filter(|b| b.kind == RefKind::Remote)
                    .take(20),
            )
            .filter(|b| b.refname != target)
            .map(|b| (b.refname.clone(), b.name.clone()))
            .collect();
        let this = cx.entity();
        button("review-base")
            .ghost()
            .small()
            .tooltip(tip)
            .child(
                h_flex()
                    .gap_1()
                    // The size of the title around it, not of a button.
                    .text_size(px(crate::settings::get(cx).ui_size))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(short_ref(&current))
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(px(12.))
                            .text_color(muted),
                    ),
            )
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.label("Compare with").max_h(px(420.)).scrollable(true);
                for (refname, name) in &choices {
                    let (r, this) = (refname.clone(), this.clone());
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(*refname == current)
                            .on_click(move |_, _, cx| {
                                let r = r.clone();
                                this.update(cx, |app, cx| app.set_review_base(r, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// The files of the review, each with its Viewed check.
    pub(super) fn render_review_files(
        &mut self,
        memo: &mut Memo,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(d) = self.review.as_ref().and_then(|r| r.diff.clone()) else {
            return div().into_any_element();
        };
        let rows = keep(memo, || {
            self.file_rows("review", &files::paths(&d.files), cx)
        });
        // A tree lists its folders first.
        let has_dirs = matches!(rows.first(), Some(FileRow::Dir(_)));
        let files = d.clone();
        let list = uniform_list(
            "review-files",
            rows.len(),
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                range
                    .map(|i| match rows[i] {
                        FileRow::Dir(ref dir) => this.dir_row("review", dir, ("review-dir", i), cx),
                        FileRow::File { ix, depth } => {
                            let selected = this.review.as_ref().is_some_and(|r| r.file == ix);
                            let viewed = this.is_viewed(ix);
                            diff::file_row(
                                &files.files[ix],
                                selected,
                                depth,
                                ("review-file", ix),
                                cx,
                            )
                            .when(viewed, |d| d.opacity(0.55))
                            .child(
                                // A click on the check is not also a
                                // click on the row.
                                div()
                                    .flex_none()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .child(
                                        checkbox(("viewed", ix))
                                            .small()
                                            .checked(viewed)
                                            .tooltip("Viewed")
                                            .on_click(cx.listener(move |this, _: &bool, _, cx| {
                                                this.toggle_viewed(ix, cx)
                                            })),
                                    ),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| this.show_review_file(ix, cx)),
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
                files::files_bar(&d.files, self.folder_buttons(&["review"], has_dirs, cx), cx)
                    .h(px(36.))
                    .border_b_1()
                    .border_color(border),
            )
            .child(list)
            .into_any_element()
    }
}

/// A short name for a ref or a commit: `main`, `origin/main`, `#12`, `abc1234`.
pub(super) fn short_ref(r: &str) -> String {
    if let Some(n) = crate::github::number_of(r) {
        return format!("#{n}");
    }
    if let Some(s) = ["refs/heads/", "refs/remotes/", "refs/tags/"]
        .iter()
        .find_map(|p| r.strip_prefix(p))
    {
        return s.to_string();
    }
    if r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit()) {
        return r[..7].to_string();
    }
    r.to_string()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::short_ref;

    #[test]
    fn short_names() {
        assert_eq!(short_ref("refs/heads/feature/x"), "feature/x");
        assert_eq!(short_ref("refs/remotes/origin/main"), "origin/main");
        assert_eq!(short_ref("refs/gibbon/pr/12"), "#12");
        assert_eq!(short_ref(&"a".repeat(40)), "aaaaaaa");
        assert_eq!(short_ref("HEAD"), "HEAD");
    }
}
