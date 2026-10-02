//! Markdown files in the diff view, rendered: the document before the
//! change beside the document after it, with the changed text marked as
//! the text diff marks it. A switch in the header shows the rendered
//! documents or the text diff. The documents load in the background when
//! the file is first shown.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::base::text::{RangeHighlight, RenderedText, TextView, TextViewState};
use similar::{Algorithm, DiffTag};

use super::diff::DiffCtx;
use super::image::{Source, Sources, byte_size, source_bytes, source_size};
use super::*;

/// Larger files show a note: the view parses and measures all of a file.
const MAX_BYTES: u64 = 2_000_000;
/// The longest time that the search for the changed lines can take. After
/// it, a large change gets coarser marks.
const DIFF_TIME: Duration = Duration::from_millis(500);

/// The changed byte ranges of the old and of the new rendered text.
type Marks = (Vec<Range<usize>>, Vec<Range<usize>>);

/// Whether `path` names a Markdown file, by its extension.
fn is_markdown_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "md" | "markdown" | "mdown" | "mkd" | "mkdn"
    )
}

/// Whether `file` is a Markdown file with a text diff, which can show as
/// text or rendered.
pub(super) fn is_markdown(file: &FileDiff) -> bool {
    !file.binary && is_markdown_path(&file.path) && file.blob_ids() != (None, None)
}

/// Whether the diff view shows `file` rendered.
pub(super) fn rendered(file: &FileDiff, cx: &App) -> bool {
    is_markdown(file) && crate::settings::get(cx).markdown_rendered
}

/// One side of a Markdown file, rendered.
struct Doc {
    state: Entity<TextViewState>,
    /// The Markdown text that `state` shows.
    text: String,
    /// The rendered text from before the last change of `text`, while the
    /// view parses the new text in the background.
    stale: Option<RenderedText>,
}

impl Doc {
    fn new(text: String, cx: &mut App) -> Self {
        let state = cx.new(|cx| TextViewState::markdown("", cx));
        let mut doc = Doc {
            state,
            text: String::new(),
            stale: None,
        };
        doc.set_text(text, cx);
        doc
    }

    /// Show `text` in the same view, which keeps its scroll position. The
    /// view parses a short text at once and a long text in the background.
    fn set_text(&mut self, text: String, cx: &mut App) {
        if self.text == text {
            return;
        }
        let before = self.state.read(cx).rendered_text();
        self.state.update(cx, |state, cx| state.set_text(&text, cx));
        let parsed = self.state.read(cx).rendered_text() != before;
        self.stale = (!parsed).then_some(before);
        self.text = text;
    }

    /// The rendered text, when the view has parsed the current text.
    fn rendered(&mut self, cx: &App) -> Option<RenderedText> {
        let now = self.state.read(cx).rendered_text();
        if self.stale.as_ref() == Some(&now) {
            return None;
        }
        self.stale = None;
        Some(now)
    }
}

/// One side of a Markdown file, loaded.
enum Side {
    Doc(Doc),
    /// Why the side shows no document.
    Note(String),
}

