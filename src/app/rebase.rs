//! Interactive rebase: plan the steps in a list, then run them.

use std::collections::HashMap;

use gpui_kit::component::input::Input;

use crate::git::{RebaseAction, RebasePlan};

use super::*;

pub(super) struct RebaseUi {
    plan: RebasePlan,
    /// Subject fields of Reword steps, by sha.
    inputs: HashMap<String, Entity<InputState>>,
}

impl GitApp {
    /// Plan a rebase of the commits from `from` (included) to HEAD.
    pub(super) fn start_rebase(&mut self, from: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let plan = cx
                .background_executor()
                .spawn(async move { git::rebase_plan(&repo, &from) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match plan {
                    Ok(plan) if plan.steps.is_empty() => {
                        this.toast(None, "There are no commits to rebase.")
                    }
                    Ok(plan) => {
                        this.rebase = Some(RebaseUi {
                            plan,
                            inputs: HashMap::new(),
                        });
                        this.view = View::Rebase;
                    }
                    Err(e) => this.toast(Some(false), e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_action(
        &mut self,
        i: usize,
        action: RebaseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ui) = self.rebase.as_mut() else {
            return;
        };
        let step = &mut ui.plan.steps[i];
        step.action = action;
        if action == RebaseAction::Reword && !ui.inputs.contains_key(&step.sha) {
            let subject = step.subject.clone();
            let input = cx.new(|cx| InputState::new(window, cx));
            input.update(cx, |s, cx| {
                s.set_value(subject, window, cx);
                s.focus(window, cx);
            });
            ui.inputs.insert(step.sha.clone(), input);
        }
        cx.notify();
    }

    fn move_step(&mut self, i: usize, up: bool, cx: &mut Context<Self>) {
        let Some(ui) = self.rebase.as_mut() else {
            return;
        };
        let j = if up { i.checked_sub(1) } else { Some(i + 1) };
        if let Some(j) = j.filter(|&j| j < ui.plan.steps.len()) {
            ui.plan.steps.swap(i, j);
            cx.notify();
        }
    }

    fn run_rebase(&mut self, cx: &mut Context<Self>) {
        let Some(ui) = self.rebase.take() else {
            return;
        };
        let mut plan = ui.plan;
        for step in &mut plan.steps {
            if step.action == RebaseAction::Reword
                && let Some(input) = ui.inputs.get(&step.sha)
            {
                step.new_subject = Some(input.read(cx).value().to_string());
            }
        }
        if let Some(problem) = git::check_plan(&plan) {
            self.toast(Some(false), problem);
            self.rebase = Some(RebaseUi {
                plan,
                inputs: ui.inputs,
            });
            cx.notify();
            return;
        }
        self.view = View::History;
        self.target = LogTarget::Head;
        self.run_op(
            "Rebasing…",
            Some("Rebase finished".into()),
            move |repo| git::rebase_run(repo, &plan),
            cx,
        );
    }

    pub(super) fn render_rebase(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let Some(ui) = self.rebase.as_ref() else {
            return div().into_any_element();
        };
        let n = ui.plan.steps.len();
        let problem = git::check_plan(&ui.plan);
        let dirty = !self.status.is_empty();
        let head = self.head_name();
        let header = h_flex()
            .flex_none()
            .px_4()
            .py_3()
            .gap_3()
            .border_b_1()
            .border_color(t.colors.border)
            .child(Icon::new(IconName::GitGraph).size(px(18.)).text_color(t.colors.primary))
            .child(
                v_flex()
                    .flex_1()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Interactive rebase"))
                    .child(div().text_size(px(12.)).text_color(muted).child(format!(
                        "Rewrite {n} commit{} of {head}. Oldest first: squash and fixup join the commit above.",
                        history::plural(n)
                    ))),
            )
            .child(
                Button::new("rebase-cancel")
                    .ghost()
                    .small()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.rebase = None;
                        this.view = View::History;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("rebase-run")
                    .primary()
                    .small()
                    .label("Start Rebase")
                    .disabled(problem.is_some() || self.busy.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.run_rebase(cx))),
            );
        let notes = v_flex()
            .flex_none()
            .px_4()
            .gap_1()
            .text_size(px(12.))
            .when(ui.plan.merges > 0, |d| {
                d.pt_2().child(div().text_color(t.colors.yellow).child(format!(
                    "{} merge commit{} in this range will be flattened.",
                    ui.plan.merges,
                    history::plural(ui.plan.merges)
                )))
            })
            .when(dirty, |d| {
                d.pt_2().child(div().text_color(muted).child(
                    "Your uncommitted changes are stashed first and restored after.",
                ))
            })
            .when_some(problem, |d, p| d.pt_2().child(div().text_color(t.colors.red).child(p)));
        let rows: Vec<AnyElement> = (0..n).map(|i| self.render_step(i, cx)).collect();
        v_flex()
            .size_full()
            .child(header)
            .child(notes)
            .child(
                div()
                    .id("rebase-steps")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_3()
                    .child(v_flex().gap_1().children(rows)),
            )
            .into_any_element()
    }

    fn render_step(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let ui = self.rebase.as_ref().expect("rebase plan");
        let step = &ui.plan.steps[i];
        let n = ui.plan.steps.len();
        let action = step.action;
        let joins = matches!(action, RebaseAction::Squash | RebaseAction::Fixup);
        let color = match action {
            RebaseAction::Pick => t.colors.foreground,
            RebaseAction::Reword => t.colors.blue,
            RebaseAction::Squash | RebaseAction::Fixup => t.colors.yellow,
            RebaseAction::Drop => t.colors.red,
        };
        let this = cx.entity();
        let picker = Button::new(("action", i))
            .small()
            .outline()
            .w(px(96.))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .text_color(color)
                    .font_family(MONO_FONT)
                    .child(action.name())
                    .child(Icon::new(IconName::ChevronDown).size(px(12.))),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for a in RebaseAction::ALL {
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{}  —  {}", a.name(), a.help()))
                            .checked(a == action)
                            .on_click(move |_, window, cx| {
                                this.update(cx, |app, cx| app.set_action(i, a, window, cx));
                            }),
                    );
                }
                menu
            });
        let text: AnyElement = match (action, ui.inputs.get(&step.sha)) {
            (RebaseAction::Reword, Some(input)) => div()
                .flex_1()
                .child(Input::new(input).small())
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(action == RebaseAction::Drop, |d| d.line_through().text_color(muted))
                .when(action == RebaseAction::Fixup, |d| d.text_color(muted))
                .child(step.subject.clone())
                .into_any_element(),
        };
        h_flex()
            .id(("step", i))
            .h(px(38.))
            .px_2()
            .gap_2()
            .rounded(t.radius)
            .border_1()
            .border_color(t.colors.border)
            .bg(if action == RebaseAction::Drop {
                t.colors.red.opacity(0.05)
            } else {
                t.colors.background
            })
            .when(joins, |d| d.ml_6())
            .child(
                div()
                    .w(px(22.))
                    .text_right()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(if joins { "↳".to_string() } else { format!("{}", i + 1) }),
            )
            .child(picker)
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .font_family(MONO_FONT)
                    .text_size(px(11.5))
                    .text_color(muted)
                    .child(step.sha[..7].to_string()),
            )
            .child(text)
            .child(
                Button::new(("up", i))
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowUp)
                    .tooltip("Move up")
                    .disabled(i == 0)
                    .on_click(cx.listener(move |this, _, _, cx| this.move_step(i, true, cx))),
            )
            .child(
                Button::new(("down", i))
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowDown)
                    .tooltip("Move down")
                    .disabled(i + 1 == n)
                    .on_click(cx.listener(move |this, _, _, cx| this.move_step(i, false, cx))),
            )
            .into_any_element()
    }
}
