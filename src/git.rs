//! Git backend: every operation runs the `git` CLI, so hooks, signing,
//! credential helpers and SSH keys behave exactly as in the terminal.
//! Everything here blocks; the UI calls it on the background executor.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

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
    Ok(PathBuf::from(
        repo.git(&["rev-parse", "--absolute-git-dir"])?.trim(),
    ))
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
    /// The commit it points at.
    pub sha: String,
    pub ahead: u32,
    pub behind: u32,
    /// It has an upstream, and the upstream is gone: someone deleted the
    /// remote branch, often when its pull request merged.
    pub gone: bool,
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
        "--format=%(refname)%1f%(refname:short)%1f%(upstream:track,nobracket)%1f%(HEAD)%1f%(objectname)",
        "refs/heads",
        "refs/remotes",
        "refs/tags",
    ])?;
    let mut list = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\x1f').collect();
        if f.len() < 5 {
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
            sha: f[4].to_string(),
            ahead,
            behind,
            gone: f[2].trim() == "gone",
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
        &[
            ("GIT_SEQUENCE_EDITOR", editor.as_str()),
            ("GIT_EDITOR", "true"),
        ],
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

impl FileDiff {
    /// The blob ids of the old and the new side, from the `index` line.
    /// Git can shorten them. None for a side that does not exist, and for
    /// both sides when the diff has no `index` line (a pure rename).
    pub fn blob_ids(&self) -> (Option<&str>, Option<&str>) {
        fn present(id: &str) -> Option<&str> {
            (!id.bytes().all(|b| b == b'0')).then_some(id)
        }
        self.header
            .iter()
            .find_map(|l| l.strip_prefix("index "))
            .and_then(|rest| rest.split(' ').next()?.split_once(".."))
            .map_or((None, None), |(old, new)| (present(old), present(new)))
    }
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

/// New files that a review of a worktree shows at most. An agent can make
/// thousands, for example when a build folder is not ignored.
const REVIEW_NEW_FILES: usize = 500;

/// What a branch changed since it left its base.
#[derive(Clone, Debug)]
pub struct BranchDiff {
    /// The last commit that the branch and the base share.
    pub merge_base: String,
    /// Commits of the branch that the base does not have.
    pub commits: usize,
    pub files: Vec<FileDiff>,
    /// New files past `REVIEW_NEW_FILES` that the diff leaves out.
    pub more_new_files: usize,
}

/// The combined change of `target` since it left `base`, as in
/// `git diff base...target`. With `worktree` (a folder that has `target`
/// checked out), the diff runs to that folder's files instead, so
/// uncommitted changes and new files count too.
pub fn branch_diff(
    repo: &Repo,
    base: &str,
    target: &str,
    worktree: Option<&Path>,
) -> Result<BranchDiff> {
    let merge_base = repo
        .git(&["merge-base", base, target])
        .map_err(|_| anyhow!("{target} and {base} have no commit in common."))?
        .trim()
        .to_string();
    let range = format!("{base}..{target}");
    let commits = repo
        .git(&["rev-list", "--count", range.as_str()])?
        .trim()
        .parse()
        .unwrap_or(0);
    let mut more_new_files = 0;
    let files = match worktree {
        None => {
            parse_patch(&repo.git(&["diff", "--no-color", "-M", merge_base.as_str(), target])?)
        }
        Some(dir) => {
            let mut files = parse_patch(&run_in(
                dir,
                &["diff", "--no-color", "-M", merge_base.as_str()],
            )?);
            let out = run_in(dir, &["ls-files", "--others", "--exclude-standard", "-z"])?;
            let new: Vec<&str> = out.split('\0').filter(|p| !p.is_empty()).collect();
            more_new_files = new.len().saturating_sub(REVIEW_NEW_FILES);
            for path in new.into_iter().take(REVIEW_NEW_FILES) {
                // `--no-index` exits 1 when the files differ, which is always here.
                let out = command(
                    dir,
                    &["diff", "--no-color", "--no-index", "--", "/dev/null", path],
                )
                .output()?;
                files.extend(parse_patch(&String::from_utf8_lossy(&out.stdout)));
            }
            files
        }
    };
    Ok(BranchDiff {
        merge_base,
        commits,
        files,
        more_new_files,
    })
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
    /// When the file last changed on disk. None when the file is gone.
    pub modified: Option<SystemTime>,
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
                    modified: None,
                });
            }
            b'u' => {
                let parts: Vec<&str> = rec.splitn(11, ' ').collect();
                entries.push(StatusEntry {
                    path: parts.last().copied().unwrap_or_default().to_string(),
                    staged: None,
                    unstaged: Some(Change::Conflicted),
                    modified: None,
                });
            }
            b'?' => entries.push(StatusEntry {
                path: rec[2..].to_string(),
                staged: None,
                unstaged: Some(Change::Untracked),
                modified: None,
            }),
            _ => {}
        }
    }
    // Git reads the times of the tracked files for the status anyway, so
    // this costs little next to it. A link is its own content, as for Git.
    for e in &mut entries {
        e.modified = std::fs::symlink_metadata(repo.root.join(&e.path))
            .and_then(|m| m.modified())
            .ok();
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

/// The size of a blob in bytes. `id` can be short.
pub fn blob_size(repo: &Repo, id: &str) -> Result<u64> {
    Ok(repo.git(&["cat-file", "-s", id])?.trim().parse()?)
}

/// The content of a blob, as Git stores it. `id` can be short.
pub fn blob(repo: &Repo, id: &str) -> Result<Vec<u8>> {
    let out = command(&repo.root, &["cat-file", "blob", id])
        .output()
        .context("could not start git cat-file")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
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
    for e in entries
        .iter()
        .filter(|e| e.unstaged == Some(Change::Untracked))
    {
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
            let index = f
                .first()?
                .strip_prefix("stash@{")?
                .strip_suffix('}')?
                .parse()
                .ok()?;
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
    let header = repo.git(&["log", "-1", "--format=%H%x1f%an%x1f%ae%x1f%at", r.as_str()])?;
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

/// The local branches whose tips `base` contains, as full ref names: the
/// branches that are merged into it.
pub fn merged_branches(repo: &Repo, base: &str) -> Result<HashSet<String>> {
    let merged = format!("--merged={base}");
    Ok(repo
        .git(&[
            "for-each-ref",
            merged.as_str(),
            "--format=%(refname)",
            "refs/heads",
        ])?
        .lines()
        .map(str::to_string)
        .collect())
}

/// Whether the repository has the commit `oid`.
pub fn has_commit(repo: &Repo, oid: &str) -> bool {
    let spec = format!("{oid}^{{commit}}");
    repo.git(&["cat-file", "-e", spec.as_str()]).is_ok()
}

/// Whether the commit `a` is `b` or one of its ancestors.
pub fn is_ancestor(repo: &Repo, a: &str, b: &str) -> bool {
    repo.git(&["merge-base", "--is-ancestor", a, b]).is_ok()
}

/// How many commits of `sha` the ref `base` does not have.
pub fn commits_not_in(repo: &Repo, sha: &str, base: &str) -> Result<u32> {
    let range = format!("{base}..{sha}");
    let out = repo.git(&["rev-list", "--count", range.as_str()])?;
    Ok(out.trim().parse()?)
}

/// Whether `base` has the changes of `sha` already: a merge of `sha` into
/// `base` changes no file. That is so after a squash merge, a rebase or a
/// cherry-pick, where `base` has other commits with the same changes. The
/// merge runs in memory (Git 2.38 or later) and only writes trees that no
/// ref points at. False on a conflict, and when unsure.
pub fn changes_in(repo: &Repo, sha: &str, base: &str) -> bool {
    let merged = repo.git(&["merge-tree", "--write-tree", base, sha]);
    let base_tree = repo.git(&["rev-parse", format!("{base}^{{tree}}").as_str()]);
    match (merged, base_tree) {
        (Ok(m), Ok(b)) => m.lines().next().map(str::trim) == Some(b.trim()),
        _ => false,
    }
}

/// Delete the local branch `name` for a cleanup, after the worktree that
/// has it checked out (with `force`, also when that worktree has changes).
/// When the branch does not point at `sha` any more, nothing is deleted:
/// a commit since the user saw the list is not lost.
pub fn delete_stale_branch(
    repo: &Repo,
    name: &str,
    sha: &str,
    worktree: Option<(&Path, bool)>,
) -> Result<()> {
    let refname = format!("refs/heads/{name}");
    let now = repo
        .git(&["rev-parse", "-q", "--verify", refname.as_str()])
        .map_err(|_| anyhow!("{name} is gone already."))?;
    if now.trim() != sha {
        bail!("{name} has new commits.");
    }
    if let Some((path, force)) = worktree {
        remove_worktree(repo, path, force)?;
    }
    // Forced: a plain delete compares with HEAD or the upstream, and the
    // caller compared with the base branch.
    delete_branch(repo, name, true)
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
    repo.git(&["push", "-u", remote.as_str(), branch])
        .map(|_| ())
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

/// The branch that other branches start from: the default branch of a
/// remote (origin first), as the local branch of that name when there is
/// one. Else `main` or `master`. A full ref name.
pub fn base_branch(repo: &Repo) -> Option<String> {
    let exists = |r: &str| repo.git(&["rev-parse", "-q", "--verify", r]).is_ok();
    let mut names: Vec<String> = repo
        .git(&["remote"])
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();
    names.sort_by_key(|n| n != "origin");
    for remote in names {
        let head = format!("refs/remotes/{remote}/HEAD");
        let Ok(target) = repo.git(&["symbolic-ref", "-q", head.as_str()]) else {
            continue;
        };
        let target = target.trim();
        let Some(name) = target.strip_prefix(&format!("refs/remotes/{remote}/")) else {
            continue;
        };
        let local = format!("refs/heads/{name}");
        return Some(if exists(&local) {
            local
        } else {
            target.to_string()
        });
    }
    ["refs/heads/main", "refs/heads/master"]
        .into_iter()
        .find(|r| exists(r))
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Worktrees

#[derive(Clone, Debug, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Full ref name, `refs/heads/x`. None when detached.
    pub branch: Option<String>,
    /// The first worktree: the one with the `.git` folder.
    pub main: bool,
    pub locked: bool,
    /// Its folder is gone. `git worktree prune` forgets it.
    pub prunable: bool,
}

impl Worktree {
    /// `feature/x` for `refs/heads/feature/x`.
    pub fn branch_name(&self) -> Option<&str> {
        let b = self.branch.as_deref()?;
        Some(b.strip_prefix("refs/heads/").unwrap_or(b))
    }

    /// The name of its folder.
    pub fn folder(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    /// The worktree as a repository, to run git in it.
    pub fn repo(&self) -> Repo {
        Repo {
            root: self.path.clone(),
            name: self.folder(),
        }
    }
}

/// The worktrees of the repository, the main one first. Bare entries are
/// left out: they have no files.
pub fn worktrees(repo: &Repo) -> Result<Vec<Worktree>> {
    Ok(parse_worktrees(&repo.git(&[
        "worktree",
        "list",
        "--porcelain",
    ])?))
}

fn parse_worktrees(text: &str) -> Vec<Worktree> {
    let mut list = Vec::new();
    for block in text.split("\n\n") {
        let mut wt = Worktree {
            path: PathBuf::new(),
            head: None,
            branch: None,
            main: list.is_empty(),
            locked: false,
            prunable: false,
        };
        let mut bare = false;
        for line in block.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => wt.path = PathBuf::from(value),
                "HEAD" => wt.head = Some(value.to_string()),
                "branch" => wt.branch = Some(value.to_string()),
                "bare" => bare = true,
                "locked" => wt.locked = true,
                "prunable" => wt.prunable = true,
                _ => {}
            }
        }
        if wt.path.as_os_str().is_empty() {
            continue;
        }
        // Symlinks such as /tmp → /private/tmp: compare the real paths.
        if let Ok(real) = wt.path.canonicalize() {
            wt.path = real;
        }
        if !bare {
            list.push(wt);
        }
    }
    list
}

/// What a worktree holds now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorktreeInfo {
    /// Changed files: staged, unstaged and new.
    pub changed: usize,
    /// Commits of its HEAD that the base branch does not have, and the reverse.
    pub ahead: u32,
    pub behind: u32,
    /// The subject of its HEAD commit.
    pub subject: String,
    /// The last change: the time of its HEAD commit or of its newest changed
    /// file, whichever is later.
    pub active: i64,
    /// A cherry-pick, rebase or merge stopped on a conflict there.
    pub paused: Option<Paused>,
}

/// How far `wt` is from `base` (a ref), and what changed in it.
pub fn worktree_info(wt: &Worktree, base: Option<&str>) -> Result<WorktreeInfo> {
    let r = wt.repo();
    let changes = status(&r)?;
    let mut info = WorktreeInfo {
        changed: changes.len(),
        paused: paused(&r),
        ..Default::default()
    };
    if let Ok(out) = r.git(&["log", "-1", "--format=%ct%x1f%s", "HEAD"])
        && let Some((time, subject)) = out.trim_end().split_once('\x1f')
    {
        info.active = time.parse().unwrap_or(0);
        info.subject = subject.to_string();
    }
    if let Some(t) = changes.iter().filter_map(|e| e.modified).max()
        && let Ok(d) = t.duration_since(std::time::UNIX_EPOCH)
    {
        info.active = info.active.max(d.as_secs() as i64);
    }
    if let Some(base) = base
        && wt.head.is_some()
    {
        let range = format!("{base}...HEAD");
        if let Ok(out) = r.git(&["rev-list", "--left-right", "--count", range.as_str()]) {
            let mut it = out.split_whitespace();
            info.behind = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            info.ahead = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }
    Ok(info)
}

/// Remove a worktree and its folder. Git refuses when the worktree has
/// changes, unless `force`: then the changes go too.
pub fn remove_worktree(repo: &Repo, path: &Path, force: bool) -> Result<()> {
    let path = path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path);
    repo.git(&args).map(|_| ())
}

/// Forget the worktrees whose folders are gone.
pub fn prune_worktrees(repo: &Repo) -> Result<()> {
    repo.git(&["worktree", "prune"]).map(|_| ())
}

/// The folder with the refs and the data of all worktrees: the `.git`
/// folder of the main worktree.
pub fn common_dir(repo: &Repo) -> Result<PathBuf> {
    let out = repo.git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    Ok(PathBuf::from(out.trim()))
}

// ---------------------------------------------------------------------------
// Activity: the moves of the branches, from their reflogs

/// How far back the activity goes, and how many moves it keeps at most.
const ACTIVITY_DAYS: i64 = 30;
const ACTIVITY_MAX: usize = 500;
/// Remote branches whose reflogs the activity reads at most, the most
/// recently changed first: a repository can have thousands.
const ACTIVITY_REMOTES: usize = 200;
/// The reflog action of a restore: "gibbon restore: moving to <sha>".
const RESTORE_ACTION: &str = "gibbon restore";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    Commit,
    Amend,
    Rebase,
    Reset,
    Merge,
    Pull,
    CherryPick,
    Revert,
    /// The branch was made.
    Created,
    /// A push moved the remote-tracking branch.
    Push,
    /// A worktree switched to another branch or commit.
    Switch,
    /// Gibbon restored the branch.
    Restore,
    Other,
}

impl MoveKind {
    /// The kind of a reflog message: "commit (amend): …" is an amend.
    fn of(message: &str) -> MoveKind {
        let action = message.split_once(": ").map_or(message, |(a, _)| a);
        match action {
            "commit" | "commit (initial)" => MoveKind::Commit,
            "commit (amend)" => MoveKind::Amend,
            "commit (merge)" => MoveKind::Merge,
            "reset" => MoveKind::Reset,
            "cherry-pick" => MoveKind::CherryPick,
            "revert" => MoveKind::Revert,
            "update by push" => MoveKind::Push,
            "checkout" => MoveKind::Switch,
            RESTORE_ACTION => MoveKind::Restore,
            a if a.starts_with("rebase") || a.starts_with("pull --rebase") => MoveKind::Rebase,
            a if a.starts_with("merge") => MoveKind::Merge,
            a if a.starts_with("pull") => MoveKind::Pull,
            "branch" if message.contains("Created from") => MoveKind::Created,
            "branch" if message.contains("Reset to") => MoveKind::Reset,
            _ => MoveKind::Other,
        }
    }

    /// The word the timeline shows.
    pub fn verb(self) -> &'static str {
        match self {
            MoveKind::Commit => "Commit",
            MoveKind::Amend => "Amend",
            MoveKind::Rebase => "Rebase",
            MoveKind::Reset => "Reset",
            MoveKind::Merge => "Merge",
            MoveKind::Pull => "Pull",
            MoveKind::CherryPick => "Cherry-pick",
            MoveKind::Revert => "Revert",
            MoveKind::Created => "Create",
            MoveKind::Push => "Push",
            MoveKind::Switch => "Switch",
            MoveKind::Restore => "Restore",
            MoveKind::Other => "Move",
        }
    }
}

