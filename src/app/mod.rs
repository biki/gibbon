//! One repository tab: sidebar, the Changes / History views, status bar.
//! The window around the tabs is in `workspace`. Git work runs on the
//! background executor; results land back here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Instant, SystemTime};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::component::{
    Icon, ResizablePanel, ResizablePanelGroup, Sizable as _, TitleBar, h_flex, h_resizable,
    resizable_panel, v_flex, v_resizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::git::{
    self, Branch, Commit, CommitDetail, FileDiff, HeadInfo, LogTarget, PatchOp, PickState, RefKind,
    Repo, StatusEntry,
};
use crate::graph::{self, Graph};
use crate::highlight::{self, DiffStyles};
use crate::{
    CleanUpBranches, CloneRepo, CommitChanges, Fetch, NewBranch, OpenRepo, OpenSettings, Pull,
    Push, Refresh, SelectNext, SelectPrev, ShowActivity, ShowAllBranches, ShowChanges, ShowHistory,
    StashChanges, TogglePalette,
};
use files::FileRow;
use hover::{Off as _, button, checkbox, hover_fill};
use pane::{Memo, Part, keep};

type IconName = gpui_kit::assets::IconName;

/// Files up to this many lines get their syntax colors in the same task as
/// their text, so the diff shows with its colors at once. A longer file
/// shows first, and its colors follow.
const COLORS_WITH_TEXT: usize = 5_000;

/// Syntax and word styles of the files of one commit or stash. A file gets
/// its styles when it is first shown: a commit can have thousands of files.
#[derive(Default)]
struct FileStyles {
    /// The commit or stash that the styles are for.
    sha: String,
    by_file: HashMap<usize, Rc<DiffStyles>>,
    /// The file that a task highlights now. Dropping the task stops the
    /// work if it has not started yet.
    task: Option<(usize, Task<()>)>,
}

impl FileStyles {
    fn new(sha: String, first: Option<(usize, DiffStyles)>) -> Self {
        FileStyles {
            sha,
            by_file: first.into_iter().map(|(ix, s)| (ix, Rc::new(s))).collect(),
            task: None,
        }
    }

    fn get(&self, sha: &str, ix: usize) -> Option<Rc<DiffStyles>> {
        if self.sha == sha {
            self.by_file.get(&ix).cloned()
        } else {
            None
        }
    }
}

/// The detail that a `FileStyles` belongs to.
#[derive(Clone, Copy)]
enum Shown {
    Commit,
    Stash,
    Review,
}

/// What changed on disk: the most that must reload for this worktree, and
/// whether another worktree of the repository changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DiskChange {
    own: Option<crate::watch::Change>,
    others: bool,
}

impl DiskChange {
    fn add(&mut self, change: crate::watch::Change) {
        match change {
            crate::watch::Change::Worktrees => self.others = true,
            own => self.own = self.own.max(Some(own)),
        }
    }

    fn merge(&mut self, other: DiskChange) {
        self.own = self.own.max(other.own);
        self.others |= other.others;
    }
}

/// How often the worktree rows refresh without a change on disk, so their
/// times stay right.
const WORKTREE_TICK: std::time::Duration = std::time::Duration::from_secs(30);
/// How often the status of the pull requests reloads while checks run, and
/// while none run. GitHub takes seconds for it, and counts the calls.
const PR_TICK_RUNNING: std::time::Duration = std::time::Duration::from_secs(60);
const PR_TICK_IDLE: std::time::Duration = std::time::Duration::from_secs(300);
/// A tab that shows again fetches when its last fetch is older than this.
const SHOWN_FETCH_GAP: std::time::Duration = std::time::Duration::from_secs(60);

mod activity;
mod branches;
mod changes;
mod cleanup;
mod clone;
mod diff;
mod files;
mod history;
mod hover;
mod palette;
mod pane;
mod pulls;
mod rebase;
mod review;
mod settings_ui;
mod sidebar;
mod stash;
mod workspace;
mod worktrees;

pub use workspace::Workspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Changes,
    History,
    /// A stash, by its stash index.
    Stash(usize),
    /// The interactive rebase planner.
    Rebase,
    /// A branch's changes since it left its base (see `review`).
    Review,
    /// The moves of all branches (see `activity`).
    Activity,
}

