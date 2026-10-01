//! GitHub pull requests through the `gh` CLI, which keeps its own sign-in.

use std::collections::HashMap;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use crate::git::{self, Repo};

#[derive(Clone, Debug)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
    pub url: String,
    /// The head branch is in a fork: a local branch of the same name is
    /// another branch.
    pub cross_repo: bool,
}

/// A pull request that is not open any more: merged, or closed without a
/// merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedPr {
    pub number: u64,
    /// The head branch.
    pub head: String,
    /// The head commit when it closed.
    pub head_oid: String,
    /// The branch it went into, or was meant for.
    pub base: String,
    pub merged: bool,
}

/// Where a pull request stands: its checks, its review and its merge.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrStatus {
    pub checks: Checks,
    pub review: Review,
    /// The head conflicts with the base branch.
    pub conflicts: bool,
    /// The base branch has commits that the head does not have, and the
    /// repository wants them before a merge.
    pub behind: bool,
}

/// The CI checks of a pull request's head commit. Skipped and neutral
/// checks do not count.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Checks {
    pub passed: usize,
    pub pending: usize,
    /// The names of the failed checks.
    pub failed: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Passed,
    Pending,
    Failed,
}

impl Checks {
    /// A failed check wins, then a running one. None without checks.
    pub fn state(&self) -> Option<CheckState> {
        if !self.failed.is_empty() {
            Some(CheckState::Failed)
        } else if self.pending > 0 {
            Some(CheckState::Pending)
        } else if self.passed > 0 {
            Some(CheckState::Passed)
        } else {
            None
        }
    }

    /// "1 failed (lint) · 5 passed", or "No checks".
    pub fn summary(&self) -> String {
        let mut parts = vec![];
        match self.failed.as_slice() {
            [] => {}
            [one] => parts.push(format!("1 failed ({one})")),
            many => parts.push(format!("{} failed ({})", many.len(), many.join(", "))),
        }
        if self.pending > 0 {
            parts.push(format!("{} running", self.pending));
        }
        if self.passed > 0 {
            parts.push(format!("{} passed", self.passed));
        }
        if parts.is_empty() {
            "No checks".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Review {
    /// The repository asks for no review.
    #[default]
    None,
    Required,
    Approved,
    ChangesRequested,
}

impl Review {
    pub fn text(self) -> Option<&'static str> {
        match self {
            Review::None => None,
            Review::Required => Some("Review required"),
            Review::Approved => Some("Approved"),
            Review::ChangesRequested => Some("Changes requested"),
        }
    }
}

impl PullRequest {
    /// Where the app keeps the fetched head of this pull request.
    pub fn refname(&self) -> String {
        format!("refs/gibbon/pr/{}", self.number)
    }
}

/// `refs/gibbon/pr/12` → 12.
pub fn number_of(refname: &str) -> Option<u64> {
    refname.strip_prefix("refs/gibbon/pr/")?.parse().ok()
}

pub const NOT_INSTALLED: &str =
    "The GitHub CLI (gh) is not installed. Install it with `brew install gh`.";

fn command(args: &[&str]) -> Command {
    let mut cmd = Command::new("gh");
    cmd.args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null());
    cmd
}

fn gh(repo: &Repo, args: &[&str]) -> Result<String> {
    let out = command(args)
        .current_dir(&repo.root)
        .output()
        .context(NOT_INSTALLED)?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A GitHub repository that the signed-in user can clone.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteRepo {
    /// `owner/name`.
    pub name: String,
    pub description: String,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    /// When someone last pushed to it, as a Unix time.
    pub pushed: i64,
}

/// Why `gh` lists no repositories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhError {
    Missing,
    SignedOut,
    Failed(String),
}

/// The signed-in user, and the last pushed 100 repositories that they own,
/// work on or see in their organizations.
pub fn your_repos() -> Result<(String, Vec<RemoteRepo>), GhError> {
    // Without `affiliations`, GitHub leaves out the repositories of the
    // organizations.
    const QUERY: &str = "query { viewer { login repositories(first: 100, \
        affiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER], \
        ownerAffiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER], \
        orderBy: {field: PUSHED_AT, direction: DESC}) { nodes { \
        nameWithOwner description isPrivate isFork isArchived pushedAt } } } }";
    let query = format!("query={QUERY}");
    let out = command(&["api", "graphql", "-f", &query])
        .output()
        .map_err(|_| GhError::Missing)?;
    match out.status.code() {
        Some(0) => {}
        // gh's exit code when no account is signed in.
        Some(4) => return Err(GhError::SignedOut),
        _ => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(GhError::Failed(err));
        }
    }
    parse_repos(&String::from_utf8_lossy(&out.stdout)).map_err(|e| GhError::Failed(e.to_string()))
}