/// One move of a branch.
#[derive(Clone, Debug, PartialEq)]
pub struct RefMove {
    /// `refs/heads/x`, `refs/remotes/origin/x`, or `HEAD` of a worktree.
    pub refname: String,
    /// For `HEAD`: the worktree's folder name. None for the main worktree.
    pub worktree: Option<String>,
    /// None when the move made the ref.
    pub old: Option<String>,
    pub new: String,
    pub time: i64,
    /// What git wrote, without the action: the subject of a commit, or
    /// "moving to HEAD~1" for a reset.
    pub detail: String,
    pub kind: MoveKind,
    /// The commits that the move added to the ref and dropped from it,
    /// when they are known (see `move_counts`).
    pub counts: Option<(u32, u32)>,
}

impl RefMove {
    /// The counts are clear from the kind, or `move_counts` must ask git.
    pub fn needs_counts(&self) -> bool {
        self.counts.is_none() && self.old.is_some() && self.kind != MoveKind::Switch
    }

    /// The move dropped commits: an amend, a reset, a rebase or a forced push.
    pub fn rewrites(&self) -> bool {
        self.counts.is_some_and(|(_, dropped)| dropped > 0)
    }
}

/// One reflog line: `<old> <new> <name> <<email>> <time> <tz>\t<message>`.
fn parse_reflog_line(line: &str) -> Option<(Option<String>, String, i64, String)> {
    let (head, message) = line.split_once('\t').unwrap_or((line, ""));
    let mut parts = head.splitn(3, ' ');
    let old = parts.next()?;
    let new = parts.next()?.to_string();
    let who = parts.next()?;
    let mut tail = who.rsplitn(3, ' ');
    let _tz = tail.next()?;
    let time = tail.next()?.parse().ok()?;
    let old = (!old.bytes().all(|b| b == b'0')).then(|| old.to_string());
    Some((old, new, time, message.trim_end().to_string()))
}

