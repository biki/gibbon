//! The window: a tab per open repository in the title bar, the shown tab
//! below it, and the welcome screen when no tab is open.

use super::*;
use crate::{CloseTab, NextTab, PrevTab};

struct Tab {
    repo: Repo,
    /// Made when the tab is first shown, so a restart loads only one tab.
    app: Option<Entity<GitApp>>,
    _subs: Vec<Subscription>,
}

/// What the session file last got: window place, tabs, shown tab, and the
/// shown tab's state.
type Snapshot = (
    Option<crate::session::Place>,
    Vec<PathBuf>,
    usize,
    Option<(PathBuf, crate::session::RepoState)>,
);

pub struct Workspace {
    focus: FocusHandle,
    tabs: Vec<Tab>,
    active: usize,
    recent: Vec<PathBuf>,
    toasts: Vec<(Option<bool>, String)>,
    last_session: Option<Snapshot>,
    save_task: Option<Task<()>>,
}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Workspace {
            focus: cx.focus_handle(),
            tabs: vec![],
            active: 0,
            recent: crate::recent::load(),
            toasts: vec![],
            last_session: None,
            save_task: None,
        }
    }

    /// Reopen the tabs of the last session. `path` (from the command line)
    /// opens in a tab too; with neither, the most recent repository opens.
    pub fn restore(&mut self, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let s = crate::session::load();
        self.tabs = s
            .tabs
            .into_iter()
            .filter(|root| root.is_dir())
            .map(|root| Tab {
                repo: Repo {
                    name: root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string()),
                    root,
                },
                app: None,
                _subs: vec![],
            })
            .collect();
        let start = match path {
            Some(p) => Some(p),
            None if self.tabs.is_empty() => self.recent.first().cloned(),
            None => None,
        };
        match start {
            Some(p) => self.open(p, window, cx),
            None if !self.tabs.is_empty() => {
                self.activate(s.active.min(self.tabs.len() - 1), window, cx)
            }
            None => {}
        }
        if let Some(app) = self.active_app().cloned() {
            app.update(cx, |app, cx| app.apply_check_env(cx));
        }
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.active_app() {
            Some(app) => app.read(cx).focus_handle(),
            None => self.focus.clone(),
        }
    }

    fn active_app(&self) -> Option<&Entity<GitApp>> {
        self.tabs.get(self.active)?.app.as_ref()
    }

    /// Show the repository that contains `path`: its tab, or a new tab.
    fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let repo = match Repo::discover(&path) {
            Ok(repo) => repo,
            Err(e) => {
                self.toasts.push((Some(false), format!("{}: {e}", path.display())));
                cx.notify();
                return;
            }
        };
        self.recent = crate::recent::push(&repo.root);
        let ix = match self.tabs.iter().position(|t| t.repo.root == repo.root) {
            Some(ix) => ix,
            None => {
                self.tabs.push(Tab {
                    repo,
                    app: None,
                    _subs: vec![],
                });
                self.tabs.len() - 1
            }
        };
        self.activate(ix, window, cx);
    }

    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Repository".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| this.open(path, window, cx));
        })
        .detach();
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(ix) else {
            return;
        };
        self.active = ix;
        let app = match &tab.app {
            Some(app) => app.clone(),
            None => {
                let app = cx.new(|cx| GitApp::new(window, cx));
                let repo = tab.repo.clone();
                app.update(cx, |app, cx| app.open_repo(repo, cx));
                tab._subs = vec![
                    // The tab strip shows what each tab is doing.
                    cx.observe(&app, |_, _, cx| cx.notify()),
                    cx.subscribe_in(&app, window, |this, _, event, window, cx| match event {
                        AppEvent::Open(path) => this.open(path.clone(), window, cx),
                    }),
                ];
                tab.app = Some(app.clone());
                app
            }
        };
        // Only the shown tab reloads on changes on disk.
        for (i, tab) in self.tabs.iter().enumerate() {
            if let Some(app) = &tab.app {
                app.update(cx, |app, cx| app.set_hidden(i != ix, cx));
            }
        }
        app.read(cx).focus_handle().focus(window, cx);
        cx.notify();
    }

    fn close(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        match shown_after_close(self.active, ix, self.tabs.len()) {
            Some(next) => self.activate(next, window, cx),
            None => {
                self.active = 0;
                self.focus.focus(window, cx);
                cx.notify();
            }
        }
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() as isize;
        if n > 1 {
            let ix = (self.active as isize + delta).rem_euclid(n) as usize;
            self.activate(ix, window, cx);
        }
    }

    /// Toasts of every tab. A background tab's toasts name its repository.
    fn show_toasts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut all = std::mem::take(&mut self.toasts);
        for (ix, tab) in self.tabs.iter().enumerate() {
            let Some(app) = &tab.app else { continue };
            let toasts = app.update(cx, |app, _| std::mem::take(&mut app.toasts));
            all.extend(toasts.into_iter().map(|(ok, msg)| {
                if ix == self.active {
                    (ok, msg)
                } else {
                    (ok, format!("{}: {msg}", tab.repo.name))
                }
            }));
        }
        for (ok, msg) in all {
            window.defer(cx, move |window, cx| {
                let note = match ok {
                    Some(true) => Notification::success(msg),
                    Some(false) => Notification::error(msg),
                    None => Notification::info(msg),
                };
                window.push_notification(note, cx);
            });
        }
    }

    /// Save the session 400 ms after it last changed.
    fn persist_session(&mut self, window: &Window, cx: &mut Context<Self>) {
        let b = window.window_bounds().get_bounds();
        let place = Some(crate::session::Place {
            x: f32::from(b.origin.x).round() as i32,
            y: f32::from(b.origin.y).round() as i32,
            w: f32::from(b.size.width).round() as i32,
            h: f32::from(b.size.height).round() as i32,
        });
        let tabs: Vec<PathBuf> = self.tabs.iter().map(|t| t.repo.root.clone()).collect();
        let repo = self.active_app().and_then(|app| {
            let app = app.read(cx);
            Some((app.repo.as_ref()?.root.clone(), app.repo_state()?))
        });
        let snapshot = (place, tabs, self.active, repo);
        if self.last_session.as_ref() == Some(&snapshot) {
            return;
        }
        self.last_session = Some(snapshot.clone());
        self.save_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            cx.background_executor()
                .spawn(async move {
                    let (place, tabs, active, repo) = snapshot;
                    let repo = repo.as_ref().map(|(root, state)| (root.as_path(), state));
                    crate::session::save(place, &tabs, active, repo)
                })
                .await;
        }));
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let actions = self
            .active_app()
            .cloned()
            .map(|app| app.update(cx, |app, cx| app.render_repo_actions(cx).into_any_element()));
        let tabs: Vec<AnyElement> = (0..self.tabs.len()).map(|ix| self.render_tab(ix, cx)).collect();
        TitleBar::new().child(
            h_flex()
                .w_full()
                .h_full()
                .pr_2()
                .gap_2()
                .items_center()
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .children(tabs),
                )
                .child(self.render_add_button(cx))
                .child(div().flex_1())
                .children(actions),
        )
    }

    fn render_tab(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let tab = &self.tabs[ix];
        let active = ix == self.active;
        let busy = tab
            .app
            .as_ref()
            .is_some_and(|app| app.read(cx).busy.is_some());
        let path: SharedString = tab.repo.root.display().to_string().into();
        h_flex()
            .id(("tab", ix))
            .group("tab")
            // One width for all tabs; they get narrower when many are open.
            .w(px(150.))
            .min_w(px(90.))
            .h(px(26.))
            .pl_2p5()
            .pr_1()
            .gap_1p5()
            .rounded(px(6.))
            .cursor_pointer()
            .when(active, |d| {
                d.bg(t.colors.list_active)
                    .text_color(t.colors.foreground)
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(!active, |d| {
                d.text_color(muted).hover(|d| d.bg(t.colors.list_hover))
            })
            .child(
                Icon::new(if busy {
                    IconName::LoaderCircle
                } else {
                    IconName::FolderGit2
                })
                .size(px(13.))
                .text_color(muted),
            )
            .child(div().flex_1().min_w_0().truncate().child(tab.repo.name.clone()))
            .child(
                div()
                    .flex_none()
                    // Shown on the shown tab, and on the others on hover.
                    .when(!active, |d| d.invisible().group_hover("tab", |s| s.visible()))
                    .child(
                        Button::new(("tab-close", ix))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip("Close tab  ⌘W")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // Not also a click on the tab.
                                cx.stop_propagation();
                                this.close(ix, window, cx)
                            })),
                    ),
            )
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(path.clone()).build(window, cx)
            })
            .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close(ix, window, cx)),
            )
            .into_any_element()
    }

    /// Recent repositories that have no tab, and Open Repository….
    fn render_add_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let closed: Vec<PathBuf> = self
            .recent
            .iter()
            .filter(|p| !self.tabs.iter().any(|t| &t.repo.root == *p))
            .cloned()
            .collect();
        Button::new("tab-add")
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .dropdown_menu(move |mut menu, _, _| {
                if !closed.is_empty() {
                    menu = menu.label("Recent repositories");
                }
                for path in &closed {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let (p, this) = (path.clone(), this.clone());
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, window, cx| {
                        let p = p.clone();
                        this.update(cx, |ws, cx| ws.open(p, window, cx));
                    }));
                }
                if !closed.is_empty() {
                    menu = menu.separator();
                }
                menu.menu("Open Repository…", Box::new(OpenRepo))
            })
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let this = cx.entity();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                Icon::new(IconName::GitGraph)
                    .size(px(40.))
                    .text_color(t.colors.primary),
            )
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Open a repository"),
            )
            .child(
                div()
                    .text_color(muted)
                    .child("Choose a folder that contains a Git repository."),
            )
            .child(
                Button::new("welcome-open")
                    .primary()
                    .label("Open Repository…")
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx))),
            )
            .when(!self.recent.is_empty(), |d| {
                d.child(
                    v_flex()
                        .w(px(360.))
                        .mt_4()
                        .gap_0p5()
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted)
                                .mb_1()
                                .child("RECENT"),
                        )
                        .children(self.recent.iter().enumerate().map(|(i, p)| {
                            let (path, this) = (p.clone(), this.clone());
                            h_flex()
                                .id(("recent", i))
                                .px_2()
                                .py_1p5()
                                .gap_2()
                                .rounded(t.radius)
                                .cursor_pointer()
                                .hover(|d| d.bg(t.colors.list_hover))
                                .child(Icon::new(IconName::FolderGit2).size(px(14.)).text_color(muted))
                                .child(
                                    div().font_weight(FontWeight::MEDIUM).child(
                                        p.file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default(),
                                    ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .child(p.display().to_string()),
                                )
                                .on_click(move |_, window, cx| {
                                    let path = path.clone();
                                    this.update(cx, |ws, cx| ws.open(path, window, cx));
                                })
                        })),
                )
            })
    }
}