fn parse_repos(json: &str) -> Result<(String, Vec<RemoteRepo>)> {
    let v: Value = serde_json::from_str(json)?;
    let viewer = &v["data"]["viewer"];
    let login = viewer["login"].as_str().unwrap_or("").to_string();
    let nodes = viewer["repositories"]["nodes"].as_array();
    let repos = nodes
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(RemoteRepo {
                name: r["nameWithOwner"].as_str()?.to_string(),
                description: r["description"].as_str().unwrap_or("").to_string(),
                private: r["isPrivate"].as_bool().unwrap_or(false),
                fork: r["isFork"].as_bool().unwrap_or(false),
                archived: r["isArchived"].as_bool().unwrap_or(false),
                pushed: r["pushedAt"]
                    .as_str()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map_or(0, |t| t.timestamp()),
            })
        })
        .collect();
    Ok((login, repos))
}

/// Open pull requests of the repository's GitHub project, newest first.
pub fn list(repo: &Repo) -> Result<Vec<PullRequest>> {
    let out = gh(
        repo,
        &[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,title,author,headRefName,baseRefName,isDraft,url,isCrossRepository",
        ],
    )?;
    let items: Vec<Value> = serde_json::from_str(&out)?;
    Ok(items
        .iter()
        .filter_map(|v| {
            Some(PullRequest {
                number: v["number"].as_u64()?,
                title: v["title"].as_str()?.to_string(),
                author: v["author"]["login"].as_str().unwrap_or("").to_string(),
                head: v["headRefName"].as_str().unwrap_or("").to_string(),
                base: v["baseRefName"].as_str().unwrap_or("").to_string(),
                draft: v["isDraft"].as_bool().unwrap_or(false),
                url: v["url"].as_str().unwrap_or("").to_string(),
                cross_repo: v["isCrossRepository"].as_bool().unwrap_or(false),
            })
        })
        .collect())
}

/// The newest `limit` pull requests that are not open, merged or not, or
/// with `head` only those of that branch. Pull requests from forks are left
/// out: their head branches are not branches of this repository.
pub fn closed(repo: &Repo, head: Option<&str>, limit: usize) -> Result<Vec<ClosedPr>> {
    let limit = limit.to_string();
    let mut args = vec![
        "pr",
        "list",
        "--state",
        // Merged ones too.
        "closed",
        "--limit",
        limit.as_str(),
        "--json",
        "number,state,headRefName,headRefOid,baseRefName,isCrossRepository",
    ];
    if let Some(head) = head {
        args.extend(["--head", head]);
    }
    parse_closed(&gh(repo, &args)?)
}

fn parse_closed(json: &str) -> Result<Vec<ClosedPr>> {
    let items: Vec<Value> = serde_json::from_str(json)?;
    Ok(items
        .iter()
        .filter(|v| !v["isCrossRepository"].as_bool().unwrap_or(false))
        .filter_map(|v| {
            Some(ClosedPr {
                number: v["number"].as_u64()?,
                head: v["headRefName"].as_str()?.to_string(),
                head_oid: v["headRefOid"].as_str()?.to_string(),
                base: v["baseRefName"].as_str().unwrap_or("").to_string(),
                merged: v["state"].as_str() == Some("MERGED"),
            })
        })
        .collect())
}

/// The commits of pull request `number`. GitHub lists 250 at most.
pub fn commit_oids(repo: &Repo, number: u64) -> Result<Vec<String>> {
    let number = number.to_string();
    let out = gh(
        repo,
        &[
            "pr",
            "view",
            number.as_str(),
            "--json",
            "commits",
            "--jq",
            ".commits[].oid",
        ],
    )?;
    Ok(out.lines().map(str::to_string).collect())
}