/// The documents of the shown Markdown file.
#[derive(Default)]
pub(super) struct MarkdownPreview {
    /// The source and the path of the file (see `DiffScroll::set_file`).
    /// When only its content changes, the documents stay and keep their
    /// scroll positions.
    file: (String, String),
    sources: Option<Sources>,
    /// None while the sides load.
    sides: Option<(Option<Side>, Option<Side>)>,
    /// The rendered texts that the marks are for.
    marked: Option<(RenderedText, RenderedText)>,
    /// The marks of `marked`, when the diff is done, and the colors that the
    /// documents show them in.
    marks: Option<(Marks, (Hsla, Hsla))>,
    /// Whether the documents scrolled to their first change.
    revealed: bool,
    _load: Option<Task<()>>,
    _diff: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl GitApp {
    /// The body of the diff view for a Markdown file, rendered.
    pub(super) fn render_markdown_diff(
        &mut self,
        file: &FileDiff,
        ctx: DiffCtx,
        source: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sources = self.side_sources(file, ctx);
        let shown = (source.to_string(), file.path.clone());
        if self.markdown.file != shown || self.markdown.sources.as_ref() != Some(&sources) {
            self.load_markdown(shown, sources, cx);
        }
        // The theme changed.
        if let Some((_, colors)) = &self.markdown.marks
            && *colors != mark_colors(cx)
        {
            self.paint_marks(false, cx);
        }
        let Some((old, new)) = &self.markdown.sides else {
            return div().flex_1().into_any_element();
        };
        let root = match self.view {
            View::Review => self.review.as_ref().and_then(|r| r.worktree.clone()),
            _ => None,
        }
        .or_else(|| self.repo.as_ref().map(|r| r.root.clone()))
        .unwrap_or_default();
        let dir = match Path::new(&file.path).parent() {
            Some(parent) => root.join(parent),
            None => root.clone(),
        };
        let both = old.is_some() && new.is_some();
        let t = cx.theme();
        let (red, green, border) = (t.colors.red, t.colors.green, t.colors.border);
        h_flex()
            .flex_1()
            .min_h_0()
            .when_some(old.as_ref(), |d, side| {
                let label = both.then_some(("Before", red));
                d.child(side_view(label, side, &root, &dir, cx))
            })
            .when(both, |d| {
                d.child(div().flex_none().w_px().h_full().bg(border))
            })
            .when_some(new.as_ref(), |d, side| {
                let label = both.then_some(("After", green));
                d.child(side_view(label, side, &root, &dir, cx))
            })
            .into_any_element()
    }

    fn load_markdown(&mut self, file: (String, String), sources: Sources, cx: &mut Context<Self>) {
        if self.markdown.file != file {
            // The documents of another file start at the top.
            self.markdown = MarkdownPreview {
                file,
                ..Default::default()
            };
        }
        self.markdown.sources = Some(sources.clone());
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let (old, new) = sources.clone();
        // Dropping the task of the content loaded before stops its work.
        self.markdown._load = Some(cx.spawn(async move |this, cx| {
            let texts = cx
                .background_executor()
                .spawn(async move {
                    let load = |s: Option<Source>| s.map(|s| read_text(&repo, &s));
                    (load(old), load(new))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.markdown.sources.as_ref() == Some(&sources) {
                    this.show_markdown(texts, cx);
                    cx.notify();
                }
            });
        }));
    }

    fn show_markdown(&mut self, texts: (Option<Loaded>, Option<Loaded>), cx: &mut Context<Self>) {
        let (old, new) = self.markdown.sides.take().unwrap_or((None, None));
        let sides = (side(old, texts.0, cx), side(new, texts.1, cx));
        self.markdown._subs = [&sides.0, &sides.1]
            .into_iter()
            .flatten()
            .filter_map(|side| match side {
                Side::Doc(doc) => {
                    Some(cx.observe(&doc.state, |this, _, cx| this.mark_markdown(cx)))
                }
                Side::Note(_) => None,
            })
            .collect();
        self.markdown.sides = Some(sides);
        self.mark_markdown(cx);
    }

    /// Mark the changed text of both documents, when both have parsed their
    /// text. The views notify for each parse.
    fn mark_markdown(&mut self, cx: &mut Context<Self>) {
        let md = &mut self.markdown;
        let Some((Some(Side::Doc(old)), Some(Side::Doc(new)))) = &mut md.sides else {
            return;
        };
        let (Some(o), Some(n)) = (old.rendered(cx), new.rendered(cx)) else {
            return;
        };
        let texts = (o, n);
        if md.marked.as_ref() == Some(&texts) {
            return;
        }
        let (old_text, new_text) = (texts.0.as_str().to_string(), texts.1.as_str().to_string());
        md.marked = Some(texts);
        // The documents keep the marks of the old texts until the diff is
        // done: they move them with the text that did not change.
        md.marks = None;
        let reveal = !std::mem::replace(&mut md.revealed, true);
        md._diff = Some(cx.spawn(async move |this, cx| {
            let marks = cx
                .background_executor()
                .spawn(async move { changes(&old_text, &new_text, Instant::now() + DIFF_TIME) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.markdown.marks = Some((marks, mark_colors(cx)));
                this.paint_marks(reveal, cx);
            });
        }));
    }

    /// Show the marks in the documents, in the colors of the theme, and
    /// scroll each document to its first change when `reveal` is set.
    fn paint_marks(&mut self, reveal: bool, cx: &mut Context<Self>) {
        let colors = mark_colors(cx);
        let md = &mut self.markdown;
        let (Some((Some(Side::Doc(old)), Some(Side::Doc(new)))), Some((marks, painted))) =
            (&md.sides, &mut md.marks)
        else {
            return;
        };
        // The ranges are for the texts that the diff read. A newer diff
        // marks newer texts.
        let now = (
            old.state.read(cx).rendered_text(),
            new.state.read(cx).rendered_text(),
        );
        if md.marked.as_ref() != Some(&now) {
            return;
        }
        *painted = colors;
        for (doc, marks, color) in [(old, &marks.0, colors.0), (new, &marks.1, colors.1)] {
            doc.state.update(cx, |state, cx| {
                let highlights = marks.iter().map(|r| RangeHighlight::new(r.clone(), color));
                // Ranges of the same rendered text cannot be invalid.
                let _ = state.set_range_highlights(highlights, cx);
                if reveal && let Some(first) = marks.first() {
                    let _ = state.reveal_range(first.clone(), cx);
                }
            });
        }
    }
}

/// The colors of the removed and of the added text, as in the text diff.
fn mark_colors(cx: &App) -> (Hsla, Hsla) {
    let t = cx.theme();
    (t.colors.red.opacity(0.30), t.colors.green.opacity(0.30))
}

/// The text of one side, or why it shows none.
type Loaded = Result<String, String>;

fn read_text(repo: &Repo, source: &Source) -> Loaded {
    let read = || -> anyhow::Result<Loaded> {
        let size = source_size(repo, source)?;
        if size > MAX_BYTES {
            return Ok(Err(format!(
                "The file is too large to show ({}).",
                byte_size(size)
            )));
        }
        let bytes = source_bytes(repo, source)?;
        Ok(Ok(String::from_utf8_lossy(&bytes).into_owned()))
    };
    read().unwrap_or_else(|e| Err(format!("Could not show the file: {e}")))
}

/// `side` with the text that loaded for it: the same view with the new
/// text, so that it keeps its scroll position.
fn side(side: Option<Side>, text: Option<Loaded>, cx: &mut App) -> Option<Side> {
    Some(match (side, text?) {
        (_, Err(note)) => Side::Note(note),
        (Some(Side::Doc(mut doc)), Ok(text)) => {
            doc.set_text(text, cx);
            Side::Doc(doc)
        }
        (_, Ok(text)) => Side::Doc(Doc::new(text, cx)),
    })
}

/// One side: a label when both sides show, and the document below it.
fn side_view(
    label: Option<(&'static str, Hsla)>,
    side: &Side,
    root: &Path,
    dir: &Path,
    cx: &App,
) -> impl IntoElement {
    let t = cx.theme();
    let body = match side {
        Side::Doc(doc) => TextView::new(&doc.state)
            .scrollable(true)
            .image_source(picture_source(root, dir))
            .size_full()
            .px_6()
            .py_4()
            .into_any_element(),
        Side::Note(text) => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(t.colors.muted_foreground)
            .child(text.clone())
            .into_any_element(),
    };
    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .when_some(label, |d, (text, color)| {
            d.child(
                h_flex()
                    .flex_none()
                    .h(px(28.))
                    .px_6()
                    .border_b_1()
                    .border_color(t.colors.border)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color)
                    .child(text),
            )
        })
        .child(div().flex_1().min_h_0().child(body))
}

/// Where a document finds its pictures. A path is relative to the folder of
/// the file, or to `root` with a leading `/`, as on GitHub. The pictures
/// come from the files on disk, also for an older side of the file.
fn picture_source(
    root: &Path,
    dir: &Path,
) -> impl Fn(&SharedUri) -> ImageSource + Send + Sync + 'static {
    let (root, dir) = (root.to_path_buf(), dir.to_path_buf());
    move |uri| {
        if uri.contains("://") || uri.starts_with("data:") {
            return uri.clone().into();
        }
        // `logo.png?raw=true` is a GitHub link to the file itself.
        let path = uri.split(['?', '#']).next().unwrap_or_default();
        let path: PathBuf = match path.strip_prefix('/') {
            Some(path) => root.join(path),
            None => dir.join(path),
        };
        path.into()
    }
}

