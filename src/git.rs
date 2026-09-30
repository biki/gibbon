//! Git backend: every operation runs the `git` CLI, so hooks, signing,
//! credential helpers and SSH keys behave exactly as in the terminal.
//! Everything here blocks; the UI calls it on the background executor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};

/// Commits the history view loads at most.
pub const LOG_LIMIT: usize = 20_000;
/// Diff lines per file before the rest is cut off.
const DIFF_LINE_LIMIT: usize = 20_000;

#[derive(Clone, Debug)]
pub struct Repo {
    pub root: PathBuf,
    pub name: String,
}

impl Repo {
    /// The repository that contains `path`.
    pub fn discover(path: &Path) -> Result<Repo> {
        let out = run_in(path, &["rev-parse", "--show-toplevel"])?;
        let root = PathBuf::from(out.trim());
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        Ok(Repo { root, name })
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        run_in(&self.root, args)
    }
}

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .args(args)
        // Never wait for a password on a terminal nobody sees.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Reads such as `status` must not rewrite the index: the file
        // watcher would see the write and reload again.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    cmd
}

fn run_in(dir: &Path, args: &[&str]) -> Result<String> {
    run_env(dir, args, &[])
}

fn run_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    let mut cmd = command(dir, args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .with_context(|| format!("could not start git {}", args.join(" ")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        };
        bail!("{msg}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repository's `.git` folder (a different place for linked worktrees).
pub fn git_dir(repo: &Repo) -> Result<PathBuf> {
    Ok(PathBuf::from(repo.git(&["rev-parse", "--absolute-git-dir"])?.trim()))
}

// ---------------------------------------------------------------------------
// Refs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug)]
pub struct Branch {
    /// Full ref name, `refs/heads/main`.
    pub refname: String,
    /// Short name, `main` or `origin/main`.
    pub name: String,
    pub kind: RefKind,
    pub ahead: u32,
    pub behind: u32,
    pub is_head: bool,
}

#[derive(Clone, Debug, Default)]
pub struct HeadInfo {
    /// Branch name, None when detached.
    pub branch: Option<String>,
    pub sha: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

pub fn head(repo: &Repo) -> HeadInfo {
    let branch = repo
        .git(&["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let sha = repo
        .git(&["rev-parse", "-q", "--verify", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string());
    let mut info = HeadInfo {
        branch,
        sha,
        ..Default::default()
    };
    if let Ok(up) = repo.git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]) {
        let up = up.trim().to_string();
        if let Ok(counts) = repo.git(&["rev-list", "--left-right", "--count", "HEAD...@{u}"]) {
            let mut it = counts.split_whitespace();
            info.ahead = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            info.behind = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        }
        info.upstream = Some(up);
    }
    info
}

pub fn branches(repo: &Repo) -> Result<Vec<Branch>> {
    let out = repo.git(&[
        "for-each-ref",
        "--sort=-committerdate",
        "--format=%(refname)%1f%(refname:short)%1f%(upstream:track,nobracket)%1f%(HEAD)",
        "refs/heads",
        "refs/remotes",
        "refs/tags",
    ])?;
    let mut list = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\x1f').collect();
        if f.len() < 4 {
            continue;
        }
        let refname = f[0].to_string();
        let kind = if refname.starts_with("refs/heads/") {
            RefKind::Local
        } else if refname.starts_with("refs/remotes/") {
            // `origin/HEAD` is an alias, not a branch.
            if refname.ends_with("/HEAD") {
                continue;
            }
            RefKind::Remote
        } else {
            RefKind::Tag
        };
        let (mut ahead, mut behind) = (0, 0);
        for part in f[2].split(',') {
            let part = part.trim();
            if let Some(n) = part.strip_prefix("ahead ") {
                ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                behind = n.parse().unwrap_or(0);
            }
        }
        list.push(Branch {
            refname,
            name: f[1].to_string(),
            kind,
            ahead,
            behind,
            is_head: f[3] == "*",
        });
    }
    Ok(list)
}

// ---------------------------------------------------------------------------
// History

#[derive(Clone, Debug)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
    /// HEAD points here (`HEAD -> main`).
    pub head: bool,
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub time: i64,
    pub subject: String,
    pub refs: Vec<RefLabel>,
}

impl Commit {
    pub fn short(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// What the history view shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogTarget {
    /// The checked-out branch.
    Head,
    /// Another branch or tag, by full ref name.
    Ref(String),
    /// Every branch, remote and tag.
    All,
}

pub fn log(repo: &Repo, target: &LogTarget) -> Result<Vec<Commit>> {
    let limit = format!("-n{LOG_LIMIT}");
    let mut args = vec![
        "log",
        "--topo-order",
        "--decorate=full",
        "--no-color",
        limit.as_str(),
        "--format=%H%x1f%P%x1f%an%x1f%at%x1f%D%x1f%s%x1e",
    ];
    match target {
        LogTarget::Head => args.push("HEAD"),
        LogTarget::Ref(r) => args.push(r.as_str()),
        LogTarget::All => args.extend(["--branches", "--remotes", "--tags", "HEAD"]),
    }
    let out = match repo.git(&args) {
        Ok(out) => out,
        // A new repository has no commits yet.
        Err(e) if e.to_string().contains("does not have any commits") => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    Ok(out
        .split('\x1e')
        .filter_map(|rec| {
            let rec = rec.trim_start_matches('\n');
            let f: Vec<&str> = rec.split('\x1f').collect();
            if f.len() < 6 {
                return None;
            }
            Some(Commit {
                sha: f[0].to_string(),
                parents: f[1].split_whitespace().map(str::to_string).collect(),
                author: f[2].to_string(),
                time: f[3].parse().unwrap_or(0),
                refs: parse_decorations(f[4]),
                subject: f[5].to_string(),
            })
        })
        .collect())
}

fn parse_decorations(d: &str) -> Vec<RefLabel> {
    let mut out = Vec::new();
    for part in d.split(", ").map(str::trim).filter(|s| !s.is_empty()) {
        let (head, name) = match part.strip_prefix("HEAD -> ") {
            Some(rest) => (true, rest),
            None => (false, part),
        };
        if name == "HEAD" {
            continue;
        }
        let label = if let Some(t) = name.strip_prefix("tag: refs/tags/") {
            RefLabel {
                name: t.to_string(),
                kind: RefKind::Tag,
                head,
            }
        } else if let Some(b) = name.strip_prefix("refs/heads/") {
            RefLabel {
                name: b.to_string(),
                kind: RefKind::Local,
                head,
            }
        } else if let Some(r) = name.strip_prefix("refs/remotes/") {
            if r.ends_with("/HEAD") {
                continue;
            }
            RefLabel {
                name: r.to_string(),
                kind: RefKind::Remote,
                head,
            }
        } else {
            continue;
        };
        out.push(label);
    }
    out
}

/// How a commit of another branch relates to the checked-out branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickState {
    /// Not in HEAD: can be picked.
    Pickable,
    /// Not in HEAD, but HEAD already has the same change (an earlier pick).
    AlreadyPicked,
}

/// For the commits of `refname` that HEAD does not contain: whether HEAD has
/// an equivalent patch. Commits missing from the map are already in HEAD.
pub fn pick_states(repo: &Repo, refname: &str) -> Result<HashMap<String, PickState>> {
    let range = format!("HEAD..{refname}");
    let mut map: HashMap<String, PickState> = repo
        .git(&["rev-list", range.as_str()])?
        .lines()
        .map(|s| (s.trim().to_string(), PickState::Pickable))
        .collect();
    let sym = format!("HEAD...{refname}");
    let marked = repo.git(&[
        "rev-list",
        "--cherry-mark",
        "--right-only",
        "--no-merges",
        sym.as_str(),
    ])?;
    for line in marked.lines() {
        if let Some(sha) = line.strip_prefix('=') {
            map.insert(sha.trim().to_string(), PickState::AlreadyPicked);
        }
    }
    Ok(map)
}

/// Apply `shas` (oldest first) on top of HEAD. With `merges`, merge
/// commits apply their changes against the first parent (`-m 1`); git
/// accepts that for the plain commits in the same list.
pub fn cherry_pick(repo: &Repo, shas: &[String], merges: bool) -> Result<String> {
    let mut args = vec!["cherry-pick", "-x"];
    if merges {
        args.extend(["-m", "1"]);
    }
    args.extend(shas.iter().map(String::as_str));
    repo.git(&args)
}

/// An operation that stopped on a conflict and waits for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paused {
    CherryPick,
    Rebase,
    Merge,
    Revert,
}

impl Paused {
    pub fn name(self) -> &'static str {
        match self {
            Paused::CherryPick => "Cherry-pick",
            Paused::Rebase => "Rebase",
            Paused::Merge => "Merge",
            Paused::Revert => "Revert",
        }
    }

    /// Merges have no skip.
    pub fn can_skip(self) -> bool {
        self != Paused::Merge
    }
}

pub fn paused(repo: &Repo) -> Option<Paused> {
    let dir = git_dir(repo).ok()?;
    if dir.join("rebase-merge").exists() || dir.join("rebase-apply").exists() {
        Some(Paused::Rebase)
    } else if dir.join("CHERRY_PICK_HEAD").exists() {
        Some(Paused::CherryPick)
    } else if dir.join("REVERT_HEAD").exists() {
        Some(Paused::Revert)
    } else if dir.join("MERGE_HEAD").exists() {
        Some(Paused::Merge)
    } else {
        None
    }
}

/// Keep the messages git prepared; never open an editor.
const NO_EDITOR: &[(&str, &str)] = &[("GIT_EDITOR", "true")];

pub fn continue_paused(repo: &Repo, p: Paused) -> Result<String> {
    let args: &[&str] = match p {
        Paused::CherryPick => &["cherry-pick", "--continue"],
        Paused::Revert => &["revert", "--continue"],
        Paused::Rebase => &["rebase", "--continue"],
        Paused::Merge => &["commit", "--no-edit"],
    };
    run_env(&repo.root, args, NO_EDITOR)
}

pub fn skip_paused(repo: &Repo, p: Paused) -> Result<String> {
    let args: &[&str] = match p {
        Paused::CherryPick => &["cherry-pick", "--skip"],
        Paused::Revert => &["revert", "--skip"],
        Paused::Rebase => &["rebase", "--skip"],
        Paused::Merge => bail!("A merge cannot skip."),
    };
    run_env(&repo.root, args, NO_EDITOR)
}

pub fn abort_paused(repo: &Repo, p: Paused) -> Result<String> {
    let args: &[&str] = match p {
        Paused::CherryPick => &["cherry-pick", "--abort"],
        Paused::Revert => &["revert", "--abort"],
        Paused::Rebase => &["rebase", "--abort"],
        Paused::Merge => &["merge", "--abort"],
    };
    repo.git(args)
}

// ---------------------------------------------------------------------------
// Interactive rebase

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebaseAction {
    Pick,
    Reword,
    Squash,
    Fixup,
    Drop,
}

impl RebaseAction {
    pub const ALL: [RebaseAction; 5] = [
        RebaseAction::Pick,
        RebaseAction::Reword,
        RebaseAction::Squash,
        RebaseAction::Fixup,
        RebaseAction::Drop,
    ];

    pub fn name(self) -> &'static str {
        match self {
            RebaseAction::Pick => "pick",
            RebaseAction::Reword => "reword",
            RebaseAction::Squash => "squash",
            RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            RebaseAction::Pick => "Keep the commit",
            RebaseAction::Reword => "Keep the commit, change its message",
            RebaseAction::Squash => "Join into the commit above, keep both messages",
            RebaseAction::Fixup => "Join into the commit above, drop this message",
            RebaseAction::Drop => "Remove the commit",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RebaseStep {
    pub sha: String,
    pub subject: String,
    pub body: String,
    pub action: RebaseAction,
    /// The new subject of a Reword step.
    pub new_subject: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RebasePlan {
    /// The commit the steps go on top of; None rewrites from the root.
    pub base: Option<String>,
    /// Oldest first, like git's todo list.
    pub steps: Vec<RebaseStep>,
    /// Merge commits in the range; the rebase flattens them.
    pub merges: usize,
}

/// The commits from `from` (included) to HEAD.
pub fn rebase_plan(repo: &Repo, from: &str) -> Result<RebasePlan> {
    let parent = format!("{from}^");
    let base = repo
        .git(&["rev-parse", "-q", "--verify", parent.as_str()])
        .ok()
        .map(|s| s.trim().to_string());
    let range = match &base {
        Some(b) => format!("{b}..HEAD"),
        None => "HEAD".to_string(),
    };
    let out = repo.git(&[
        "log",
        "--reverse",
        "--topo-order",
        "--no-merges",
        "--format=%H%x1f%s%x1f%b%x1e",
        range.as_str(),
    ])?;
    let steps = out
        .split('\x1e')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.trim_start_matches('\n').splitn(3, '\x1f').collect();
            (f.len() == 3).then(|| RebaseStep {
                sha: f[0].to_string(),
                subject: f[1].to_string(),
                body: f[2].trim_end().to_string(),
                action: RebaseAction::Pick,
                new_subject: None,
            })
        })
        .collect();
    let merges = repo
        .git(&["rev-list", "--count", "--merges", range.as_str()])
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    Ok(RebasePlan {
        base,
        steps,
        merges,
    })
}

/// A problem that stops the plan from running, if any.
pub fn check_plan(plan: &RebasePlan) -> Option<&'static str> {
    match plan
        .steps
        .iter()
        .find(|s| s.action != RebaseAction::Drop)
        .map(|s| s.action)
    {
        None => Some("Every commit is dropped. Keep at least one."),
        Some(RebaseAction::Squash | RebaseAction::Fixup) => {
            Some("The first kept commit cannot squash: nothing comes before it.")
        }
        _ => None,
    }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run the plan with `git rebase -i`, writing git's todo list for it.
/// Uncommitted changes are stashed before and restored after.
pub fn rebase_run(repo: &Repo, plan: &RebasePlan) -> Result<String> {
    if let Some(problem) = check_plan(plan) {
        bail!("{problem}");
    }
    // Inside .git: a paused rebase still needs the message files.
    let dir = git_dir(repo)?.join("gibbon-rebase");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let mut todo = String::new();
    for (i, st) in plan.steps.iter().enumerate() {
        let verb = match st.action {
            RebaseAction::Pick | RebaseAction::Reword => "pick",
            RebaseAction::Squash => "squash",
            RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        };
        todo.push_str(&format!("{verb} {} {}\n", st.sha, st.subject));
        if st.action == RebaseAction::Reword {
            let subject = st.new_subject.as_deref().unwrap_or(&st.subject).trim();
            let msg = if st.body.trim().is_empty() {
                format!("{subject}\n")
            } else {
                format!("{subject}\n\n{}\n", st.body.trim_end())
            };
            let file = dir.join(format!("message-{i}"));
            std::fs::write(&file, msg)?;
            todo.push_str(&format!(
                "exec git commit --amend --allow-empty --no-verify -F {}\n",
                sh_quote(&file.to_string_lossy())
            ));
        }
    }
    let todo_file = dir.join("todo");
    std::fs::write(&todo_file, todo)?;
    let editor = format!("cp {}", sh_quote(&todo_file.to_string_lossy()));
    let mut args = vec!["rebase", "-i", "--autostash"];
    match &plan.base {
        Some(b) => args.push(b.as_str()),
        None => args.push("--root"),
    }
    run_env(
        &repo.root,
        &args,
        &[("GIT_SEQUENCE_EDITOR", editor.as_str()), ("GIT_EDITOR", "true")],
    )
}

// ---------------------------------------------------------------------------
// Diffs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
    Hunk,
    /// "\ No newline at end of file" and similar notes.
    Note,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChange {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Clone, Debug)]
pub struct FileDiff {
    /// `diff --git` up to the first hunk, for building partial patches.
    pub header: Vec<String>,
    pub path: String,
    pub old_path: Option<String>,
    pub change: FileChange,
    pub binary: bool,
    pub additions: u32,
    pub deletions: u32,
    pub lines: Vec<DiffLine>,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct CommitDetail {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub committer: String,
    pub commit_time: i64,
    pub message: String,
    pub files: Vec<FileDiff>,
}

pub fn commit_detail(repo: &Repo, sha: &str) -> Result<CommitDetail> {
    let header = repo.git(&[
        "show",
        "-s",
        "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ct%x1f%B",
        sha,
    ])?;
    let f: Vec<&str> = header.splitn(8, '\x1f').collect();
    if f.len() < 8 {
        bail!("unexpected git show output for {sha}");
    }
    let patch = repo.git(&[
        "show",
        "--format=",
        "--no-color",
        "--patch",
        "-M",
        "--diff-merges=first-parent",
        sha,
    ])?;
    Ok(CommitDetail {
        sha: f[0].to_string(),
        parents: f[1].split_whitespace().map(str::to_string).collect(),
        author: f[2].to_string(),
        email: f[3].to_string(),
        time: f[4].parse().unwrap_or(0),
        committer: f[5].to_string(),
        commit_time: f[6].parse().unwrap_or(0),
        message: f[7].trim_end().to_string(),
        files: parse_patch(&patch),
    })
}

/// Parse `git diff` / `git show` output into files and lines.
pub fn parse_patch(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let (mut old_no, mut new_no) = (0u32, 0u32);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest
                .rsplit_once(" b/")
                .map(|(_, b)| b.to_string())
                .unwrap_or_else(|| rest.to_string());
            files.push(FileDiff {
                header: vec![line.to_string()],
                path,
                old_path: None,
                change: FileChange::Modified,
                binary: false,
                additions: 0,
                deletions: 0,
                lines: Vec::new(),
                truncated: false,
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        let in_hunk = !file.lines.is_empty();
        if !in_hunk {
            if line.starts_with("new file mode") {
                file.change = FileChange::Added;
            } else if line.starts_with("deleted file mode") {
                file.change = FileChange::Deleted;
            } else if let Some(from) = line.strip_prefix("rename from ") {
                file.change = FileChange::Renamed;
                file.old_path = Some(from.to_string());
            } else if let Some(to) = line.strip_prefix("rename to ") {
                file.path = to.to_string();
            } else if line.starts_with("Binary files ") {
                file.binary = true;
            }
            if !line.starts_with("@@") {
                file.header.push(line.to_string());
                continue;
            }
        }
        if file.lines.len() >= DIFF_LINE_LIMIT {
            file.truncated = true;
            continue;
        }
        if line.starts_with("@@") {
            // @@ -a,b +c,d @@ context
            let mut it = line.split_whitespace().skip(1);
            let parse = |s: Option<&str>| {
                s.and_then(|s| s[1..].split(',').next()?.parse::<u32>().ok())
                    .unwrap_or(0)
            };
            old_no = parse(it.next());
            new_no = parse(it.next());
            // A hunk row carries the hunk's start lines.
            file.lines.push(DiffLine {
                kind: LineKind::Hunk,
                old_no: Some(old_no),
                new_no: Some(new_no),
                text: line.to_string(),
            });
            continue;
        }
        let (kind, body) = match line.as_bytes().first() {
            Some(b'+') => (LineKind::Add, &line[1..]),
            Some(b'-') => (LineKind::Del, &line[1..]),
            Some(b' ') => (LineKind::Context, &line[1..]),
            Some(b'\\') => (LineKind::Note, line),
            _ => (LineKind::Context, line),
        };
        let text = body.replace('\t', "    ");
        let dl = match kind {
            LineKind::Add => {
                file.additions += 1;
                new_no += 1;
                DiffLine {
                    kind,
                    old_no: None,
                    new_no: Some(new_no - 1),
                    text,
                }
            }
            LineKind::Del => {
                file.deletions += 1;
                old_no += 1;
                DiffLine {
                    kind,
                    old_no: Some(old_no - 1),
                    new_no: None,
                    text,
                }
            }
            LineKind::Context => {
                old_no += 1;
                new_no += 1;
                DiffLine {
                    kind,
                    old_no: Some(old_no - 1),
                    new_no: Some(new_no - 1),
                    text,
                }
            }
            _ => DiffLine {
                kind,
                old_no: None,
                new_no: None,
                text,
            },
        };
        file.lines.push(dl);
    }
    files
}

// ---------------------------------------------------------------------------
// Working tree

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl Change {
    pub fn letter(self) -> &'static str {
        match self {
            Change::Added => "A",
            Change::Modified => "M",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::Untracked => "U",
            Change::Conflicted => "!",
        }
    }
}

#[derive(Clone, Debug)]
pub struct StatusEntry {
    pub path: String,
    /// Change in the index (staged), if any.
    pub staged: Option<Change>,
    /// Change in the working tree (unstaged), if any.
    pub unstaged: Option<Change>,
}

fn change_of(c: u8) -> Option<Change> {
    match c {
        b'A' => Some(Change::Added),
        b'M' | b'T' => Some(Change::Modified),
        b'D' => Some(Change::Deleted),
        b'R' | b'C' => Some(Change::Renamed),
        _ => None,
    }
}

pub fn status(repo: &Repo) -> Result<Vec<StatusEntry>> {
    let out = repo.git(&["status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
    let mut entries = Vec::new();
    let mut fields = out.split('\0').peekable();
    while let Some(rec) = fields.next() {
        if rec.is_empty() {
            continue;
        }
        let kind = rec.as_bytes()[0];
        match kind {
            b'1' | b'2' => {
                // 1 XY sub mH mI mW hH hI path
                // 2 XY sub mH mI mW hH hI Xscore path \0 origPath
                let n = if kind == b'1' { 9 } else { 10 };
                let parts: Vec<&str> = rec.splitn(n, ' ').collect();
                let xy = parts.get(1).map(|s| s.as_bytes()).unwrap_or(b"..");
                let path = parts.last().copied().unwrap_or_default().to_string();
                // A rename carries its old path as the next field.
                if kind == b'2' {
                    fields.next();
                }
                entries.push(StatusEntry {
                    path,
                    staged: change_of(xy[0]),
                    unstaged: change_of(xy[1]),
                });
            }
            b'u' => {
                let parts: Vec<&str> = rec.splitn(11, ' ').collect();
                entries.push(StatusEntry {
                    path: parts.last().copied().unwrap_or_default().to_string(),
                    staged: None,
                    unstaged: Some(Change::Conflicted),
                });
            }
            b'?' => entries.push(StatusEntry {
                path: rec[2..].to_string(),
                staged: None,
                unstaged: Some(Change::Untracked),
            }),
            _ => {}
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// The diff of one working-tree file, staged or unstaged.
pub fn file_diff(repo: &Repo, entry: &StatusEntry, staged: bool) -> Result<Option<FileDiff>> {
    let path = entry.path.as_str();
    let text = if staged {
        repo.git(&["diff", "--cached", "--no-color", "-M", "--", path])?
    } else if entry.unstaged == Some(Change::Untracked) {
        // `--no-index` exits 1 when the files differ, which is always here.
        let out = command(
            &repo.root,
            &["diff", "--no-color", "--no-index", "--", "/dev/null", path],
        )
        .output()?;
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        repo.git(&["diff", "--no-color", "--", path])?
    };
    Ok(parse_patch(&text).into_iter().next())
}

pub fn stage(repo: &Repo, paths: &[String]) -> Result<()> {
    let mut args = vec!["add", "-A", "--"];
    args.extend(paths.iter().map(String::as_str));
    repo.git(&args).map(|_| ())
}

pub fn unstage(repo: &Repo, paths: &[String]) -> Result<()> {
    let has_head = repo.git(&["rev-parse", "-q", "--verify", "HEAD"]).is_ok();
    let mut args = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        vec!["rm", "--cached", "-r", "-q", "--"]
    };
    args.extend(paths.iter().map(String::as_str));
    repo.git(&args).map(|_| ())
}

pub fn stage_all(repo: &Repo) -> Result<()> {
    repo.git(&["add", "-A"]).map(|_| ())
}

pub fn unstage_all(repo: &Repo) -> Result<()> {
    if repo.git(&["rev-parse", "-q", "--verify", "HEAD"]).is_ok() {
        repo.git(&["reset", "-q"]).map(|_| ())
    } else {
        repo.git(&["rm", "--cached", "-r", "-q", "."]).map(|_| ())
    }
}

pub fn commit(repo: &Repo, message: &str) -> Result<String> {
    if message.trim().is_empty() {
        bail!("The commit message is empty.");
    }
    run_with_stdin(&repo.root, &["commit", "-F", "-"], message)
}

fn run_with_stdin(dir: &Path, args: &[&str], input: &str) -> Result<String> {
    use std::io::Write as _;
    let mut child = command(dir, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start git {}", args.join(" ")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no stdin"))?
        .write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        };
        bail!("{msg}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Which way a partial patch goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchOp {
    /// Unstaged diff → index.
    Stage,
    /// Staged diff → back out of the index.
    Unstage,
    /// Unstaged diff → back out of the working tree.
    Discard,
}

/// Whether hunks and lines of this file can be staged one by one.
pub fn supports_partial(file: &FileDiff) -> bool {
    !file.binary && matches!(file.change, FileChange::Modified | FileChange::Renamed)
}

/// A patch with only the `chosen` lines (indexes into `file.lines`) of
/// `file`. Stage applies it forward to the index; Unstage and Discard
/// apply it in reverse, so their unchosen lines keep the other side's text.
pub fn partial_patch(
    file: &FileDiff,
    chosen: &std::collections::HashSet<usize>,
    op: PatchOp,
) -> Option<String> {
    let reverse = op != PatchOp::Stage;
    let mut out = String::new();
    for h in &file.header {
        out.push_str(h);
        out.push('\n');
    }
    let mut any = false;
    // new side minus old side of the hunks written so far
    let mut delta: i64 = 0;
    let lines = &file.lines;
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Hunk {
            i += 1;
            continue;
        }
        let (old_start, new_start) = (
            lines[i].old_no.unwrap_or(0) as i64,
            lines[i].new_no.unwrap_or(0) as i64,
        );
        let mut body = String::new();
        let (mut old_n, mut new_n) = (0i64, 0i64);
        let mut changed = false;
        // The "\ No newline" note follows the line it belongs to.
        let mut last_kept = false;
        i += 1;
        while i < lines.len() && lines[i].kind != LineKind::Hunk {
            let l = &lines[i];
            let pick = chosen.contains(&i);
            let emit = |body: &mut String, prefix: char| {
                body.push(prefix);
                body.push_str(&l.text);
                body.push('\n');
            };
            match l.kind {
                LineKind::Context => {
                    emit(&mut body, ' ');
                    (old_n, new_n, last_kept) = (old_n + 1, new_n + 1, true);
                }
                LineKind::Del if pick => {
                    emit(&mut body, '-');
                    (old_n, changed, last_kept) = (old_n + 1, true, true);
                }
                LineKind::Add if pick => {
                    emit(&mut body, '+');
                    (new_n, changed, last_kept) = (new_n + 1, true, true);
                }
                // An unchosen line stays as it is on the side the patch
                // applies to: the old side forward, the new side in reverse.
                LineKind::Del if !reverse => {
                    emit(&mut body, ' ');
                    (old_n, new_n, last_kept) = (old_n + 1, new_n + 1, true);
                }
                LineKind::Add if reverse => {
                    emit(&mut body, ' ');
                    (old_n, new_n, last_kept) = (old_n + 1, new_n + 1, true);
                }
                LineKind::Del | LineKind::Add => last_kept = false,
                LineKind::Note => {
                    if last_kept {
                        body.push_str(&l.text);
                        body.push('\n');
                    }
                }
                LineKind::Hunk => {}
            }
            i += 1;
        }
        if !changed {
            continue;
        }
        // Forward, the old side is the real target; in reverse, the new side.
        let (os, ns) = if reverse {
            (new_start - delta, new_start)
        } else {
            (old_start, old_start + delta)
        };
        out.push_str(&format!("@@ -{os},{old_n} +{ns},{new_n} @@\n"));
        out.push_str(&body);
        delta += new_n - old_n;
        any = true;
    }
    any.then_some(out)
}

pub fn apply_patch(repo: &Repo, patch: &str, op: PatchOp) -> Result<()> {
    let mut args = vec!["apply", "--recount", "--whitespace=nowarn"];
    match op {
        PatchOp::Stage => args.push("--cached"),
        PatchOp::Unstage => args.extend(["--cached", "-R"]),
        PatchOp::Discard => args.push("-R"),
    }
    args.push("-");
    run_with_stdin(&repo.root, &args, patch).map(|_| ())
}

/// Throw away the unstaged changes of files. Untracked files are deleted.
pub fn discard(repo: &Repo, entries: &[StatusEntry]) -> Result<()> {
    let tracked: Vec<&str> = entries
        .iter()
        .filter(|e| e.unstaged.is_some() && e.unstaged != Some(Change::Untracked))
        .map(|e| e.path.as_str())
        .collect();
    if !tracked.is_empty() {
        let mut args = vec!["restore", "--worktree", "--"];
        args.extend(tracked);
        repo.git(&args)?;
    }
    for e in entries.iter().filter(|e| e.unstaged == Some(Change::Untracked)) {
        let path = repo.root.join(&e.path);
        std::fs::remove_file(&path).with_context(|| format!("could not delete {}", e.path))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Stash

#[derive(Clone, Debug)]
pub struct Stash {
    pub index: usize,
    /// The reflog subject: "On main: message" or "WIP on main: abc123 subject".
    pub message: String,
    pub time: i64,
}

impl Stash {
    pub fn refname(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }

    /// The message without git's "On branch:" prefix.
    pub fn title(&self) -> &str {
        let m = self.message.as_str();
        match m.split_once(": ") {
            Some((head, rest)) if head.starts_with("On ") => rest,
            Some((head, rest)) if head.starts_with("WIP on ") => {
                // "abc1234 subject": drop the short sha.
                rest.split_once(' ').map(|(_, s)| s).unwrap_or(rest)
            }
            _ => m,
        }
    }

    /// The branch the stash was made on.
    pub fn branch(&self) -> Option<&str> {
        let head = self.message.split_once(": ")?.0;
        head.strip_prefix("WIP on ")
            .or_else(|| head.strip_prefix("On "))
    }
}

pub fn stashes(repo: &Repo) -> Result<Vec<Stash>> {
    let out = repo.git(&["stash", "list", "--format=%gd%x1f%gs%x1f%ct"])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            let index = f.first()?.strip_prefix("stash@{")?.strip_suffix('}')?.parse().ok()?;
            Some(Stash {
                index,
                message: f.get(1)?.to_string(),
                time: f.get(2)?.parse().unwrap_or(0),
            })
        })
        .collect())
}

/// Stash every change, new files included.
pub fn stash_push(repo: &Repo, message: &str) -> Result<String> {
    let mut args = vec!["stash", "push", "--include-untracked"];
    if !message.trim().is_empty() {
        args.extend(["-m", message.trim()]);
    }
    repo.git(&args)
}

pub fn stash_apply(repo: &Repo, index: usize, pop: bool) -> Result<String> {
    let r = format!("stash@{{{index}}}");
    repo.git(&["stash", if pop { "pop" } else { "apply" }, r.as_str()])
}

pub fn stash_drop(repo: &Repo, index: usize) -> Result<String> {
    let r = format!("stash@{{{index}}}");
    repo.git(&["stash", "drop", r.as_str()])
}

/// A stash as a commit: its message, time and full diff, new files included.
pub fn stash_detail(repo: &Repo, stash: &Stash) -> Result<CommitDetail> {
    let r = stash.refname();
    let header = repo.git(&[
        "log",
        "-1",
        "--format=%H%x1f%an%x1f%ae%x1f%at",
        r.as_str(),
    ])?;
    let f: Vec<&str> = header.trim_end().splitn(4, '\x1f').collect();
    if f.len() < 4 {
        bail!("unexpected git log output for {r}");
    }
    let patch = repo.git(&[
        "stash",
        "show",
        "-p",
        "--include-untracked",
        "--no-color",
        "-M",
        r.as_str(),
    ])?;
    let time = f[3].parse().unwrap_or(stash.time);
    Ok(CommitDetail {
        sha: f[0].to_string(),
        parents: vec![],
        author: f[1].to_string(),
        email: f[2].to_string(),
        time,
        committer: f[1].to_string(),
        commit_time: time,
        message: stash.title().to_string(),
        files: parse_patch(&patch),
    })
}

// ---------------------------------------------------------------------------
// Branches and remotes

/// Check out a local branch, or create a tracking branch for a remote one.
pub fn switch(repo: &Repo, branch: &Branch) -> Result<()> {
    match branch.kind {
        RefKind::Local => repo.git(&["switch", branch.name.as_str()]).map(|_| ()),
        RefKind::Remote => {
            let local = branch
                .name
                .split_once('/')
                .map(|(_, b)| b)
                .unwrap_or(&branch.name);
            let exists = repo
                .git(&[
                    "rev-parse",
                    "-q",
                    "--verify",
                    format!("refs/heads/{local}").as_str(),
                ])
                .is_ok();
            if exists {
                repo.git(&["switch", local]).map(|_| ())
            } else {
                repo.git(&["switch", "--track", branch.name.as_str()])
                    .map(|_| ())
            }
        }
        RefKind::Tag => repo
            .git(&["switch", "--detach", branch.name.as_str()])
            .map(|_| ()),
    }
}

/// An error message for a bad branch name, or None when git accepts it.
pub fn check_branch_name(repo: &Repo, name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("Enter a branch name.".into());
    }
    repo.git(&["check-ref-format", "--branch", name])
        .err()
        .map(|_| format!("\"{name}\" is not a valid branch name."))
}

/// Create `name` at `start` (HEAD when None); with `switch`, check it out.
pub fn create_branch(repo: &Repo, name: &str, start: Option<&str>, switch: bool) -> Result<()> {
    let mut args = if switch {
        vec!["switch", "-c", name]
    } else {
        vec!["branch", name]
    };
    args.extend(start);
    repo.git(&args).map(|_| ())
}

pub fn rename_branch(repo: &Repo, old: &str, new: &str) -> Result<()> {
    repo.git(&["branch", "-m", old, new]).map(|_| ())
}

/// Delete a local branch. Without `force`, git refuses unmerged branches.
pub fn delete_branch(repo: &Repo, name: &str, force: bool) -> Result<()> {
    repo.git(&["branch", if force { "-D" } else { "-d" }, name])
        .map(|_| ())
}

/// The remote and the branch name on it for a remote-tracking ref such as
/// `refs/remotes/origin/feature/x`. Remote names can contain slashes, so the
/// longest remote that matches wins.
pub fn remote_branch(repo: &Repo, refname: &str) -> Result<(String, String)> {
    let rest = refname
        .strip_prefix("refs/remotes/")
        .ok_or_else(|| anyhow!("{refname} is not a remote branch."))?;
    repo.git(&["remote"])?
        .lines()
        .filter_map(|r| Some((r, rest.strip_prefix(r)?.strip_prefix('/')?)))
        .max_by_key(|(r, _)| r.len())
        .map(|(r, b)| (r.to_string(), b.to_string()))
        .ok_or_else(|| anyhow!("No remote of this repository has {rest}."))
}

/// Delete a branch on its remote, and its remote-tracking ref. When the
/// remote has no such branch any more, delete only the stale ref.
pub fn delete_remote_branch(repo: &Repo, refname: &str) -> Result<()> {
    let (remote, branch) = remote_branch(repo, refname)?;
    // The full name, so a tag with the same name cannot make it ambiguous.
    let spec = format!("refs/heads/{branch}");
    match repo.git(&["push", remote.as_str(), "--delete", spec.as_str()]) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("remote ref does not exist") => {
            repo.git(&["update-ref", "-d", refname]).map(|_| ())
        }
        Err(e) => Err(e),
    }
}

/// Remote names with their fetch URLs.
pub fn remotes(repo: &Repo) -> Result<Vec<(String, String)>> {
    let out = repo.git(&["remote", "-v"])?;
    let mut list: Vec<(String, String)> = out
        .lines()
        .filter(|l| l.ends_with("(fetch)"))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), it.next()?.to_string()))
        })
        .collect();
    list.dedup();
    Ok(list)
}

