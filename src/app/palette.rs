//! Command palette (⌘K): actions, branches, pull requests, repositories.

use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};

use super::*;

/// What a palette section holds; the confirmed row indexes into it.
enum Section {
    Actions,
    Switch(Vec<Branch>),
    Browse(Vec<(String, String)>),
    Review(Vec<(String, String)>),
    Prs(Vec<crate::github::PullRequest>),
    Repos(Vec<PathBuf>),
}

/// Remote branches listed at most (the newest first).
const MAX_REMOTE: usize = 300;

impl GitApp {
    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        let has_repo = self.repo.is_some();
        let mut sections: Vec<Section> = vec![Section::Actions];
        if has_repo {
            let switch: Vec<Branch> = self
                .branches
                .iter()
                .filter(|b| b.kind == RefKind::Local && !b.is_head)
                .cloned()
                .collect();
            if !switch.is_empty() {
                sections.push(Section::Switch(switch));
            }
            let browse: Vec<(String, String)> = self
                .branches
                .iter()
                .filter(|b| b.kind == RefKind::Local)
                .chain(
                    self.branches
                        .iter()
                        .filter(|b| b.kind == RefKind::Remote)
                        .take(MAX_REMOTE),
                )
                .map(|b| (b.refname.clone(), b.name.clone()))
                .collect();
            if !browse.is_empty() {
                sections.push(Section::Browse(browse));
            }
            let review: Vec<(String, String)> = self
                .branches
                .iter()
                .filter(|b| b.kind == RefKind::Local && Some(&b.refname) != self.base.as_ref())
                .map(|b| (b.refname.clone(), b.name.clone()))
                .collect();
            if !review.is_empty() {
                sections.push(Section::Review(review));
            }
            if !self.prs.is_empty() {
                sections.push(Section::Prs(self.prs.clone()));
            }
        }
        let current = self.repo.as_ref().map(|r| r.root.clone());
        let repos: Vec<PathBuf> = crate::recent::load()
            .into_iter()
            .filter(|p| Some(p) != current.as_ref())
            .collect();
        if !repos.is_empty() {
            sections.push(Section::Repos(repos));
        }
        let sections = Rc::new(sections);
        let this = cx.entity();
        let field = state.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let (this, sections) = (this.clone(), sections.clone());
            let mut command = Command::new(&field)
                .placeholder("Type a command, a branch or a repository…")
                .max_h(px(420.));
            for section in sections.iter() {
                command = command.group(group(section, has_repo));
            }
            let command = command
                .on_confirm(move |path, window, cx| {
                    window.close_dialog(cx);
                    let Some(section) = sections.get(path.section) else {
                        return;
                    };
                    let row = path.row;
                    this.update(cx, |app, cx| match section {
                        // Actions dispatch themselves.
                        Section::Actions => {}
                        Section::Switch(list) => {
                            if let Some(b) = list.get(row) {
                                app.switch_branch(b.clone(), cx)
                            }
                        }
                        Section::Browse(list) => {
                            if let Some((r, _)) = list.get(row) {
                                app.show_target(LogTarget::Ref(r.clone()), cx)
                            }
                        }
                        Section::Review(list) => {
                            if let Some((r, _)) = list.get(row) {
                                app.start_review(r.clone(), cx)
                            }
                        }
                        Section::Prs(list) => {
                            if let Some(p) = list.get(row) {
                                app.browse_pr(p.clone(), false, cx)
                            }
                        }
                        Section::Repos(list) => {
                            if let Some(p) = list.get(row) {
                                cx.emit(AppEvent::Open(p.clone()))
                            }
                        }
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx));
            dialog
                .w(px(600.))
                .p_0()
                .close_button(false)
                .margin_top(px(90.))
                .child(command)
        });
        state.update(cx, |s, cx| s.focus(window, cx));
    }
}

fn action(label: &str, icon: IconName, a: impl Action) -> CommandItem {
    CommandItem::new()
        .label(label.to_string())
        .icon(Icon::new(icon))
        .action(Box::new(a))
}

fn group(section: &Section, has_repo: bool) -> CommandGroup {
    match section {
        Section::Actions => {
            let mut items = vec![];
            if has_repo {
                items.extend([
                    action("Fetch", IconName::RefreshCw, Fetch),
                    action("Pull", IconName::ArrowDownToLine, Pull),
                    action("Push", IconName::ArrowUpFromLine, Push),
                    action("Commit", IconName::GitCommitHorizontal, CommitChanges),
                    action("New Branch…", IconName::GitBranchPlus, NewBranch),
                    action("Stash Changes…", IconName::Archive, StashChanges),
                    action("Show Changes", IconName::FilePen, ShowChanges),
                    action("Show History", IconName::GitCommitVertical, ShowHistory),
                    action("Show All Branches", IconName::GitGraph, ShowAllBranches),
                    action("Refresh", IconName::RotateCw, Refresh),
                ]);
            }
            items.push(action("Open Repository…", IconName::FolderGit2, OpenRepo));
            items.push(action("Settings…", IconName::Settings, OpenSettings));
            CommandGroup::new().label("Actions").items(items)
        }
        Section::Switch(list) => CommandGroup::new().label("Switch to branch").items(
            list.iter().map(|b| {
                CommandItem::new()
                    .label(format!("Switch to {}", b.name))
                    .keywords([b.name.clone(), "checkout".into()])
                    .icon(Icon::new(IconName::GitBranch))
            }),
        ),
        Section::Browse(list) => CommandGroup::new().label("Browse branch").items(list.iter().map(
            |(_, name)| {
                CommandItem::new()
                    .label(format!("Browse {name}"))
                    .keywords([name.clone(), "pick".into()])
                    .icon(Icon::new(if name.contains('/') {
                        IconName::Cloud
                    } else {
                        IconName::GitBranch
                    }))
            },
        )),
        Section::Review(list) => CommandGroup::new().label("Review branch").items(list.iter().map(
            |(_, name)| {
                CommandItem::new()
                    .label(format!("Review {name}"))
                    .keywords([name.clone(), "diff".into(), "compare".into()])
                    .icon(Icon::new(IconName::GitCompare))
            },
        )),
        Section::Prs(list) => CommandGroup::new()
            .label("Pull requests")
            .items(list.iter().map(|p| {
                CommandItem::new()
                    .label(format!("#{} {}", p.number, p.title))
                    .keywords([p.head.clone(), p.author.clone(), format!("{}", p.number)])
                    .icon(Icon::new(IconName::GitPullRequest))
            })),
        Section::Repos(list) => CommandGroup::new()
            .label("Recent repositories")
            .items(list.iter().map(|p| {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                CommandItem::new()
                    .label(format!("Open {name}"))
                    .keywords([p.display().to_string()])
                    .icon(Icon::new(IconName::FolderGit2))
            })),
    }
}
