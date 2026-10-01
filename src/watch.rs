//! Watches a repository (FSEvents through `notify`) and reports what kind
//! of change happened, so the app reloads only what it must.

use std::path::{Path, PathBuf};

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// What changed, in order of how much must reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// Another worktree of the repository: reload the worktree list.
    Worktrees,
    /// Files in the working tree or the index: reload the status.
    Files,
    /// HEAD or refs: reload branches and history too.
    Refs,
}

/// The folders of the watched worktree and of its repository.
pub struct Dirs {
    pub root: PathBuf,
    /// The `.git` folder of this worktree.
    pub git_dir: PathBuf,
    /// The `.git` folder with the refs. It is `git_dir` in the main worktree.
    pub common_dir: PathBuf,
    /// The folders of the other worktrees.
    pub others: Vec<PathBuf>,
}

pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
}

pub fn watch(dirs: &Dirs) -> notify::Result<(RepoWatcher, UnboundedReceiver<Change>)> {
    let (tx, rx) = unbounded();
    let filter = Filter::new(dirs);
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if let Some(change) = event.paths.iter().filter_map(|p| filter.classify(p)).max() {
            let _ = tx.unbounded_send(change);
        }
    })?;
    watcher.watch(&dirs.root, RecursiveMode::Recursive)?;
    let mut watched = vec![dirs.root.clone()];
    let extra = [&dirs.git_dir, &dirs.common_dir]
        .into_iter()
        .chain(&dirs.others);
    for dir in extra {
        if watched.iter().any(|w| dir.starts_with(w)) {
            continue;
        }
        // A folder that is gone (a pruned worktree) is no reason to stop.
        if dir.is_dir() && watcher.watch(dir, RecursiveMode::Recursive).is_ok() {
            watched.push(dir.clone());
        }
    }
    Ok((RepoWatcher { _watcher: watcher }, rx))
}

struct Filter {
    git_dir: PathBuf,
    common_dir: PathBuf,
    /// This worktree first, then the others, each with its ignore rules.
    roots: Vec<(PathBuf, Gitignore)>,
}

impl Filter {
    fn new(dirs: &Dirs) -> Filter {
        let rules = |root: &Path| {
            let mut b = GitignoreBuilder::new(root);
            // Missing files are fine: add() reports them and we move on.
            let _ = b.add(root.join(".gitignore"));
            let _ = b.add(dirs.common_dir.join("info").join("exclude"));
            b.build().unwrap_or_else(|_| Gitignore::empty())
        };
        Filter {
            git_dir: dirs.git_dir.clone(),
            common_dir: dirs.common_dir.clone(),
            roots: std::iter::once(&dirs.root)
                .chain(&dirs.others)
                .map(|root| (root.clone(), rules(root)))
                .collect(),
        }
    }

    fn classify(&self, path: &Path) -> Option<Change> {
        if let Ok(rel) = path.strip_prefix(&self.git_dir) {
            return git_dir_change(rel);
        }
        if let Ok(rel) = path.strip_prefix(&self.common_dir) {
            return common_dir_change(rel);
        }
        // A worktree can be inside another one: the deepest root wins.
        let (ix, (root, ignore)) = self
            .roots
            .iter()
            .enumerate()
            .filter(|(_, (root, _))| path.starts_with(root))
            .max_by_key(|(_, (root, _))| root.as_os_str().len())?;
        let rel = path.strip_prefix(root).ok()?;
        if ignore
            .matched_path_or_any_parents(rel, path.is_dir())
            .is_ignore()
        {
            return None;
        }
        Some(if ix == 0 {
            Change::Files
        } else {
            Change::Worktrees
        })
    }
}

/// Which files inside this worktree's `.git` folder matter, and how much.
/// In the main worktree, that folder also holds the other worktrees' data.
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
        "worktrees" => Some(Change::Worktrees),
        _ => None,
    }
}

/// Which files of the main `.git` folder matter to a linked worktree.
fn common_dir_change(rel: &Path) -> Option<Change> {
    let s = rel.to_string_lossy();
    if s.ends_with(".lock") || s.starts_with("objects") {
        return None;
    }
    match s.split('/').next().unwrap_or("") {
        "packed-refs" | "refs" => Some(Change::Refs),
        // The main worktree and the other linked worktrees.
        "HEAD" | "index" | "worktrees" => Some(Change::Worktrees),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, Dirs, Filter, common_dir_change, git_dir_change};
    use std::path::{Path, PathBuf};

    #[test]
    fn git_dir_paths() {
        assert_eq!(git_dir_change(Path::new("HEAD")), Some(Change::Refs));
        assert_eq!(
            git_dir_change(Path::new("refs/heads/main")),
            Some(Change::Refs)
        );
        assert_eq!(git_dir_change(Path::new("index")), Some(Change::Files));
        assert_eq!(git_dir_change(Path::new("index.lock")), None);
        assert_eq!(git_dir_change(Path::new("objects/ab/cdef")), None);
        assert_eq!(git_dir_change(Path::new("logs/HEAD")), None);
        assert_eq!(
            git_dir_change(Path::new("worktrees/a/index")),
            Some(Change::Worktrees)
        );
        assert_eq!(
            common_dir_change(Path::new("refs/heads/a")),
            Some(Change::Refs)
        );
        assert_eq!(
            common_dir_change(Path::new("index")),
            Some(Change::Worktrees)
        );
        assert_eq!(common_dir_change(Path::new("logs/HEAD")), None);
    }

    #[test]
    fn nested_worktrees_report_as_other_worktrees() {
        // The main worktree at /r, an agent's worktree inside it.
        let main = Filter::new(&Dirs {
            root: PathBuf::from("/r"),
            git_dir: PathBuf::from("/r/.git"),
            common_dir: PathBuf::from("/r/.git"),
            others: vec![PathBuf::from("/r/.agents/one")],
        });
        assert_eq!(main.classify(Path::new("/r/src/a.rs")), Some(Change::Files));
        assert_eq!(
            main.classify(Path::new("/r/.agents/one/b.rs")),
            Some(Change::Worktrees)
        );
        assert_eq!(
            main.classify(Path::new("/r/.git/worktrees/one/HEAD")),
            Some(Change::Worktrees)
        );
        assert_eq!(
            main.classify(Path::new("/r/.git/refs/heads/one")),
            Some(Change::Refs)
        );

        // The same repository, seen from the agent's worktree.
        let agent = Filter::new(&Dirs {
            root: PathBuf::from("/r/.agents/one"),
            git_dir: PathBuf::from("/r/.git/worktrees/one"),
            common_dir: PathBuf::from("/r/.git"),
            others: vec![PathBuf::from("/r")],
        });
        assert_eq!(
            agent.classify(Path::new("/r/.agents/one/b.rs")),
            Some(Change::Files)
        );
        assert_eq!(
            agent.classify(Path::new("/r/src/a.rs")),
            Some(Change::Worktrees)
        );
        assert_eq!(
            agent.classify(Path::new("/r/.git/worktrees/one/index")),
            Some(Change::Files)
        );
        assert_eq!(
            agent.classify(Path::new("/r/.git/worktrees/one/HEAD")),
            Some(Change::Refs)
        );
        assert_eq!(
            agent.classify(Path::new("/r/.git/refs/heads/one")),
            Some(Change::Refs)
        );
        assert_eq!(
            agent.classify(Path::new("/r/.git/index")),
            Some(Change::Worktrees)
        );
    }
}
