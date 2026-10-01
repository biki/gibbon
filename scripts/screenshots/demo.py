#!/usr/bin/env python3
"""Build the demo repository for the README screenshots (see
scripts/screenshots.sh): a copy of Gibbon's own history up to a fixed
commit, plus agent branches in worktrees, two merges, a squash merge, a
cherry-pick, remote branches that are gone, an amend, a reset that drops a
commit, a stash and uncommitted work.

    scripts/screenshots/demo.py <work folder>

It writes <work>/home/Developer/gibbon (the HOME of the screenshot runs is
<work>/home, so the status bar shows ~/Developer/gibbon), its worktrees in
<work>/home/Developer/gibbon-worktrees and its remote in <work>/remote.git.
The times of the new commits and moves are relative to now, so the views
show "12m ago", not a fixed date. The edits use anchors in the code of the
fixed commits, so they keep working while Gibbon's code changes."""
import datetime
import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
WORK = os.path.abspath(sys.argv[1])
D = f"{WORK}/home/Developer"
REMOTE = f"{WORK}/remote.git"
MAIN = f"{D}/gibbon"
WT = f"{D}/gibbon-worktrees"
# The demo's main: Gibbon's history up to this commit.
START = "5cafb98"

shutil.rmtree(D, ignore_errors=True)
shutil.rmtree(REMOTE, ignore_errors=True)
os.makedirs(WT)


def you(key, default):
    r = subprocess.run(["git", "-C", ROOT, "config", key], capture_output=True, text=True)
    return r.stdout.strip() or default


NAME = you("user.name", "Gibbon Demo")
EMAIL = you("user.email", "demo@example.com")
# Keep the global Git config (signing, hooks, aliases) out of the demo.
ENV = dict(
    os.environ,
    GIT_CONFIG_GLOBAL="/dev/null",
    GIT_CONFIG_NOSYSTEM="1",
    GIT_AUTHOR_NAME=NAME,
    GIT_AUTHOR_EMAIL=EMAIL,
    GIT_COMMITTER_NAME=NAME,
    GIT_COMMITTER_EMAIL=EMAIL,
    GIT_EDITOR="true",
)
# The script reads like a morning of work that ends at 12:00. That noon is
# 15 minutes ago, so the last move shows as "16m ago".
NOON = datetime.datetime.now().astimezone().replace(second=0, microsecond=0) - datetime.timedelta(
    minutes=15
)


def at(hm, days=0):
    """A time of the demo morning (`days` before it) as a Git date."""
    h, m = map(int, hm.split(":"))
    t = NOON + datetime.timedelta(days=days, hours=h - 12, minutes=m)
    return t.isoformat()


def git(cwd, *args, when=None):
    env = dict(ENV)
    if when:
        env["GIT_AUTHOR_DATE"] = env["GIT_COMMITTER_DATE"] = when
    r = subprocess.run(["git", *args], cwd=cwd, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} in {cwd} failed:\n{r.stdout}\n{r.stderr}")
    return r.stdout.strip()


def edit(root, path, anchor, text, how="after"):
    p = f"{root}/{path}"
    s = open(p).read()
    assert s.count(anchor) == 1, (path, anchor, s.count(anchor))
    if how == "after":
        s = s.replace(anchor, anchor + text)
    elif how == "before":
        s = s.replace(anchor, text + anchor)
    else:
        s = s.replace(anchor, text)
    open(p, "w").write(s)


def commit(root, msg, when, *paths):
    git(root, "add", *(paths or ["-A"]))
    git(root, "commit", "-q", "-m", msg, when=when)


# The remote and the clone. The clone's reflog entry goes back a day.
git(WORK, "init", "-q", "--bare", "--initial-branch=main", REMOTE)
subprocess.run(
    ["git", "-C", ROOT, "push", "-q", "--no-verify", REMOTE, f"{START}:refs/heads/main"],
    check=True,
)
git(D, "clone", "-q", REMOTE, MAIN, when=at("10:40", -1))
git(MAIN, "config", "core.hooksPath", "/dev/null")

