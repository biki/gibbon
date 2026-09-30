//! Per-line styles for a diff: syntax colors from tree-sitter, and the
//! changed words of paired removed / added lines.
//!
//! The old side (context + removed lines) and the new side (context + added
//! lines) are highlighted as two texts, so strings and comments that span
//! lines keep their colors. Runs on the background executor.

use std::ops::Range;

use gpui_kit::HighlightStyle;
use gpui_kit::component::Rope;
use gpui_kit::component::highlighter::{HighlightTheme, SyntaxHighlighter};
use similar::{Algorithm, DiffOp};

use crate::git::{FileDiff, LineKind};

pub type Spans = Vec<(Range<usize>, HighlightStyle)>;
/// Changed byte ranges of the old line and of the new line.
pub type WordRanges = (Vec<Range<usize>>, Vec<Range<usize>>);

#[derive(Default)]
pub struct DiffStyles {
    /// Syntax spans per diff line, byte ranges within the line.
    pub syntax: Vec<Spans>,
    /// Changed byte ranges per line, for removed and added lines.
    pub words: Vec<Vec<Range<usize>>>,
}

pub fn compute(file: &FileDiff, theme: &HighlightTheme) -> DiffStyles {
    DiffStyles {
        syntax: syntax(file, theme),
        words: words(file),
    }
}

/// The tree-sitter language for a path, if the kit has its grammar.
pub fn language_for(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "makefile" | "gnumakefile" => return Some("make"),
        "cmakelists.txt" => return Some("cmake"),
        "cargo.lock" => return Some("toml"),
        ".bashrc" | ".zshrc" | ".profile" => return Some("bash"),
        _ => {}
    }
    let ext = lower.rsplit_once('.')?.1;
    Some(match ext {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "json" | "jsonc" => "json",
        "py" | "pyi" => "python",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "cs" => "csharp",
        "css" | "scss" => "css",
        "html" | "htm" => "html",
        "md" | "markdown" | "mdx" => "markdown",
        "toml" => "toml",
        "yml" | "yaml" => "yaml",
        "sh" | "bash" | "zsh" => "bash",
        "sql" => "sql",
        "lua" => "lua",
        "php" => "php",
        "scala" => "scala",
        "zig" => "zig",
        "ex" | "exs" => "elixir",
        "proto" => "proto",
        "graphql" | "gql" => "graphql",
        "svelte" => "svelte",
        "astro" => "astro",
        "erb" => "erb",
        "ejs" => "ejs",
        "cmake" => "cmake",
        "mk" => "make",
        _ => return None,
    })
}

fn syntax(file: &FileDiff, theme: &HighlightTheme) -> Vec<Spans> {
    let n = file.lines.len();
    let mut out = vec![Vec::new(); n];
    let Some(lang) = language_for(&file.path) else {
        return out;
    };
    let (mut old, mut new) = (String::new(), String::new());
    let (mut old_at, mut new_at) = (vec![None; n], vec![None; n]);
    for (i, line) in file.lines.iter().enumerate() {
        match line.kind {
            LineKind::Context => {
                old_at[i] = Some(push_line(&mut old, &line.text));
                new_at[i] = Some(push_line(&mut new, &line.text));
            }
            LineKind::Add => new_at[i] = Some(push_line(&mut new, &line.text)),
            LineKind::Del => old_at[i] = Some(push_line(&mut old, &line.text)),
            // Hunks are not adjacent: a blank line keeps them apart.
            LineKind::Hunk => {
                old.push('\n');
                new.push('\n');
            }
            LineKind::Note => {}
        }
    }
    let old_styles = highlight(lang, &old, theme);
    let new_styles = highlight(lang, &new, theme);
    for i in 0..n {
        out[i] = match (new_at[i].clone(), old_at[i].clone()) {
            (Some(at), _) => slice(&new_styles, at),
            (None, Some(at)) => slice(&old_styles, at),
            (None, None) => continue,
        };
    }
    out
}

fn push_line(buf: &mut String, line: &str) -> Range<usize> {
    let start = buf.len();
    buf.push_str(line);
    let end = buf.len();
    buf.push('\n');
    start..end
}

fn highlight(lang: &str, text: &str, theme: &HighlightTheme) -> Spans {
    if text.is_empty() {
        return vec![];
    }
    let mut h = SyntaxHighlighter::new(lang);
    h.update(None, &Rope::from(text), None);
    h.styles(&(0..text.len()), theme)
}

