# Changelog

This file lists the notable changes of each Gibbon version, newest first.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-10-03

### Added

- Word wrap for diffs: a button beside Unified / Split, a palette command and a setting ([#2])
- Expand all and Collapse all buttons for the folder tree of each file list ([#5])

### Changed

- Changed files flash in the Changes list only while the Gibbon window is active ([#1])

### Fixed

- Syntax colors for TSX, JSX, C++, Kotlin, Swift, C#, CMake, Proto and GraphQL files ([#4])
- The diff of another file starts at the top, not at the scroll position of the last file ([#3])

## [0.1.0] - 2026-10-01

The first release: a native Git client for macOS, built on GPUI.

### Added

- Tabs for repositories, and Clone Repository… with the repositories of your GitHub account
- Commit history with a graph of one branch or of all branches
- Diffs with syntax colors, changed words, unified and split views, images and rendered Markdown
- Staging and discard per file, per hunk and per line
- Cherry-pick into your branch from a browsed branch or pull request
- Review Changes: all changes of a branch as one diff, with Viewed marks
- Interactive rebase
- Worktrees in the sidebar, with their changed files and their commits
- Activity: the moves of all branches from Git's reflogs, with Restore
- Clean Up Branches…: delete merged branches with their worktrees, squash merges included
- Branches, remotes and stashes, and Continue, Skip and Abort when a conflict stops Git
- Pull requests through the GitHub CLI, with their checks, reviews and merge conflicts
- Command palette and keyboard shortcuts
- Light and dark themes, five color themes, and a choice of interface and code fonts
- Automatic updates from the GitHub releases

[Unreleased]: https://github.com/biki/gibbon/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/biki/gibbon/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/biki/gibbon/releases/tag/v0.1.0
[#1]: https://github.com/biki/gibbon/issues/1
[#2]: https://github.com/biki/gibbon/issues/2
[#3]: https://github.com/biki/gibbon/issues/3
[#4]: https://github.com/biki/gibbon/issues/4
[#5]: https://github.com/biki/gibbon/issues/5