/// The tab to show after closing tab `closed` of `len + 1` tabs, while tab
/// `active` was shown. The tab to the right takes the closed tab's place.
fn shown_after_close(active: usize, closed: usize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else if closed < active {
        Some(active - 1)
    } else {
        Some(active.min(len - 1))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.show_toasts(window, cx);
        self.persist_session(window, cx);
        let (bg, fg) = (cx.theme().colors.background, cx.theme().colors.foreground);
        let body = match self.active_app() {
            Some(app) => app.clone().into_any_element(),
            None => self.render_welcome(cx).into_any_element(),
        };
        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &OpenRepo, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.close(this.active, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.step(-1, window, cx)))
            .on_action(|_: &OpenSettings, window, cx| settings_ui::open_settings(window, cx))
            .size_full()
            .bg(bg)
            .text_color(fg)
            .font_family(crate::theme::ui_font(cx))
            .text_size(px(crate::settings::get(cx).ui_size))
            .child(self.render_title_bar(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::shown_after_close;

    #[test]
    fn closing_keeps_the_shown_tab_or_takes_its_neighbour() {
        // Tabs 0..4, tab 2 shown.
        assert_eq!(shown_after_close(2, 0, 3), Some(1), "a tab to the left");
        assert_eq!(shown_after_close(2, 3, 3), Some(2), "a tab to the right");
        assert_eq!(shown_after_close(2, 2, 3), Some(2), "the shown tab: its right neighbour");
        assert_eq!(shown_after_close(3, 3, 3), Some(2), "the shown last tab: its left neighbour");
        assert_eq!(shown_after_close(0, 0, 0), None, "the only tab");
    }
}
