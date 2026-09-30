use std::path::PathBuf;

use gpui_kit::assets::AllAssets;
use gpui_kit::component::{Root, TitleBar};
use gpui_kit::*;

mod app;
mod git;
mod github;
mod graph;
mod highlight;
mod recent;
mod settings;
mod theme;
mod watch;

actions!(
    gibbon,
    [
        Quit,
        HideApp,
        OpenRepo,
        Refresh,
        ShowChanges,
        ShowHistory,
        ShowAllBranches,
        Fetch,
        Pull,
        Push,
        CommitChanges,
        NewBranch,
        StashChanges,
        TogglePalette,
        OpenSettings,
        SelectPrev,
        SelectNext,
    ]
);

fn main() {
    // Started from Finder, the app gets a minimal PATH: add Homebrew so
    // `git` and `gh` resolve as they do in the terminal.
    let path = std::env::var("PATH").unwrap_or_default();
    let extra: Vec<&str> = ["/opt/homebrew/bin", "/usr/local/bin"]
        .into_iter()
        .filter(|p| !path.split(':').any(|q| q == *p))
        .collect();
    if !extra.is_empty() {
        let full = format!("{}:{path}", extra.join(":"));
        // SAFETY: no other threads exist yet.
        unsafe { std::env::set_var("PATH", full) };
    }
    migrate_data_dir();
    // `gibbon <path>` opens that repository; otherwise the last one.
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let app = gpui_kit::application().with_assets(AllAssets);
    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        bind_keys(cx);
        install_menus(cx);
        cx.set_global(settings::load());
        if let Err(e) = theme::load_fonts(cx) {
            eprintln!("font load error: {e:#}");
        }
        theme::apply(cx);
        open_window(cx, arg.clone());
        if !background() {
            cx.activate(true);
        }
    });
}

/// Settings and recent repositories lived in `gittool/` before the rename.
fn migrate_data_dir() {
    let Some(base) = dirs::data_dir() else { return };
    let (old, new) = (base.join("gittool"), base.join("gibbon"));
    if old.is_dir() && !new.exists() {
        let _ = std::fs::rename(old, new);
    }
}

/// `GIBBON_BACKGROUND=1`: never take focus (automated UI checks).
fn background() -> bool {
    std::env::var_os("GIBBON_BACKGROUND").is_some()
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-o", OpenRepo, Some("GitApp")),
        KeyBinding::new("cmd-r", Refresh, Some("GitApp")),
        KeyBinding::new("cmd-1", ShowChanges, Some("GitApp")),
        KeyBinding::new("cmd-2", ShowHistory, Some("GitApp")),
        KeyBinding::new("cmd-3", ShowAllBranches, Some("GitApp")),
        KeyBinding::new("cmd-shift-f", Fetch, Some("GitApp")),
        KeyBinding::new("cmd-shift-p", Pull, Some("GitApp")),
        KeyBinding::new("cmd-p", Push, Some("GitApp")),
        KeyBinding::new("cmd-enter", CommitChanges, Some("GitApp")),
        KeyBinding::new("cmd-shift-n", NewBranch, Some("GitApp")),
        KeyBinding::new("cmd-alt-s", StashChanges, Some("GitApp")),
        KeyBinding::new("cmd-k", TogglePalette, Some("GitApp")),
        KeyBinding::new("cmd-,", OpenSettings, Some("GitApp")),
        KeyBinding::new("up", SelectPrev, Some("CommitList")),
        KeyBinding::new("down", SelectNext, Some("CommitList")),
    ]);
}

fn install_menus(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.set_menus([
        Menu::new("Gibbon").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Command Palette…", TogglePalette),
            MenuItem::separator(),
            MenuItem::action("Hide Gibbon", HideApp),
            MenuItem::action("Quit Gibbon", Quit),
        ]),
        Menu::new("File").items([MenuItem::action("Open Repository…", OpenRepo)]),
        Menu::new("View").items([
            MenuItem::action("Changes", ShowChanges),
            MenuItem::action("History", ShowHistory),
            MenuItem::action("All Branches", ShowAllBranches),
            MenuItem::separator(),
            MenuItem::action("Refresh", Refresh),
        ]),
        Menu::new("Repository").items([
            MenuItem::action("Fetch", Fetch),
            MenuItem::action("Pull", Pull),
            MenuItem::action("Push", Push),
            MenuItem::separator(),
            MenuItem::action("New Branch…", NewBranch),
            MenuItem::action("Stash Changes…", StashChanges),
            MenuItem::action("Commit", CommitChanges),
        ]),
    ]);
}

fn open_window(cx: &mut App, path: Option<PathBuf>) {
    let bounds = Bounds::centered(None, size(px(1320.), px(840.)), cx);
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(900.), px(560.))),
            focus: !background(),
            // Occluded windows stop painting: keep UI-check windows on top.
            kind: if background() {
                WindowKind::PopUp
            } else {
                WindowKind::Normal
            },
            ..TitleBar::window_options()
        },
        |window, cx| {
            let view = cx.new(|cx| app::GitApp::new(window, cx));
            let start = path.or_else(|| recent::load().into_iter().next());
            if let Some(p) = start {
                view.update(cx, |app, cx| {
                    app.open(p, cx);
                    app.apply_check_env(cx);
                });
            }
            window
                .observe_window_appearance(|_, cx| {
                    theme::apply(cx);
                    cx.refresh_windows();
                })
                .detach();
            view.read(cx).focus_handle().focus(window, cx);
            cx.new(|cx| Root::new(view, window, cx))
        },
    );
    if let Err(e) = result {
        eprintln!("failed to open window: {e}");
    }
}