/// The changed text of `old` and of `new`, by line: a line that changed a
/// little gets the changed words, as in the text diff, and other changed
/// lines get the whole line. The search for the changed lines stops at
/// `deadline`.
fn changes(old: &str, new: &str, deadline: Instant) -> Marks {
    let (ol, nl) = (lines(old), lines(new));
    let ot: Vec<&str> = ol.iter().map(|r| &old[r.clone()]).collect();
    let nt: Vec<&str> = nl.iter().map(|r| &new[r.clone()]).collect();
    let ops = similar::capture_diff_slices_deadline(Algorithm::Myers, &ot, &nt, Some(deadline));
    let (mut om, mut nm) = (Vec::new(), Vec::new());
    let shift = |at: usize| move |r: Range<usize>| (r.start + at)..(r.end + at);
    for op in ops {
        let (tag, o, n) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        // Pair the changed lines one by one, as the text diff does.
        let paired = o.len().min(n.len());
        for k in 0..paired {
            let (a, b) = (ol[o.start + k].clone(), nl[n.start + k].clone());
            if ot[o.start + k] == nt[n.start + k] {
                continue;
            }
            match crate::highlight::changed_words(&old[a.clone()], &new[b.clone()]) {
                Some((ow, nw)) => {
                    om.extend(ow.into_iter().map(shift(a.start)));
                    nm.extend(nw.into_iter().map(shift(b.start)));
                }
                None => {
                    om.push(a);
                    nm.push(b);
                }
            }
        }
        om.extend(ol[o.start + paired..o.end].iter().cloned());
        nm.extend(nl[n.start + paired..n.end].iter().cloned());
    }
    om.retain(|r| !r.is_empty());
    nm.retain(|r| !r.is_empty());
    (om, nm)
}

