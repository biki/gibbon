//! Blame: who last changed each line of the shown file, in the place of its
//! diff. A click on a line shows the commit that wrote it, at that line, so
//! you can step back through the changes of a line.
//!
//! The blame is of the version that the view shows: the commit, the stash,
//! the staged file, the file on disk, or the branch under review.

use crate::git::{Blame, BlameOf};

use super::diff::DiffFile;
use super::*;

/// Width of the column of the commits beside the lines.
pub(super) const BLAME_W: f32 = 270.;

/// The blame of one file, ready to show.
pub(super) struct BlameData {
    pub(super) blame: Blame,
    /// The lines as a diff of unchanged lines (see `Blame::as_diff`).
    pub(super) file: Rc<FileDiff>,
    pub(super) styles: Rc<DiffStyles>,
    /// How new each commit is, from 0 for the oldest to 1 for the newest.
    pub(super) heat: Vec<f32>,
}

/// `BlameData` without the `Rc`s, which a background task cannot send.
type BlameParts = (Blame, FileDiff, DiffStyles, Vec<f32>);

impl BlameData {
    /// The parts of the blame of `path`. For a background task.
    fn parts(
        blame: Blame,
        path: &str,
        theme: &gpui_kit::component::highlighter::HighlightTheme,
    ) -> BlameParts {
        let file = blame.as_diff(path);
        let text = blame.text.join("\n");
        let styles = if text.len() as u64 > highlight::MAX_FILE_BYTES {
            DiffStyles::default()
        } else {
            highlight::compute(&file, theme, &(None, Some(text)))
        };
        // Lines that are not committed are the newest.
        let mut heat = vec![1.; blame.commits.len()];
        let mut order: Vec<usize> = (0..blame.commits.len())
            .filter(|&i| blame.commits[i].committed())
            .collect();
        order.sort_by_key(|&i| blame.commits[i].time);
        let last = order.len().saturating_sub(1).max(1) as f32;
        for (rank, &i) in order.iter().enumerate() {
            heat[i] = rank as f32 / last;
        }
        (blame, file, styles, heat)
    }

    fn new((blame, file, styles, heat): BlameParts) -> Rc<Self> {
        Rc::new(BlameData {
            blame,
            file: Rc::new(file),
            styles: Rc::new(styles),
            heat,
        })
    }
}

/// What a blame is of: it loads again when this changes.
#[derive(Clone, PartialEq)]
struct BlameKey {
    of: BlameOf,
    path: String,
    /// `DiffFile::content_id` of the diff: a new diff of the file on disk
    /// is a new blame too.
    content: usize,
}

pub(super) struct BlameUi {
    key: BlameKey,
    /// None while the first blame of the file loads. A blame that loads
    /// again keeps the last one until it is done.
    data: Option<Result<Rc<BlameData>, String>>,
}

/// What the diff pane shows in the place of the diff.
pub(super) enum BlameState {
    Loading,
    Ready(Rc<BlameData>),
    Failed(String),
}

/// Git can blame the lines of a text file that still exists.
pub(super) fn blameable(file: &FileDiff) -> bool {
    !file.binary && file.change != git::FileChange::Deleted
}

impl GitApp {
    /// The version of the shown file that a blame is of.
    fn blame_of(&self) -> Option<BlameOf> {
        Some(match self.view {
            View::History => BlameOf::Commit(self.detail.as_ref()?.sha.clone()),
            View::Stash(_) => BlameOf::Commit(self.stash_detail.as_ref()?.sha.clone()),
            View::Changes => match self.change_sel {
                Some((_, true)) => BlameOf::Index,
                _ => BlameOf::Worktree(self.repo.as_ref()?.root.clone()),
            },
            View::Review => {
                let ui = self.review.as_ref()?;
                match &ui.worktree {
                    Some(dir) => BlameOf::Worktree(dir.clone()),
                    None => BlameOf::Commit(ui.target.clone()),
                }
            }
            View::Rebase | View::Activity | View::Agents => return None,
        })
    }

