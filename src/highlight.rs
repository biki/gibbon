//! Per-line styles for a diff: syntax colors from tree-sitter, and the
//! changed words of paired removed / added lines.
//!
//! The old side (context + removed lines) and the new side (context + added
//! lines) are highlighted as two texts, so strings and comments that span
//! lines keep their colors. Runs on the background executor.

use std::ops::Range;
use std::sync::Once;

use gpui_kit::HighlightStyle;
use gpui_kit::component::Rope;
use gpui_kit::component::highlighter::{
    GrammarConfig, HighlightTheme, LanguageRegistry, SyntaxHighlighter,
};
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

/// Repair the grammars of the kit whose highlight queries are missing,
/// incomplete or do not compile: their files had no colors, or only a few.
/// Call it before the first highlight. Later calls do nothing.
pub fn register_languages() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let registry = LanguageRegistry::singleton();
        for config in repaired_languages(registry) {
            registry.register(&config.name.clone(), &config);
        }
    });
}

fn repaired_languages(registry: &LanguageRegistry) -> Vec<GrammarConfig> {
    let mut out = Vec::new();
    // The JavaScript query of the kit has no JSX tags.
    if let Some(mut js) = registry.language("javascript") {
        js.highlights = format!(
            "{}{}",
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            js.highlights
        )
        .into();
        out.push(js);
    }
    // The kit gives TSX only the TypeScript add-on query: no keywords,
    // strings or comments. Use the full TypeScript query, plus JSX.
    if let Some(ts) = registry.language("typescript") {
        out.push(GrammarConfig {
            name: "tsx".into(),
            language: Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            highlights: format!(
                "{}{}",
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                ts.highlights
            )
            .into(),
            ..ts
        });
    }
    // The C++ query only adds to the C query.
    if let Some(mut cpp) = registry.language("cpp") {
        cpp.highlights = format!("{}{}", cpp.highlights, tree_sitter_c::HIGHLIGHT_QUERY).into();
        out.push(cpp);
    }
    let own = |name: &str, language: tree_sitter::Language, highlights: &str| {
        GrammarConfig::new(name, language, vec![], highlights, "", "")
    };
    // The Kotlin query of the kit names a node that its grammar lacks.
    out.push(own(
        "kotlin",
        tree_sitter_kotlin_sg::LANGUAGE.into(),
        tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY,
    ));
    // The kit has no query for these.
    out.push(own(
        "swift",
        tree_sitter_swift::LANGUAGE.into(),
        tree_sitter_swift::HIGHLIGHTS_QUERY,
    ));
    out.push(own(
        "csharp",
        tree_sitter_c_sharp::LANGUAGE.into(),
        tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
    ));
    out.push(own(
        "cmake",
        tree_sitter_cmake::LANGUAGE.into(),
        tree_sitter_cmake::HIGHLIGHTS_QUERY,
    ));
    // The grammars of these come without a query.
    for (name, query) in [("proto", PROTO_QUERY), ("graphql", GRAPHQL_QUERY)] {
        if let Some(language) = registry.language(name).and_then(|c| c.language) {
            out.push(own(name, language, query));
        }
    }
    out
}

const PROTO_QUERY: &str = r#"
[
  "syntax" "edition" "package" "import" "weak" "public" "option"
  "message" "enum" "oneof" "map" "extend" "extensions" "service" "rpc"
  "returns" "stream" "repeated" "optional" "required" "reserved" "to" "max"
] @keyword
[(key_type) (type) (message_name) (enum_name) (service_name)] @type
(rpc_name) @function
[(string) "\"proto2\"" "\"proto3\""] @string
(escape_sequence) @string.escape
[(int_lit) (float_lit)] @number
[(true) (false)] @boolean
(comment) @comment
["(" ")" "[" "]" "{" "}" "<" ">"] @punctuation.bracket
"#;