/// The status of the open pull requests, by number. A separate call from
/// `list`: GitHub takes seconds for the checks of many pull requests.
pub fn statuses(repo: &Repo) -> Result<HashMap<u64, PrStatus>> {
    let out = gh(
        repo,
        &[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup",
        ],
    )?;
    let items: Vec<Value> = serde_json::from_str(&out)?;
    Ok(items
        .iter()
        .filter_map(|v| Some((v["number"].as_u64()?, parse_status(v))))
        .collect())
}

/// How one check ended, in the order of how much it matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    Skipped,
    Passed,
    Pending,
    Failed,
}

/// A check run (GitHub Actions and apps) or a commit status (older CI).
fn check_of(v: &Value) -> Option<(String, Outcome)> {
    let s = |key: &str| v[key].as_str().unwrap_or("");
    match s("__typename") {
        "CheckRun" => {
            let outcome = if s("status") != "COMPLETED" {
                Outcome::Pending
            } else {
                match s("conclusion") {
                    "SUCCESS" => Outcome::Passed,
                    "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
                    | "STARTUP_FAILURE" => Outcome::Failed,
                    _ => Outcome::Skipped,
                }
            };
            // The same job can run for several events: one name per workflow.
            Some((format!("{}\0{}", s("workflowName"), s("name")), outcome))
        }
        "StatusContext" => {
            let outcome = match s("state") {
                "SUCCESS" => Outcome::Passed,
                "PENDING" | "EXPECTED" => Outcome::Pending,
                "FAILURE" | "ERROR" => Outcome::Failed,
                _ => Outcome::Skipped,
            };
            Some((format!("\0{}", s("context")), outcome))
        }
        _ => None,
    }
}

fn parse_status(v: &Value) -> PrStatus {
    // The worst outcome of each check, in the order GitHub lists them.
    let mut order: Vec<String> = vec![];
    let mut worst: HashMap<String, Outcome> = HashMap::new();
    for item in v["statusCheckRollup"].as_array().into_iter().flatten() {
        let Some((key, outcome)) = check_of(item) else {
            continue;
        };
        match worst.get_mut(&key) {
            Some(w) => *w = (*w).max(outcome),
            None => {
                order.push(key.clone());
                worst.insert(key, outcome);
            }
        }
    }
    let mut checks = Checks::default();
    for key in order {
        let name = key.split_once('\0').map_or(key.as_str(), |(_, n)| n);
        match worst[&key] {
            Outcome::Passed => checks.passed += 1,
            Outcome::Pending => checks.pending += 1,
            Outcome::Failed => checks.failed.push(name.to_string()),
            Outcome::Skipped => {}
        }
    }
    let review = match v["reviewDecision"].as_str().unwrap_or("") {
        "APPROVED" => Review::Approved,
        "CHANGES_REQUESTED" => Review::ChangesRequested,
        "REVIEW_REQUIRED" => Review::Required,
        _ => Review::None,
    };
    PrStatus {
        checks,
        review,
        conflicts: v["mergeable"].as_str() == Some("CONFLICTING"),
        behind: v["mergeStateStatus"].as_str() == Some("BEHIND"),
    }
}

/// The remote that points at the pull requests' repository.
fn base_remote(repo: &Repo) -> Result<String> {
    let name = gh(
        repo,
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ],
    )?;
    let name = name.trim().to_lowercase();
    let remotes = git::remotes(repo)?;
    remotes
        .iter()
        .find(|(_, url)| {
            let url = url.to_lowercase();
            url.ends_with(&format!("{name}.git")) || url.ends_with(&name)
        })
        .or_else(|| remotes.iter().find(|(n, _)| n == "origin"))
        .map(|(n, _)| n.clone())
        .with_context(|| format!("No remote points at {name}."))
}

/// Fetch the head of a pull request (from a fork too) into `pr.refname()`.
pub fn fetch(repo: &Repo, pr: &PullRequest) -> Result<()> {
    let remote = base_remote(repo)?;
    let spec = format!("+refs/pull/{}/head:{}", pr.number, pr.refname());
    git::fetch_refspec(repo, &remote, &spec)
}

pub fn checkout(repo: &Repo, number: u64) -> Result<String> {
    gh(repo, &["pr", "checkout", &number.to_string()])
}