    /// The blame to show in the place of the diff of `file`, when blame is
    /// on. It starts to load when the shown file changes.
    pub(super) fn shown_blame(
        &mut self,
        file: &DiffFile,
        cx: &mut Context<Self>,
    ) -> Option<BlameState> {
        if !self.blame_on || !blameable(file) {
            return None;
        }
        let key = BlameKey {
            of: self.blame_of()?,
            path: file.path.clone(),
            content: file.content_id(),
        };
        if self.blame.as_ref().map(|b| &b.key) != Some(&key) {
            self.load_blame(key, cx);
        }
        Some(match &self.blame.as_ref()?.data {
            None => BlameState::Loading,
            Some(Ok(d)) => BlameState::Ready(d.clone()),
            Some(Err(e)) => BlameState::Failed(e.clone()),
        })
    }

    fn load_blame(&mut self, key: BlameKey, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.blame_epoch += 1;
        let epoch = self.blame_epoch;
        let data = self
            .blame
            .take()
            .filter(|b| b.key.of == key.of && b.key.path == key.path)
            .and_then(|b| b.data);
        self.blame = Some(BlameUi {
            key: key.clone(),
            data,
        });
        let theme = cx.theme().highlight_theme.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let blame = git::blame(&repo, &key.of, &key.path)?;
                    anyhow::Ok(BlameData::parts(blame, &key.path, &theme))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.blame_epoch {
                    return;
                }
                if let Some(ui) = this.blame.as_mut() {
                    ui.data = Some(result.map(BlameData::new).map_err(|e| e.to_string()));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Turn blame on or off for the diff pane of every view.
    pub(super) fn toggle_blame(&mut self, cx: &mut Context<Self>) {
        self.blame_on = !self.blame_on;
        cx.notify();
    }

    /// The commit whose lines a blame marks as new: the shown commit or
    /// stash. Lines that are not committed are always new.
    pub(super) fn blame_own(&self) -> Option<&str> {
        match self.view {
            View::History => self.detail.as_ref().map(|d| d.sha.as_str()),
            View::Stash(_) => self.stash_detail.as_ref().map(|d| d.sha.as_str()),
            _ => None,
        }
    }

    /// Show the commit that wrote line `line` of the blame, in History, at
    /// that line of its diff. Blame turns off: the diff shows the change.
    pub(super) fn show_blamed(&mut self, data: &BlameData, line: usize, cx: &mut Context<Self>) {
        let Some(&(ix, no)) = data.blame.lines.get(line) else {
            return;
        };
        let c = &data.blame.commits[ix];
        if !c.committed() {
            return;
        }
        // The blamed commit is in the history of the blamed version.
        let target = match (self.view, &self.review) {
            (View::History, _) => self.target.clone(),
            (View::Review, Some(ui)) => LogTarget::Ref(ui.target.clone()),
            _ => LogTarget::Head,
        };
        self.blame_on = false;
        self.pending_line = Some((c.sha.clone(), c.path.clone(), no));
        self.pending_path = Some(c.path.clone());
        self.view = View::History;
        let found = (self.target == target)
            .then(|| self.commits.iter().position(|x| x.sha == c.sha))
            .flatten();
        match found {
            Some(ix) => {
                self.selected = HashSet::from([ix]);
                self.anchor = Some(ix);
                self.cursor = Some(ix);
                self.log_scroll.scroll_to_item(ix, ScrollStrategy::Center);
                self.load_detail(cx);
            }
            None => {
                if self.target != target {
                    self.target = target;
                    self.cursor = None;
                    self.selected.clear();
                    self.commits = Rc::new(vec![]);
                }
                self.pending_commit = Some(c.sha.clone());
                self.jump = Some(c.sha.clone());
                self.load_log(cx);
            }
        }
        cx.notify();
    }

    /// The log that loaded for a jump to `sha` does not have it.
    pub(super) fn missed_jump(&mut self, sha: &str, cx: &mut Context<Self>) {
        self.pending_path = None;
        self.pending_line = None;
        let short = &sha[..sha.len().min(7)];
        let msg = if self.query.is_empty() {
            format!("Commit {short} is not in the loaded history.")
        } else {
            format!("Commit {short} does not match the search. Clear the search to see it.")
        };
        self.toast(None, msg, cx);
    }
}