/// The moves in the reflog file `path` of `refname`, oldest first. `keep`
/// says which kinds the timeline shows for this ref.
fn read_reflog(
    path: &Path,
    refname: &str,
    worktree: Option<&str>,
    since: i64,
    keep: &dyn Fn(MoveKind) -> bool,
) -> Vec<RefMove> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return vec![];
    };
    text.lines()
        .filter_map(parse_reflog_line)
        .filter(|(_, _, time, _)| *time >= since)
        .filter_map(|(old, new, time, message)| {
            let kind = MoveKind::of(&message);
            if !keep(kind) {
                return None;
            }
            let detail = message
                .split_once(": ")
                .map_or(String::new(), |(_, d)| d.to_string());
            let counts = match kind {
                MoveKind::Commit | MoveKind::CherryPick | MoveKind::Revert => Some((1, 0)),
                MoveKind::Amend => Some((1, 1)),
                MoveKind::Created => Some((0, 0)),
                _ => None,
            };
            Some(RefMove {
                refname: refname.to_string(),
                worktree: worktree.map(str::to_string),
                old,
                new,
                time,
                detail,
                kind,
                counts,
            })
        })
        .collect()
}

/// The files under `dir`, as (path relative to `dir`, file).
fn files_under(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                out.push((rel.to_string_lossy().into_owned(), p));
            }
        }
    }
    out
}