pub fn fetch_refspec(repo: &Repo, remote: &str, spec: &str) -> Result<()> {
    repo.git(&["fetch", "--no-tags", remote, spec]).map(|_| ())
}

/// Push a branch and set its upstream (to the first remote without one).
pub fn push_branch(repo: &Repo, branch: &str) -> Result<()> {
    let key = format!("branch.{branch}.remote");
    let remote = match repo.git(&["config", "--get", key.as_str()]) {
        Ok(r) => r.trim().to_string(),
        Err(_) => remotes(repo)?
            .into_iter()
            .map(|(n, _)| n)
            .find(|n| n == "origin")
            .or_else(|| remotes(repo).ok()?.into_iter().next().map(|(n, _)| n))
            .ok_or_else(|| anyhow!("This repository has no remote."))?,
    };
    repo.git(&["push", "-u", remote.as_str(), branch]).map(|_| ())
}

pub fn fetch(repo: &Repo) -> Result<String> {
    repo.git(&["fetch", "--all", "--prune"])
}

pub fn pull(repo: &Repo) -> Result<String> {
    repo.git(&["pull"])
}

pub fn push(repo: &Repo, head: &HeadInfo) -> Result<String> {
    if head.upstream.is_some() {
        return repo.git(&["push"]);
    }
    let remote = repo
        .git(&["remote"])?
        .lines()
        .next()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("This repository has no remote."))?;
    repo.git(&["push", "-u", remote.as_str(), "HEAD"])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository in a fresh temp folder, removed on drop.
    struct TempRepo(Repo);

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0.root);
        }
    }

    fn temp_repo(name: &str) -> TempRepo {
        let root = std::env::temp_dir().join(format!("gibbon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let repo = Repo {
            root,
            name: name.into(),
        };
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "Test"],
            &["config", "user.email", "test@example.com"],
            &["config", "commit.gpgsign", "false"],
        ] {
            repo.git(args).unwrap();
        }
        TempRepo(repo)
    }

    fn commit_file(repo: &Repo, file: &str, line: &str, msg: &str) -> String {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(repo.root.join(file))
            .unwrap();
        writeln!(f, "{line}").unwrap();
        repo.git(&["add", "-A"]).unwrap();
        repo.git(&["commit", "-q", "-m", msg]).unwrap();
        repo.git(&["rev-parse", "HEAD"]).unwrap().trim().to_string()
    }

    #[test]
    fn pick_states_and_cherry_pick() {
        let t = temp_repo("pick");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        r.git(&["switch", "-q", "-c", "feature"]).unwrap();
        let f1 = commit_file(r, "b.txt", "one", "feature one");
        let f2 = commit_file(r, "c.txt", "two", "feature two");
        let f3 = commit_file(r, "d.txt", "three", "feature three");
        r.git(&["switch", "-q", "main"]).unwrap();
        // main picked f2 earlier: a different commit with the same patch.
        r.git(&["cherry-pick", &f2]).unwrap();

        let states = pick_states(r, "refs/heads/feature").unwrap();
        assert_eq!(states.get(&f1), Some(&PickState::Pickable));
        assert_eq!(states.get(&f2), Some(&PickState::AlreadyPicked));
        assert_eq!(states.get(&f3), Some(&PickState::Pickable));
        assert_eq!(states.len(), 3, "the shared base is in HEAD");

        // Pick f3 then f1 as the UI orders them: oldest first.
        cherry_pick(r, &[f1.clone(), f3.clone()], false).unwrap();
        assert_eq!(head(r).branch.as_deref(), Some("main"));
        let subjects: Vec<String> = log(r, &LogTarget::Head)
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect();
        assert_eq!(&subjects[..3], ["feature three", "feature one", "feature two"]);
        let states = pick_states(r, "refs/heads/feature").unwrap();
        assert!(states.values().all(|s| *s == PickState::AlreadyPicked));
    }

    #[test]
    fn conflicting_pick_can_be_aborted() {
        let t = temp_repo("conflict");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        r.git(&["switch", "-q", "-c", "feature"]).unwrap();
        let f = commit_file(r, "a.txt", "feature", "feature edit");
        r.git(&["switch", "-q", "main"]).unwrap();
        commit_file(r, "a.txt", "main", "main edit");

        assert!(cherry_pick(r, &[f], false).is_err());
        assert_eq!(paused(r), Some(Paused::CherryPick));
        let st = status(r).unwrap();
        assert_eq!(st[0].unstaged, Some(Change::Conflicted));
        abort_paused(r, Paused::CherryPick).unwrap();
        assert_eq!(paused(r), None);
        assert!(status(r).unwrap().is_empty());
    }

    #[test]
    fn status_and_staging() {
        let t = temp_repo("status");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        std::fs::write(r.root.join("a.txt"), "changed\n").unwrap();
        std::fs::write(r.root.join("new.txt"), "new\n").unwrap();
        let st = status(r).unwrap();
        assert_eq!(st.len(), 2);
        assert_eq!(st[0].unstaged, Some(Change::Modified));
        assert_eq!(st[1].unstaged, Some(Change::Untracked));

        let untracked = file_diff(r, &st[1], false).unwrap().unwrap();
        assert_eq!(untracked.change, FileChange::Added);
        assert_eq!(untracked.additions, 1);

        stage(r, &["a.txt".into()]).unwrap();
        let st = status(r).unwrap();
        assert_eq!(st[0].staged, Some(Change::Modified));
        assert_eq!(st[0].unstaged, None);
        let d = file_diff(r, &st[0], true).unwrap().unwrap();
        assert_eq!((d.additions, d.deletions), (1, 1));

        commit(r, "edit a").unwrap();
        let st = status(r).unwrap();
        assert_eq!(st.len(), 1, "only the untracked file is left");
    }

    fn lines_where(f: &FileDiff, kind: LineKind, text: &str) -> usize {
        f.lines
            .iter()
            .position(|l| l.kind == kind && l.text == text)
            .unwrap()
    }

    /// base: 1..=10, one per line. Worktree: 2 → two, 9 → nine.
    fn two_hunk_repo(name: &str) -> TempRepo {
        let t = temp_repo(name);
        let r = &t.0;
        let base: String = (1..=10).map(|n| format!("{n}\n")).collect();
        std::fs::write(r.root.join("n.txt"), &base).unwrap();
        r.git(&["add", "-A"]).unwrap();
        r.git(&["commit", "-q", "-m", "base"]).unwrap();
        let changed = base.replace("2\n", "two\n").replace("9\n", "nine\n");
        std::fs::write(r.root.join("n.txt"), changed).unwrap();
        t
    }

    fn unstaged(r: &Repo) -> FileDiff {
        let e = status(r).unwrap().remove(0);
        file_diff(r, &e, false).unwrap().unwrap()
    }

    fn staged(r: &Repo) -> FileDiff {
        let e = status(r).unwrap().remove(0);
        file_diff(r, &e, true).unwrap().unwrap()
    }

    #[test]
    fn stage_one_hunk_then_unstage_one_line() {
        let t = two_hunk_repo("hunks");
        let r = &t.0;
        let d = unstaged(r);
        assert!(supports_partial(&d));
        // Stage the second change only: "9" → "nine".
        let chosen: std::collections::HashSet<usize> = [
            lines_where(&d, LineKind::Del, "9"),
            lines_where(&d, LineKind::Add, "nine"),
        ]
        .into();
        let patch = partial_patch(&d, &chosen, PatchOp::Stage).unwrap();
        apply_patch(r, &patch, PatchOp::Stage).unwrap();
        let index = r.git(&["show", ":n.txt"]).unwrap();
        assert!(index.contains("\n2\n") && index.contains("nine"), "{index}");

        // Unstage only the removal of "9": the index keeps "9" and "nine".
        let s = staged(r);
        let chosen: std::collections::HashSet<usize> = [lines_where(&s, LineKind::Del, "9")].into();
        let patch = partial_patch(&s, &chosen, PatchOp::Unstage).unwrap();
        apply_patch(r, &patch, PatchOp::Unstage).unwrap();
        let index = r.git(&["show", ":n.txt"]).unwrap();
        assert!(index.contains("\n9\nnine\n"), "{index}");
        // The working tree never changed.
        let wt = std::fs::read_to_string(r.root.join("n.txt")).unwrap();
        assert!(wt.contains("two") && wt.contains("nine") && !wt.contains("\n9\n"));
    }

    #[test]
    fn discard_one_hunk() {
        let t = two_hunk_repo("discard");
        let r = &t.0;
        let d = unstaged(r);
        let chosen: std::collections::HashSet<usize> = [
            lines_where(&d, LineKind::Del, "2"),
            lines_where(&d, LineKind::Add, "two"),
        ]
        .into();
        let patch = partial_patch(&d, &chosen, PatchOp::Discard).unwrap();
        apply_patch(r, &patch, PatchOp::Discard).unwrap();
        let wt = std::fs::read_to_string(r.root.join("n.txt")).unwrap();
        assert!(wt.contains("\n2\n") && wt.contains("nine"), "{wt}");
    }

    #[test]
    fn discard_whole_files() {
        let t = two_hunk_repo("discard-files");
        let r = &t.0;
        std::fs::write(r.root.join("new.txt"), "x\n").unwrap();
        let entries = status(r).unwrap();
        discard(r, &entries).unwrap();
        assert!(status(r).unwrap().is_empty());
        assert!(!r.root.join("new.txt").exists());
    }

    #[test]
    fn branch_create_rename_delete() {
        let t = temp_repo("branches");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        assert!(check_branch_name(r, "feature/ok").is_none());
        assert!(check_branch_name(r, "bad..name").is_some());
        create_branch(r, "topic", None, true).unwrap();
        assert_eq!(head(r).branch.as_deref(), Some("topic"));
        commit_file(r, "b.txt", "topic", "topic work");
        r.git(&["switch", "-q", "main"]).unwrap();
        rename_branch(r, "topic", "topic2").unwrap();
        // Not merged: a plain delete refuses, a forced one works.
        let err = delete_branch(r, "topic2", false).unwrap_err().to_string();
        assert!(err.contains("not fully merged"), "{err}");
        delete_branch(r, "topic2", true).unwrap();
        assert!(branches(r).unwrap().iter().all(|b| b.name != "topic2"));
    }

    #[test]
    fn delete_remote_branches() {
        let origin = temp_repo("remote-origin");
        let t = temp_repo("remote-clone");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        let url = origin.0.root.to_string_lossy().into_owned();
        r.git(&["remote", "add", "origin", url.as_str()]).unwrap();
        r.git(&["push", "-q", "origin", "main:feature/x", "main:gone"]).unwrap();
        r.git(&["fetch", "-q", "origin"]).unwrap();
        let remote_names = |r: &Repo| -> Vec<String> {
            branches(r)
                .unwrap()
                .into_iter()
                .filter(|b| b.kind == RefKind::Remote)
                .map(|b| b.name)
                .collect()
        };
        assert_eq!(
            remote_branch(r, "refs/remotes/origin/feature/x").unwrap(),
            ("origin".into(), "feature/x".into())
        );

        delete_remote_branch(r, "refs/remotes/origin/feature/x").unwrap();
        assert!(origin.0.git(&["rev-parse", "-q", "--verify", "refs/heads/feature/x"]).is_err());
        assert_eq!(remote_names(r), ["origin/gone"]);

        // Someone else deleted it already: only the stale ref goes.
        origin.0.git(&["branch", "-D", "gone"]).unwrap();
        delete_remote_branch(r, "refs/remotes/origin/gone").unwrap();
        assert!(remote_names(r).is_empty());
    }

    #[test]
    fn stash_round_trip() {
        let t = two_hunk_repo("stash");
        let r = &t.0;
        std::fs::write(r.root.join("new.txt"), "fresh\n").unwrap();
        stash_push(r, "half done").unwrap();
        assert!(status(r).unwrap().is_empty());
        let list = stashes(r).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title(), "half done");
        assert_eq!(list[0].branch(), Some("main"));
        let d = stash_detail(r, &list[0]).unwrap();
        let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"n.txt") && paths.contains(&"new.txt"), "{paths:?}");
        stash_apply(r, 0, true).unwrap();
        assert!(stashes(r).unwrap().is_empty());
        assert_eq!(status(r).unwrap().len(), 2);
    }

    fn subjects(r: &Repo) -> Vec<String> {
        log(r, &LogTarget::Head)
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect()
    }

    #[test]
    fn interactive_rebase_plan() {
        let t = temp_repo("rebase");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        let one = commit_file(r, "b.txt", "one", "one");
        commit_file(r, "c.txt", "two", "two");
        commit_file(r, "d.txt", "three", "three");
        commit_file(r, "e.txt", "four", "four");
        // Dirty tree: the rebase stashes it and puts it back.
        std::fs::write(r.root.join("a.txt"), "local edit\n").unwrap();

        let mut plan = rebase_plan(r, &one).unwrap();
        let names: Vec<&str> = plan.steps.iter().map(|s| s.subject.as_str()).collect();
        assert_eq!(names, ["one", "two", "three", "four"]);
        // four first, reword one, squash two into one, drop three.
        let four = plan.steps.pop().unwrap();
        plan.steps.insert(0, four);
        plan.steps[1].action = RebaseAction::Reword;
        plan.steps[1].new_subject = Some("one, reworded".into());
        plan.steps[2].action = RebaseAction::Fixup;
        plan.steps[3].action = RebaseAction::Drop;
        rebase_run(r, &plan).unwrap();

        assert_eq!(subjects(r), ["one, reworded", "four", "base"]);
        assert!(r.root.join("c.txt").exists(), "fixup keeps the change");
        assert!(!r.root.join("d.txt").exists(), "drop removes it");
        let a = std::fs::read_to_string(r.root.join("a.txt")).unwrap();
        assert_eq!(a, "local edit\n");
        assert_eq!(paused(r), None);
    }

    #[test]
    fn rebase_plan_checks() {
        let step = |action| RebaseStep {
            sha: "x".into(),
            subject: "s".into(),
            body: String::new(),
            action,
            new_subject: None,
        };
        let plan = |actions: &[RebaseAction]| RebasePlan {
            base: None,
            steps: actions.iter().map(|a| step(*a)).collect(),
            merges: 0,
        };
        use RebaseAction::*;
        assert!(check_plan(&plan(&[Pick, Squash])).is_none());
        assert!(check_plan(&plan(&[Drop, Fixup])).is_some());
        assert!(check_plan(&plan(&[Drop, Drop])).is_some());
    }

    #[test]
    fn rebase_conflict_pauses_and_aborts() {
        let t = temp_repo("rebase-conflict");
        let r = &t.0;
        let first = commit_file(r, "a.txt", "one", "one");
        commit_file(r, "a.txt", "two", "two");
        commit_file(r, "a.txt", "three", "three");
        let mut plan = rebase_plan(r, &first).unwrap();
        // Swapping two edits of the same line conflicts.
        plan.steps.swap(1, 2);
        assert!(rebase_run(r, &plan).is_err());
        assert_eq!(paused(r), Some(Paused::Rebase));
        abort_paused(r, Paused::Rebase).unwrap();
        assert_eq!(paused(r), None);
        assert_eq!(subjects(r), ["three", "two", "one"]);
    }

    #[test]
    fn parse_patch_numbers_lines() {
        let text = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n\
                    @@ -1,3 +1,3 @@\n keep\n-old\n+new\n keep2\n";
        let files = parse_patch(text);
        assert_eq!(files.len(), 1);
        let l = &files[0].lines;
        assert_eq!(l[0].kind, LineKind::Hunk);
        assert_eq!((l[1].old_no, l[1].new_no), (Some(1), Some(1)));
        assert_eq!((l[2].kind, l[2].old_no, l[2].new_no), (LineKind::Del, Some(2), None));
        assert_eq!((l[3].kind, l[3].old_no, l[3].new_no), (LineKind::Add, None, Some(2)));
        assert_eq!((l[4].old_no, l[4].new_no), (Some(3), Some(3)));
    }
}