# --- spike/libgit2: an old experiment, pushed, not merged -------------------
git(MAIN, "switch", "-q", "-c", "spike/libgit2", "e4ed514", when=at("22:05", -1))
edit(MAIN, "Cargo.toml", "[dependencies]\n", 'git2 = { version = "0.20", default-features = false }\n')
commit(MAIN, "chore(deps): add git2 to try it for the log", at("22:10", -1))
edit(MAIN, "src/git.rs", "impl Commit {\n", """    /// Spike: read the log with libgit2 instead of `git log`.
    #[allow(dead_code)]
    pub fn walk(repo: &git2::Repository, limit: usize) -> Vec<Commit> {
        let mut walk = repo.revwalk().expect("revwalk");
        walk.push_head().ok();
        walk.take(limit)
            .filter_map(|oid| repo.find_commit(oid.ok()?).ok())
            .map(|c| Commit {
                sha: c.id().to_string(),
                parents: c.parent_ids().map(|p| p.to_string()).collect(),
                author: c.author().name().unwrap_or_default().to_string(),
                time: c.time().seconds(),
                subject: c.summary().unwrap_or_default().to_string(),
                refs: Vec::new(),
            })
            .collect()
    }

""")
commit(MAIN, "perf(history): read the log with git2 (spike)", at("22:31", -1))
git(MAIN, "push", "-q", "-u", "origin", "spike/libgit2", when=at("22:32", -1))
git(MAIN, "switch", "-q", "main", when=at("22:33", -1))

# --- feature/review-commit-list: merged with a merge commit ----------------
git(MAIN, "switch", "-q", "-c", "feature/review-commit-list", "60c06be", when=at("10:58"))
edit(MAIN, "src/app/review.rs", """    /// Commits of `target` that `base` does not have.
    commits: usize,
""", """    /// The commits of `target` that `base` does not have, newest first,
    /// listed beside the files.
    log: Vec<Commit>,
""", how="replace")
commit(MAIN, "feat(review): list the commits of the branch beside the files", at("11:15"))
edit(MAIN, "src/app/review.rs", """    log: Vec<Commit>,
""", """    /// One commit of `log` to show alone, or None for the whole branch.
    only: Option<String>,
""")
commit(MAIN, "feat(review): filter the diff to one commit", at("11:28"))
git(MAIN, "switch", "-q", "main", when=at("11:40"))

# --- claude/tab-drag-fix: a worktree, merged, its remote branch deleted ----
TAB = f"{WT}/claude-tab-drag-fix"
git(MAIN, "worktree", "add", "-q", "-b", "claude/tab-drag-fix", TAB, "93a835e", when=at("10:31"))
edit(TAB, "src/app/workspace.rs", """    /// Where the pointer took the tab, from its top left corner.
    grab: Point<Pixels>,
""", """    /// Where the pointer took the tab, from its top left corner. Kept
    /// while the strip scrolls, so the tab stays under the pointer.
    grab: Point<Pixels>,
""", how="replace")
commit(TAB, "fix(tabs): keep the dragged tab under the pointer", at("10:40"))
git(TAB, "push", "-q", "-u", "origin", "claude/tab-drag-fix", when=at("10:41"))

# --- fix/fetch-backoff: pushed, squash-merged, remote branch deleted -------
git(MAIN, "switch", "-q", "-c", "fix/fetch-backoff", "main", when=at("11:35"))
edit(MAIN, "src/app/mod.rs", """            || self.fetched_at.is_some_and(|at| at.elapsed() < gap)
""", """            || self.fetched_at.is_some_and(|at| at.elapsed() < self.fetch_gap(gap))
""", how="replace")
edit(MAIN, "src/app/mod.rs", """    /// Open pull requests from GitHub. Quietly empty for other hosts.
""", """    /// After a failed fetch, wait three times as long before the next one.
    fn fetch_gap(&self, gap: std::time::Duration) -> std::time::Duration {
        if self.auto_fetch_failed { gap * 3 } else { gap }
    }

""", how="before")
commit(MAIN, "fix(fetch): wait longer after a failed fetch", at("11:38"))
git(MAIN, "push", "-q", "-u", "origin", "fix/fetch-backoff", when=at("11:39"))
git(MAIN, "switch", "-q", "main", when=at("11:40"))

