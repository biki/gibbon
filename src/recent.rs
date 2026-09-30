//! Recently opened repositories, newest first, in
//! `~/Library/Application Support/gibbon/recent.json`.

use std::path::{Path, PathBuf};

const MAX: usize = 10;

fn file() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("gibbon").join("recent.json"))
}

pub fn load() -> Vec<PathBuf> {
    let Some(f) = file() else { return vec![] };
    std::fs::read_to_string(f)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}

/// Move `root` to the front of the list and save it.
pub fn push(root: &Path) -> Vec<PathBuf> {
    let mut list = load();
    list.retain(|p| p != root);
    list.insert(0, root.to_path_buf());
    list.truncate(MAX);
    save(&list);
    list
}

/// Remove all repositories but `keep` from the list and save it.
pub fn clear(keep: &[PathBuf]) -> Vec<PathBuf> {
    let mut list = load();
    list.retain(|p| keep.contains(p));
    save(&list);
    list
}

fn save(list: &[PathBuf]) {
    let Some(f) = file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(f, s);
    }
}
