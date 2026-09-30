//! Watches a repository (FSEvents through `notify`) and reports what kind
//! of change happened, so the app reloads only what it must.

use std::path::{Path, PathBuf};

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// What changed, in order of how much must reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// Files in the working tree or the index: reload the status.
    Files,
    /// HEAD or refs: reload branches and history too.
    Refs,
}

pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
}

/// Start watching `root`; `git_dir` is the repository's `.git` folder.
pub fn watch(root: &Path, git_dir: &Path) -> notify::Result<(RepoWatcher, UnboundedReceiver<Change>)> {
    let (tx, rx) = unbounded();
    let filter = Filter::new(root, git_dir);
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if let Some(change) = event.paths.iter().filter_map(|p| filter.classify(p)).max() {
            let _ = tx.unbounded_send(change);
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    if !git_dir.starts_with(root) {
        watcher.watch(git_dir, RecursiveMode::Recursive)?;
    }
    Ok((RepoWatcher { _watcher: watcher }, rx))
}

struct Filter {
    root: PathBuf,
    git_dir: PathBuf,
    ignore: Gitignore,
}

impl Filter {
    fn new(root: &Path, git_dir: &Path) -> Filter {
        let mut b = GitignoreBuilder::new(root);
        // Missing files are fine: add() reports them and we move on.
        let _ = b.add(root.join(".gitignore"));
        let _ = b.add(git_dir.join("info").join("exclude"));
        Filter {
            root: root.to_path_buf(),
            git_dir: git_dir.to_path_buf(),
            ignore: b.build().unwrap_or_else(|_| Gitignore::empty()),
        }
    }

    fn classify(&self, path: &Path) -> Option<Change> {
        if let Ok(rel) = path.strip_prefix(&self.git_dir) {
            return git_dir_change(rel);
        }
        let rel = path.strip_prefix(&self.root).ok()?;
        let is_dir = path.is_dir();
        if self.ignore.matched_path_or_any_parents(rel, is_dir).is_ignore() {
            return None;
        }
        Some(Change::Files)
    }
}

/// Which files inside `.git` matter, and how much.
fn git_dir_change(rel: &Path) -> Option<Change> {
    let s = rel.to_string_lossy();
    if s.ends_with(".lock") || s.starts_with("objects") {
        return None;
    }
    let first = s.split('/').next().unwrap_or("");
    match first {
        "HEAD" | "packed-refs" | "refs" | "FETCH_HEAD" | "ORIG_HEAD" => Some(Change::Refs),
        "index" | "CHERRY_PICK_HEAD" | "MERGE_HEAD" | "REBASE_HEAD" | "rebase-merge"
        | "rebase-apply" => Some(Change::Files),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, git_dir_change};
    use std::path::Path;

    #[test]
    fn git_dir_paths() {
        assert_eq!(git_dir_change(Path::new("HEAD")), Some(Change::Refs));
        assert_eq!(git_dir_change(Path::new("refs/heads/main")), Some(Change::Refs));
        assert_eq!(git_dir_change(Path::new("index")), Some(Change::Files));
        assert_eq!(git_dir_change(Path::new("index.lock")), None);
        assert_eq!(git_dir_change(Path::new("objects/ab/cdef")), None);
        assert_eq!(git_dir_change(Path::new("logs/HEAD")), None);
    }
}
