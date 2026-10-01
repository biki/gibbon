# Contributing

## Setup

```sh
git config core.hooksPath .githooks   # once per clone: checks formatting and commit messages
cargo test
```

## Formatting

The code follows the default rustfmt style, with no `rustfmt.toml`. Run
`cargo fmt` before you commit. A `pre-commit` hook rejects staged Rust files
that rustfmt would change.

`.git-blame-ignore-revs` lists the commits that only reformat the code.
GitHub skips them in its blame view. For `git blame`, run once per clone:
`git config blame.ignoreRevsFile .git-blame-ignore-revs`.

## Development loop

```sh
scripts/dev.sh ~/path/to/repo
```

The loop rebuilds Gibbon and restarts it on every save to `src/` or
`assets/` (about 2 seconds for a debug build). A failed build keeps the
running app. Gibbon reopens on the same screen and leaves the focus in your
editor. Quit Gibbon (<kbd>⌘</kbd><kbd>Q</kbd>) to end the loop.

For visual tuning, <kbd>⌘</kbd><kbd>⌥</kbd><kbd>I</kbd> opens the GPUI
inspector in debug builds. Pick an element and edit its style live, then copy
the change into the code.

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

## Releases

A tag `v<version>` publishes a release. The version must be the one in
`Cargo.toml`. `.github/workflows/release.yml` builds Gibbon.app on an Apple
Silicon runner, signs it, and attaches it to a GitHub release as
`Gibbon.zip`.

The releases have no Apple notarization. A self-signed certificate,
*Gibbon Release*, signs them, so each release has the same code identity:

- macOS keeps the folder permissions of Gibbon after an update.
- An installed Gibbon accepts an update only when it has the same
  certificate. A changed or foreign download fails this check.

Once, before the first release:

1. Run `scripts/make-signing-cert.sh`. It writes the certificate to
   `~/.gibbon-signing/` and imports it into your login keychain, so
   `scripts/bundle.sh` signs your own builds with it too.
2. Store the certificate as GitHub secrets with the two commands that the
   script prints.
3. Keep a copy of `~/.gibbon-signing/` outside this Mac. With a new
   certificate, installed apps reject all later updates, and users must
   install Gibbon again.

For each release:

1. Set the new `version` in `Cargo.toml`, run `cargo build` to update
   `Cargo.lock`, and commit both: `chore: release 0.2.0`.
2. Tag the commit and push the tag:
   `git tag v0.2.0 && git push origin v0.2.0`.