/// The moves of the last 30 days, newest first: every move of the local
/// branches, the pushes of the remote branches, and the switches of each
/// worktree. The reflogs are read from their files, so a repository in the
/// reftable format shows none.
pub fn ref_moves(repo: &Repo) -> Result<Vec<RefMove>> {
    let common = common_dir(repo)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let since = now - ACTIVITY_DAYS * 86_400;
    let logs = common.join("logs");
    let mut moves = vec![];
    for (rel, path) in files_under(&logs.join("refs/heads")) {
        let refname = format!("refs/heads/{rel}");
        moves.extend(read_reflog(&path, &refname, None, since, &|k| {
            k != MoveKind::Switch
        }));
    }
    let mut remotes = files_under(&logs.join("refs/remotes"));
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    remotes.sort_by_cached_key(|(_, p)| std::cmp::Reverse(modified(p)));
    for (rel, path) in remotes.into_iter().take(ACTIVITY_REMOTES) {
        let refname = format!("refs/remotes/{rel}");
        moves.extend(read_reflog(&path, &refname, None, since, &|k| {
            k == MoveKind::Push
        }));
    }
    let switches = |k| k == MoveKind::Switch;
    moves.extend(read_reflog(
        &logs.join("HEAD"),
        "HEAD",
        None,
        since,
        &switches,
    ));
    if let Ok(entries) = std::fs::read_dir(common.join("worktrees")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let head = e.path().join("logs/HEAD");
            moves.extend(read_reflog(&head, "HEAD", Some(&name), since, &switches));
        }
    }
    // Newest first. Moves in the same second keep their order in the file.
    let mut moves: Vec<(usize, RefMove)> = moves.into_iter().enumerate().collect();
    moves.sort_by(|(ia, a), (ib, b)| b.time.cmp(&a.time).then(ib.cmp(ia)));
    Ok(moves
        .into_iter()
        .map(|(_, m)| m)
        .take(ACTIVITY_MAX)
        .collect())
}

