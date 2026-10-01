//! Pull request status: the checks, the review and the merge state, on the
//! pull request rows and on the rows of their branches and worktrees. An
//! agent opens a pull request and waits for its checks: the sidebar shows
//! the result without a visit to GitHub.

use gpui_kit::component::tooltip::Tooltip;

use crate::github::{CheckState, PrStatus, PullRequest, Review};

use super::*;

impl GitApp {
    /// The open pull request whose head is the local branch `name`. A pull
    /// request from a fork has another branch of that name.
    pub(super) fn pr_of_branch(&self, name: &str) -> Option<&PullRequest> {
        self.prs.iter().find(|p| !p.cross_repo && p.head == name)
    }

    fn pr_status_of(&self, number: u64) -> Option<&PrStatus> {
        self.pr_status.get(&number)
    }

    /// The icon of the checks of pull request `number`, if it has checks.
    pub(super) fn pr_checks_icon(&self, number: u64, cx: &App) -> Option<AnyElement> {
        let state = self.pr_status_of(number)?.checks.state()?;
        Some(checks_icon(state, cx).into_any_element())
    }

    /// The icons of pull request `number`: checks, review and conflicts.
    pub(super) fn pr_badges(&self, number: u64, cx: &App) -> Vec<AnyElement> {
        let t = cx.theme();
        let Some(s) = self.pr_status_of(number) else {
            return vec![];
        };
        let mut out = vec![];
        if s.conflicts {
            out.push(
                Icon::new(IconName::GitMergeConflict)
                    .size(px(12.))
                    .text_color(t.colors.red)
                    .into_any_element(),
            );
        }
        let review = match s.review {
            Review::Approved => Some((IconName::UserCheck, t.colors.green)),
            Review::ChangesRequested => Some((IconName::UserX, t.colors.red)),
            // The usual state of an open pull request: the tooltip says it.
            Review::Required | Review::None => None,
        };
        if let Some((icon, color)) = review {
            out.push(Icon::new(icon).size(px(12.)).text_color(color).into_any_element());
        }
        if let Some(state) = s.checks.state() {
            out.push(checks_icon(state, cx).into_any_element());
        }
        out
    }

    /// The status of pull request `number` in words, for tooltips and the
    /// browse banner. Empty until the status loads.
    pub(super) fn pr_status_lines(&self, number: u64) -> Vec<String> {
        let Some(s) = self.pr_status_of(number) else {
            return vec![];
        };
        let mut lines = vec![format!("Checks: {}", s.checks.summary())];
        let mut merge = vec![];
        if let Some(review) = s.review.text() {
            merge.push(review.to_string());
        }
        if s.conflicts {
            merge.push("Conflicts with the base branch".to_string());
        } else if s.behind {
            merge.push("Behind the base branch".to_string());
        }
        if !merge.is_empty() {
            lines.push(merge.join(" · "));
        }
        lines
    }

    /// The tooltip lines of a pull request row.
    pub(super) fn pr_tip(&self, p: &PullRequest) -> Rc<Vec<String>> {
        let mut lines = vec![
            format!("#{} {}", p.number, p.title),
            format!("{} → {} · {}", p.head, p.base, p.author),
        ];
        lines.extend(self.pr_status_lines(p.number));
        Rc::new(lines)
    }
}

fn checks_icon(state: CheckState, cx: &App) -> Icon {
    let t = cx.theme();
    let (icon, color) = match state {
        CheckState::Passed => (IconName::CircleCheck, t.colors.green),
        CheckState::Pending => (IconName::CircleDashed, t.colors.yellow),
        CheckState::Failed => (IconName::CircleX, t.colors.red),
    };
    Icon::new(icon).size(px(12.)).text_color(color)
}

/// A tooltip of lines: the first one bold, the others muted.
pub(super) fn lines_tooltip(
    lines: Rc<Vec<String>>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| {
        let lines = lines.clone();
        Tooltip::element(move |_, cx| {
            let muted = cx.theme().colors.muted_foreground;
            v_flex()
                .gap_0p5()
                .max_w(px(420.))
                .children(lines.first().map(|l| {
                    div().font_weight(FontWeight::SEMIBOLD).child(l.clone())
                }))
                .children(lines.iter().skip(1).map(|l| div().text_color(muted).child(l.clone())))
        })
        .build(window, cx)
    }
}
