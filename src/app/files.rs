//! File lists: a flat list sorted by name, or a tree of folders.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use super::*;

/// Tree rows move right by this much per folder level: the width of the
/// chevron and its gap, so a file badge lines up with its folder's icon.
const INDENT: f32 = 16.;

/// The paths of `files`, for `layout`.
pub(super) fn paths(files: &[FileDiff]) -> Vec<&str> {
    files.iter().map(|f| f.path.as_str()).collect()
}

/// Row indent of a file at `depth` of a tree.
pub(super) fn file_indent(depth: usize) -> Pixels {
    px(8. + INDENT * depth as f32)
}

/// A folder of the tree.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Dir {
    /// The folder's path.
    pub key: String,
    /// Several levels ("src/app") when the folders between hold nothing else.
    pub name: String,
    pub depth: usize,
    pub collapsed: bool,
}

/// One row of a file list.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum FileRow {
    Dir(Dir),
    /// The file at `ix` of the input. `depth` is `None` in the flat list.
    File {
        ix: usize,
        depth: Option<usize>,
    },
}

/// Folder and name of a path.
fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// Ignore case first, then compare exactly, so the order is stable. It
/// compares char by char: a sort calls it often, and must not make strings.
fn name_cmp(a: &str, b: &str) -> Ordering {
    fn lower(s: &str) -> impl Iterator<Item = char> + '_ {
        s.chars().flat_map(char::to_lowercase)
    }
    lower(a).cmp(lower(b)).then_with(|| a.cmp(b))
}

#[derive(Default)]
struct Node<'a> {
    dirs: BTreeMap<&'a str, Node<'a>>,
    files: Vec<usize>,
}

/// The rows for `paths`: sorted by file name, or as a tree with folders
/// first. `desc` sorts Z to A. The contents of `collapsed` folders are left out.
pub(super) fn layout(
    paths: &[&str],
    tree: bool,
    desc: bool,
    collapsed: &dyn Fn(&str) -> bool,
) -> Vec<FileRow> {
    let order = |o: Ordering| if desc { o.reverse() } else { o };
    if !tree {
        let mut ix: Vec<usize> = (0..paths.len()).collect();
        ix.sort_by(|&a, &b| {
            let ((dir_a, a), (dir_b, b)) = (split(paths[a]), split(paths[b]));
            order(name_cmp(a, b).then_with(|| name_cmp(dir_a, dir_b)))
        });
        return ix
            .into_iter()
            .map(|ix| FileRow::File { ix, depth: None })
            .collect();
    }
    let mut root = Node::default();
    for (i, path) in paths.iter().enumerate() {
        let mut node = &mut root;
        for part in split(path).0.split('/').filter(|p| !p.is_empty()) {
            node = node.dirs.entry(part).or_default();
        }
        node.files.push(i);
    }
    let mut rows = Vec::new();
    push_node(&root, "", 0, paths, &order, collapsed, &mut rows);
    rows
}

fn push_node(
    node: &Node,
    prefix: &str,
    depth: usize,
    paths: &[&str],
    order: &dyn Fn(Ordering) -> Ordering,
    collapsed: &dyn Fn(&str) -> bool,
    rows: &mut Vec<FileRow>,
) {
    let mut dirs: Vec<_> = node.dirs.iter().collect();
    dirs.sort_by(|a, b| order(name_cmp(a.0, b.0)));
    for (&name, mut sub) in dirs {
        let mut key = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        let mut label = name.to_string();
        while sub.files.is_empty() && sub.dirs.len() == 1 {
            let Some((&next, only)) = sub.dirs.iter().next() else {
                break;
            };
            key = format!("{key}/{next}");
            label = format!("{label}/{next}");
            sub = only;
        }
        let shut = collapsed(&key);
        rows.push(FileRow::Dir(Dir {
            key: key.clone(),
            name: label,
            depth,
            collapsed: shut,
        }));
        if !shut {
            push_node(sub, &key, depth + 1, paths, order, collapsed, rows);
        }
    }
    let mut files = node.files.clone();
    files.sort_by(|&a, &b| order(name_cmp(split(paths[a]).1, split(paths[b]).1)));
    rows.extend(files.into_iter().map(|ix| FileRow::File {
        ix,
        depth: Some(depth),
    }));
}

/// The first file that `layout` shows.
pub(super) fn first(
    paths: &[&str],
    tree: bool,
    desc: bool,
    collapsed: &dyn Fn(&str) -> bool,
) -> Option<usize> {
    layout(paths, tree, desc, collapsed)
        .into_iter()
        .find_map(|r| match r {
            FileRow::File { ix, .. } => Some(ix),
            FileRow::Dir(_) => None,
        })
}

impl GitApp {
    /// The rows of the file list `scope`, in the view and order of the
    /// settings.
    pub(super) fn file_rows(&self, scope: &'static str, paths: &[&str], cx: &App) -> Vec<FileRow> {
        let s = crate::settings::get(cx);
        layout(paths, s.file_tree, s.file_sort_desc, &|dir| {
            self.collapsed_dirs.contains(&(scope, dir.to_string()))
        })
    }

    pub(super) fn toggle_dir(&mut self, scope: &'static str, key: String, cx: &mut Context<Self>) {
        let key = (scope, key);
        if !self.collapsed_dirs.remove(&key) {
            self.collapsed_dirs.insert(key);
        }
        cx.notify();
    }

