# Contributing

## Setup

```sh
git config core.hooksPath .githooks   # once per clone: checks commit messages
cargo test
```

## Commit messages

Gibbon uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
A `commit-msg` hook checks every commit locally, and CI checks every push to
`main` and every pull request title.

```
<type>(<scope>): <description>

[optional body: what changed and why]

[optional footer: BREAKING CHANGE: …, Refs #12]
```

- Write the description in the imperative, in lower case, with no period at
  the end: `add a split view`, not `Added a split view.`
- Keep the subject line at 72 characters or less.
- The scope is optional. Use one when the change stays in one area.

### Types

| Type | Use for |
| --- | --- |
| `feat` | A new feature for users |
| `fix` | A bug fix for users |
| `perf` | Faster or lighter, same behavior |
| `refactor` | Code change with no change in behavior |
| `style` | Formatting only (`cargo fmt`) |
| `test` | Tests only |
| `docs` | README, CONTRIBUTING, code comments |
| `build` | Cargo, dependencies, `scripts/bundle.sh`, the icon |
| `ci` | GitHub Actions |
| `chore` | Anything else that users do not see |
| `revert` | Undo an earlier commit |

### Scopes

| Scope | Area |
| --- | --- |
| `history` | Commit list, commit detail |
| `graph` | Commit graph lanes and painting |
| `diff` | Diff view, syntax colors, word diff |
| `staging` | Changes view, stage / unstage / discard, commit box |
| `pick` | Cherry-pick and browse mode |
| `rebase` | Interactive rebase |
| `stash` | Stashes |
| `branches` | Create, rename, delete, switch |
| `github` | Pull requests through `gh` |
| `palette` | Command palette |
| `settings` | Settings and their dialog |
| `theme` | Colors and fonts |
| `watch` | File watcher |
| `git` | The Git backend in `src/git.rs` |
| `sidebar` | The sidebar |
| `deps` | Dependency updates (with `build`) |

### Breaking changes

Add `!` after the type or scope and explain it in a footer:

```
feat(settings)!: move settings to settings.toml

BREAKING CHANGE: settings.json is no longer read. Set your choices again.
```

### Examples

```
feat(diff): add a split view
fix(staging): keep the line selection when the diff reloads
perf(graph): reuse lane paths between frames
build(deps): update gpui-kit to 0.7
docs: add keyboard shortcuts to the README
```

Git's own messages (`Merge …`, `Revert "…"`, `fixup! …`, `squash! …`) pass
the check. In an emergency, `git commit --no-verify` skips the hook, but CI
still checks the subject.
