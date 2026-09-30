//! The parts of a tab, each in its own cached view. GPUI renders a cached
//! view again only when that view notifies. So a hover, a scroll or a fade
//! in one part renders only that part. All parts render again when the tab
//! or the settings change.
//!
//! The tab itself must not be a cached view: when a cached view renders
//! again, GPUI also renders all the cached views in it.

use std::any::Any;

use super::*;

/// A part of the tab, in its own view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Part {
    Sidebar,
    /// The commit list.
    Log,
    /// The shown commit: message, author and files.
    Commit,
    /// The files of the shown stash.
    StashFiles,
    /// The files of the shown review.
    ReviewFiles,
    /// The timeline of the Activity view.
    Activity,
    /// The file list and the commit box of Changes.
    Changes,
    /// The diff of the shown view.
    Diff,
}

/// What a part derives from the tab, for example the rows of a list. The
/// pane keeps it until the tab or the settings change, so a render for a
/// hover or a scroll does not compute it again.
pub(super) type Memo = Option<Rc<dyn Any>>;

/// The value that `memo` keeps, or else the value of `f`, which `memo` then
/// keeps.
pub(super) fn keep<T: 'static>(memo: &mut Memo, f: impl FnOnce() -> T) -> Rc<T> {
    if let Some(value) = memo.clone().and_then(|m| m.downcast::<T>().ok()) {
        return value;
    }
    let value = Rc::new(f());
    *memo = Some(value.clone());
    value
}

pub(super) struct Pane {
    app: WeakEntity<GitApp>,
    part: Part,
    memo: Memo,
    _subs: [Subscription; 2],
}

impl Pane {
    pub(super) fn new(app: &Entity<GitApp>, part: Part, cx: &mut Context<Self>) -> Self {
        fn changed(pane: &mut Pane, cx: &mut Context<Pane>) {
            pane.memo = None;
            cx.notify();
        }
        Pane {
            app: app.downgrade(),
            part,
            memo: None,
            _subs: [
                cx.observe(app, |pane, _, cx| changed(pane, cx)),
                cx.observe_global::<crate::settings::Settings>(changed),
            ],
        }
    }
}

impl Render for Pane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (part, memo) = (self.part, &mut self.memo);
        self.app
            .update(cx, |app, cx| app.render_part(part, memo, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl GitApp {
    /// The view of `part`, made when it is first shown.
    pub(super) fn pane(&mut self, part: Part, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity();
        self.panes
            .entry(part)
            .or_insert_with(|| cx.new(|cx| Pane::new(&app, part, cx)))
            .clone()
            .cached(StyleRefinement::default().size_full())
            .into_any_element()
    }

    fn render_part(&mut self, part: Part, memo: &mut Memo, cx: &mut Context<Self>) -> AnyElement {
        match part {
            Part::Sidebar => self.render_sidebar(memo, cx).into_any_element(),
            Part::Log => self.render_log(memo, cx).into_any_element(),
            Part::Commit => self.render_commit(memo, cx),
            Part::StashFiles => self.render_stash_files(memo, cx),
            Part::ReviewFiles => self.render_review_files(memo, cx),
            Part::Activity => self.render_moves(memo, cx),
            Part::Changes => self.render_change_list(memo, cx).into_any_element(),
            Part::Diff => self.render_shown_diff(memo, cx),
        }
    }
}