# --- codex/new-worktree: a worktree with small fix-up commits --------------
NEW = f"{WT}/codex-new-worktree"
git(MAIN, "worktree", "add", "-q", "-b", "codex/new-worktree", NEW, "fdf02ff", when=at("11:12"))
edit(NEW, "src/git.rs", """/// Remove a worktree and its folder. Git refuses when the worktree has
""", """/// Add a worktree at `path` with a new branch `branch` from `start`.
pub fn add_worktree(repo: &Repo, path: &Path, branch: &str, start: &str) -> Result<()> {
    check_branch_name(branch)?;
    let path = path.to_string_lossy();
    repo.git(&["worktree", "add", "-b", branch, &path, start])
        .map(|_| ())
}

""", how="before")
commit(NEW, "feat(git): add a worktree with a new branch", at("11:20"))
edit(NEW, "src/app/worktrees.rs", "\n    menu\n}\n", """

/// The folder for a new worktree: next to the repository, in
/// `<repo>-worktrees/<branch>`, with the slashes of the branch as dashes.
pub(super) fn new_worktree_path(root: &Path, branch: &str) -> PathBuf {
    let name = root.file_name().unwrap_or_default().to_string_lossy();
    root.with_file_name(format!("{name}-worktrees"))
        .join(branch.replace('/', "-"))
}
""")
commit(NEW, "feat(worktrees): add a New Worktree dialog", at("11:31"))
edit(NEW, "src/app/worktrees.rs", """    let name = root.file_name().unwrap_or_default().to_string_lossy();
""", """    let name = root
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
""", how="replace")
commit(NEW, "fix lint", at("11:35"))
edit(NEW, "src/app/worktrees.rs", """        .join(branch.replace('/', "-"))
""", """        .join(branch.replace(['/', '\\\\'], "-"))
""", how="replace")
commit(NEW, "fix test", at("11:39"))

# --- claude/badges: a worktree where an agent still works ------------
BADGES = f"{WT}/claude-badges"
git(MAIN, "worktree", "add", "-q", "-b", "claude/badges", BADGES, "5cafb98", when=at("11:34"))
edit(BADGES, "src/git.rs", """    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}
""", """
/// A coding agent that made or co-wrote a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
    Copilot,
    /// Any other `[bot]` author.
    Bot,
}

impl Agent {
    /// The agent of a commit, from its `Co-Authored-By` trailers or its
    /// author.
    pub fn of(message: &str, author_email: &str) -> Option<Agent> {
        if author_email.contains("[bot]@") {
            return Some(Agent::Bot);
        }
        message
            .lines()
            .filter_map(|l| l.strip_prefix("Co-Authored-By:"))
            .find_map(|who| Agent::named(who.trim()))
    }

    fn named(who: &str) -> Option<Agent> {
        let who = who.to_ascii_lowercase();
        if who.contains("claude") {
            Some(Agent::Claude)
        } else if who.contains("codex") {
            Some(Agent::Codex)
        } else if who.contains("copilot") {
            Some(Agent::Copilot)
        } else {
            None
        }
    }
}
""")
commit(BADGES, "feat(git): find the agent of a commit in its trailers", at("11:36"))
edit(BADGES, "src/app/history.rs", "fn ref_badge(r: &git::RefLabel, cx: &App) -> Div {\n", """/// A small badge for the commits that an agent made.
fn agent_badge(agent: git::Agent, cx: &App) -> Div {
    let t = cx.theme();
    let name = match agent {
        git::Agent::Claude => "Claude",
        git::Agent::Codex => "Codex",
        git::Agent::Copilot => "Copilot",
        git::Agent::Bot => "bot",
    };
    h_flex()
        .flex_none()
        .h(px(18.))
        .px_1p5()
        .gap_1()
        .rounded(px(4.))
        .bg(t.colors.muted)
        .text_color(t.colors.muted_foreground)
        .text_size(px(11.))
        .child(Icon::new(IconName::Bot).size(px(11.)))
        .child(name)
}

""", how="before")
commit(BADGES, "feat(history): badge the commits of agents", at("11:44"))
edit(BADGES, "src/git.rs", """            .filter_map(|l| l.strip_prefix("Co-Authored-By:"))
""", """            .filter_map(|l| {
                let (key, who) = l.split_once(':')?;
                key.eq_ignore_ascii_case("co-authored-by").then_some(who)
            })
""", how="replace")
commit(BADGES, "fix(git): match trailer keys in any case", at("11:49"))
git(BADGES, "push", "-q", "-u", "origin", "claude/badges", when=at("11:50"))
TESTS = """
    #[test]
    fn agents_from_trailers() {
        let msg = "feat: x\\n\\nCo-authored-by: Claude <noreply@anthropic.com>";
        assert_eq!(Agent::of(msg, "me@example.com"), Some(Agent::Claude));
        assert_eq!(Agent::of("fix: y", "me@example.com"), None);
    }
"""
edit(BADGES, "src/git.rs", "mod tests {\n    use super::*;\n", TESTS)
commit(BADGES, "test(git): cover agent trailers", at("11:55"))
edit(BADGES, "src/git.rs", """        assert_eq!(Agent::of("fix: y", "me@example.com"), None);
""", """        assert_eq!(
            Agent::of("chore: bump", "dependabot[bot]@users.noreply.github.com"),
            Some(Agent::Bot)
        );
""")
git(BADGES, "add", "-A")
git(BADGES, "commit", "-q", "--amend", "-m", "test(git): cover agent trailers and bot authors", when=at("11:57"))
edit(BADGES, "src/app/history.rs", "/// A small badge for the commits that an agent made.\n", """/// Which commits the log shows.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum AuthorFilter {
    #[default]
    All,
    Agents,
    Humans,
}

""", how="before")
commit(BADGES, "wip: filter the graph by agent", at("11:58"))
git(BADGES, "reset", "-q", "--hard", "HEAD~1", when=at("11:59"))