    /// A folder row of the list `scope`: a click opens or closes it.
    pub(super) fn dir_row(
        &self,
        scope: &'static str,
        dir: &Dir,
        id: impl Into<ElementId>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let muted = t.colors.muted_foreground;
        let key = dir.key.clone();
        h_flex()
            .id(id)
            .w_full()
            .h(px(28.))
            .pl(file_indent(dir.depth))
            .pr_2()
            .gap_1()
            .rounded(t.radius)
            .cursor_pointer()
            .child(hover_fill(t.colors.list_hover, t.radius))
            .child(
                Icon::new(if dir.collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(px(12.))
                .text_color(muted),
            )
            .child(
                Icon::new(if dir.collapsed {
                    IconName::Folder
                } else {
                    IconName::FolderOpen
                })
                .size(px(14.))
                .text_color(muted),
            )
            .child(
                div()
                    .ml_1()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(dir.name.clone()),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_dir(scope, key.clone(), cx)))
            .into_any_element()
    }
}

/// The bar above the files of a commit or stash: count, line counts, and
/// the view buttons.
pub(super) fn files_bar(files: &[FileDiff], cx: &App) -> Div {
    let t = cx.theme();
    let adds: u32 = files.iter().map(|f| f.additions).sum();
    let dels: u32 = files.iter().map(|f| f.deletions).sum();
    h_flex()
        .flex_none()
        .h(px(30.))
        .px_3()
        .gap_2()
        .text_size(px(12.))
        .text_color(t.colors.muted_foreground)
        .child(div().flex_1().min_w_0().truncate().child(format!(
            "{} file{} changed",
            files.len(),
            history::plural(files.len())
        )))
        .child(div().text_color(t.colors.green).child(format!("+{adds}")))
        .child(div().text_color(t.colors.red).child(format!("−{dels}")))
        .child(view_buttons(cx))
}

/// The sort button and the list or tree switch, for the bar above a file
/// list. They change the settings, so every file list follows.
pub(super) fn view_buttons(cx: &App) -> impl IntoElement {
    let s = crate::settings::get(cx);
    let desc = s.file_sort_desc;
    h_flex()
        .flex_none()
        .gap_1()
        .child(
            Button::new("files-sort")
                .ghost()
                .small()
                .icon(Icon::new(if desc {
                    IconName::ArrowDownZA
                } else {
                    IconName::ArrowDownAZ
                }))
                .tooltip(if desc { "Names Z to A" } else { "Names A to Z" })
                .on_click(|_, _, cx| {
                    crate::settings::update(cx, |s| s.file_sort_desc = !s.file_sort_desc)
                }),
        )
        .child(segmented(
            "files-view",
            &[
                (Segment::Icon(IconName::List, "List, sorted by name"), false),
                (
                    Segment::Icon(IconName::ListTree, "Tree, grouped by folder"),
                    true,
                ),
            ],
            s.file_tree,
            |tree, _, cx| crate::settings::update(cx, |s| s.file_tree = tree),
            cx,
        ))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::{FileRow, layout, split};

    const PATHS: [&str; 6] = [
        "src/app/mod.rs",
        "README.md",
        "src/app/diff.rs",
        "src/main.rs",
        "assets/icons/app.svg",
        "Cargo.toml",
    ];

    fn show(rows: &[FileRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                FileRow::Dir(d) => format!("{} {}/", d.depth, d.name),
                FileRow::File { ix, depth } => match depth {
                    Some(d) => format!("{d} {}", split(PATHS[*ix]).1),
                    None => PATHS[*ix].to_string(),
                },
            })
            .collect()
    }

    #[test]
    fn list_sorts_by_file_name() {
        let rows = layout(&PATHS, false, false, &|_| false);
        assert_eq!(
            show(&rows),
            [
                "assets/icons/app.svg",
                "Cargo.toml",
                "src/app/diff.rs",
                "src/main.rs",
                "src/app/mod.rs",
                "README.md",
            ]
        );
        let rows = layout(&PATHS, false, true, &|_| false);
        assert_eq!(show(&rows)[0], "README.md");
        assert_eq!(show(&rows)[5], "assets/icons/app.svg");
    }

    #[test]
    fn tree_puts_folders_first_and_joins_single_folders() {
        let rows = layout(&PATHS, true, false, &|_| false);
        assert_eq!(
            show(&rows),
            [
                "0 assets/icons/",
                "1 app.svg",
                "0 src/",
                "1 app/",
                "2 diff.rs",
                "2 mod.rs",
                "1 main.rs",
                "0 Cargo.toml",
                "0 README.md",
            ]
        );
    }

    #[test]
    fn tree_hides_collapsed_folders() {
        let rows = layout(&PATHS, true, true, &|dir| dir == "src/app");
        assert_eq!(
            show(&rows),
            [
                "0 src/",
                "1 app/",
                "1 main.rs",
                "0 assets/icons/",
                "1 app.svg",
                "0 README.md",
                "0 Cargo.toml",
            ]
        );
        assert!(matches!(&rows[1], FileRow::Dir(d) if d.key == "src/app" && d.collapsed));
    }
}