/// The byte ranges of the lines of `s`, without their line ends.
fn lines(s: &str) -> Vec<Range<usize>> {
    let mut at = 0;
    s.split('\n')
        .map(|line| {
            let range = at..at + line.len();
            at = range.end + 1;
            range
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::{changes, is_markdown_path, lines};
    use std::time::{Duration, Instant};

    fn marked<'a>(text: &'a str, marks: &[std::ops::Range<usize>]) -> Vec<&'a str> {
        marks.iter().map(|r| &text[r.clone()]).collect()
    }

    fn diff(old: &str, new: &str) -> (Vec<String>, Vec<String>) {
        let (om, nm) = changes(old, new, Instant::now() + Duration::from_secs(5));
        let own = |text: &str, m: &[_]| marked(text, m).into_iter().map(String::from).collect();
        (own(old, &om), own(new, &nm))
    }

    #[test]
    fn markdown_paths() {
        assert!(is_markdown_path("README.md"));
        assert!(is_markdown_path("docs/Guide.MARKDOWN"));
        assert!(!is_markdown_path("v1.md/notes"));
        assert!(!is_markdown_path("md"));
        assert!(!is_markdown_path("src/main.rs"));
    }

    #[test]
    fn lines_without_line_ends() {
        let text = "a\nbc\n";
        let got: Vec<_> = lines(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(got, ["a", "bc", ""]);
    }

    #[test]
    fn a_small_edit_marks_the_changed_words() {
        let (old, new) = diff(
            "Title\nThe quick brown fox jumps.\nEnd\n",
            "Title\nThe quick red fox jumps.\nEnd\n",
        );
        assert_eq!(old, ["brown"]);
        assert_eq!(new, ["red"]);
    }

    #[test]
    fn new_and_removed_lines_are_marked_whole() {
        let (old, new) = diff("One\nTwo\nThree\n", "One\nThree\nFour and more\n");
        assert_eq!(old, ["Two"]);
        assert_eq!(new, ["Four and more"]);
    }

    #[test]
    fn a_rewritten_line_is_marked_whole() {
        let (old, new) = diff(
            "Intro\nCats sleep all day.\n",
            "Intro\nTests run on CI now.\n",
        );
        assert_eq!(old, ["Cats sleep all day."]);
        assert_eq!(new, ["Tests run on CI now."]);
    }

    #[test]
    fn the_same_text_has_no_marks() {
        assert_eq!(diff("a\nb\n", "a\nb\n"), (vec![], vec![]));
    }
}
