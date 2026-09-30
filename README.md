<div align="center">

<img src="assets/icon/gibbon.png" width="128" alt="Gibbon icon">

# Gibbon

**Swing between branches.**

A native Git client for macOS, written in Rust on
[GPUI](https://github.com/zed-industries/zed), the GPU-rendered UI framework
behind the Zed editor.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![macOS 14+](https://img.shields.io/badge/macOS-14%2B-black)
![Rust](https://img.shields.io/badge/rust-2024_edition-orange)

</div>

## Why Gibbon

- **Native.** No Electron and no web view. The commit list and the diffs are
  virtualized: Gibbon loads up to 20,000 commits and draws only the rows on
  screen.
- **Cherry-pick the right way around.** Stay on your branch, browse another
  one, and pick the commits you need into yours.
- **Real Git underneath.** Every operation runs the `git` CLI. Your hooks,
  commit signing, credential helpers and SSH keys work as they do in the
  terminal.
- **Keyboard first.** The common actions have shortcuts and live in the
  command palette (<kbd>⌘</kbd><kbd>K</kbd>).
- **Free and open source.** MIT licensed, no account, no telemetry.

## Features

### Tabs

- Each open repository has a tab in the title bar. Opening a repository adds
  a tab, or shows its tab if it is open already.
- **+** lists the recent repositories that have no tab.
- Close a tab with its **×**, a middle-click or <kbd>⌘</kbd><kbd>W</kbd>.

### History and graph

- A commit graph with branch, remote and tag badges, authors and dates.
- **All branches** draws one graph for every branch, remote branch and tag.
- Commit details: message, author and committer, changed files and the diff.

### Diffs

- Syntax colors for about 35 languages (tree-sitter).
- Word-level highlights show exactly what changed inside a line.
- Unified or split (side-by-side) view.
- Changed files show as a list sorted by name (A to Z or Z to A) or as a
  tree of folders. All file lists use the same choice.

### Staging

- Stage and unstage per file, per hunk or per line. Click the line numbers to
  select lines (<kbd>⇧</kbd>-click for a range), then **Stage lines**.
- Discard a file, a hunk, some lines or everything. Gibbon always asks first.
- Gibbon watches the repository and refreshes by itself.

### Cherry-pick into your branch

Click a branch or a pull request in the sidebar to browse it. You stay on your
branch the whole time:

- Commits your branch already has are dimmed.
- Commits you picked before are marked **picked**.
- Select commits (click, <kbd>⌘</kbd>-click, <kbd>⇧</kbd>-click) and click
  **Pick into …**. Merge commits pick against their first parent.

### Interactive rebase

Right-click a commit ▸ **Interactive Rebase from Here…**. Set *pick*,
*reword*, *squash*, *fixup* or *drop* per commit and reorder them. Uncommitted
changes are stashed first and restored after.

### Branches, stashes and conflicts

- Create, rename, delete and switch branches (double-click to switch).
- Delete a remote branch on its remote: right-click it in the sidebar.
- Stash all changes, look at a stash's diff, then apply, pop or drop it.
- When a cherry-pick, rebase, merge or revert stops on a conflict, Gibbon
  shows **Continue**, **Skip** and **Abort**.

### GitHub

- Open pull requests appear in the sidebar, through the
  [GitHub CLI](https://cli.github.com) and its sign-in. Click one to browse its
  commits (forks too) and pick from it, or check it out.
- **Create Pull Request…** on a local branch pushes it and opens GitHub's form.

### Make it yours

Settings (<kbd>⌘</kbd><kbd>,</kbd>) choose the look:

- Light and dark themes, or follow macOS.
- Eight interface fonts: SF Pro, Geist, IBM Plex Sans, Manrope, DM Sans,
  Figtree, Instrument Sans and Inter.
- Eight code fonts: SF Mono, Geist Mono, IBM Plex Mono, Fira Code,
  Source Code Pro, DM Mono, Martian Mono and JetBrains Mono.
- Text sizes and the default diff view.

Gibbon remembers its window and its tabs, and reopens each repository where
you left it: the same view, branch, commit and file.

## Getting started

> [!IMPORTANT]
> Gibbon targets macOS 14 or later. So far it is tested on macOS 27 on Apple
> Silicon only.

Prerequisites:

- [Rust](https://rustup.rs) (the repository pins the toolchain in
  `rust-toolchain.toml`)
- Xcode Command Line Tools
- Optional: the [GitHub CLI](https://cli.github.com) (`brew install gh`) for
  pull requests

Run from source:

```sh
git clone https://github.com/biki/gibbon.git && cd gibbon
cargo run --release -- /path/to/your/repo
```

Build the app and drag it into `/Applications`:

```sh
scripts/bundle.sh      # → target/release/bundle/Gibbon.app
```

The script signs with a *Developer ID Application* certificate when your
keychain has one, and ad hoc otherwise.

## Keyboard shortcuts

| Action | Shortcut |
| --- | --- |
| Command palette | <kbd>⌘</kbd><kbd>K</kbd> |
| Settings | <kbd>⌘</kbd><kbd>,</kbd> |
| Open repository (in a new tab) | <kbd>⌘</kbd><kbd>O</kbd> |
| Close tab | <kbd>⌘</kbd><kbd>W</kbd> |
| Previous / next tab | <kbd>⇧</kbd><kbd>⌘</kbd><kbd>[</kbd> / <kbd>⇧</kbd><kbd>⌘</kbd><kbd>]</kbd>, or <kbd>⌃</kbd><kbd>⇧</kbd><kbd>⇥</kbd> / <kbd>⌃</kbd><kbd>⇥</kbd> |
| Changes / History / All branches | <kbd>⌘</kbd><kbd>1</kbd> / <kbd>⌘</kbd><kbd>2</kbd> / <kbd>⌘</kbd><kbd>3</kbd> |
| Refresh | <kbd>⌘</kbd><kbd>R</kbd> |
| Commit | <kbd>⌘</kbd><kbd>↵</kbd> |
| New branch | <kbd>⇧</kbd><kbd>⌘</kbd><kbd>N</kbd> |
| Stash changes | <kbd>⌥</kbd><kbd>⌘</kbd><kbd>S</kbd> |
| Fetch / Pull / Push | <kbd>⇧</kbd><kbd>⌘</kbd><kbd>F</kbd> / <kbd>⇧</kbd><kbd>⌘</kbd><kbd>P</kbd> / <kbd>⌘</kbd><kbd>P</kbd> |
| Previous / next commit | <kbd>↑</kbd> / <kbd>↓</kbd> |

## Where your data lives

| What | Where |
| --- | --- |
| Settings, recent repositories and the last session | `~/Library/Application Support/gibbon/` |
| Fetched pull request heads | `refs/gibbon/pr/<number>` in your repository |
| Messages of a paused interactive rebase | `.git/gibbon-rebase/` in your repository |

## Known limits

- Hunk and line staging works for changed text files. New, deleted and binary
  files stage as a whole.
- The split view cuts lines at 1,200 characters and has no horizontal scroll.
- The file watcher reads the top-level `.gitignore` only, so ignored files in
  deeper folders can cause extra (harmless) refreshes.
- Interactive rebase flattens merge commits and has no *edit* or *exec* step.
- Pull requests: open ones only, no reviews or comments.
- No in-app updates and no notarized builds yet.

## Development

```sh
git config core.hooksPath .githooks   # checks commit messages
scripts/dev.sh ~/path/to/repo         # rebuild and restart on every save
cargo test                            # Git backend, graph, highlighting and watcher tests
```

`scripts/dev.sh` rebuilds Gibbon and restarts it when a source file changes,
in about 2 seconds. Gibbon reopens on the same screen and does not take focus
from your editor. In debug builds, <kbd>⌘</kbd><kbd>⌥</kbd><kbd>I</kbd> opens
the GPUI inspector: pick an element and edit its style live.

Commits follow [Conventional Commits](https://www.conventionalcommits.org/).
See [CONTRIBUTING.md](CONTRIBUTING.md) for the types and scopes.

Environment variables for automated UI checks:

| Variable | Effect |
| --- | --- |
| `GIBBON_BACKGROUND=1` | Pop-up window that stays on top but never takes focus (GPUI stops painting hidden windows) |
| `GIBBON_BROWSE=<branch or ref>` | Start in browse mode |
| `GIBBON_VIEW=changes\|all` | Start in that view |
| `GIBBON_FILE=<path>` | Select that changed file |
| `GIBBON_DIFF=split` | Split diffs |
| `GIBBON_REBASE=<sha>` | Open the rebase planner from that commit |
| `GIBBON_STASH=<n>` | Open `stash@{n}` |
| `GIBBON_DIALOG=new-branch\|stash\|palette\|settings` | Open that dialog |
| `GIBBON_INSPECTOR=1` | Open the inspector (debug builds) |
| `GIBBON_NO_ACTIVATE=1` | Open the window without taking focus (the dev loop sets it) |

## Built with

[GPUI](https://github.com/zed-industries/zed) and
[GPUI Kit](https://github.com/longbridge/gpui-kit) for the UI,
[tree-sitter](https://tree-sitter.github.io) for syntax colors,
[similar](https://github.com/mitsuhiko/similar) for word diffs and
[notify](https://github.com/notify-rs/notify) for file watching.

## License

[MIT](LICENSE) © 2026 Benjamin Kaspar.
All bundled fonts use the SIL Open Font License 1.1; each license is in
`assets/fonts/`. `scripts/fetch-fonts.sh` downloads them from Google Fonts.
SF Pro and SF Mono come from macOS and are not bundled.
