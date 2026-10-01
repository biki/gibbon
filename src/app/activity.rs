//! Activity: the moves of all branches, newest first, from their reflogs.
//! Commits, amends, rebases, resets, merges, pulls and pushes, with the
//! commits that each move added and dropped. A move that dropped commits
//! can be undone: Restore moves the branch back.

use std::ops::Range;

use gpui_kit::component::menu::ContextMenuExt as _;

use super::review::short_ref;
use super::*;

/// Moves whose commits a load counts at most. Counting runs git once per
/// move, and the counts of earlier loads stay.
const COUNTS_PER_LOAD: usize = 200;

/// A row of the timeline.
enum Row {
    /// The day of the moves below.
    Day(SharedString),
    /// Index into `moves`.
    Move(usize),
}

/// What the timeline keeps between renders.
struct ActivityMemo {
    rows: Vec<Row>,
    /// "14:32" by move index.
    times: Vec<SharedString>,
}

const ROW_H: f32 = 34.;

impl GitApp {
    /// Load the moves again, and count the commits of the new ones.
    pub(super) fn load_activity(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.moves_epoch += 1;
        let epoch = self.moves_epoch;
        let known = self.move_counts.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut moves = git::ref_moves(&repo)?;
                    let mut found = vec![];
                    for m in moves.iter_mut().filter(|m| m.needs_counts()) {
                        let Some(old) = m.old.clone() else {
                            continue;
                        };
                        let key = (old, m.new.clone());
                        m.counts = match known.get(&key) {
                            Some(counts) => *counts,
                            None if found.len() < COUNTS_PER_LOAD => {
                                let counts = git::move_counts(&repo, &key.0, &key.1);
                                found.push((key, counts));
                                counts
                            }
                            None => None,
                        };
                    }
                    anyhow::Ok((moves, found))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.moves_epoch {
                    return;
                }
                match result {
                    Ok((moves, found)) => {
                        this.move_counts.extend(found);
                        this.moves = Rc::new(moves);
                        let now = now();
                        // A first visit starts with nothing new.
                        if this.activity_seen.is_none() || this.view == View::Activity {
                            this.activity_seen = Some(now);
                        }
                    }
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Show the timeline. The moves since the last visit keep a dot.
    pub(super) fn show_activity(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Activity {
            self.activity_mark = self.activity_seen.unwrap_or_else(now);
            self.activity_seen = Some(now());
            self.view = View::Activity;
        }
        cx.notify();
    }

    /// Moves since the last visit to the timeline.
    pub(super) fn new_moves(&self) -> usize {
        let Some(seen) = self.activity_seen else {
            return 0;
        };
        self.moves.iter().take_while(|m| m.time > seen).count()
    }

    fn activity_memo(&self) -> ActivityMemo {
        use chrono::{Local, TimeZone as _};
        let today = Local::now().date_naive();
        let mut rows = vec![];
        let mut times = vec![];
        let mut day = None;
        for (i, m) in self.moves.iter().enumerate() {
            let at = Local.timestamp_opt(m.time, 0).single();
            let d = at.map(|a| a.date_naive());
            if d != day {
                day = d;
                let label = match d {
                    Some(d) if d == today => "Today".to_string(),
                    Some(d) if today.pred_opt() == Some(d) => "Yesterday".to_string(),
                    Some(d) => d.format("%A, %b %-d").to_string(),
                    None => String::new(),
                };
                rows.push(Row::Day(label.into()));
            }
            rows.push(Row::Move(i));
            times.push(
                at.map(|a| a.format("%H:%M").to_string())
                    .unwrap_or_default()
                    .into(),
            );
        }
        ActivityMemo { rows, times }
    }

    pub(super) fn render_activity(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let n = self.moves.len();
        let rewrites = self.moves.iter().filter(|m| m.rewrites()).count();
        let mut summary = format!(
            "{n} move{} of the last 30 days, newest first",
            history::plural(n)
        );
        if rewrites > 0 {
            summary.push_str(&format!(
                " · {rewrites} move{} dropped commits. Restore undoes the last move of a branch.",
                history::plural(rewrites)
            ));
        }
        let header = h_flex()
            .flex_none()
            .px_3()
            .py_2()
            .gap_3()
            .border_b_1()
            .border_color(t.colors.border)
            .child(
                Icon::new(IconName::Activity)
                    .size(px(18.))
                    .text_color(t.colors.primary),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Activity"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(muted)
                            .truncate()
                            .child(summary),
                    ),
            );
        v_flex()
            .size_full()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.pane(Part::Activity, cx)),
            )
            .into_any_element()
    }

    /// The timeline, in its own pane.
    pub(super) fn render_moves(&mut self, memo: &mut Memo, cx: &mut Context<Self>) -> AnyElement {
        let memo = keep(memo, || self.activity_memo());
        let muted = cx.theme().colors.muted_foreground;
        if self.moves.is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(muted)
                .child(Icon::new(IconName::Activity).size(px(28.)))
                .child("No branch moved in the last 30 days.")
                .into_any_element();
        }
        let n = memo.rows.len();
        uniform_list(
            "moves",
            n,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                range
                    .map(|i| match memo.rows[i] {
                        Row::Day(ref label) => day_row(label.clone(), i, cx),
                        Row::Move(m) => this.render_move(m, i, &memo.times[m], cx),
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .size_full()
        .py_1()
        .into_any_element()
    }

    fn render_move(
        &self,
        mi: usize,
        ix: usize,
        time: &SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let m = &self.moves[mi];
        let local = m.refname.starts_with("refs/heads/");
        let rewrites = m.rewrites();
        // Only the last move of a branch gets the button: restoring an
        // earlier one also takes away the moves after it.
        let last = self.moves.iter().position(|o| o.refname == m.refname) == Some(mi);
        let (icon, color) = move_icon(m.kind, rewrites, cx);
        let fresh = m.time > self.activity_mark;
        let branch = match (&m.worktree, m.refname.as_str()) {
            (Some(w), "HEAD") => format!("HEAD in {w}"),
            (None, "HEAD") => "HEAD".to_string(),
            (_, r) => short_ref(r),
        };
        // A linked worktree that has the branch checked out.
        let checkout = local
            .then(|| self.checkout_of(&m.refname))
            .flatten()
            .filter(|w| !w.main)
            .map(|w| w.folder());
        let detail = match m.kind {
            MoveKind::Restore => format!("to {}", &m.new[..7.min(m.new.len())]),
            MoveKind::Push => String::new(),
            _ => m.detail.clone(),
        };
        let (added, dropped) = m.counts.unwrap_or((0, 0));
        let shas = match &m.old {
            Some(old) => format!(
                "{} → {}",
                &old[..7.min(old.len())],
                &m.new[..7.min(m.new.len())]
            ),
            None => m.new[..7.min(m.new.len())].to_string(),
        };
        let mv = m.clone();
        let this = cx.entity();
        h_flex()
            .id(("move", ix))
            .h(px(ROW_H))
            .w_full()
            .px_3()
            .gap_2()
            .child(hover_fill(t.colors.list_hover, px(0.)))
            .child(
                div()
                    .flex_none()
                    .size(px(6.))
                    .rounded_full()
                    .when(fresh, |d| d.bg(t.colors.primary)),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(40.))
                    .text_size(px(11.5))
                    .text_color(muted)
                    .child(time.clone()),
            )
            .child(Icon::new(icon).size(px(14.)).text_color(color))
            .child(
                h_flex()
                    .flex_none()
                    .max_w(px(220.))
                    .h(px(20.))
                    .px_1p5()
                    .gap_1()
                    .rounded(px(4.))
                    .bg(if local {
                        t.colors.primary.opacity(0.14)
                    } else {
                        t.colors.muted
                    })
                    .text_color(if local { t.colors.primary } else { muted })
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().min_w_0().truncate().child(branch))
                    .when_some(checkout, |d, w| {
                        d.child(div().flex_none().text_color(muted).child(format!("· {w}")))
                    }),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::MEDIUM)
                            .child(m.kind.verb()),
                    )
                    .child(div().min_w_0().truncate().text_color(muted).child(detail)),
            )
            .when(added > 0 && m.kind != MoveKind::Created, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(11.5))
                        .text_color(t.colors.green)
                        .child(format!("+{added}")),
                )
            })
            .when(dropped > 0, |d| {
                d.child(
                    div()
                        .flex_none()
                        .h(px(18.))
                        .px_1p5()
                        .rounded(px(4.))
                        .bg(t.colors.red.opacity(0.14))
                        .text_color(t.colors.red)
                        .text_size(px(11.))
                        .child(format!("{dropped} dropped")),
                )
            })
            .child(
                div()
                    .flex_none()
                    .w(px(130.))
                    .text_right()
                    .font_family(crate::theme::mono_font(cx))
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(shas),
            )
            .child(
                // The same width on every row, so the columns line up.
                div().flex_none().w(px(76.)).flex().justify_end().when(
                    rewrites && local && last && m.old.is_some(),
                    |d| {
                        let (mv, this) = (mv.clone(), this.clone());
                        d.child(
                            button(("restore", ix))
                                .xsmall()
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .child(Icon::new(IconName::RotateCcw).size(px(12.)))
                                        .child("Restore"),
                                )
                                .tooltip("Undo this move: the branch goes back to where it was")
                                .on_click(move |_, window, cx| {
                                    let (r, to) = (mv.refname.clone(), mv.old.clone());
                                    this.update(cx, |app, cx| {
                                        if let Some(to) = to {
                                            app.restore_dialog(r, to, window, cx)
                                        }
                                    });
                                }),
                        )
                    },
                ),
            )
            .context_menu(move |menu, _, _| move_menu(menu, &mv, this.clone()))
            .into_any_element()
    }

    /// The restore dialog of the newest move that dropped commits (UI checks).
    pub(super) fn check_restore_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let found = self
            .moves
            .iter()
            .find(|m| m.rewrites() && m.refname.starts_with("refs/heads/"))
            .and_then(|m| Some((m.refname.clone(), m.old.clone()?)));
        if let Some((refname, to)) = found {
            self.restore_dialog(refname, to, window, cx);
        }
    }

    /// Count what a restore of the local branch `refname` to `to` changes,
    /// then confirm it and move the branch.
    fn restore_dialog(
        &mut self,
        refname: String,
        to: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let branch = short_ref(&refname);
        // The newest move of a branch is where it is now.
        let Some(now_at) = self
            .moves
            .iter()
            .find(|m| m.refname == refname)
            .map(|m| m.new.clone())
        else {
            return;
        };
        if now_at == to {
            self.toast(None, format!("{branch} is there already."), cx);
            return;
        }
        let checkout = self.checkout_of(&refname).cloned();
        let (from, target) = (now_at.clone(), to.clone());
        cx.spawn_in(window, async move |this, cx| {
            let counts = cx
                .background_executor()
                .spawn(async move { git::move_counts(&repo, &from, &target) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let short = |s: &str| s[..7.min(s.len())].to_string();
                let mut body = format!(
                    "This moves {branch} from {} to {}.",
                    short(&now_at),
                    short(&to)
                );
                match counts {
                    Some((back, 0)) => body.push_str(&format!(
                        " {back} commit{} come{} back.",
                        history::plural(back as usize),
                        if back == 1 { "s" } else { "" }
                    )),
                    Some((back, gone)) => body.push_str(&format!(
                        " {back} commit{} come{} back, and {gone} commit{} leave{} the branch.",
                        history::plural(back as usize),
                        if back == 1 { "s" } else { "" },
                        history::plural(gone as usize),
                        if gone == 1 { "s" } else { "" }
                    )),
                    None => {}
                }
                if let Some(w) = &checkout {
                    body.push_str(&format!(
                        " The worktree {} has {branch} checked out, so Gibbon runs \
                         git reset --keep there. That keeps uncommitted changes, and it \
                         stops if they conflict.",
                        w.folder()
                    ));
                }
                body.push_str(" The restore shows in Activity too, so you can undo it.");
                let dir = checkout.as_ref().map(|w| w.path.clone());
                let (refname, to, now_at) = (refname.clone(), to.clone(), now_at.clone());
                this.confirm(
                    &format!("Restore {branch}?"),
                    body,
                    "Restore",
                    move |this, cx| {
                        let (refname, to, now_at, dir) =
                            (refname.clone(), to.clone(), now_at.clone(), dir.clone());
                        let msg = format!("Restored {} to {}", short_ref(&refname), short(&to));
                        this.run_op(
                            "Restoring…",
                            Some(msg),
                            move |repo| {
                                git::restore_branch(repo, &refname, &to, &now_at, dir.as_deref())
                            },
                            cx,
                        )
                    },
                    window,
                    cx,
                );
            });
        })
        .detach();
    }
}

