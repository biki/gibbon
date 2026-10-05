//! Where you left off: the window's place, the open tabs, and per repository
//! the view, the browsed branch, the selected commit and file. Saved in
//! `~/Library/Application Support/gibbon/session.json`, so a restart (or the
//! dev loop's rebuild) opens the same screen again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoState {
    /// "changes", "history", "review", "activity" or "stash:<n>".
    pub view: String,
    /// None for the checked-out branch, "all", or a full ref name.
    pub target: Option<String>,
    pub commit: Option<String>,
    pub file: usize,
    /// A working-tree file: (path, staged side).
    pub change: Option<(String, bool)>,
    /// The shown review, when the view is "review".
    pub review: Option<ReviewState>,
    /// The files marked as viewed, by the ref under review. Each is a path
    /// and a hash of its diff, so a file that changes again is not viewed.
    pub viewed: HashMap<String, Vec<String>>,
    /// The base that you chose for the review of each branch.
    pub review_bases: HashMap<String, String>,
    /// Moves of the branches after this time are new.
    pub activity_seen: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewState {
    /// Full ref names or commits.
    pub target: String,
    pub base: String,
    /// The worktree whose uncommitted changes the review shows.
    pub worktree: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub window: Option<Place>,
    /// The open tabs, left to right.
    pub tabs: Vec<PathBuf>,
    /// The index of the shown tab.
    pub active: usize,
    pub repos: HashMap<PathBuf, RepoState>,
}

fn file() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("gibbon").join("session.json"))
}

pub fn load() -> Session {
    file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save the window place, the tabs and one repository's state, keeping the
/// states of the other repositories.
pub fn save(
    window: Option<Place>,
    tabs: &[PathBuf],
    active: usize,
    repo: Option<(&Path, &RepoState)>,
) {
    let mut s = load();
    if window.is_some() {
        s.window = window;
    }
    s.tabs = tabs.to_vec();
    s.active = active;
    if let Some((root, state)) = repo {
        s.repos.insert(root.to_path_buf(), state.clone());
    }
    let Some(f) = file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(&s) {
        let _ = std::fs::write(f, text);
    }
}
