//! User settings, saved as JSON in
//! `~/Library/Application Support/gibbon/settings.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    /// Theme colors, an id from `theme::SCHEMES`.
    pub theme: String,
    /// Interface text size in pixels.
    pub ui_size: f32,
    /// Diff text size in pixels.
    pub code_size: f32,
    /// Diffs open side by side.
    pub split_diff: bool,
    /// File lists show a tree of folders, not a flat list.
    pub file_tree: bool,
    /// File lists sort names Z to A.
    pub file_sort_desc: bool,
    /// Interface font, an id from `fonts::UI_FONTS`.
    pub ui_font: String,
    /// Code font, an id from `fonts::CODE_FONTS`.
    pub code_font: String,
    /// The size of the first panel of each split, by the split's id, as the
    /// user last dragged it.
    pub panes: BTreeMap<String, f32>,
    /// The commit detail shows the whole message, not only the subject.
    pub commit_body: bool,
    /// The folder that the last clone went into.
    pub clone_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            appearance: Appearance::System,
            theme: crate::theme::DEFAULT.into(),
            ui_size: 13.,
            code_size: 12.,
            split_diff: false,
            file_tree: false,
            file_sort_desc: false,
            ui_font: crate::fonts::DEFAULT_UI.into(),
            code_font: crate::fonts::DEFAULT_CODE.into(),
            panes: BTreeMap::new(),
            commit_body: false,
            clone_dir: None,
        }
    }
}

impl Global for Settings {}

fn file() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("gibbon").join("settings.json"))
}

pub fn load() -> Settings {
    file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(s: &Settings) {
    let Some(f) = file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(f, text);
    }
}

pub fn get(cx: &App) -> &Settings {
    cx.global::<Settings>()
}

/// Change and save a setting of the layout, which needs no new theme.
pub fn update_layout(cx: &mut App, f: impl FnOnce(&mut Settings)) {
    let s = cx.global_mut::<Settings>();
    f(s);
    save(s);
}

/// Change, save and apply the settings.
pub fn update(cx: &mut App, f: impl FnOnce(&mut Settings)) {
    let s = cx.global_mut::<Settings>();
    f(s);
    save(s);
    crate::theme::apply(cx);
    cx.refresh_windows();
}