use crate::git::MoveKind;

/// The icon of a move, and its color: yellow when it dropped commits.
fn move_icon(kind: MoveKind, rewrites: bool, cx: &App) -> (IconName, Hsla) {
    let t = cx.theme();
    let icon = match kind {
        MoveKind::Commit => IconName::GitCommitHorizontal,
        MoveKind::Amend => IconName::SquarePen,
        MoveKind::Rebase => IconName::Waypoints,
        MoveKind::Reset => IconName::Undo2,
        MoveKind::Merge => IconName::GitMerge,
        MoveKind::Pull => IconName::ArrowDownToLine,
        MoveKind::CherryPick => IconName::Cherry,
        MoveKind::Revert => IconName::Undo,
        MoveKind::Created => IconName::GitBranchPlus,
        MoveKind::Push => IconName::ArrowUpFromLine,
        MoveKind::Switch => IconName::ArrowRightLeft,
        MoveKind::Restore => IconName::RotateCcw,
        MoveKind::Other => IconName::Dot,
    };
    let color = if rewrites {
        t.colors.yellow
    } else if kind == MoveKind::Restore {
        t.colors.green
    } else {
        t.colors.muted_foreground
    };
    (icon, color)
}

fn day_row(label: SharedString, ix: usize, cx: &App) -> AnyElement {
    let t = cx.theme();
    h_flex()
        .id(("day", ix))
        .h(px(ROW_H))
        .w_full()
        .px_3()
        .items_end()
        .pb_1()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.colors.muted_foreground)
        .child(label.to_uppercase())
        .into_any_element()
}