# --- main: merges, a squash merge, a cherry-pick, a push --------------------
git(MAIN, "merge", "-q", "--no-ff", "--no-edit", "feature/review-commit-list", when=at("11:41"))
git(MAIN, "merge", "-q", "--no-ff", "--no-edit", "claude/tab-drag-fix", when=at("11:43"))
git(NEW, "rebase", "-q", "main", when=at("11:44"))
git(NEW, "push", "-q", "-u", "origin", "codex/new-worktree", when=at("11:45"))
pick = git(NEW, "log", "--format=%H", "--grep=add a worktree with a new branch", "-1")
git(MAIN, "cherry-pick", "-x", pick, when=at("11:45"))
git(MAIN, "merge", "-q", "--squash", "fix/fetch-backoff", when=at("11:46"))
git(MAIN, "commit", "-q", "-m", "fix(fetch): wait longer after a failed fetch", when=at("11:46"))
git(MAIN, "push", "-q", "origin", "--delete", "claude/tab-drag-fix", when=at("11:47"))
git(MAIN, "push", "-q", "origin", "--delete", "fix/fetch-backoff", when=at("11:47"))
git(MAIN, "push", "-q", "origin", "main", when=at("11:53"))

# --- a stash on main --------------------------------------------------------
edit(MAIN, "src/graph.rs", "const LINE_W: f32 = 1.6;\n", "const LINE_W: f32 = 2.0;\n", how="replace")
git(MAIN, "stash", "push", "-q", "-m", "try thicker graph lines", when=at("11:56"))

# --- uncommitted work: on main and in the agent's worktree ------------------
edit(MAIN, "src/theme.rs", """    Scheme {
        id: "orchid",
""", """    Scheme {
        id: "ember",
        name: "Ember",
        note: "warm red",
        dark: EMBER_DARK,
        light: EMBER_LIGHT,
        lane: 3,
    },
""", how="before")
git(MAIN, "add", "src/theme.rs")
edit(MAIN, "src/theme.rs", "pub fn palette(cx: &App) -> &'static Palette {\n", """const EMBER_DARK: Palette = Palette {
    bg: 0x171312,
    sidebar: 0x0F0D0C,
    elevated: 0x221C1B,
    border: 0x352C2A,
    input: 0x4A3F3C,
    selection: 0x45231B,
    accent: 0xF2774D,
    accent_fg: 0x1C0A05,
    ..GIBBON_DARK
};

const EMBER_LIGHT: Palette = Palette {
    sidebar: 0xF5ECE9,
    border: 0xDDCFCB,
    input: 0xC4B2AC,
    selection: 0xF8C9B8,
    accent: 0xB0401A,
    ..GIBBON_LIGHT
};

""", how="before")
edit(MAIN, "README.md", "- Five color themes with the same contrast: Gibbon (gold, the default),\n  Indigo, Canopy, Lagoon and Orchid.\n",
     "- Six color themes with the same contrast: Gibbon (gold, the default),\n  Indigo, Canopy, Lagoon, Ember and Orchid.\n", how="replace")

edit(BADGES, "src/app/history.rs", "/// A small badge for the commits that an agent made.\n", """/// Which commits the log shows: all, the agents' or the humans'.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum AuthorFilter {
    #[default]
    All,
    Agents,
    Humans,
}

impl AuthorFilter {
    pub(super) fn keeps(self, agent: Option<git::Agent>) -> bool {
        match self {
            AuthorFilter::All => true,
            AuthorFilter::Agents => agent.is_some(),
            AuthorFilter::Humans => agent.is_none(),
        }
    }
}

""", how="before")
open(f"{BADGES}/src/app/agents.rs", "w").write("""//! The agent filter of the commit list: a menu in the log header.

use super::*;

impl GitApp {
    pub(super) fn set_author_filter(&mut self, filter: AuthorFilter, cx: &mut Context<Self>) {
        if self.author_filter == filter {
            return;
        }
        self.author_filter = filter;
        self.reload_log(cx);
    }
}
""")

print(MAIN)