/// The commits that moving a ref from `old` to `new` added and dropped.
/// None when git no longer has one of them.
pub fn move_counts(repo: &Repo, old: &str, new: &str) -> Option<(u32, u32)> {
    let range = format!("{old}...{new}");
    let out = repo
        .git(&["rev-list", "--left-right", "--count", range.as_str()])
        .ok()?;
    let mut it = out.split_whitespace();
    let dropped = it.next()?.parse().ok()?;
    let added = it.next()?.parse().ok()?;
    Some((added, dropped))
}

/// Move the local branch `refname` to `to`, only if it still points at
/// `expect`. A branch that `worktree` has checked out moves with
/// `git reset --keep` there: its files follow, uncommitted changes stay,
/// and git stops when they would conflict. The restore is a move in the
/// reflog too, so it can be undone the same way.
pub fn restore_branch(
    repo: &Repo,
    refname: &str,
    to: &str,
    expect: &str,
    worktree: Option<&Path>,
) -> Result<String> {
    let at = repo.git(&["rev-parse", "-q", "--verify", refname])?;
    if at.trim() != expect {
        bail!("{refname} moved since the timeline loaded. Look again, then restore.");
    }
    match worktree {
        Some(dir) => run_env(
            dir,
            &["reset", "--keep", "-q", to],
            &[("GIT_REFLOG_ACTION", RESTORE_ACTION)],
        ),
        None => {
            let msg = format!("{RESTORE_ACTION}: moving to {to}");
            repo.git(&["update-ref", "-m", msg.as_str(), refname, to, expect])
        }
    }
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
        assert_eq!(
            &subjects[..3],
            ["feature three", "feature one", "feature two"]
        );
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

    #[test]
    fn blobs_of_a_binary_file() {
        let t = temp_repo("blobs");
        let r = &t.0;
        let (old, new) = (b"\x89PNG\0old".to_vec(), b"\x89PNG\0new!".to_vec());
        std::fs::write(r.root.join("a.png"), &old).unwrap();
        stage(r, &["a.png".into()]).unwrap();
        let st = status(r).unwrap();
        let added = file_diff(r, &st[0], true).unwrap().unwrap();
        assert!(added.binary);
        let (none, id) = added.blob_ids();
        assert_eq!(none, None, "an added file has no old side");
        assert_eq!(blob(r, id.unwrap()).unwrap(), old);
        assert_eq!(blob_size(r, id.unwrap()).unwrap(), old.len() as u64);

        commit(r, "add a.png").unwrap();
        std::fs::write(r.root.join("a.png"), &new).unwrap();
        let st = status(r).unwrap();
        let changed = file_diff(r, &st[0], false).unwrap().unwrap();
        let (was, _) = changed.blob_ids();
        assert_eq!(was, id, "the old side is the committed blob");
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
        r.git(&["push", "-q", "origin", "main:feature/x", "main:gone"])
            .unwrap();
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
        assert!(
            origin
                .0
                .git(&["rev-parse", "-q", "--verify", "refs/heads/feature/x"])
                .is_err()
        );
        assert_eq!(remote_names(r), ["origin/gone"]);

        // Someone else deleted it already: only the stale ref goes.
        origin.0.git(&["branch", "-D", "gone"]).unwrap();
        delete_remote_branch(r, "refs/remotes/origin/gone").unwrap();
        assert!(remote_names(r).is_empty());
    }

    #[test]
    fn merged_and_gone_branches_and_their_cleanup() {
        let origin = temp_repo("cleanup-origin");
        let t = temp_repo("cleanup");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        let url = origin.0.root.to_string_lossy().into_owned();
        r.git(&["remote", "add", "origin", url.as_str()]).unwrap();
        // merged: its commit is in main now.
        r.git(&["switch", "-q", "-c", "merged"]).unwrap();
        commit_file(r, "b.txt", "merged", "merged work");
        r.git(&["switch", "-q", "main"]).unwrap();
        r.git(&["merge", "-q", "--no-ff", "-m", "merge", "merged"])
            .unwrap();
        // gone: pushed, then its remote branch was deleted (as GitHub does
        // after a squash merge), so main does not have its commit.
        r.git(&["switch", "-q", "-c", "gone"]).unwrap();
        let gone_sha = commit_file(r, "c.txt", "gone", "gone work");
        r.git(&["push", "-q", "-u", "origin", "gone"]).unwrap();
        origin.0.git(&["branch", "-D", "gone"]).unwrap();
        r.git(&["fetch", "-q", "--prune", "origin"]).unwrap();
        // squashed: gone too, and main has its changes in another commit.
        r.git(&["switch", "-q", "-c", "squashed", "main"]).unwrap();
        let squashed_sha = commit_file(r, "e.txt", "squashed", "squashed work");
        r.git(&["push", "-q", "-u", "origin", "squashed"]).unwrap();
        r.git(&["switch", "-q", "main"]).unwrap();
        r.git(&["merge", "-q", "--squash", "squashed"]).unwrap();
        r.git(&["commit", "-q", "-m", "squash"]).unwrap();
        origin.0.git(&["branch", "-D", "squashed"]).unwrap();
        r.git(&["fetch", "-q", "--prune", "origin"]).unwrap();
        // open: work that is not merged anywhere.
        r.git(&["switch", "-q", "-c", "open", "main"]).unwrap();
        commit_file(r, "d.txt", "open", "open work");
        r.git(&["switch", "-q", "main"]).unwrap();

        let list = branches(r).unwrap();
        let get = |n: &str| list.iter().find(|b| b.name == n).unwrap().clone();
        assert!(get("gone").gone);
        assert!(!get("merged").gone && !get("open").gone && !get("main").gone);
        assert_eq!(get("gone").sha, gone_sha);
        let merged = merged_branches(r, "refs/heads/main").unwrap();
        assert!(merged.contains("refs/heads/merged") && merged.contains("refs/heads/main"));
        assert!(!merged.contains("refs/heads/gone") && !merged.contains("refs/heads/open"));
        assert_eq!(commits_not_in(r, &gone_sha, "refs/heads/main").unwrap(), 1);
        assert_eq!(
            commits_not_in(r, &get("merged").sha, "refs/heads/main").unwrap(),
            0
        );
        assert!(get("squashed").gone);
        assert_eq!(
            commits_not_in(r, &squashed_sha, "refs/heads/main").unwrap(),
            1
        );
        assert!(changes_in(r, &squashed_sha, "refs/heads/main"));
        assert!(has_commit(r, &squashed_sha) && !has_commit(r, &"0".repeat(40)));
        assert!(is_ancestor(r, &get("merged").sha, "refs/heads/main"));
        assert!(!is_ancestor(r, &gone_sha, "refs/heads/main"));
        assert!(!changes_in(r, &gone_sha, "refs/heads/main"));
        assert!(!changes_in(r, &get("open").sha, "refs/heads/main"));

        // A branch with a worktree: the worktree goes first.
        let wt_path = r.root.join("merged-wt");
        let wt_arg = wt_path.to_string_lossy().into_owned();
        r.git(&["worktree", "add", "-q", wt_arg.as_str(), "merged"])
            .unwrap();
        delete_stale_branch(r, "merged", &get("merged").sha, Some((&wt_path, false))).unwrap();
        assert!(!wt_path.exists());
        assert_eq!(worktrees(r).unwrap().len(), 1);

        // A branch that moved after the list loaded stays.
        commit_file(r, "a.txt", "more", "main moves");
        r.git(&["switch", "-q", "gone"]).unwrap();
        commit_file(r, "c.txt", "late", "late work");
        r.git(&["switch", "-q", "main"]).unwrap();
        let err = delete_stale_branch(r, "gone", &gone_sha, None).unwrap_err();
        assert!(err.to_string().contains("new commits"), "{err}");
        let late = branches(r)
            .unwrap()
            .into_iter()
            .find(|b| b.name == "gone")
            .unwrap();
        // Forced: git would refuse, as main does not have the commits.
        delete_stale_branch(r, "gone", &late.sha, None).unwrap();
        let names: Vec<String> = branches(r)
            .unwrap()
            .into_iter()
            .filter(|b| b.kind == RefKind::Local)
            .map(|b| b.name)
            .collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"main".to_string()) && names.contains(&"open".to_string()));
        let config = r
            .git(&["config", "--get-regexp", "^branch\\."])
            .unwrap_or_default();
        assert!(!config.contains("branch.gone."), "{config}");
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
        assert!(
            paths.contains(&"n.txt") && paths.contains(&"new.txt"),
            "{paths:?}"
        );
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
    fn worktrees_list_count_and_remove() {
        let t = temp_repo("worktrees");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        assert_eq!(base_branch(r).as_deref(), Some("refs/heads/main"));
        let wt_path = r.root.join("agent-wt");
        let wt_arg = wt_path.to_string_lossy().into_owned();
        r.git(&["worktree", "add", "-q", "-b", "agent", wt_arg.as_str()])
            .unwrap();

        let list = worktrees(r).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].main && !list[1].main);
        assert_eq!(list[0].path, r.root.canonicalize().unwrap());
        let wt = &list[1];
        assert_eq!(wt.path, wt_path.canonicalize().unwrap());
        assert_eq!(wt.branch_name(), Some("agent"));
        assert_eq!(wt.folder(), "agent-wt");

        // The agent commits once and leaves one file changed.
        let a = wt.repo();
        commit_file(&a, "b.txt", "one", "agent one");
        std::fs::write(wt.path.join("c.txt"), "new\n").unwrap();
        let info = worktree_info(wt, Some("refs/heads/main")).unwrap();
        assert_eq!((info.changed, info.ahead, info.behind), (1, 1, 0));
        assert_eq!(info.subject, "agent one");
        assert!(info.active > 0);
        assert_eq!(info.paused, None);

        // A changed worktree needs force.
        assert!(remove_worktree(r, &wt.path, false).is_err());
        remove_worktree(r, &wt.path, true).unwrap();
        assert!(!wt.path.exists());
        assert_eq!(worktrees(r).unwrap().len(), 1);
    }

    #[test]
    fn branch_diff_shows_only_the_branch() {
        let t = temp_repo("branch-diff");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        r.git(&["switch", "-q", "-c", "feature"]).unwrap();
        commit_file(r, "b.txt", "one", "feature one");
        commit_file(r, "b.txt", "two", "feature two");
        r.git(&["switch", "-q", "main"]).unwrap();
        // main moves on: its change is not part of the review.
        commit_file(r, "a.txt", "main", "main edit");

        let d = branch_diff(r, "refs/heads/main", "refs/heads/feature", None).unwrap();
        assert_eq!(d.commits, 2);
        let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["b.txt"]);
        assert_eq!((d.files[0].additions, d.files[0].deletions), (2, 0));

        // In a worktree: an uncommitted edit and a new file count too.
        let wt = r.root.join("wt");
        let wt_arg = wt.to_string_lossy().into_owned();
        r.git(&["worktree", "add", "-q", wt_arg.as_str(), "feature"])
            .unwrap();
        std::fs::write(wt.join("b.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(wt.join("new.txt"), "fresh\n").unwrap();
        let d = branch_diff(r, "refs/heads/main", "refs/heads/feature", Some(&wt)).unwrap();
        let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["b.txt", "new.txt"]);
        assert_eq!(d.files[0].additions, 3);
        assert_eq!(d.files[1].change, FileChange::Added);
        assert_eq!(d.more_new_files, 0);

        let err = branch_diff(r, "refs/heads/main", "refs/heads/nope", None).unwrap_err();
        assert!(err.to_string().contains("no commit in common"), "{err}");
    }

    #[test]
    fn activity_lists_moves_and_restores_them() {
        let t = temp_repo("activity");
        let r = &t.0;
        commit_file(r, "a.txt", "base", "base");
        r.git(&["switch", "-q", "-c", "agent"]).unwrap();
        commit_file(r, "b.txt", "one", "one");
        let two = commit_file(r, "b.txt", "two", "two");
        r.git(&["commit", "-q", "--amend", "-m", "two, amended"])
            .unwrap();
        let amended = r.git(&["rev-parse", "HEAD"]).unwrap().trim().to_string();
        r.git(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        r.git(&["switch", "-q", "main"]).unwrap();

        let moves = ref_moves(r).unwrap();
        let agent: Vec<(MoveKind, &str)> = moves
            .iter()
            .filter(|m| m.refname == "refs/heads/agent")
            .map(|m| (m.kind, m.detail.as_str()))
            .collect();
        assert_eq!(
            agent,
            [
                (MoveKind::Reset, "moving to HEAD~1"),
                (MoveKind::Amend, "two, amended"),
                (MoveKind::Commit, "two"),
                (MoveKind::Commit, "one"),
                (MoveKind::Created, "Created from HEAD"),
            ]
        );
        // Only the switches of HEAD: its commits are the branches' moves.
        let head: Vec<MoveKind> = moves
            .iter()
            .filter(|m| m.refname == "HEAD")
            .map(|m| m.kind)
            .collect();
        assert_eq!(head, [MoveKind::Switch, MoveKind::Switch]);
        let reset = &moves[moves
            .iter()
            .position(|m| m.kind == MoveKind::Reset)
            .unwrap()];
        assert!(reset.needs_counts());
        let old = reset.old.clone().unwrap();
        assert_eq!(old, amended);
        assert_eq!(move_counts(r, &old, &reset.new), Some((0, 1)));
        assert_eq!(
            moves
                .iter()
                .find(|m| m.kind == MoveKind::Amend)
                .unwrap()
                .counts,
            Some((1, 1))
        );

        // Restore the branch to before the reset. It is not checked out.
        let now = reset.new.clone();
        assert!(
            restore_branch(r, "refs/heads/agent", &old, &two, None).is_err(),
            "moved since"
        );
        restore_branch(r, "refs/heads/agent", &old, &now, None).unwrap();
        assert_eq!(r.git(&["rev-parse", "agent"]).unwrap().trim(), amended);
        let latest = ref_moves(r)
            .unwrap()
            .into_iter()
            .find(|m| m.refname == "refs/heads/agent")
            .unwrap();
        assert_eq!(latest.kind, MoveKind::Restore);

        // Checked out with an uncommitted change: the change stays.
        r.git(&["switch", "-q", "agent"]).unwrap();
        std::fs::write(r.root.join("a.txt"), "local\n").unwrap();
        restore_branch(r, "refs/heads/agent", &now, &amended, Some(&r.root)).unwrap();
        assert_eq!(r.git(&["rev-parse", "HEAD"]).unwrap().trim(), now);
        assert_eq!(
            std::fs::read_to_string(r.root.join("a.txt")).unwrap(),
            "local\n"
        );
        let latest = ref_moves(r)
            .unwrap()
            .into_iter()
            .find(|m| m.refname == "refs/heads/agent")
            .unwrap();
        assert_eq!(
            (latest.kind, latest.new.as_str()),
            (MoveKind::Restore, now.as_str())
        );
    }

    #[test]
    fn reflog_lines_and_kinds() {
        let line = "0000000000000000000000000000000000000000 abc Ben Kaspar <b@x.y> 1790792775 +0200\tbranch: Created from HEAD";
        let (old, new, time, msg) = parse_reflog_line(line).unwrap();
        assert_eq!((old, new.as_str(), time), (None, "abc", 1790792775));
        assert_eq!(msg, "branch: Created from HEAD");
        assert_eq!(
            MoveKind::of("rebase (finish): refs/heads/x onto abc"),
            MoveKind::Rebase
        );
        assert_eq!(
            MoveKind::of("pull --rebase (finish): refs/heads/x onto abc"),
            MoveKind::Rebase
        );
        assert_eq!(MoveKind::of("pull: Fast-forward"), MoveKind::Pull);
        assert_eq!(MoveKind::of("merge feature: Fast-forward"), MoveKind::Merge);
        assert_eq!(MoveKind::of("branch: Reset to main"), MoveKind::Reset);
        assert_eq!(MoveKind::of("update by push"), MoveKind::Push);
        assert_eq!(MoveKind::of("commit (initial): first"), MoveKind::Commit);
        assert_eq!(
            MoveKind::of("gibbon restore: moving to abc"),
            MoveKind::Restore
        );
    }

    #[test]
    fn worktree_list_parsing() {
        let text = "worktree /r\nHEAD 1111\nbranch refs/heads/main\n\n\
                    worktree /r-bare\nbare\n\n\
                    worktree /gone\nHEAD 2222\ndetached\nlocked why\nprunable gitdir file points to non-existent location\n";
        let list = parse_worktrees(text);
        assert_eq!(list.len(), 2, "the bare entry is left out");
        assert_eq!(list[0].branch_name(), Some("main"));
        assert!(list[0].main && !list[0].locked);
        assert_eq!(list[1].path, PathBuf::from("/gone"));
        assert_eq!(list[1].branch, None);
        assert!(!list[1].main && list[1].locked && list[1].prunable);
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
        assert_eq!(
            (l[2].kind, l[2].old_no, l[2].new_no),
            (LineKind::Del, Some(2), None)
        );
        assert_eq!(
            (l[3].kind, l[3].old_no, l[3].new_no),
            (LineKind::Add, None, Some(2))
        );
        assert_eq!((l[4].old_no, l[4].new_no), (Some(3), Some(3)));
    }
}