fn move_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    m: &git::RefMove,
    this: Entity<GitApp>,
) -> gpui_kit::component::menu::PopupMenu {
    let mut menu = menu;
    if m.refname.starts_with("refs/heads/") {
        let branch = short_ref(&m.refname);
        let (app, r, old) = (this.clone(), m.refname.clone(), m.old.clone());
        menu = menu.item(
            PopupMenuItem::new(format!("Restore {branch} to Before This…"))
                .disabled(old.is_none())
                .on_click(move |_, window, cx| {
                    let (r, old) = (r.clone(), old.clone());
                    app.update(cx, |app, cx| {
                        if let Some(old) = old {
                            app.restore_dialog(r, old, window, cx)
                        }
                    });
                }),
        );
        let (app, r, new) = (this.clone(), m.refname.clone(), m.new.clone());
        menu = menu
            .item(
                PopupMenuItem::new(format!("Restore {branch} to After This…")).on_click(
                    move |_, window, cx| {
                        let (r, new) = (r.clone(), new.clone());
                        app.update(cx, |app, cx| app.restore_dialog(r, new, window, cx));
                    },
                ),
            )
            .separator();
        let (app, r) = (this.clone(), m.refname.clone());
        menu = menu.item(PopupMenuItem::new(format!("Browse {branch}")).on_click(
            move |_, _, cx| {
                let r = r.clone();
                app.update(cx, |app, cx| app.show_target(LogTarget::Ref(r), cx));
            },
        ));
    }
    let new = m.new.clone();
    menu =
        menu.item(PopupMenuItem::new("Copy SHA").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(new.clone()))
        }));
    if let Some(old) = m.old.clone() {
        menu = menu.item(
            PopupMenuItem::new("Copy SHA Before").on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(old.clone()))
            }),
        );
    }
    menu
}

/// Seconds since 1970.
fn now() -> i64 {
    chrono::Local::now().timestamp()
}
