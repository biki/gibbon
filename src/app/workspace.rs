//! The window: a tab per open repository in the title bar, the shown tab
//! below it, and the welcome screen when no tab is open.

use std::path::Path;

use gpui_kit::component::menu::ContextMenuExt as _;

use super::*;
use crate::{CloseTab, NextTab, PrevTab};

struct Tab {
    repo: Repo,
    /// Made when the tab is first shown, so a restart loads only one tab.
    app: Option<Entity<GitApp>>,
    /// A git operation runs (`AppEvent::Busy`). Render reads this flag, not
    /// the tab: a tab that render reads redraws the window on each change.
    busy: bool,
    _subs: Vec<Subscription>,
}

/// A tab while the pointer drags it. GPUI draws it under the pointer, and
/// it stays in the tab strip: it moves only sideways, as far as the tabs go.
#[derive(Clone)]
struct DraggedTab {
    name: SharedString,
    busy: bool,
    /// Where the pointer took the tab, from its top left corner.
    grab: Point<Pixels>,
    /// The tab's bounds when the drag started.
    start: Bounds<Pixels>,
    /// The left and right edges of the tabs.
    strip: (Pixels, Pixels),
}

impl Render for DraggedTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // GPUI puts the top left corner at `at`.
        let at = window.mouse_position() - self.grab;
        let width = self.start.size.width;
        let left = at.x.min(self.strip.1 - width).max(self.strip.0);
        // GPUI draws it outside the workspace, which sets the font.
        tab_frame(true, cx)
            .relative()
            .left(left - at.x)
            .top(self.start.top() - at.y)
            .w(width)
            .pr_2p5()
            .shadow_md()
            .font_family(crate::theme::ui_font(cx))
            .text_size(px(crate::settings::get(cx).ui_size))
            .children(tab_label(
                self.name.clone(),
                self.busy,
                cx.theme().colors.muted_foreground,
            ))
    }
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
    /// Where the tabs were in the last frame.
    tab_bounds: Vec<Bounds<Pixels>>,
    /// The tab that the pointer drags, and where the pointer took it, from
    /// the tab's left edge. Render clears it when no drag runs.
    dragged: Option<(PathBuf, Pixels)>,
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
            tab_bounds: vec![],
            dragged: None,
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
                busy: false,
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
                self.toasts
                    .push((Some(false), format!("{}: {e}", path.display())));
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
                    busy: false,
                    _subs: vec![],
                });
                self.tabs.len() - 1
            }
        };
        self.activate(ix, window, cx);
    }

    /// Forget the recent repositories, but not the ones open in a tab.
    fn clear_recent(&mut self, cx: &mut Context<Self>) {
        let open: Vec<PathBuf> = self.tabs.iter().map(|t| t.repo.root.clone()).collect();
        self.recent = crate::recent::clear(&open);
        cx.notify();
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
                // Before opening: opening can already send events.
                tab._subs = vec![cx.subscribe_in(&app, window, Self::on_app_event)];
                let repo = tab.repo.clone();
                app.update(cx, |app, cx| app.open_repo(repo, cx));
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

    /// Close all tabs but tab `ix`.
    fn close_others(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.truncate(ix + 1);
        self.tabs.drain(..ix);
        self.activate(0, window, cx);
    }

    fn tab_of(&self, root: &Path) -> Option<usize> {
        self.tabs.iter().position(|t| t.repo.root == root)
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() as isize;
        if n > 1 {
            let ix = (self.active as isize + delta).rem_euclid(n) as usize;
            self.activate(ix, window, cx);
        }
    }

    /// Put the dragged tab where its middle is, with the pointer at `x`.
    /// The tabs between take one step towards the tab's old place.
    fn drag_tab(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some((root, grab)) = self.dragged.clone() else {
            return;
        };
        let Some(from) = self.tab_of(&root) else {
            return;
        };
        let Some(width) = self.tab_bounds.get(from).map(|b| b.size.width) else {
            return;
        };
        match slot_at(&self.tab_bounds, x - grab + width / 2.) {
            Some(to) if to != from && to < self.tabs.len() => {
                let tab = self.tabs.remove(from);
                self.tabs.insert(to, tab);
                self.active = index_after_move(self.active, from, to);
                cx.notify();
            }
            _ => {}
        }
    }

    fn on_app_event(
        &mut self,
        app: &Entity<GitApp>,
        event: &AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|t| t.app.as_ref() == Some(app)) else {
            return;
        };
        match event {
            AppEvent::Open(path) => self.open(path.clone(), window, cx),
            AppEvent::OpenWorktree(path) => {
                let new = self.tab_of(path).is_none();
                self.open(path.clone(), window, cx);
                if new && let Some(app) = self.active_app().cloned() {
                    app.update(cx, |app, cx| app.show_changes_if_new(cx));
                }
            }
            AppEvent::Forget(path) => {
                if let Some(ix) = self.tab_of(path) {
                    self.close(ix, window, cx);
                }
            }
            AppEvent::Busy(busy) => {
                self.tabs[ix].busy = *busy;
                cx.notify();
            }
            AppEvent::Toast(ok, msg) => {
                // A hidden tab's toasts name its repository.
                let msg = if ix == self.active {
                    msg.clone()
                } else {
                    format!("{}: {msg}", self.tabs[ix].repo.name)
                };
                self.toasts.push((*ok, msg));
                cx.notify();
            }
        }
    }

    /// Toasts of the tabs and of the window.
    fn show_toasts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (ok, msg) in std::mem::take(&mut self.toasts) {
            window.defer(cx, move |window, cx| {
                window.push_notification(toast(ok, msg), cx)
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
        let tabs: Vec<AnyElement> = (0..self.tabs.len())
            .map(|ix| self.render_tab(ix, cx))
            .collect();
        let this = cx.entity();
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
                        .children(tabs)
                        .on_children_prepainted(move |bounds, _, cx| {
                            this.update(cx, |this, _| this.tab_bounds = bounds)
                        })
                        .on_drag_move(cx.listener(|this, e: &DragMoveEvent<DraggedTab>, _, cx| {
                            this.drag_tab(e.event.position.x, cx)
                        })),
                )
                .child(self.render_add_button(cx))
                .child(div().flex_1())
                .children(actions),
        )
    }

    fn render_tab(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let tab = &self.tabs[ix];
        let active = ix == self.active;
        let root = tab.repo.root.clone();
        let path: SharedString = root.display().to_string().into();
        let name: SharedString = tab.repo.name.clone().into();
        // Its place stays empty while the pointer drags it.
        let dragged = self.dragged.as_ref().is_some_and(|(r, _)| *r == root);
        let drag = DraggedTab {
            name: name.clone(),
            busy: tab.busy,
            grab: Point::default(),
            start: Bounds::default(),
            strip: Default::default(),
        };
        let this = cx.entity();
        let (menu_ws, menu_root, alone) = (this.clone(), root.clone(), self.tabs.len() < 2);
        tab_frame(active, cx)
            .id(("tab", ix))
            .group("tab")
            // One width for all tabs; they get narrower when many are open.
            .w(px(150.))
            .min_w(px(90.))
            .cursor_pointer()
            .when(dragged, |d| d.invisible())
            .when(!active, |d| {
                d.child(hover_fill(t.colors.list_hover, px(6.)))
            })
            .children(tab_label(name, tab.busy, t.colors.muted_foreground))
            .child(
                div()
                    .flex_none()
                    // Shown on the shown tab, and on the others on hover.
                    .when(!active, |d| {
                        d.invisible().group_hover("tab", |s| s.visible())
                    })
                    .child(
                        button(("tab-close", ix))
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
            // A drag moves the tab, not the window: the title bar moves the
            // window on a drag that starts in it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close(ix, window, cx)),
            )
            .on_drag(drag, move |drag, grab, _, cx| {
                let (start, strip) = this.update(cx, |this, cx| {
                    this.dragged = Some((root.clone(), grab.x));
                    cx.notify();
                    let tabs = &this.tab_bounds;
                    let strip = match (tabs.first(), tabs.last()) {
                        (Some(first), Some(last)) => (first.left(), last.right()),
                        _ => Default::default(),
                    };
                    (tabs.get(ix).copied().unwrap_or_default(), strip)
                });
                cx.new(|_| DraggedTab {
                    grab,
                    start,
                    strip,
                    ..drag.clone()
                })
            })
            .context_menu(move |menu, _, _| {
                // The menu finds its tab when an item is chosen: the tabs
                // can move while it is open.
                let (ws, root) = (menu_ws.clone(), menu_root.clone());
                let (ws2, root2, dir) = (ws.clone(), root.clone(), root.clone());
                menu.item(
                    PopupMenuItem::new("Close Tab").on_click(move |_, window, cx| {
                        ws.update(cx, |ws, cx| {
                            if let Some(ix) = ws.tab_of(&root) {
                                ws.close(ix, window, cx)
                            }
                        })
                    }),
                )
                .item(
                    PopupMenuItem::new("Close Other Tabs")
                        .disabled(alone)
                        .on_click(move |_, window, cx| {
                            ws2.update(cx, |ws, cx| {
                                if let Some(ix) = ws.tab_of(&root2) {
                                    ws.close_others(ix, window, cx)
                                }
                            })
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Open in Finder").on_click(move |_, _, _| {
                        let _ = std::process::Command::new("open").arg(&dir).spawn();
                    }),
                )
            })
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
        button("tab-add")
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
                    let this = this.clone();
                    menu = menu
                        .separator()
                        .item(
                            PopupMenuItem::new("Clear Recent").on_click(move |_, _, cx| {
                                this.update(cx, |ws, cx| ws.clear_recent(cx));
                            }),
                        )
                        .separator();
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
                button("welcome-open")
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
                            h_flex()
                                .mb_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(muted)
                                        .child("RECENT"),
                                )
                                .child(
                                    button("recent-clear")
                                        .ghost()
                                        .xsmall()
                                        .label("Clear")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.clear_recent(cx)),
                                        ),
                                ),
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
                                .child(hover_fill(t.colors.list_hover, t.radius))
                                .child(
                                    Icon::new(IconName::FolderGit2)
                                        .size(px(14.))
                                        .text_color(muted),
                                )
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

