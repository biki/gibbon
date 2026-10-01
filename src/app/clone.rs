//! The Clone dialog: paste a URL, type `owner/name`, or choose one of your
//! GitHub repositories, then the folder for the clone. The window opens the
//! clone in a tab when it is done.

use std::path::{Path, PathBuf};

use futures::StreamExt as _;
use gpui_kit::component::input::{Input, MoveDown, MoveUp};
use gpui_kit::component::progress::Progress;

use super::*;
use crate::clone::{self, Source};
use crate::github::{GhError, RemoteRepo};

/// What `gh` told about the user.
enum Account {
    Loading,
    SignedIn {
        login: String,
        repos: Vec<RemoteRepo>,
    },
    Unavailable(GhError),
}

/// A clone that runs.
struct Running {
    job: clone::Job,
    progress: clone::Progress,
    /// The user stopped it, and it has not ended yet.
    stopping: bool,
}

pub(super) struct CloneDialog {
    query: Entity<InputState>,
    path: Entity<InputState>,
    account: Account,
    /// The chosen row of the list, by `owner/name`.
    selected: Option<String>,
    /// The folder that the clone goes into, unless the user typed a path.
    parent: PathBuf,
    /// The user typed the path: another repository no longer changes it.
    path_typed: bool,
    running: Option<Running>,
    /// Why the last clone did not start or failed.
    error: Option<String>,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
    _load: Task<()>,
    _clone: Option<Task<()>>,
}

/// A clone is done.
pub(super) struct Cloned {
    pub path: PathBuf,
    /// `owner/name`, or the folder name, for the toast.
    pub name: String,
}

impl EventEmitter<Cloned> for CloneDialog {}

