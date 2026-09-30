//! Branch dialogs: create, rename, delete.

use gpui_kit::component::input::Input;

use super::*;

type OnText = Rc<dyn Fn(&mut GitApp, String, &mut Context<GitApp>)>;

impl GitApp {
    /// A dialog with one text field. Enter or the OK button submits.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prompt(
        &mut self,
        title: &str,
        label: String,
        initial: &str,
        ok: &str,
        on_ok: impl Fn(&mut GitApp, String, &mut Context<GitApp>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |s, cx| s.set_value(initial.to_string(), window, cx));
        let on_ok: OnText = Rc::new(on_ok);
        let submit = on_ok.clone();
        self.prompt_sub = Some(cx.subscribe_in(
            &input,
            window,
            move |this, state, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    let value = state.read(cx).value().to_string();
                    window.close_dialog(cx);
                    this.prompt_sub = None;
                    submit(this, value, cx);
                }
            },
        ));
        let this = cx.entity();
        let muted = cx.theme().colors.muted_foreground;
        let (title, ok): (SharedString, SharedString) =
            (title.to_string().into(), ok.to_string().into());
        let field = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let (this, on_ok, field) = (this.clone(), on_ok.clone(), field.clone());
            dialog
                .title(title.clone())
                .w(px(420.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_size(px(12.)).text_color(muted).child(label.clone()))
                        .child(Input::new(&field)),
                )
                .footer(dialog_footer(ok.clone(), ButtonVariant::Primary, move |_, cx| {
                    let value = field.read(cx).value().to_string();
                    this.update(cx, |app, cx| {
                        app.prompt_sub = None;
                        on_ok(app, value, cx)
                    });
                }))
        });
        input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Create a branch at `start` (HEAD when None) and switch to it.
    pub(super) fn new_branch_dialog(
        &mut self,
        start: Option<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = match (&start, self.refs_loaded) {
            (Some((_, label)), _) => label.clone(),
            (None, true) => self.head_name(),
            (None, false) => "the current commit".into(),
        };
        self.prompt(
            "New branch",
            format!("Create a branch from {from} and switch to it."),
            "",
            "Create",
            move |this, name, cx| {
                let name = name.trim().to_string();
                let Some(repo) = this.repo.clone() else {
                    return;
                };
                if let Some(err) = git::check_branch_name(&repo, &name) {
                    this.toast(Some(false), err, cx);
                    return;
                }
                let start = start.as_ref().map(|(s, _)| s.clone());
                this.target = LogTarget::Head;
                this.run_op(
                    "Creating branch…",
                    Some(format!("Switched to the new branch {name}")),
                    move |repo| {
                        git::create_branch(repo, &name, start.as_deref(), true)
                            .map(|_| String::new())
                    },
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(super) fn rename_branch_dialog(
        &mut self,
        branch: Branch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old = branch.name.clone();
        self.prompt(
            "Rename branch",
            format!("New name for {old}:"),
            &old.clone(),
            "Rename",
            move |this, name, cx| {
                let name = name.trim().to_string();
                let Some(repo) = this.repo.clone() else {
                    return;
                };
                if name == old {
                    return;
                }
                if let Some(err) = git::check_branch_name(&repo, &name) {
                    this.toast(Some(false), err, cx);
                    return;
                }
                let old = old.clone();
                this.run_op(
                    "Renaming…",
                    Some(format!("Renamed {old} to {name}")),
                    move |repo| git::rename_branch(repo, &old, &name).map(|_| String::new()),
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Confirm, delete; if git refuses an unmerged branch, confirm again.
    pub(super) fn delete_branch_dialog(
        &mut self,
        branch: Branch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = branch.name.clone();
        self.confirm(
            "Delete branch?",
            format!("This deletes the local branch {name}. Its remote branch stays."),
            "Delete",
            move |this, cx| this.delete_branch(name.clone(), cx),
            window,
            cx,
        );
    }

    fn delete_branch(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let Some(window) = cx.active_window() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let n = name.clone();
            let result = cx
                .background_executor()
                .spawn(async move { git::delete_branch(&repo, &n, false) })
                .await;
            let _ = window.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(()) => {
                        this.toast(Some(true), format!("Deleted {name}"), cx);
                        this.reload(cx);
                    }
                    Err(e) if e.to_string().contains("not fully merged") => {
                        let n = name.clone();
                        this.confirm(
                            "Branch not merged",
                            format!(
                                "{name} has commits that no other branch contains. \
                                 Delete it anyway? You lose those commits."
                            ),
                            "Delete anyway",
                            move |this, cx| {
                                let n = n.clone();
                                this.run_op(
                                    "Deleting…",
                                    Some(format!("Deleted {n}")),
                                    move |repo| {
                                        git::delete_branch(repo, &n, true).map(|_| String::new())
                                    },
                                    cx,
                                )
                            },
                            window,
                            cx,
                        );
                    }
                    Err(e) => this.toast(Some(false), e.to_string(), cx),
                });
            });
        })
        .detach();
    }
}