/// Push the branch if it has no upstream, then open GitHub's form.
pub fn create_in_browser(repo: &Repo, branch: &str) -> Result<String> {
    git::push_branch(repo, branch)?;
    gh(repo, &["pr", "create", "--web", "--head", branch])
}

pub fn open_url(url: &str) {
    let _ = Command::new("open").arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_counts_each_check_once() {
        let v: Value = serde_json::from_str(
            r#"{
              "number": 7,
              "reviewDecision": "CHANGES_REQUESTED",
              "mergeable": "CONFLICTING",
              "mergeStateStatus": "DIRTY",
              "statusCheckRollup": [
                {"__typename": "CheckRun", "name": "lint", "workflowName": "CI", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"__typename": "CheckRun", "name": "lint", "workflowName": "CI", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"__typename": "CheckRun", "name": "test", "workflowName": "CI", "status": "IN_PROGRESS", "conclusion": ""},
                {"__typename": "CheckRun", "name": "triage", "workflowName": "Bots", "status": "COMPLETED", "conclusion": "SKIPPED"},
                {"__typename": "CheckRun", "name": "triage", "workflowName": "Bots", "status": "COMPLETED", "conclusion": "SKIPPED"},
                {"__typename": "StatusContext", "context": "deploy/preview", "state": "SUCCESS"}
              ]
            }"#,
        )
        .unwrap();
        let s = parse_status(&v);
        assert_eq!(s.checks.failed, ["lint"]);
        assert_eq!((s.checks.pending, s.checks.passed), (1, 1));
        assert_eq!(s.checks.state(), Some(CheckState::Failed));
        assert_eq!(s.checks.summary(), "1 failed (lint) · 1 running · 1 passed");
        assert_eq!(s.review, Review::ChangesRequested);
        assert!(s.conflicts && !s.behind);
    }

    #[test]
    fn closed_pull_requests_without_forks() {
        let prs = parse_closed(
            r#"[
              {"number": 12, "state": "MERGED", "headRefName": "agent/a", "headRefOid": "aaa",
               "baseRefName": "dev", "isCrossRepository": false},
              {"number": 11, "state": "CLOSED", "headRefName": "agent/b", "headRefOid": "bbb",
               "baseRefName": "dev", "isCrossRepository": false},
              {"number": 10, "state": "MERGED", "headRefName": "agent/a", "headRefOid": "ccc",
               "baseRefName": "dev", "isCrossRepository": true}
            ]"#,
        )
        .unwrap();
        assert_eq!(prs.len(), 2, "the fork's pull request is left out");
        assert_eq!((prs[0].number, prs[0].merged), (12, true));
        assert_eq!((prs[1].head.as_str(), prs[1].merged), ("agent/b", false));
    }

    #[test]
    fn repositories_of_the_viewer() {
        let (login, repos) = parse_repos(
            r#"{"data": {"viewer": {"login": "biki", "repositories": {"nodes": [
              {"nameWithOwner": "biki/gibbon", "description": "A Git client", "isPrivate": false,
               "isFork": false, "isArchived": false, "pushedAt": "2026-10-01T07:20:47Z"},
              {"nameWithOwner": "acme/tools", "description": null, "isPrivate": true,
               "isFork": true, "isArchived": true, "pushedAt": null}
            ]}}}}"#,
        )
        .unwrap();
        assert_eq!(login, "biki");
        assert_eq!(repos.len(), 2);
        assert_eq!(
            (repos[0].name.as_str(), repos[0].pushed),
            ("biki/gibbon", 1_790_839_247)
        );
        assert_eq!(repos[1].description, "");
        assert!(repos[1].private && repos[1].fork && repos[1].archived);
    }

    #[test]
    fn status_without_checks_or_review() {
        let v: Value = serde_json::from_str(
            r#"{"number": 8, "reviewDecision": "", "mergeable": "MERGEABLE",
                "mergeStateStatus": "BEHIND", "statusCheckRollup": []}"#,
        )
        .unwrap();
        let s = parse_status(&v);
        assert_eq!(s.checks.state(), None);
        assert_eq!(s.checks.summary(), "No checks");
        assert_eq!(s.review, Review::None);
        assert!(!s.conflicts && s.behind);
    }
}
