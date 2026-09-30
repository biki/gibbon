# Gibbon

A native Git client for macOS, written in Rust on
[GPUI](https://github.com/zed-industries/zed) through
[gpui-kit](https://crates.io/crates/gpui-kit). The stack follows
[Tusk](https://github.com/alpcanaydin/tusk).

## Run

```sh
cargo run --release -- /path/to/repo   # from source
scripts/bundle.sh                      # → target/release/bundle/Gibbon.app
```

Without a path, the app opens the last repository. Drag `Gibbon.app` into
`/Applications` to keep it.

## Features

**History**
- A virtualized commit list with a commit graph, ref badges, authors and
  dates. The app loads up to 20,000 commits.
- **All branches** shows one graph for every branch, remote branch and tag.
- Commit detail: message, author and committer, changed files, and the diff.

**Diffs**
- Syntax colors for about 35 languages (tree-sitter), and the changed words
  of a changed line in a stronger color.
- Unified or split (side by side) view.

**Changes**
- Stage and unstage per file, per hunk, or per line: click the line numbers
  to select lines (⇧-click for a range), then **Stage lines**.
- Discard a file, a hunk, some lines, or everything. The app asks first.
- Commit box, and **Stash** for all changes.
- The app watches the repository and refreshes by itself.

**Cherry-pick from the branch you are on**
- Click a branch or a pull request in the sidebar to browse it. You stay on
  your branch. The list dims the commits your branch has and marks earlier
  picks with "picked".
- Select commits (click, ⌘-click, ⇧-click) and click **Pick into …**. Merge
  commits pick against their first parent.

**Interactive rebase**
- Right-click a commit in History ▸ **Interactive Rebase from Here…**.
- Set pick, reword, squash, fixup or drop per commit, and move commits up
  and down. Uncommitted changes are stashed first and restored after.

**When an operation stops on a conflict**
- A cherry-pick, rebase, merge or revert that stops shows **Continue**,
  **Skip** and **Abort** in the Changes view.

**Branches and stashes**
- Create (⇧⌘N, or from a branch or commit), rename, delete, switch
  (double-click).
- Stashes in the sidebar: look at the diff, then **Apply**, **Pop** or
  **Drop**.

**GitHub**
- Open pull requests in the sidebar, through the `gh` CLI and its sign-in.
  Click one to browse its commits (forks too), or check it out.
- **Create Pull Request…** on a local branch pushes it and opens GitHub's form.

**Everywhere**
- Command palette (⌘K): actions, branches, pull requests, repositories.
- Settings (⌘,): appearance, text sizes, default diff view.
- Fetch, Pull, Push with ahead and behind counts. Inter for the UI,
  JetBrains Mono for code.

All Git work runs the `git` CLI. Hooks, commit signing, credential helpers
and SSH keys behave as they do in your terminal.

## Shortcuts

| Action | Shortcut |
| --- | --- |
| Command palette | ⌘K |
| Settings | ⌘, |
| Open repository | ⌘O |
| Changes / History / All branches | ⌘1 / ⌘2 / ⌘3 |
| Refresh | ⌘R |
| Commit | ⌘↵ |
| New branch | ⇧⌘N |
| Stash changes | ⌥⌘S |
| Fetch / Pull / Push | ⇧⌘F / ⇧⌘P / ⌘P |
| Previous / next commit | ↑ / ↓ |

## Limits

- Hunk and line staging works for changed text files. New, deleted and
  binary files stage as a whole.
- The split view cuts lines at 1,200 characters and has no horizontal scroll.
- The file watcher reads the top-level `.gitignore` only. Ignored files in
  deeper folders cause extra (harmless) refreshes.
- Interactive rebase flattens merge commits and has no `edit` or `exec` step.
- Pull requests: open ones only, no reviews or comments.
- No in-app updates yet (Sparkle needs a hosted feed). `bundle.sh` signs with
  a Developer ID when your keychain has one, else ad hoc. It does not
  notarize.

## Development

```sh
cargo test          # git backend, graph, highlight and watcher tests
```

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

## Licenses

The code is MIT. Inter and JetBrains Mono use the SIL Open Font License 1.1
(`assets/fonts/`). The theme code follows a pattern from Tusk (MIT).