/// The box of a tab, filled while the tab is shown.
fn tab_frame(filled: bool, cx: &App) -> Div {
    let t = cx.theme();
    h_flex()
        .h(px(26.))
        .pl_2p5()
        .pr_1()
        .gap_1p5()
        .rounded(px(6.))
        .when(filled, |d| {
            d.bg(t.colors.list_active)
                .text_color(t.colors.foreground)
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!filled, |d| d.text_color(t.colors.muted_foreground))
}

/// A tab's icon and name. A spinner is the icon while a git operation runs.
fn tab_label(name: SharedString, busy: bool, muted: Hsla) -> [AnyElement; 2] {
    let icon = if busy {
        busy_spinner(muted).into_any_element()
    } else {
        Icon::new(IconName::FolderGit2)
            .size(px(13.))
            .text_color(muted)
            .into_any_element()
    };
    [
        icon,
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .child(name)
            .into_any_element(),
    ]
}

/// A toast with its icon centered on the first line of the message.
/// gpui-kit puts the icon of `Notification::success` 18 px from the top,
/// which is below the line when the interface size is not 16 px.
fn toast(ok: Option<bool>, msg: String) -> Notification {
    let msg = SharedString::from(msg);
    Notification::new().content(move |_, _, cx| {
        let t = cx.theme();
        let (icon, color) = match ok {
            Some(true) => (IconName::CircleCheck, t.colors.success),
            Some(false) => (IconName::CircleX, t.colors.danger),
            None => (IconName::Info, t.colors.info),
        };
        // The icon box is one line high, so a long message wraps below it.
        let line = rems(1.25);
        h_flex()
            .items_start()
            .gap_3()
            .text_sm()
            .line_height(line)
            .child(
                h_flex()
                    .flex_none()
                    .h(line)
                    .child(Icon::new(icon).text_color(color)),
            )
            .child(div().flex_1().min_w_0().child(msg.clone()))
            .into_any_element()
    })
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

/// The new place of tab `ix` after tab `from` moves to `to`.
fn index_after_move(ix: usize, from: usize, to: usize) -> usize {
    if ix == from {
        to
    } else if from < ix && ix <= to {
        ix - 1
    } else if to <= ix && ix < from {
        ix + 1
    } else {
        ix
    }
}

/// The tab at `x`, of tabs with these bounds: the first or last tab past
/// the ends, and `None` in the gap between two tabs.
fn slot_at(tabs: &[Bounds<Pixels>], x: Pixels) -> Option<usize> {
    let (first, last) = (tabs.first()?, tabs.last()?);
    if x < first.left() {
        Some(0)
    } else if x >= last.right() {
        Some(tabs.len() - 1)
    } else {
        tabs.iter().position(|b| b.left() <= x && x < b.right())
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.show_toasts(window, cx);
        self.persist_session(window, cx);
        // The window draws a frame when a drag ends, so the dragged tab
        // shows again.
        if !cx.has_active_drag() {
            self.dragged = None;
        }
        let (bg, fg) = (cx.theme().colors.background, cx.theme().colors.foreground);
        let body = match self.active_app() {
            // Not cached: GPUI renders every cached view in a cached view
            // that renders again. The parts of the tab are cached views (see
            // `pane`), so when only the window changes (the tab strip, a
            // toast), the tab renders only the frame around them.
            Some(app) => app.clone().into_any_element(),
            None => self.render_welcome(cx).into_any_element(),
        };
        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &OpenRepo, window, cx| this.prompt_open(window, cx)))
            .on_action(
                cx.listener(|this, _: &CloseTab, window, cx| this.close(this.active, window, cx)),
            )
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
    use super::{index_after_move, shown_after_close, slot_at};
    use gpui_kit::{Bounds, point, px, size};

    #[test]
    fn moving_a_tab_shifts_the_tabs_between() {
        // Tab 1 moves to 3: tabs 2 and 3 take a step to the left.
        let moved: Vec<usize> = (0..5).map(|ix| index_after_move(ix, 1, 3)).collect();
        assert_eq!(moved, [0, 3, 1, 2, 4]);
        // Tab 3 moves to 1: tabs 1 and 2 take a step to the right.
        let moved: Vec<usize> = (0..5).map(|ix| index_after_move(ix, 3, 1)).collect();
        assert_eq!(moved, [0, 2, 3, 1, 4]);
    }

    #[test]
    fn a_point_finds_the_tab_under_it() {
        // Tabs 100 px wide with 4 px between them, from x = 10.
        let tab = |i: f32| Bounds::new(point(px(10. + 104. * i), px(0.)), size(px(100.), px(26.)));
        let tabs = [tab(0.), tab(1.), tab(2.)];
        assert_eq!(slot_at(&tabs, px(0.)), Some(0), "left of the tabs");
        assert_eq!(slot_at(&tabs, px(60.)), Some(0));
        assert_eq!(slot_at(&tabs, px(112.)), None, "between two tabs");
        assert_eq!(slot_at(&tabs, px(114.)), Some(1));
        assert_eq!(slot_at(&tabs, px(500.)), Some(2), "right of the tabs");
        assert_eq!(slot_at(&[], px(0.)), None, "no tabs");
    }

    #[test]
    fn closing_keeps_the_shown_tab_or_takes_its_neighbour() {
        // Tabs 0..4, tab 2 shown.
        assert_eq!(shown_after_close(2, 0, 3), Some(1), "a tab to the left");
        assert_eq!(shown_after_close(2, 3, 3), Some(2), "a tab to the right");
        assert_eq!(
            shown_after_close(2, 2, 3),
            Some(2),
            "the shown tab: its right neighbour"
        );
        assert_eq!(
            shown_after_close(3, 3, 3),
            Some(2),
            "the shown last tab: its left neighbour"
        );
        assert_eq!(shown_after_close(0, 0, 0), None, "the only tab");
    }
}
