//! GitHub pull requests through the `gh` CLI, which keeps its own sign-in.

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

fn gh(repo: &Repo, args: &[&str]) -> Result<String> {
    let out = Command::new("gh")
        .current_dir(&repo.root)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .context("The GitHub CLI (gh) is not installed. Install it with `brew install gh`.")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
            "number,title,author,headRefName,baseRefName,isDraft,url",
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
            })
        })
        .collect())
}

/// The remote that points at the pull requests' repository.
fn base_remote(repo: &Repo) -> Result<String> {
    let name = gh(
        repo,
        &["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"],
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