/// The spans inside `at`, relative to its start. `styles` is sorted and
/// does not overlap.
fn slice(styles: &Spans, at: Range<usize>) -> Spans {
    let first = styles.partition_point(|(r, _)| r.end <= at.start);
    styles[first..]
        .iter()
        .take_while(|(r, _)| r.start < at.end)
        .filter_map(|(r, style)| {
            let (a, b) = (r.start.max(at.start), r.end.min(at.end));
            (a < b && *style != HighlightStyle::default())
                .then(|| ((a - at.start)..(b - at.start), *style))
        })
        .collect()
}

/// Pair each run of removed lines with the added lines after it, line by
/// line, and mark the words that differ.
fn words(file: &FileDiff) -> Vec<Vec<Range<usize>>> {
    let lines = &file.lines;
    let n = lines.len();
    let mut out = vec![Vec::new(); n];
    let mut i = 0;
    while i < n {
        if lines[i].kind != LineKind::Del {
            i += 1;
            continue;
        }
        let dels = i;
        while i < n && lines[i].kind == LineKind::Del {
            i += 1;
        }
        let adds = i;
        while i < n && lines[i].kind == LineKind::Add {
            i += 1;
        }
        for k in 0..(adds - dels).min(i - adds) {
            let (o, a) = (dels + k, adds + k);
            if let Some((old, new)) = changed_words(&lines[o].text, &lines[a].text) {
                out[o] = old;
                out[a] = new;
            }
        }
    }
    out
}

/// Words, runs of spaces, and single punctuation characters.
fn tokens(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let class = |c: char| {
            if c.is_alphanumeric() || c == '_' {
                0
            } else if c.is_whitespace() {
                1
            } else {
                2
            }
        };
        let k = class(c);
        let mut end = start + c.len_utf8();
        if k != 2 {
            while let Some(&(i, next)) = chars.peek() {
                if class(next) != k {
                    break;
                }
                end = i + next.len_utf8();
                chars.next();
            }
        }
        out.push(start..end);
    }
    out
}

/// Changed ranges in both lines, or None when the lines share too little
/// for a word diff to help.
pub fn changed_words(old: &str, new: &str) -> Option<WordRanges> {
    let (ot, nt) = (tokens(old), tokens(new));
    let ow: Vec<&str> = ot.iter().map(|r| &old[r.clone()]).collect();
    let nw: Vec<&str> = nt.iter().map(|r| &new[r.clone()]).collect();
    let ops = similar::capture_diff_slices(Algorithm::Myers, &ow, &nw);
    let (mut o, mut n) = (Vec::new(), Vec::new());
    let mut same = 0;
    let span = |t: &[Range<usize>], at: usize, len: usize| t[at].start..t[at + len - 1].end;
    for op in ops {
        match op {
            DiffOp::Equal { old_index, len, .. } => {
                same += span(&ot, old_index, len).len();
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => o.push(span(&ot, old_index, old_len)),
            DiffOp::Insert {
                new_index, new_len, ..
            } => n.push(span(&nt, new_index, new_len)),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                o.push(span(&ot, old_index, old_len));
                n.push(span(&nt, new_index, new_len));
            }
        }
    }
    let longest = old.trim().len().max(new.trim().len()).max(1);
    if same * 10 < longest * 4 || (o.is_empty() && n.is_empty()) {
        return None;
    }
    Some((o, n))
}

#[cfg(test)]
mod tests {
    use super::{changed_words, language_for};

    #[test]
    fn word_diff_marks_only_the_change() {
        let (o, n) = changed_words("let port = 8080;", "let port = 3000;").unwrap();
        assert_eq!(o, vec![11..15]);
        assert_eq!(n, vec![11..15]);
    }

    #[test]
    fn unrelated_lines_get_no_word_diff() {
        assert!(changed_words("fn main() {}", "// a comment about it").is_none());
    }

    #[test]
    fn languages_by_extension() {
        assert_eq!(language_for("src/app/mod.rs"), Some("rust"));
        assert_eq!(language_for("web/App.tsx"), Some("tsx"));
        assert_eq!(language_for("Makefile"), Some("make"));
        assert_eq!(language_for("LICENSE"), None);
    }
}