impl CloneDialog {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search your repositories, or paste a URL")
        });
        let path = cx.new(|cx| InputState::new(window, cx));
        // Enter reaches the dialog's `on_ok` (see `open`).
        let subs = vec![
            cx.subscribe_in(&query, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::Change = ev {
                    this.error = None;
                    this.reselect(window, cx);
                }
            }),
            cx.subscribe_in(&path, window, |this, state, ev: &InputEvent, _, cx| {
                // Only the user changes the text: `set_value` sends no
                // event. An empty field follows the repository again.
                if let InputEvent::Change = ev {
                    this.path_typed = !state.read(cx).value().trim().is_empty();
                    this.error = None;
                    cx.notify();
                }
            }),
        ];
        let load = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { crate::github::your_repos() })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.account = match result {
                    Ok((login, repos)) => Account::SignedIn { login, repos },
                    Err(e) => {
                        this.query.update(cx, |s, cx| {
                            s.set_placeholder("URL or owner/name", window, cx)
                        });
                        Account::Unavailable(e)
                    }
                };
                this.reselect(window, cx);
            });
        });
        let parent = crate::settings::get(cx)
            .clone_dir
            .clone()
            .filter(|d| d.is_dir())
            .unwrap_or_else(clone::default_parent);
        let mut this = CloneDialog {
            query,
            path,
            account: Account::Loading,
            selected: None,
            parent,
            path_typed: false,
            running: None,
            error: None,
            scroll: ScrollHandle::new(),
            _subs: subs,
            _load: load,
            _clone: None,
        };
        this.update_path(window, cx);
        this
    }

    /// The dialog reads the entity on each frame, so a change shows at once.
    pub(super) fn open(this: Entity<Self>, window: &mut Window, cx: &mut App) {
        let query = this.read(cx).query.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let d = this.read(cx);
            // A running clone ends with Stop only.
            let idle = d.running.is_none();
            let start = this.clone();
            dialog
                .title("Clone repository")
                .w(px(560.))
                .keyboard(idle)
                .close_button(idle)
                .overlay_closable(idle)
                // A text field passes Enter on to the dialog, which closes
                // by default: the drop would stop the clone that Enter
                // starts. Enter clones, and never closes the dialog.
                .on_ok(move |_, window, cx| {
                    start.update(cx, |d, cx| d.start(window, cx));
                    false
                })
                .child(d.render_body(&this, cx))
                .footer(d.render_footer(&this, cx))
        });
        query.update(cx, |s, cx| s.focus(window, cx));
    }

    /// The repositories that match the search, the last pushed first.
    fn rows(&self, cx: &App) -> Vec<&RemoteRepo> {
        let Account::SignedIn { repos, .. } = &self.account else {
            return vec![];
        };
        let query = self.query.read(cx).value();
        let needle = match Source::parse(&query) {
            // The URL of a GitHub repository finds its row.
            Some(Source {
                github: Some(full),
                spec,
                ..
            }) if spec.contains(':') => full,
            _ => query.trim().to_string(),
        }
        .to_lowercase();
        repos
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&needle))
            .collect()
    }

    /// What Clone clones: the chosen row, else what the user typed.
    fn source(&self, cx: &App) -> Option<Source> {
        let rows = self.rows(cx);
        match self
            .selected
            .as_ref()
            .filter(|s| rows.iter().any(|r| &r.name == *s))
        {
            Some(name) => Source::parse(name),
            None => Source::parse(&self.query.read(cx).value()),
        }
    }

    /// The folder for the clone, or why it cannot be there.
    fn dest(&self, cx: &App) -> Result<PathBuf, String> {
        let text = self.path.read(cx).value().trim().to_string();
        if text.is_empty() {
            return Err("Type the folder for the clone.".into());
        }
        let path = clone::expand_home(&text);
        if !path.is_absolute() {
            return Err("Type a full path, such as ~/Developer/name.".into());
        }
        // Git clones into an empty folder too.
        let taken =
            path.is_file() || std::fs::read_dir(&path).is_ok_and(|mut d| d.next().is_some());
        if taken {
            return Err(format!("{} exists already.", clone::tilde(&path)));
        }
        Ok(path)
    }

    /// Choose a row after the search changed: the row of the repository
    /// that the user typed, else the first row that the search finds.
    fn reselect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        let rows = self.rows(cx);
        let selected = match Source::parse(&query) {
            Some(typed) => typed.github.and_then(|full| {
                rows.iter()
                    .find(|r| r.name.eq_ignore_ascii_case(&full))
                    .map(|r| r.name.clone())
            }),
            None if query.trim().is_empty() => None,
            None => rows.first().map(|r| r.name.clone()),
        };
        self.selected = selected;
        self.scroll.scroll_to_item(0);
        self.update_path(window, cx);
        cx.notify();
    }

    /// Move the choice up or down the list (↑ and ↓ in the search field).
    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        let rows = self.rows(cx);
        let at = self
            .selected
            .as_ref()
            .and_then(|s| rows.iter().position(|r| &r.name == s));
        let ix = match at {
            Some(i) => (i as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
            None if !rows.is_empty() => 0,
            None => return,
        };
        self.selected = Some(rows[ix].name.clone());
        self.scroll.scroll_to_item(ix);
        self.update_path(window, cx);
        cx.notify();
    }

    fn select(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        self.selected = Some(name);
        self.error = None;
        self.update_path(window, cx);
        cx.notify();
    }

    /// Show the folder of the clone, unless the user typed one.
    fn update_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path_typed {
            return;
        }
        let path = match self.source(cx) {
            Some(s) => self.parent.join(&s.name),
            None => self.parent.clone(),
        };
        let text = clone::tilde(&path);
        self.path.update(cx, |p, cx| {
            if p.value().as_ref() != text.as_str() {
                p.set_value(text, window, cx)
            }
        });
    }

    fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.parent = dir;
                this.path_typed = false;
                this.error = None;
                this.update_path(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        let Some(source) = self.source(cx) else {
            return;
        };
        let dest = match self.dest(cx) {
            Ok(dest) => dest,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        // Until `gh` answers, git clones.
        let use_gh = matches!(self.account, Account::SignedIn { .. });
        let name = source.github.clone().unwrap_or_else(|| source.name.clone());
        let (job, mut events) = match clone::start(&source, &dest, use_gh) {
            Ok(started) => started,
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        self.error = None;
        self.running = Some(Running {
            job,
            progress: clone::Progress {
                line: format!("Cloning {name}…"),
                done: 0.,
            },
            stopping: false,
        });
        self._clone = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.next().await {
                let done = matches!(event, clone::Event::Done(_));
                let _ = this.update_in(cx, |this, window, cx| {
                    this.on_event(event, &dest, &name, window, cx)
                });
                if done {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn on_event(
        &mut self,
        event: clone::Event,
        dest: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            clone::Event::Progress(p) => {
                if let Some(r) = self.running.as_mut().filter(|r| !r.stopping) {
                    r.progress = p;
                }
            }
            clone::Event::Done(result) => {
                let stopped = self.running.take().is_some_and(|r| r.stopping);
                match result {
                    Ok(()) => {
                        if let Some(parent) = dest.parent().map(Path::to_path_buf) {
                            crate::settings::update_layout(cx, |s| s.clone_dir = Some(parent));
                        }
                        cx.emit(Cloned {
                            path: dest.to_path_buf(),
                            name: name.to_string(),
                        });
                        window.close_dialog(cx);
                    }
                    Err(_) if stopped => {}
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }
        cx.notify();
    }

    /// Stop the clone. Git deletes the folder that it made.
    fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(r) = &mut self.running {
            r.job.stop();
            r.stopping = true;
            r.progress.line = "Stopping…".into();
            cx.notify();
        }
    }

    fn render_body(&self, this: &Entity<Self>, cx: &App) -> impl IntoElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let running = self.running.is_some();
        let (up, down, choose) = (this.clone(), this.clone(), this.clone());
        // ↑ and ↓ move in the list, not in the field.
        let search = div()
            .capture_action(move |_: &MoveUp, window, cx| {
                up.update(cx, |d, cx| d.step(-1, window, cx));
                cx.stop_propagation();
            })
            .capture_action(move |_: &MoveDown, window, cx| {
                down.update(cx, |d, cx| d.step(1, window, cx));
                cx.stop_propagation();
            })
            // The medium size cuts the text at small interface sizes.
            .child(Input::new(&self.query).small().disabled(running));
        let dest_error = self
            .source(cx)
            .and_then(|_| self.dest(cx).err())
            .filter(|_| !running);
        v_flex()
            .gap_3()
            .child(search)
            .child(self.render_repos(this, cx))
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child("Clone into"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .child(Input::new(&self.path).small().disabled(running)),
                            )
                            .child(
                                button("clone-choose")
                                    .small()
                                    .label("Choose…")
                                    .off(running)
                                    .on_click(move |_, window, cx| {
                                        choose.update(cx, |d, cx| d.choose_folder(window, cx))
                                    }),
                            ),
                    )
                    .when_some(dest_error, |d, e| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.colors.danger)
                                .child(e),
                        )
                    }),
            )
            .when_some(self.running.as_ref(), |d, r| {
                d.child(
                    v_flex()
                        .gap_1p5()
                        .child(
                            Progress::new("clone-progress")
                                // Sliding until git reports a percentage.
                                .loading(r.progress.done == 0.)
                                .value(r.progress.done * 100.),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(muted)
                                .truncate()
                                .child(r.progress.line.clone()),
                        ),
                )
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.colors.danger)
                        .child(e),
                )
            })
    }

    /// Your GitHub repositories, or why there are none.
    fn render_repos(&self, this: &Entity<Self>, cx: &App) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let note = |text: String| {
            div()
                .text_size(px(12.))
                .text_color(muted)
                .child(text)
                .into_any_element()
        };
        let login = match &self.account {
            Account::SignedIn { login, .. } => login.clone(),
            Account::Loading => {
                return h_flex()
                    .h(px(LIST_HEIGHT + 22.))
                    .justify_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(busy_spinner(muted))
                    .child("Loading your GitHub repositories…")
                    .into_any_element();
            }
            Account::Unavailable(GhError::Missing) => {
                return note(
                    "Install the GitHub CLI (brew install gh), and Gibbon lists your \
                     GitHub repositories here."
                        .into(),
                );
            }
            Account::Unavailable(GhError::SignedOut) => {
                return note(
                    "Sign in to GitHub with gh auth login in a terminal, and Gibbon lists \
                     your repositories here."
                        .into(),
                );
            }
            Account::Unavailable(GhError::Failed(e)) => {
                return note(format!(
                    "Gibbon could not list your GitHub repositories: {e}"
                ));
            }
        };
        let rows = self.rows(cx);
        let mut list = v_flex()
            .id("clone-repos")
            .track_scroll(&self.scroll)
            .h(px(LIST_HEIGHT))
            .overflow_y_scroll()
            .p_1()
            .rounded(t.radius)
            .border_1()
            .border_color(t.colors.border);
        if rows.is_empty() {
            let query = self.query.read(cx).value();
            let text = match Source::parse(&query) {
                Some(Source {
                    github: Some(full), ..
                }) => format!("Clone gets {full} from GitHub."),
                Some(s) => format!("Clone gets {}.", s.url),
                None if query.trim().is_empty() => {
                    "Your GitHub account has no repositories. Paste a URL, or type owner/name."
                        .into()
                }
                None => "No repository matches. Type owner/name, or paste a URL.".into(),
            };
            list = list.child(div().p_2().text_size(px(12.)).text_color(muted).child(text));
        }
        for (i, r) in rows.iter().enumerate() {
            list = list.child(self.render_row(i, r, this, cx));
        }
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("YOUR REPOSITORIES"),
                    )
                    .child(format!("GitHub: {login}")),
            )
            .child(list)
            .into_any_element()
    }

    fn render_row(
        &self,
        ix: usize,
        r: &RemoteRepo,
        this: &Entity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let selected = self.selected.as_deref() == Some(r.name.as_str());
        let (owner, name) = r.name.split_once('/').unwrap_or(("", &r.name));
        let icon = if r.private {
            IconName::Lock
        } else if r.fork {
            IconName::GitFork
        } else {
            IconName::BookMarked
        };
        let (pick, full) = (this.clone(), r.name.clone());
        h_flex()
            .id(("clone-repo", ix))
            .px_2()
            .py_1()
            .gap_2()
            .rounded(t.radius)
            .cursor_pointer()
            .when(selected, |d| d.bg(t.colors.list_active))
            .when(!selected, |d| {
                d.child(hover_fill(t.colors.list_hover, t.radius))
            })
            .child(Icon::new(icon).size(px(14.)).text_color(muted))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .min_w_0()
                            .child(div().text_color(muted).child(format!("{owner}/")))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(name.to_string()),
                            )
                            .when(r.archived, |d| {
                                d.child(
                                    div()
                                        .ml_1p5()
                                        .px_1()
                                        .rounded(px(4.))
                                        .border_1()
                                        .border_color(t.colors.border)
                                        .text_size(px(10.))
                                        .text_color(muted)
                                        .child("archived"),
                                )
                            }),
                    )
                    .when(!r.description.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(muted)
                                .truncate()
                                .child(r.description.clone()),
                        )
                    }),
            )
            .when(r.pushed > 0, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(fmt_age(r.pushed)),
                )
            })
            // A double click clones.
            .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                let full = full.clone();
                pick.update(cx, |d, cx| {
                    d.select(full, window, cx);
                    if e.click_count >= 2 {
                        d.start(window, cx);
                    }
                })
            })
    }

    fn render_footer(&self, this: &Entity<Self>, cx: &App) -> impl IntoElement {
        let (stop, start) = (this.clone(), this.clone());
        let ready = self.source(cx).is_some() && self.dest(cx).is_ok();
        let cancel = match &self.running {
            Some(r) => button("clone-stop")
                .label("Stop")
                .off(r.stopping)
                .on_click(move |_, _, cx| stop.update(cx, |d, cx| d.stop(cx))),
            None => button("dialog-cancel")
                .label("Cancel")
                .on_click(|_, window, cx| window.close_dialog(cx)),
        };
        h_flex().w_full().justify_end().gap_2().child(cancel).child(
            button("dialog-ok")
                .primary()
                .label(if self.running.is_some() {
                    "Cloning…"
                } else {
                    "Clone"
                })
                .loading(self.running.is_some())
                .off(self.running.is_some() || !ready)
                .on_click(move |_, window, cx| start.update(cx, |d, cx| d.start(window, cx))),
        )
    }
}

/// The height of the repository list: about six rows.
const LIST_HEIGHT: f32 = 264.;