const GRAPHQL_QUERY: &str = r#"
[
  "query" "mutation" "subscription" "fragment" "on" "type" "interface"
  "union" "enum" "input" "scalar" "schema" "extend" "directive"
  "implements" "repeatable"
] @keyword
(named_type (name) @type)
(operation_definition (name) @function)
(fragment_name (name) @function)
(field (name) @property)
(field_definition (name) @property)
(argument (name) @attribute)
(variable) @variable
(directive "@" @attribute (name) @attribute)
[(string_value) (description)] @string
[(int_value) (float_value)] @number
(boolean_value) @boolean
[(null_value) (enum_value)] @constant
(comment) @comment
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
"#;

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
    use gpui_kit::HighlightStyle;
    use gpui_kit::component::highlighter::{HighlightTheme, LanguageRegistry};

    /// The text of the colored spans of `text`.
    fn colored(lang: &str, text: &str) -> Vec<String> {
        super::register_languages();
        let theme = HighlightTheme::default_dark();
        super::highlight(lang, text, &theme)
            .into_iter()
            .filter(|(_, style)| *style != HighlightStyle::default())
            .map(|(r, _)| text[r].to_string())
            .collect()
    }

    #[test]
    fn repaired_queries_compile() {
        super::register_languages();
        let registry = LanguageRegistry::singleton();
        for config in super::repaired_languages(registry) {
            let source = format!(
                "{}{}{}",
                config.injections, config.locals, config.highlights
            );
            let query = tree_sitter::Query::new(config.language.as_ref().unwrap(), &source);
            assert!(query.is_ok(), "{}: {:?}", config.name, query.err());
        }
    }

    #[test]
    fn every_language_has_colors() {
        let samples = [
            (
                "web/App.tsx",
                "export function App() { return <div>\"hi\"</div>; } // c",
            ),
            (
                "web/App.jsx",
                "function App() { return <div>{1}</div>; } // c",
            ),
            ("web/app.ts", "export const x: number = 1; // c"),
            ("src/main.rs", "fn main() {} // c"),
            ("main.cpp", "int main() { return 0; } // c"),
            ("main.c", "int main() { return 0; } // c"),
            ("Main.kt", "fun main() { val s = \"s\" } // c"),
            ("main.swift", "func main() { let s = \"s\" } // c"),
            ("Main.cs", "class A { void F() { string s = \"s\"; } } // c"),
            ("CMakeLists.txt", "set(A \"s\") # c"),
            ("api.proto", "message A { string s = 1; } // c"),
            ("schema.graphql", "query A { b(c: \"s\") { d } } # c"),
            ("main.go", "package main\nfunc main() {} // c"),
            ("main.py", "def f():\n    return 1  # c"),
            ("a.rb", "def f\n  1 # c\nend"),
            ("A.java", "class A { void f() {} } // c"),
            ("a.php", "<?php function f() { return 1; } // c"),
            ("A.scala", "object A { def f = \"s\" }"),
            ("a.lua", "local function f() return 1 end -- c"),
            ("a.zig", "pub fn main() void {} // c"),
            ("a.ex", "defmodule A do\nend # c"),
            ("a.sh", "f() { echo \"s\"; } # c"),
            ("a.sql", "SELECT a FROM b; -- c"),
            ("a.css", "a { color: red; } /* c */"),
            ("a.html", "<div class=\"a\"></div>"),
            ("a.json", "{\"a\": 1}"),
            ("a.toml", "a = \"s\" # c"),
            ("a.yaml", "a: \"s\" # c"),
            ("a.md", "# Title\n"),
            ("a.svelte", "<div class=\"a\"></div>"),
            ("a.astro", "<div class=\"a\"></div>"),
            ("a.erb", "<div><%= f %></div>"),
            ("a.ejs", "<div><%= f %></div>"),
            ("Makefile", "all: a.o"),
        ];
        for (path, text) in samples {
            let lang = language_for(path).unwrap();
            assert!(!colored(lang, text).is_empty(), "{path}: no colors");
        }
    }

    #[test]
    fn tsx_colors_keywords_strings_comments_and_tags() {
        let words = colored(
            "tsx",
            "export function App(p: Props) { return <div className=\"a\">{p.x}</div>; } // c",
        );
        for word in ["export", "function", "return", "div", "\"a\"", "// c"] {
            assert!(words.iter().any(|w| w == word), "{word} in {words:?}");
        }
    }

    #[test]
    fn proto_colors_the_syntax_version() {
        let words = colored("proto", "syntax = \"proto3\";\n");
        assert_eq!(words, ["syntax", "\"proto3\""]);
    }

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
