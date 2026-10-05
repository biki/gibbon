# Backlog

Ideas for Gibbon, most useful first. Much Git work is done by agents now.
So the goal of this list is to help you see, review and repair the work that
agents do on a repository.

Status: **done**, **in progress**, **planned** or **idea**.

## Big items

### 1. Worktrees in the sidebar — done (first version)

Parallel agents (Claude Code, Codex, Conductor) often each work in a linked
worktree. Gibbon did not list worktrees, and its file watcher did not see
edits in other worktrees.

- A **Worktrees** section in the sidebar, one row per worktree: its branch,
  the number of changed files, the commits ahead of the base branch, and the
  time of the last change.
- The rows update when an agent edits, stages or commits in a worktree.
- ⌘-click a row to open that worktree in a tab, on its Changes view.
- Right-click a row to browse its commits, review its branch, open it in
  Finder, copy its path, or remove it (with or without its branch).
- **Agents** (⌘5) — done (first version). A card per worktree: its state
  (Working, Quiet, Idle, Ready to review, Conflict, No changes), the agent
  that runs there (found with `ps` and `lsof`), the last commit, the files
  that changed last, the changed lines, and Review, Open Tab and Commits.
- **Worktree tabs** — done. A worktree's tab shows its branch, its changed
  files and its state, and opens next to its repository's tabs.
- **A look without a new tab** — done. A click on a row reviews the
  worktree in this tab, and ⌘-click opens its tab.

Next steps:

- **Waiting for an answer** — idea. *Quiet* cannot tell an agent that waits
  for you from one that runs a long command. The session files of Claude
  Code (`~/.claude/projects/`) and Codex (`~/.codex/sessions/`) can tell.
- **Overlap warning** — idea. Mark worktrees that change the same files;
  `git merge-tree --write-tree` tells a real conflict from a clean overlap.
- **Agents in other folders** — idea. Count an agent also for the worktrees
  where the commands that it starts run.

- **New Worktree…** — idea. Make a worktree and a branch from Gibbon, to
  start an agent in it.
- **Branch behind the base** — idea. The row shows the commits ahead only.
  The tooltip shows both counts.

### 2. Review a whole branch as one diff — done (first version)

Agent branches often have many small commits, such as "fix lint" or
"fix test". To review the result, you want one diff: `git diff base...branch`.

- **Review Changes** on a branch, a worktree or a pull request shows the
  combined diff against a base branch that you can change.
- A **Viewed** check on each file, as on GitHub. The check clears when the
  file changes again.
- For a worktree, the review can include its uncommitted changes.

Next steps:

- **Uncommitted changes at start-up** — done. A review that opens before the
  worktree rows load includes the worktree's files.
- **The base of a review** — done. It is the base you chose for the branch
  before, the base of its pull request, the branch it was made from (from
  its reflog), or the base branch. The worktree rows and cards count
  against it too.
- **Commit list in the review** — idea. Show the commits of the branch beside
  the files, and filter the diff to one commit.
- **Comments** — idea. Notes on lines that you can copy into a prompt for the
  agent.

### 3. Activity timeline with undo — done (first version)

Agents run `reset --hard`, `rebase`, `commit --amend` and `push --force`.
Git's reflog records each move of each branch.

- An **Activity** view lists the moves of all branches, newest first:
  commits, amends, rebases, resets, merges, pulls and pushes.
- Each move shows the commits that it added and the commits that it dropped.
  A move that drops commits has a yellow icon and a red "dropped" count.
- **Restore** moves a branch back to its place before or after a move.
  The restore is a move too, so you can undo it.
- The moves that came in after your last visit are marked as new.

Next steps:

- **Show the change of a move** — idea. The diff from before a move to after
  it, for example what a rebase changed in the files.
- **Reftable repositories** — idea. Read the reflogs with `git reflog` when
  the repository has no reflog files.
- **Deleted branches** — idea. Git deletes the reflog of a deleted branch.
  Keep its last commit, so a delete can be undone.

## Smaller items

- **Agent badges** — idea. Read the `Co-Authored-By` trailers and the bot
  authors. Put a small badge on agent commits in the graph, and add a
  filter: agent, human, or all.
- **Pull request status in the sidebar** — done. The checks, the review
  decision and merge conflicts on the pull request rows, and the checks on
  the rows of their branches and worktrees. Next: the reviews and comments
  themselves, and a link from a failed check to its log.
- **Ahead and behind counts against the base branch** — idea. The sidebar
  reads only `upstream:track`. A local agent branch has no upstream until the
  agent pushes it, so the sidebar shows no counts for it.
- **Branch cleanup** — done. **Clean Up Branches…** lists the local
  branches that are merged into the base branch or whose remote branch is
  gone, and deletes the chosen ones with their worktrees. A gone branch with
  a merged pull request on GitHub, or whose changes are in the base branch,
  loses nothing and starts selected. Next: local branches that were picked
  or squash-merged but never pushed (the checks run only for gone branches
  now), a stacked pull request whose parent branch never reached the base
  branch (it counts as merged now), and an undo for the deletes (see
  **Deleted branches** above).
- **Sort Changes by recent edits** — done. The sort button of the Changes
  list has **Recent Edits First**: the newest file on disk first. The files
  that changed in the last refresh flash for a moment.
- **Blame and file history** — idea. Go from a line in the diff to the
  commit that wrote it, and from that commit to its pull request.
- **History search** — idea. Filter the history by message, author or path.
- **Recover dropped stashes** — idea. `git fsck --unreachable` finds stash
  commits that a `stash drop` or `stash clear` removed.
