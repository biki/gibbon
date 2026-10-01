# Gibbon

A native Git client for macOS in Rust on GPUI (gpui-kit). See README.md.

## Commits

- Follow Conventional Commits as CONTRIBUTING.md defines them: the types, the
  scopes, imperative lower-case descriptions, no period, subject ≤ 72 chars.
- `scripts/check-commit-subject.sh "<subject>"` checks a subject; the
  `.githooks/commit-msg` hook runs it when `core.hooksPath` is `.githooks`.
- One logical change per commit. Explain the why in the body when it is not
  obvious from the subject.

## Build and test

- `cargo build`, `cargo test`, `cargo clippy` (keep clippy warning-free).
- `cargo fmt` before each commit (default rustfmt style). The
  `.githooks/pre-commit` hook rejects staged Rust files that are not
  formatted.
- `scripts/bundle.sh` builds `target/release/bundle/Gibbon.app`.
- `scripts/dev.sh [repo]` is the dev loop (`src/bin/dev.rs`): rebuild and
  restart on save. `cargo run` still starts the app (`default-run`).
- UI checks: run with `GIBBON_BACKGROUND=1` and the other `GIBBON_*`
  variables in README.md, then capture the window. Never send synthetic
  clicks: they land in whatever window is on top.