pub struct GitApp {
    focus: FocusHandle,
    list_focus: FocusHandle,
    repo: Option<Repo>,
    head: HeadInfo,
    /// Refs and status of the open repository have loaded once.
    refs_loaded: bool,
    branches: Vec<Branch>,
    status: Vec<StatusEntry>,
    /// When this tab saw each deleted file of `status` go: a deleted file has
    /// no time on disk. The files that were gone at the first load are not in
    /// it.
    deleted_at: HashMap<String, SystemTime>,
    /// When a status load saw each file change on disk: its rows flash for
    /// `changes::FLASH`.
    edited: HashMap<String, Instant>,
    stashes: Vec<git::Stash>,
    prs: Vec<crate::github::PullRequest>,
    /// Checks, review and merge state of the pull requests, by number.
    pr_status: HashMap<u64, crate::github::PrStatus>,
    /// When `pr_status` last loaded.
    pr_status_at: Option<std::time::Instant>,
    pr_epoch: u64,
    /// When the last fetch of this tab started.
    fetched_at: Option<Instant>,
    /// An automatic fetch runs. With `true` it shows its result: the user
    /// clicked Fetch while it ran.
    auto_fetching: Option<bool>,
    /// The last automatic fetch failed: the next failures show no toast.
    auto_fetch_failed: bool,
    /// The worktrees of the repository, the main one first.
    worktrees: Vec<git::Worktree>,
    /// The worktree of this tab, an index into `worktrees`.
    current_worktree: Option<usize>,
    /// What each worktree holds, by its folder, with the load it came from.
    worktree_info: HashMap<PathBuf, (u64, git::WorktreeInfo)>,
    worktree_epoch: u64,
    /// The branch that others start from, a full ref name.
    base: Option<String>,
    /// The local branches that `base` contains, full ref names.
    merged: HashSet<String>,
    /// The rows of the open cleanup dialog (see `cleanup`).
    cleanup: Vec<cleanup::CleanupRow>,
    cleanup_epoch: u64,
    stash_detail: Option<Rc<CommitDetail>>,
    stash_styles: FileStyles,
    stash_file: usize,
    rebase: Option<rebase::RebaseUi>,
    review: Option<review::ReviewUi>,
    review_styles: FileStyles,
    review_epoch: u64,
    /// Files marked as viewed, by the ref under review (see `review`).
    viewed: HashMap<String, HashSet<String>>,
    /// A branch to review once the refs load (UI checks).
    check_review: Option<String>,
    /// The moves of the branches, newest first (see `activity`).
    moves: Rc<Vec<git::RefMove>>,
    /// The commits that moves added and dropped, by (old, new): git counts
    /// each move once.
    move_counts: HashMap<(String, String), Option<(u32, u32)>>,
    moves_epoch: u64,
    /// Moves after this time are new. None until the moves first load.
    activity_seen: Option<i64>,
    /// While the timeline is shown, moves after this time have a dot.
    activity_mark: i64,
    /// A cherry-pick, rebase or merge stopped on conflicts.
    paused: Option<git::Paused>,
    view: View,
    target: LogTarget,
    commits: Rc<Vec<Commit>>,
    graph: Rc<Graph>,
    /// For a browsed branch: its commits that HEAD does not contain.
    picks: Rc<HashMap<String, PickState>>,
    log_loading: bool,
    selected: HashSet<usize>,
    anchor: Option<usize>,
    cursor: Option<usize>,
    detail: Option<Rc<CommitDetail>>,
    /// Syntax and word styles of the files of `detail` that were shown.
    detail_styles: FileStyles,
    detail_file: usize,
    /// Selected working-tree file: (path, staged side).
    change_sel: Option<(String, bool)>,
    change_diff: Option<Rc<FileDiff>>,
    change_styles: Option<Rc<DiffStyles>>,
    /// Selected Add / Del lines of `change_diff`.
    line_sel: HashSet<usize>,
    line_anchor: Option<usize>,
    message: Entity<TextareaState>,
    filter: Entity<InputState>,
    /// Enter-to-submit for the open text dialog.
    prompt_sub: Option<Subscription>,
    /// A dialog to open on the first frame (UI checks).
    check_dialog: Option<String>,
    collapsed: HashSet<&'static str>,
    /// Branch sections that show all their branches, not the short list.
    expanded: HashSet<&'static str>,
    /// Closed folders of the file trees: (list, folder path).
    collapsed_dirs: HashSet<(&'static str, String)>,
    busy: Option<SharedString>,
    log_scroll: UniformListScrollHandle,
    side_scroll: UniformListScrollHandle,
    /// The scroll state of each diff view, by its list id.
    diff_scroll: HashMap<&'static str, diff::DiffScroll>,
    /// A restored selection, applied when the log or the commit loads.
    pending_commit: Option<String>,
    pending_file: Option<usize>,
    /// Open the inspector on the first frame (UI checks, debug builds).
    check_inspector: bool,
    _watcher: Option<crate::watch::RepoWatcher>,
    _watch_task: Option<Task<()>>,
    /// The other worktrees that the watcher was started for.
    watched: Option<Vec<PathBuf>>,
    _tick: Option<Task<()>>,
    /// The session had a state for this repository.
    restored: bool,
    /// The tab is not shown: changes on disk wait in `hidden_change`.
    hidden: bool,
    /// What changed on disk while the tab was hidden.
    hidden_change: DiskChange,
    log_epoch: u64,
    detail_epoch: u64,
    diff_epoch: u64,
    /// The parts of the tab in their own views (see `pane`).
    panes: HashMap<Part, Entity<pane::Pane>>,
    _subs: Vec<Subscription>,
}

impl GitApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("Summary\n\nDescription (optional)")
        });
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter branches"));
        let subs = vec![
            cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify()),
            // Light / dark switch: syntax colors come from the theme.
            cx.observe_global::<gpui_kit::component::theme::Theme>(|this, cx| {
                this.detail_styles = FileStyles::default();
                this.stash_styles = FileStyles::default();
                this.review_styles = FileStyles::default();
                this.highlight_shown(cx);
                this.load_change_diff(cx);
            }),
        ];
        GitApp {
            focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            repo: None,
            head: HeadInfo::default(),
            refs_loaded: false,
            branches: vec![],
            status: vec![],
            deleted_at: HashMap::new(),
            edited: HashMap::new(),
            stashes: vec![],
            prs: vec![],
            pr_status: HashMap::new(),
            pr_status_at: None,
            pr_epoch: 0,
            fetched_at: None,
            auto_fetching: None,
            auto_fetch_failed: false,
            worktrees: vec![],
            current_worktree: None,
            worktree_info: HashMap::new(),
            worktree_epoch: 0,
            base: None,
            merged: HashSet::new(),
            cleanup: vec![],
            cleanup_epoch: 0,
            stash_detail: None,
            stash_styles: FileStyles::default(),
            stash_file: 0,
            rebase: None,
            review: None,
            review_styles: FileStyles::default(),
            review_epoch: 0,
            viewed: HashMap::new(),
            check_review: None,
            moves: Rc::new(vec![]),
            move_counts: HashMap::new(),
            moves_epoch: 0,
            activity_seen: None,
            activity_mark: 0,
            paused: None,
            view: View::History,
            target: LogTarget::Head,
            commits: Rc::new(vec![]),
            graph: Rc::new(Graph {
                rows: vec![],
                width: 1,
            }),
            picks: Rc::new(HashMap::new()),
            log_loading: false,
            selected: HashSet::new(),
            anchor: None,
            cursor: None,
            detail: None,
            detail_styles: FileStyles::default(),
            detail_file: 0,
            change_sel: None,
            change_diff: None,
            change_styles: None,
            line_sel: HashSet::new(),
            line_anchor: None,
            message,
            filter,
            prompt_sub: None,
            check_dialog: None,
            collapsed: HashSet::from(["tags"]),
            expanded: HashSet::new(),
            collapsed_dirs: HashSet::new(),
            busy: None,
            log_scroll: UniformListScrollHandle::new(),
            side_scroll: UniformListScrollHandle::new(),
            diff_scroll: HashMap::new(),
            pending_commit: None,
            pending_file: None,
            check_inspector: false,
            _watcher: None,
            _watch_task: None,
            watched: None,
            _tick: None,
            restored: false,
            hidden: false,
            hidden_change: DiskChange::default(),
            log_epoch: 0,
            detail_epoch: 0,
            diff_epoch: 0,
            panes: HashMap::new(),
            _subs: subs,
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Ask the window to show a toast.
    fn toast(&mut self, ok: Option<bool>, msg: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(AppEvent::Toast(ok, msg.into()));
    }

    /// Set what runs. The tab strip shows a spinner while it runs.
    fn set_busy(&mut self, busy: Option<SharedString>, cx: &mut Context<Self>) {
        if self.busy.is_some() != busy.is_some() {
            cx.emit(AppEvent::Busy(busy.is_some()));
        }
        self.busy = busy;
    }

    // -----------------------------------------------------------------------
    // Loading

    /// Show `repo`, where it was left. Each tab calls this once.
    pub(super) fn open_repo(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.repo = Some(repo);
        self.restore_session();
        // The first reload starts the watcher: it needs the worktrees.
        self.reload(cx);
        if !self.auto_fetch(SHOWN_FETCH_GAP, cx) {
            self.load_prs(cx);
        }
        self._tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(WORKTREE_TICK).await;
                let shown = this.update(cx, |this, cx| {
                    if !this.hidden && this.busy.is_none() {
                        this.load_worktree_info(false, cx);
                        if this.pr_status_due() {
                            this.load_pr_status(cx);
                        }
                        let minutes = crate::settings::get(cx).auto_fetch;
                        let every = std::time::Duration::from_secs(60 * u64::from(minutes));
                        this.auto_fetch(every, cx);
                    }
                });
                if shown.is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// Reload refs and status, then the log.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let r = repo.clone();
            let (head, branches, status, paused, stashes, worktrees, merged, dirs) = cx
                .background_executor()
                .spawn(async move {
                    let worktrees = git::worktrees(&r).map(|list| {
                        let own = r.root.canonicalize().unwrap_or_else(|_| r.root.clone());
                        let current = list.iter().position(|w| w.path == own);
                        (list, current, git::base_branch(&r))
                    });
                    let merged = match &worktrees {
                        Ok((_, _, Some(base))) => {
                            git::merged_branches(&r, base).unwrap_or_default()
                        }
                        _ => HashSet::new(),
                    };
                    let dirs = git::git_dir(&r).map(|git_dir| {
                        let common = git::common_dir(&r).unwrap_or_else(|_| git_dir.clone());
                        (git_dir, common)
                    });
                    (
                        git::head(&r),
                        git::branches(&r),
                        git::status(&r),
                        git::paused(&r),
                        git::stashes(&r),
                        worktrees,
                        merged,
                        dirs,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.repo.as_ref().map(|r| &r.root) != Some(&repo.root) {
                    return;
                }
                this.head = head;
                match branches {
                    Ok(b) => this.branches = b,
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                match status {
                    Ok(s) => this.set_status(s),
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                this.refs_loaded = true;
                this.paused = paused;
                this.stashes = stashes.unwrap_or_default();
                if let Ok((list, current, base)) = worktrees {
                    this.worktree_info
                        .retain(|path, _| list.iter().any(|w| &w.path == path));
                    this.worktrees = list;
                    this.current_worktree = current;
                    this.base = base;
                }
                this.merged = merged;
                let others: Vec<PathBuf> = this
                    .worktrees
                    .iter()
                    .enumerate()
                    .filter(|&(i, _)| Some(i) != this.current_worktree)
                    .map(|(_, w)| w.path.clone())
                    .collect();
                if this.watched.as_ref() != Some(&others) {
                    this.start_watcher(dirs.map_err(|e| e.to_string()), others, cx);
                }
                this.load_worktree_info(false, cx);
                if let Some(target) = this.check_review.take() {
                    this.start_review(target, cx);
                } else if this.view == View::Review {
                    this.load_review(cx);
                }
                this.load_activity(cx);
                if let View::Stash(i) = this.view {
                    if this.stashes.iter().any(|s| s.index == i) {
                        this.stash_file = 0;
                        this.load_stash_detail(cx);
                    } else {
                        this.view = View::Changes;
                    }
                }
                // The browsed branch may be gone (deleted, pruned).
                // Pull request heads are not branches: keep them.
                if let LogTarget::Ref(r) = &this.target
                    && crate::github::number_of(r).is_none()
                    && !this.branches.iter().any(|b| &b.refname == r)
                {
                    this.target = LogTarget::Head;
                }
                this.ensure_change_selection(cx);
                this.load_change_diff(cx);
                this.load_log(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Reload on changes outside the app: status for file edits, everything
    /// for ref moves, the worktree rows for changes in `others`. Bursts
    /// (saves, checkouts, builds) settle for 300 ms. `dirs` is this
    /// worktree's `.git` folder and the one with the refs.
    fn start_watcher(
        &mut self,
        dirs: Result<(PathBuf, PathBuf), String>,
        others: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        use futures::StreamExt as _;
        self._watcher = None;
        self._watch_task = None;
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.watched = Some(others.clone());
        let started = dirs.and_then(|(git_dir, common_dir)| {
            crate::watch::watch(&crate::watch::Dirs {
                root: repo.root.clone(),
                git_dir,
                common_dir,
                others,
            })
            .map_err(|e| e.to_string())
        });
        let (watcher, mut rx) = match started {
            Ok(w) => w,
            Err(e) => {
                self.toast(None, format!("Automatic refresh is off: {e}"), cx);
                return;
            }
        };
        self._watcher = Some(watcher);
        self._watch_task = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(300))
                    .await;
                let mut change = DiskChange::default();
                change.add(first);
                while let Ok(more) = rx.try_recv() {
                    change.add(more);
                }
                if this
                    .update(cx, |this, cx| this.disk_changed(change, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// Reload what `change` touched. A hidden tab reloads when it is shown.
    fn disk_changed(&mut self, change: DiskChange, cx: &mut Context<Self>) {
        use crate::watch::Change;
        // A running operation reloads when it ends.
        if self.busy.is_some() {
            return;
        }
        if self.hidden {
            self.hidden_change.merge(change);
            return;
        }
        match change.own {
            // A full reload includes the worktrees.
            Some(Change::Refs) => return self.reload(cx),
            Some(Change::Files) => self.reload_status(cx),
            _ => {}
        }
        if change.others {
            self.load_worktree_info(false, cx);
            // Switches in other worktrees.
            self.load_activity(cx);
        }
        // A review with uncommitted changes shows files on disk.
        if self.view == View::Review && self.review.as_ref().is_some_and(|r| r.worktree.is_some()) {
            self.load_review(cx);
        }
    }

    /// Hide or show this tab. Showing it reloads what changed while hidden,
    /// and fetches.
    pub(super) fn set_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        let shown = self.hidden && !hidden;
        self.hidden = hidden;
        if shown {
            let change = std::mem::take(&mut self.hidden_change);
            if change != DiskChange::default() {
                self.disk_changed(change, cx);
            }
            self.auto_fetch(SHOWN_FETCH_GAP, cx);
        }
    }

    /// Load what the worktrees hold, or with `own_only` what this tab's
    /// worktree holds. A repository with one worktree shows no rows.
    fn load_worktree_info(&mut self, own_only: bool, cx: &mut Context<Self>) {
        if self.worktrees.len() < 2 {
            return;
        }
        self.worktree_epoch += 1;
        let epoch = self.worktree_epoch;
        let list: Vec<git::Worktree> = self
            .worktrees
            .iter()
            .enumerate()
            .filter(|&(i, w)| !w.prunable && (!own_only || Some(i) == self.current_worktree))
            .map(|(_, w)| w.clone())
            .collect();
        let base = self.base.clone();
        cx.spawn(async move |this, cx| {
            // One task per worktree: each runs `git status` in its folder.
            let tasks: Vec<_> = list
                .into_iter()
                .map(|wt| {
                    let base = base.clone();
                    cx.background_executor().spawn(async move {
                        let info = git::worktree_info(&wt, base.as_deref());
                        (wt.path, info)
                    })
                })
                .collect();
            let results = futures::future::join_all(tasks).await;
            let _ = this.update(cx, |this, cx| {
                for (path, info) in results {
                    let Ok(info) = info else {
                        continue;
                    };
                    // A load that started later keeps its result.
                    let slot = this.worktree_info.entry(path).or_default();
                    if slot.0 < epoch {
                        *slot = (epoch, info);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reload only the working-tree status and the shown file diff.
    fn reload_status(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let r = repo.clone();
            let (status, paused) = cx
                .background_executor()
                .spawn(async move { (git::status(&r), git::paused(&r)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.repo.as_ref().map(|r| &r.root) != Some(&repo.root) {
                    return;
                }
                if let Ok(s) = status {
                    this.set_status(s);
                }
                this.paused = paused;
                this.ensure_change_selection(cx);
                this.load_change_diff(cx);
                // This worktree's row in the sidebar.
                this.load_worktree_info(true, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Take a new working-tree status. A file that is new in the list, or
    /// whose time on disk moved, flashes in the Changes list. A file that is
    /// newly deleted gets the time of now. Before the first load
    /// (`refs_loaded`), no file is new.
    fn set_status(&mut self, status: Vec<StatusEntry>) {
        let (now, flash) = (SystemTime::now(), Instant::now());
        let before: HashMap<&str, Option<SystemTime>> = self
            .status
            .iter()
            .map(|e| (e.path.as_str(), e.modified))
            .collect();
        self.edited.retain(|_, at| at.elapsed() < changes::FLASH);
        let mut deleted_at = HashMap::new();
        for e in &status {
            let old = before.get(e.path.as_str());
            if self.refs_loaded && old != Some(&e.modified) {
                self.edited.insert(e.path.clone(), flash);
            }
            if e.modified.is_some() {
                continue;
            }
            let time = match old {
                Some(None) => self.deleted_at.get(&e.path).copied(),
                _ => self.refs_loaded.then_some(now),
            };
            if let Some(t) = time {
                deleted_at.insert(e.path.clone(), t);
            }
        }
        self.deleted_at = deleted_at;
        self.status = status;
    }

    fn load_log(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.log_epoch += 1;
        let epoch = self.log_epoch;
        self.log_loading = true;
        let target = self.target.clone();
        let foreign = self.foreign_target().map(str::to_string);
        let keep = self.pending_commit.take().or_else(|| {
            self.cursor
                .and_then(|i| self.commits.get(i))
                .map(|c| c.sha.clone())
        });
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let commits = git::log(&repo, &target)?;
                    let graph = graph::layout(&commits);
                    let picks = match &foreign {
                        Some(r) => git::pick_states(&repo, r)?,
                        None => HashMap::new(),
                    };
                    anyhow::Ok((commits, graph, picks))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.log_epoch {
                    return;
                }
                this.log_loading = false;
                match result {
                    Ok((commits, graph, picks)) => {
                        let ix = keep
                            .and_then(|sha| commits.iter().position(|c| c.sha == sha))
                            .or((!commits.is_empty()).then_some(0));
                        this.commits = Rc::new(commits);
                        this.graph = Rc::new(graph);
                        this.picks = Rc::new(picks);
                        this.selected = ix.into_iter().collect();
                        this.anchor = ix;
                        this.cursor = ix;
                        if let Some(ix) = ix {
                            this.log_scroll.scroll_to_item(ix, ScrollStrategy::Top);
                        }
                        this.load_detail(cx);
                    }
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_detail(&mut self, cx: &mut Context<Self>) {
        let (Some(repo), Some(commit)) = (
            self.repo.clone(),
            self.cursor.and_then(|i| self.commits.get(i)),
        ) else {
            self.detail = None;
            return;
        };
        if self.detail.as_ref().is_some_and(|d| d.sha == commit.sha) {
            return;
        }
        let sha = commit.sha.clone();
        self.detail_epoch += 1;
        let epoch = self.detail_epoch;
        // The colors of the commit shown before are not needed any more.
        self.detail_styles.task = None;
        let pending = self.pending_file;
        let shown = self.shown_file_rule("commit", cx);
        let theme = cx.theme().highlight_theme.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let d = git::commit_detail(&repo, &sha)?;
                    let ix = pending
                        .filter(|&i| i < d.files.len())
                        .or_else(|| shown(&d.files))
                        .unwrap_or(0);
                    let styles = colors_with_text(&d, ix, &theme);
                    anyhow::Ok((d, ix, styles))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.detail_epoch {
                    return;
                }
                match result {
                    Ok((d, ix, styles)) => {
                        this.pending_file = None;
                        this.detail_file = ix;
                        this.detail_styles =
                            FileStyles::new(d.sha.clone(), styles.map(|s| (ix, s)));
                        this.detail = Some(Rc::new(d));
                        this.highlight_shown(cx);
                    }
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Which file a new commit or stash shows first: the first file of the
    /// list `scope`, in the order and view of the settings. For a
    /// background task.
    fn shown_file_rule(
        &self,
        scope: &'static str,
        cx: &App,
    ) -> impl FnOnce(&[FileDiff]) -> Option<usize> + Send + 'static {
        let s = crate::settings::get(cx);
        let (tree, desc) = (s.file_tree, s.file_sort_desc);
        let closed: HashSet<String> = self
            .collapsed_dirs
            .iter()
            .filter(|(list, _)| *list == scope)
            .map(|(_, dir)| dir.clone())
            .collect();
        move |files| {
            files::first(&files::paths(files), tree, desc, &|dir| {
                closed.contains(dir)
            })
        }
    }

    /// Keep a working-tree file selected while there are changes.
    fn ensure_change_selection(&mut self, cx: &App) {
        let valid = self.change_sel.as_ref().is_some_and(|(path, staged)| {
            self.status.iter().any(|e| {
                &e.path == path
                    && if *staged {
                        e.staged.is_some()
                    } else {
                        e.unstaged.is_some()
                    }
            })
        });
        if !valid {
            // The first file shown, or any file when all folders are closed.
            self.change_sel = self.first_change(cx).or_else(|| {
                self.status
                    .first()
                    .map(|e| (e.path.clone(), e.staged.is_some()))
            });
        }
    }

    fn load_change_diff(&mut self, cx: &mut Context<Self>) {
        let (Some(repo), Some((path, staged))) = (self.repo.clone(), self.change_sel.clone())
        else {
            self.change_diff = None;
            return;
        };
        let Some(entry) = self.status.iter().find(|e| e.path == path).cloned() else {
            self.change_diff = None;
            return;
        };
        self.diff_epoch += 1;
        let epoch = self.diff_epoch;
        let theme = cx.theme().highlight_theme.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let diff = git::file_diff(&repo, &entry, staged)?;
                    let styles = diff.as_ref().map(|d| highlight::compute(d, &theme));
                    anyhow::Ok((diff, styles))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.diff_epoch {
                    return;
                }
                match result {
                    Ok((d, styles)) => {
                        // Line numbers change with the diff: drop the selection.
                        let same =
                            this.change_diff
                                .as_ref()
                                .zip(d.as_ref())
                                .is_some_and(|(a, b)| {
                                    a.path == b.path && a.lines.len() == b.lines.len()
                                });
                        if !same {
                            this.line_sel.clear();
                            this.line_anchor = None;
                        }
                        this.change_diff = d.map(Rc::new);
                        this.change_styles = styles.map(Rc::new);
                    }
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Start the syntax colors of the shown file of the commit and of the
    /// stash, where that file has none yet.
    fn highlight_shown(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.detail.clone() {
            self.highlight_file(Shown::Commit, d, self.detail_file, cx);
        }
        if let Some(d) = self.stash_detail.clone() {
            self.highlight_file(Shown::Stash, d, self.stash_file, cx);
        }
        if let Some((d, ix)) = self
            .review
            .as_ref()
            .and_then(|r| Some((r.diff.clone()?, r.file)))
        {
            self.highlight_file(Shown::Review, d, ix, cx);
        }
    }

    fn file_styles(&mut self, of: Shown) -> &mut FileStyles {
        match of {
            Shown::Commit => &mut self.detail_styles,
            Shown::Stash => &mut self.stash_styles,
            Shown::Review => &mut self.review_styles,
        }
    }

    fn highlight_file(
        &mut self,
        of: Shown,
        detail: Rc<CommitDetail>,
        ix: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = detail.files.get(ix) else {
            return;
        };
        let theme = cx.theme().highlight_theme.clone();
        let styles = self.file_styles(of);
        if styles.sha != detail.sha {
            *styles = FileStyles::new(detail.sha.clone(), None);
        }
        if styles.by_file.contains_key(&ix) || styles.task.as_ref().is_some_and(|(i, _)| *i == ix) {
            return;
        }
        let (file, sha) = (file.clone(), detail.sha.clone());
        let task = cx.spawn(async move |this, cx| {
            let computed = cx
                .background_executor()
                .spawn(async move { highlight::compute(&file, &theme) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let styles = this.file_styles(of);
                if styles.sha == sha {
                    styles.by_file.insert(ix, Rc::new(computed));
                    cx.notify();
                }
            });
        });
        // A task for another file stops, if it has not started yet.
        self.file_styles(of).task = Some((ix, task));
    }

    /// Stage, unstage or discard some lines of the shown working-tree diff.
    fn apply_lines(&mut self, chosen: HashSet<usize>, op: PatchOp, cx: &mut Context<Self>) {
        let Some(file) = self.change_diff.clone() else {
            return;
        };
        let Some(patch) = git::partial_patch(&file, &chosen, op) else {
            return;
        };
        self.line_sel.clear();
        self.line_anchor = None;
        let label = match op {
            PatchOp::Stage => "Staging…",
            PatchOp::Unstage => "Unstaging…",
            PatchOp::Discard => "Discarding…",
        };
        self.run_op(
            label,
            None,
            move |repo| git::apply_patch(repo, &patch, op).map(|_| String::new()),
            cx,
        );
    }

    fn confirm_discard_lines(
        &mut self,
        chosen: HashSet<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = chosen.len();
        self.confirm(
            "Discard changes?",
            format!(
                "This removes {n} changed line{} from the file on disk. You cannot undo it.",
                history::plural(n)
            ),
            "Discard",
            move |this, cx| this.apply_lines(chosen.clone(), PatchOp::Discard, cx),
            window,
            cx,
        );
    }

    fn confirm_discard_files(
        &mut self,
        entries: Vec<StatusEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let body = match entries.as_slice() {
            [one] if one.unstaged == Some(git::Change::Untracked) => format!(
                "This deletes the new file {} from disk. You cannot undo it.",
                one.path
            ),
            [one] => format!(
                "This throws away the unstaged changes of {}. You cannot undo it.",
                one.path
            ),
            many => format!(
                "This throws away the unstaged changes of {} files and deletes the \
                 new ones. You cannot undo it.",
                many.len()
            ),
        };
        self.confirm(
            "Discard changes?",
            body,
            "Discard",
            move |this, cx| {
                let entries = entries.clone();
                this.run_op(
                    "Discarding…",
                    None,
                    move |repo| git::discard(repo, &entries).map(|_| String::new()),
                    cx,
                )
            },
            window,
            cx,
        );
    }

    /// A dialog with Cancel and a red confirm button.
    fn confirm(
        &self,
        title: &str,
        body: String,
        ok: &str,
        on_ok: impl Fn(&mut GitApp, &mut Context<GitApp>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity();
        let on_ok = Rc::new(on_ok);
        let (title, ok): (SharedString, SharedString) =
            (title.to_string().into(), ok.to_string().into());
        window.open_dialog(cx, move |dialog, _, _| {
            let (this, on_ok) = (this.clone(), on_ok.clone());
            dialog
                .title(title.clone())
                .w(px(420.))
                .child(div().text_size(px(13.)).child(body.clone()))
                .footer(dialog_footer(
                    ok.clone(),
                    ButtonVariant::Danger,
                    move |_, cx| {
                        this.update(cx, |app, cx| on_ok(app, cx));
                    },
                ))
        });
    }

    // -----------------------------------------------------------------------
    // Operations

    /// Run a git operation in the background, show the result, then reload.
    fn run_op(
        &mut self,
        label: &str,
        ok_msg: Option<String>,
        op: impl FnOnce(&Repo) -> anyhow::Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.set_busy(Some(label.to_string().into()), cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let result = op(&repo);
                    (result, git::paused(&repo))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.set_busy(None, cx);
                match result {
                    (Ok(_), _) => {
                        if let Some(msg) = ok_msg {
                            this.toast(Some(true), msg, cx);
                        }
                    }
                    (Err(_), Some(p)) => {
                        this.view = View::Changes;
                        this.toast(
                            Some(false),
                            format!(
                                "The {} stopped on a conflict. Resolve the files, \
                                 stage them, then click Continue.",
                                p.name().to_lowercase()
                            ),
                            cx,
                        );
                    }
                    (Err(e), None) => this.toast(Some(false), e.to_string(), cx),
                }
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn head_name(&self) -> String {
        if !self.refs_loaded {
            return "…".into();
        }
        self.head
            .branch
            .clone()
            .unwrap_or_else(|| "detached HEAD".into())
    }

    /// The browsed ref, when it is not the checked-out branch.
    fn foreign_target(&self) -> Option<&str> {
        match &self.target {
            LogTarget::Ref(r) => {
                let head_ref = self.head.branch.as_ref().map(|b| format!("refs/heads/{b}"));
                (head_ref.as_deref() != Some(r.as_str())).then_some(r.as_str())
            }
            _ => None,
        }
    }

    fn show_target(&mut self, target: LogTarget, cx: &mut Context<Self>) {
        let target = match target {
            LogTarget::Ref(r)
                if self.head.branch.as_ref().map(|b| format!("refs/heads/{b}"))
                    == Some(r.clone()) =>
            {
                LogTarget::Head
            }
            t => t,
        };
        self.view = View::History;
        if self.target != target {
            self.target = target;
            self.cursor = None;
            self.selected.clear();
            self.commits = Rc::new(vec![]);
            self.load_log(cx);
        }
        cx.notify();
    }

    /// What to reopen for the current repository.
    fn repo_state(&self) -> Option<crate::session::RepoState> {
        self.repo.as_ref()?;
        let view = match self.view {
            View::Changes => "changes".to_string(),
            View::Stash(n) => format!("stash:{n}"),
            View::Review if self.review.is_some() => "review".to_string(),
            View::Activity => "activity".to_string(),
            View::History | View::Rebase | View::Review => "history".to_string(),
        };
        let target = match &self.target {
            LogTarget::Head => None,
            LogTarget::All => Some("all".to_string()),
            LogTarget::Ref(r) => Some(r.clone()),
        };
        // Until the log and the commit load, keep what was restored.
        let commit = self.pending_commit.clone().or_else(|| {
            self.cursor
                .and_then(|i| self.commits.get(i))
                .map(|c| c.sha.clone())
        });
        Some(crate::session::RepoState {
            view,
            target,
            commit,
            file: self.pending_file.unwrap_or(self.detail_file),
            change: self.change_sel.clone(),
            review: self.review.as_ref().map(|r| crate::session::ReviewState {
                target: r.target.clone(),
                base: r.base.clone(),
                worktree: r.worktree.clone(),
            }),
            viewed: self.viewed_to_save(),
            activity_seen: self.activity_seen,
        })
    }

    /// Reopen where this repository was left.
    fn restore_session(&mut self) {
        let Some(root) = self.repo.as_ref().map(|r| r.root.clone()) else {
            return;
        };
        let Some(s) = crate::session::load().repos.remove(&root) else {
            return;
        };
        self.restored = true;
        self.viewed = s
            .viewed
            .into_iter()
            .map(|(target, keys)| (target, keys.into_iter().collect()))
            .collect();
        // The review loads with the refs.
        self.review = s
            .review
            .map(|r| review::ReviewUi::new(r.target, r.base, r.worktree));
        self.view = match s.view.as_str() {
            "changes" => View::Changes,
            "review" if self.review.is_some() => View::Review,
            "activity" => View::Activity,
            v => match v.strip_prefix("stash:").and_then(|n| n.parse().ok()) {
                Some(n) => View::Stash(n),
                None => View::History,
            },
        };
        self.target = match s.target.as_deref() {
            None => LogTarget::Head,
            Some("all") => LogTarget::All,
            Some(r) => LogTarget::Ref(r.to_string()),
        };
        self.pending_commit = s.commit;
        self.pending_file = Some(s.file);
        self.change_sel = s.change;
        self.activity_seen = s.activity_seen;
        self.activity_mark = s.activity_seen.unwrap_or(0);
    }

    /// Start state for automated UI checks:
    /// `GIBBON_BROWSE=<branch>`, `GIBBON_VIEW=changes|all|activity`,
    /// `GIBBON_FILE=<path>` (a changed file), `GIBBON_DIFF=split`,
    /// `GIBBON_REBASE=<sha>`, `GIBBON_STASH=<n>`, `GIBBON_REVIEW=<branch>`,
    /// `GIBBON_DIALOG=new-branch|stash|palette|settings|restore|cleanup`,
    /// `GIBBON_INSPECTOR=1`.
    pub fn apply_check_env(&mut self, cx: &mut Context<Self>) {
        let var = |k: &str| std::env::var(k).ok();
        if let Some(b) = var("GIBBON_BROWSE") {
            self.browse(&b, cx);
        }
        match var("GIBBON_VIEW").as_deref() {
            Some("changes") => self.view = View::Changes,
            Some("all") => self.show_target(LogTarget::All, cx),
            Some("activity") => self.show_activity(cx),
            _ => {}
        }
        if let Some(f) = var("GIBBON_FILE") {
            self.change_sel = Some((f, false));
        }
        if var("GIBBON_DIFF").as_deref() == Some("split") {
            // For this run only: not saved.
            cx.global_mut::<crate::settings::Settings>().split_diff = true;
        }
        if let Some(sha) = var("GIBBON_REBASE") {
            self.start_rebase(sha, cx);
        }
        if let Some(b) = var("GIBBON_REVIEW") {
            self.check_review = Some(if b.starts_with("refs/") {
                b
            } else {
                format!("refs/heads/{b}")
            });
        }
        if let Some(n) = var("GIBBON_STASH").and_then(|n| n.parse().ok()) {
            self.view = View::Stash(n);
        }
        self.check_dialog = var("GIBBON_DIALOG");
        self.check_inspector = var("GIBBON_INSPECTOR").is_some();
    }

    /// Browse a branch by short name, before the refs have loaded.
    pub fn browse(&mut self, name: &str, cx: &mut Context<Self>) {
        let refname = if name.starts_with("refs/") {
            name.to_string()
        } else {
            format!("refs/heads/{name}")
        };
        self.show_target(LogTarget::Ref(refname), cx);
    }

    fn switch_branch(&mut self, branch: Branch, cx: &mut Context<Self>) {
        let name = branch.name.clone();
        self.target = LogTarget::Head;
        self.run_op(
            "Switching…",
            Some(format!("Switched to {name}")),
            move |repo| git::switch(repo, &branch).map(|_| String::new()),
            cx,
        );
    }

    /// Selected commits that can be picked into HEAD, oldest first.
    fn pickable_selection(&self) -> Vec<String> {
        let mut ixs: Vec<usize> = self.selected.iter().copied().collect();
        ixs.sort_unstable_by(|a, b| b.cmp(a));
        ixs.into_iter()
            .filter_map(|i| self.commits.get(i))
            .filter(|c| self.picks.get(&c.sha) == Some(&PickState::Pickable))
            .map(|c| c.sha.clone())
            .collect()
    }

    fn pick_selected(&mut self, cx: &mut Context<Self>) {
        let shas = self.pickable_selection();
        let merges = shas
            .iter()
            .any(|sha| self.commits.iter().any(|c| &c.sha == sha && c.is_merge()));
        if shas.is_empty() {
            let head = self.head_name();
            self.toast(
                None,
                format!("Select commits that {head} does not have yet."),
                cx,
            );
            return;
        }
        let n = shas.len();
        let msg = format!(
            "Picked {n} commit{} into {}",
            if n == 1 { "" } else { "s" },
            self.head_name()
        );
        self.run_op(
            "Picking…",
            Some(msg),
            move |repo| git::cherry_pick(repo, &shas, merges),
            cx,
        );
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        if let Some(show) = &mut self.auto_fetching {
            // The automatic fetch that runs shows its result.
            *show = true;
            return;
        }
        if self.repo.is_some() && self.busy.is_none() {
            self.fetched_at = Some(Instant::now());
        }
        self.run_op(
            "Fetching…",
            Some("Fetched all remotes".into()),
            git::fetch,
            cx,
        );
        self.load_prs(cx);
    }

    /// Fetch all remotes when automatic fetch is on and the last fetch is
    /// older than `gap`. It runs next to other operations, and shows no
    /// toast. It shows only its first error until a fetch works again. Also
    /// reload the pull requests. Returns whether a fetch started.
    fn auto_fetch(&mut self, gap: std::time::Duration, cx: &mut Context<Self>) -> bool {
        let Some(repo) = self.repo.clone() else {
            return false;
        };
        if crate::settings::get(cx).auto_fetch == 0
            || self.auto_fetching.is_some()
            || self.busy.is_some()
            || self.fetched_at.is_some_and(|at| at.elapsed() < gap)
        {
            return false;
        }
        self.fetched_at = Some(Instant::now());
        self.auto_fetching = Some(false);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { git::fetch(&repo) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let show = this.auto_fetching.take() == Some(true);
                let failed_before = std::mem::replace(&mut this.auto_fetch_failed, result.is_err());
                match result {
                    Ok(_) if show => this.toast(Some(true), "Fetched all remotes", cx),
                    Err(e) if show || !failed_before => this.toast(Some(false), e.to_string(), cx),
                    _ => {}
                }
                // The watcher reloads the refs that the fetch moved.
                if this._watcher.is_none() {
                    this.reload(cx);
                }
                cx.notify();
            });
        })
        .detach();
        self.load_prs(cx);
        true
    }

    /// Open pull requests from GitHub. Quietly empty for other hosts.
    fn load_prs(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let r = repo.clone();
            let prs = cx
                .background_executor()
                .spawn(async move { crate::github::list(&r) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.repo.as_ref().map(|r| &r.root) == Some(&repo.root) {
                    this.prs = prs.unwrap_or_default();
                    this.load_pr_status(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Load the checks, reviews and merge states of the pull requests.
    fn load_pr_status(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.prs.is_empty() {
            self.pr_status.clear();
            return;
        }
        self.pr_epoch += 1;
        let epoch = self.pr_epoch;
        // A slow load does not start again on each tick.
        self.pr_status_at = Some(std::time::Instant::now());
        cx.spawn(async move |this, cx| {
            let status = cx
                .background_executor()
                .spawn(async move { crate::github::statuses(&repo) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.pr_epoch {
                    return;
                }
                // Keep the last status when GitHub does not answer.
                if let Ok(status) = status {
                    this.pr_status = status;
                    this.pr_status_at = Some(std::time::Instant::now());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The pull request status is old: a minute while checks run, else
    /// five minutes.
    fn pr_status_due(&self) -> bool {
        use crate::github::CheckState;
        if self.prs.is_empty() {
            return false;
        }
        let running = self
            .pr_status
            .values()
            .any(|s| s.checks.state() == Some(CheckState::Pending));
        let every = if running {
            PR_TICK_RUNNING
        } else {
            PR_TICK_IDLE
        };
        self.pr_status_at.is_none_or(|at| at.elapsed() >= every)
    }

    /// Fetch a pull request's head and browse it like a branch, or with
    /// `review` review it against its base branch.
    fn browse_pr(&mut self, pr: crate::github::PullRequest, review: bool, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.set_busy(Some(format!("Fetching #{}…", pr.number).into()), cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let p = pr.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::github::fetch(&repo, &p) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.set_busy(None, cx);
                match result {
                    Ok(()) if review => this.start_pr_review(&pr, cx),
                    Ok(()) => this.show_target(LogTarget::Ref(pr.refname()), cx),
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pull(&mut self, cx: &mut Context<Self>) {
        self.run_op("Pulling…", Some("Pulled".into()), git::pull, cx);
    }

    fn push(&mut self, cx: &mut Context<Self>) {
        let head = self.head.clone();
        self.run_op(
            "Pushing…",
            Some("Pushed".into()),
            move |repo| git::push(repo, &head),
            cx,
        );
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        if !self.status.iter().any(|e| e.staged.is_some()) {
            self.toast(None, "Stage the changes to commit first.", cx);
            return;
        }
        let msg = self.message.read(cx).value().to_string();
        if msg.trim().is_empty() {
            self.toast(None, "Write a commit message first.", cx);
            return;
        }
        self.set_busy(Some("Committing…".into()), cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { git::commit(&repo, &msg) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.set_busy(None, cx);
                match result {
                    Ok(_) => {
                        this.message.update(cx, |m, cx| m.set_value("", window, cx));
                        this.toast(Some(true), "Committed", cx);
                    }
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }

    // -----------------------------------------------------------------------
    // Rendering

    /// The title bar's part for this tab: the branch, what runs, and Fetch,
    /// Pull and Push.
    pub(super) fn render_repo_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let this = cx.entity();
        let branch_label = self.head_name();
        let locals: Vec<Branch> = self
            .branches
            .iter()
            .filter(|b| b.kind == RefKind::Local)
            .take(40)
            .cloned()
            .collect();
        let switch_this = this.clone();
        let branch_button = button("title-branch")
            .ghost()
            .small()
            .off(self.repo.is_none())
            .child(
                h_flex()
                    .gap_1p5()
                    .child(
                        Icon::new(IconName::GitBranch)
                            .size(px(14.))
                            .text_color(muted),
                    )
                    .child(branch_label)
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(px(12.))
                            .text_color(muted),
                    ),
            )
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu
                    .menu("New Branch…", Box::new(NewBranch))
                    .menu("Clean Up Branches…", Box::new(CleanUpBranches))
                    .separator()
                    .label("Switch branch")
                    .max_h(px(420.))
                    .scrollable(true);
                for b in &locals {
                    let (branch, this) = (b.clone(), switch_this.clone());
                    menu = menu.item(
                        PopupMenuItem::new(b.name.clone())
                            .checked(b.is_head)
                            .on_click(move |_, _, cx| {
                                let branch = branch.clone();
                                this.update(cx, |app, cx| app.switch_branch(branch, cx));
                            }),
                    );
                }
                menu
            });

        let count = |n: u32| {
            (n > 0).then(|| {
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(n.to_string())
            })
        };
        let no_repo = self.repo.is_none();
        let busy = self.busy.clone();
        h_flex()
            .flex_none()
            .h_full()
            .gap_0p5()
            .items_center()
            .when_some(busy, |d, b| {
                d.child(
                    h_flex()
                        .gap_1p5()
                        .mr_2()
                        .text_size(px(12.))
                        .text_color(muted)
                        .child(busy_spinner(muted))
                        .child(b),
                )
            })
            .child(branch_button)
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(t.colors.border))
            .child(
                button("fetch")
                    .ghost()
                    .small()
                    .off(no_repo)
                    .tooltip("Fetch all remotes  ⇧⌘F")
                    .child(
                        h_flex()
                            .gap_1()
                            .child(if self.auto_fetching.is_some() {
                                busy_spinner(muted).with_size(px(14.)).into_any_element()
                            } else {
                                Icon::new(IconName::RefreshCw)
                                    .size(px(14.))
                                    .into_any_element()
                            })
                            .child("Fetch"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.fetch(cx))),
            )
            .child(
                button("pull")
                    .ghost()
                    .small()
                    .off(no_repo)
                    .tooltip("Pull  ⇧⌘P")
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Icon::new(IconName::ArrowDownToLine).size(px(14.)))
                            .child("Pull")
                            .children(count(self.head.behind)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.pull(cx))),
            )
            .child(
                button("push")
                    .ghost()
                    .small()
                    .off(no_repo)
                    .tooltip("Push  ⌘P")
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Icon::new(IconName::ArrowUpFromLine).size(px(14.)))
                            .child("Push")
                            .children(count(self.head.ahead)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.push(cx))),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let path = self
            .repo
            .as_ref()
            .map(|r| crate::clone::tilde(&r.root))
            .unwrap_or_default();
        let n = self.commits.len();
        let commits = if self.log_loading {
            "Loading history…".to_string()
        } else if n >= git::LOG_LIMIT {
            format!("{} commits (limit)", fmt_int(n))
        } else {
            format!("{} commits", fmt_int(n))
        };
        h_flex()
            .h(px(26.))
            .flex_none()
            .px_3()
            .gap_3()
            .border_t_1()
            .border_color(t.colors.border)
            .bg(t.colors.status_bar)
            .text_size(px(11.))
            .text_color(t.colors.muted_foreground)
            .child(div().flex_1().truncate().child(path))
            .when(self.repo.is_some(), |d| d.child(commits))
            .when_some(self.head.sha.as_ref(), |d, sha| {
                d.child(
                    div()
                        .font_family(crate::theme::mono_font(cx))
                        .child(sha[..sha.len().min(7)].to_string()),
                )
            })
    }
}

/// What a tab asks of the window around it.
pub(super) enum AppEvent {
    /// Open this repository in a tab, or show its tab.
    Open(PathBuf),
    /// Open this worktree in a tab, on its Changes view when it is new, or
    /// show its tab.
    OpenWorktree(PathBuf),
    /// This worktree is gone: close its tab.
    Forget(PathBuf),
    /// A git operation started (true) or ended (false).
    Busy(bool),
    /// Show a toast: success (true), error (false) or information (None).
    Toast(Option<bool>, String),
}

impl EventEmitter<AppEvent> for GitApp {}

impl Render for GitApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        if std::mem::take(&mut self.check_inspector) {
            window.defer(cx, |window, cx| window.toggle_inspector(cx));
        }
        // The restore dialog waits for the moves, the cleanup for the refs.
        let waits = match self.check_dialog.as_deref() {
            Some("restore") => self.moves.is_empty(),
            Some("cleanup") => !self.refs_loaded,
            _ => false,
        };
        if let Some(which) = self.check_dialog.take_if(|_| !waits) {
            match which.as_str() {
                "new-branch" => self.new_branch_dialog(None, window, cx),
                "stash" => self.stash_dialog(window, cx),
                "palette" => self.open_palette(window, cx),
                "settings" => settings_ui::open_settings(window, cx),
                "restore" => self.check_restore_dialog(window, cx),
                "cleanup" => self.cleanup_dialog(window, cx),
                _ => {}
            }
        }
        let body = match (&self.repo, self.view) {
            (None, _) => div().into_any_element(),
            (Some(_), view) => split("main-split", false)
                .child(
                    split_panel("main-split", 250., 190.0..420., cx)
                        .child(self.pane(Part::Sidebar, cx)),
                )
                .child(resizable_panel().child(match view {
                    View::Changes => self.render_changes(cx).into_any_element(),
                    View::History => self.render_history(cx).into_any_element(),
                    View::Stash(i) => self.render_stash(i, cx),
                    View::Rebase => self.render_rebase(cx),
                    View::Review => self.render_review(cx),
                    View::Activity => self.render_activity(cx),
                }))
                .into_any_element(),
        };
        v_flex()
            .id("git-app")
            .key_context("GitApp")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| {
                this.reload(cx);
                this.load_prs(cx);
            }))
            .on_action(cx.listener(|this, _: &ShowChanges, _, cx| {
                this.view = View::Changes;
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &ShowHistory, _, cx| this.show_target(LogTarget::Head, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ShowAllBranches, _, cx| {
                    this.show_target(LogTarget::All, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ShowActivity, _, cx| this.show_activity(cx)))
            .on_action(cx.listener(|this, _: &Fetch, _, cx| this.fetch(cx)))
            .on_action(cx.listener(|this, _: &Pull, _, cx| this.pull(cx)))
            .on_action(cx.listener(|this, _: &Push, _, cx| this.push(cx)))
            .on_action(cx.listener(|this, _: &CommitChanges, window, cx| this.commit(window, cx)))
            .on_action(
                cx.listener(|this, _: &TogglePalette, window, cx| this.open_palette(window, cx)),
            )
            .on_action(cx.listener(|this, _: &StashChanges, window, cx| {
                if this.repo.is_some() {
                    this.stash_dialog(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NewBranch, window, cx| {
                if this.repo.is_some() {
                    this.new_branch_dialog(None, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &CleanUpBranches, window, cx| {
                if this.refs_loaded {
                    this.cleanup_dialog(window, cx)
                }
            }))
            .size_full()
            .child(div().flex_1().min_h_0().child(body))
            .child(self.render_status_bar(cx))
    }
}

/// What a segment shows: a word, or an icon with a tooltip.
#[derive(Clone, Copy)]
pub(super) enum Segment {
    Text(&'static str),
    Icon(IconName, &'static str),
}

impl From<&'static str> for Segment {
    fn from(text: &'static str) -> Self {
        Segment::Text(text)
    }
}

/// A segmented control: the chosen option is an accent pill.
pub(super) fn segmented<L: Into<Segment> + Copy, T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(L, T)],
    current: T,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let t = cx.theme();
    let on_change = Rc::new(on_change);
    h_flex()
        .id(id)
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(7.))
        .bg(t.colors.border)
        .children(options.iter().enumerate().map(|(i, &(label, value))| {
            let selected = value == current;
            let cb = on_change.clone();
            let (content, tip) = match label.into() {
                Segment::Text(text) => (text.into_any_element(), None),
                Segment::Icon(icon, tip) => {
                    (Icon::new(icon).size(px(14.)).into_any_element(), Some(tip))
                }
            };
            div()
                .id((id, i))
                .h(px(22.))
                .px_2()
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(12.))
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(t.colors.primary)
                        .text_color(t.colors.primary_foreground)
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!selected, |d| {
                    d.text_color(t.colors.muted_foreground)
                        .hover(|d| d.text_color(t.colors.foreground))
                })
                .child(content)
                .when_some(tip, |d, tip| {
                    d.tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                    })
                })
                .on_click(move |_, window, cx| cb(value, window, cx))
        }))
}

/// The turning circle that shows a git operation runs. It stands still when
/// macOS reduces motion.
fn busy_spinner(color: Hsla) -> Spinner {
    Spinner::new()
        .icon(Icon::new(IconName::LoaderCircle))
        .with_size(px(13.))
        .color(color)
}

/// A split of the tab that remembers the size of its first panel: a drag
/// saves it in the settings, and `split_panel` starts there.
fn split(id: &'static str, vertical: bool) -> ResizablePanelGroup {
    let group = if vertical {
        v_resizable(id)
    } else {
        h_resizable(id)
    };
    group.on_resize(move |state, _, cx| {
        let Some(size) = state.read(cx).sizes().first().copied() else {
            return;
        };
        let size = f32::from(size).round();
        crate::settings::update_layout(cx, |s| {
            s.panes.insert(id.to_string(), size);
        });
    })
}

/// The first panel of the split `id`, at the size it was last dragged to,
/// else at `default`.
fn split_panel(
    id: &'static str,
    default: f32,
    range: std::ops::Range<f32>,
    cx: &App,
) -> ResizablePanel {
    let size = crate::settings::get(cx)
        .panes
        .get(id)
        .copied()
        .unwrap_or(default)
        .clamp(range.start, range.end);
    resizable_panel()
        .size(px(size))
        .size_range(px(range.start)..px(range.end))
}

/// Cancel and a confirm button, right-aligned; both close the dialog.
fn dialog_footer(
    ok: SharedString,
    variant: ButtonVariant,
    on_ok: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_end()
        .gap_2()
        .child(
            button("dialog-cancel")
                .label("Cancel")
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(
            button("dialog-ok")
                .with_variant(variant)
                .label(ok)
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    on_ok(window, cx);
                }),
        )
}

/// The styles of file `ix` of `detail`, when it is short enough to wait for
/// (see `COLORS_WITH_TEXT`).
fn colors_with_text(
    detail: &CommitDetail,
    ix: usize,
    theme: &gpui_kit::component::highlighter::HighlightTheme,
) -> Option<DiffStyles> {
    let file = detail.files.get(ix)?;
    (file.lines.len() <= COLORS_WITH_TEXT).then(|| highlight::compute(file, theme))
}

/// 1234567 -> "1,234,567".
fn fmt_int(n: usize) -> String {
    let digits = n.to_string();
    let groups: Vec<&str> = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|g| std::str::from_utf8(g).unwrap_or_default())
        .collect();
    groups.join(",")
}

/// A short age for narrow rows: "now", "3m", "5h", "2d", then a date.
fn fmt_age(ts: i64) -> String {
    use chrono::{Local, TimeZone as _};
    let Some(at) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    match (Local::now() - at).num_seconds() {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 7 * 86_400 => format!("{}d", s / 86_400),
        _ => at.format("%b %-d").to_string(),
    }
}

/// "3m ago" within a week, then a date.
fn fmt_time(ts: i64) -> String {
    use chrono::{Local, TimeZone as _};
    let Some(at) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    let secs = (Local::now() - at).num_seconds();
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s if s < 7 * 86_400 => format!("{}d ago", s / 86_400),
        _ if at.format("%Y").to_string() == Local::now().format("%Y").to_string() => {
            at.format("%b %-d").to_string()
        }
        _ => at.format("%b %-d, %Y").to_string(),
    }
}

fn fmt_full_time(ts: i64) -> String {
    use chrono::{Local, TimeZone as _};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|at| at.format("%a, %b %-d %Y at %H:%M").to_string())
        .unwrap_or_default()
}
