//! Clone a repository: read what the user typed, then run `gh repo clone`
//! for a GitHub repository while `gh` is signed in, else `git clone`.

use std::io::Read as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, anyhow};
use futures::channel::mpsc;

/// A repository to clone: a URL, a local path or `owner/name` on GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// What the user typed, without the spaces around it.
    pub spec: String,
    /// `owner/name` when the repository is on github.com.
    pub github: Option<String>,
    /// What `git clone` gets.
    pub url: String,
    /// The folder name of the clone.
    pub name: String,
}

impl Source {
    /// The repository that `text` names, or None when `text` is no URL, no
    /// path and no `owner/name`.
    pub fn parse(text: &str) -> Option<Source> {
        let spec = text.trim();
        if spec.is_empty() || spec.contains(char::is_whitespace) {
            return None;
        }
        let (host, path) = if let Some((_, rest)) = spec.split_once("://") {
            let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
            (Some(host_name(host)), path.to_string())
        } else if spec.starts_with(['/', '~', '.']) {
            (None, spec.to_string())
        } else if let Some((host, path)) = spec.split_once(':')
            && !host.contains('/')
        {
            // The scp form, `git@github.com:owner/name.git`.
            (Some(host_name(host)), path.to_string())
        } else if is_shorthand(spec) {
            (Some("github.com".to_string()), spec.to_string())
        } else {
            return None;
        };
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let on_github = matches!(host.as_deref(), Some("github.com" | "www.github.com"));
        let github = match parts.as_slice() {
            [owner, name, ..] if on_github => Some(format!("{owner}/{}", strip_git(name))),
            _ => None,
        };
        let name = match &github {
            Some(full) => full.rsplit('/').next().unwrap_or_default().to_string(),
            None => strip_git(parts.last()?).to_string(),
        };
        if name.is_empty() || name == "." || name == ".." {
            return None;
        }
        // GitHub's web pages (`…/tree/main/src`) are no clone URLs, and
        // `owner/name` is none either. The SSH forms stay as they are.
        let ssh = spec.starts_with("ssh://") || !spec.contains("://");
        let url = match &github {
            Some(full) if !ssh || is_shorthand(spec) => format!("https://github.com/{full}.git"),
            _ => expand_home(spec).to_string_lossy().into_owned(),
        };
        Some(Source {
            spec: spec.to_string(),
            github,
            url,
            name,
        })
    }
}

/// `git@host:22` → `host`.
fn host_name(host: &str) -> String {
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split_once(':').map_or(host, |(h, _)| h);
    host.to_lowercase()
}

fn strip_git(name: &str) -> &str {
    name.strip_suffix(".git").unwrap_or(name)
}

/// `owner/name`, as GitHub spells them.
fn is_shorthand(text: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    matches!(text.split_once('/'), Some((owner, name)) if ok(owner) && ok(name))
}

/// `~/x` → `/Users/me/x`.
pub fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if path == "~" => dirs::home_dir().unwrap_or_default(),
        _ => PathBuf::from(path),
    }
}

/// `/Users/me/x` → `~/x`.
pub fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The folder that clones go into when the user chose none yet: a
/// developer folder in the home folder, else the home folder.
pub fn default_parent() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    [
        "Developer",
        "Projects",
        "Repositories",
        "Code",
        "src",
        "dev",
        "repos",
        "GitHub",
    ]
    .iter()
    .map(|d| home.join(d))
    .find(|d| d.is_dir())
    .unwrap_or(home)
}

/// What a clone reports while it runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// Git's last progress line, such as
    /// "Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s".
    pub line: String,
    /// How much of the whole clone is done, from 0 to 1.
    pub done: f32,
}

pub enum Event {
    Progress(Progress),
    /// The clone ended. A stopped clone ends with an error too.
    Done(Result<()>),
}

/// A running clone. A stop or a drop ends it: git then deletes the folder
/// that it made.
pub struct Job {
    pid: i32,
    ended: Arc<AtomicBool>,
}

impl Job {
    pub fn stop(&self) {
        if !self.ended.load(Ordering::SeqCst) {
            // The whole process group: `gh` does not pass a signal on to
            // its `git`. SAFETY: kill only sends a signal.
            unsafe { libc::kill(-self.pid, libc::SIGTERM) };
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Clone `source` into `dest`, through `gh` when `use_gh` and the
/// repository is on GitHub. `gh` gives git its sign-in for private
/// repositories, and adds the `upstream` remote to the clone of a fork.
pub fn start(
    source: &Source,
    dest: &Path,
    use_gh: bool,
) -> Result<(Job, mpsc::UnboundedReceiver<Event>)> {
    let dest = dest.to_string_lossy().into_owned();
    let mut cmd = if use_gh && let Some(github) = &source.github {
        // gh takes a URL or `owner/name`, without `.git`.
        let spec = if source.spec.contains(':') {
            &source.spec
        } else {
            github
        };
        let mut cmd = Command::new("gh");
        cmd.args(["repo", "clone", spec, &dest, "--", "--progress"])
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1");
        cmd
    } else {
        let mut cmd = Command::new("git");
        cmd.args(["clone", "--progress", "--", &source.url, &dest]);
        cmd
    };
    // Never wait for a password on a terminal nobody sees. English
    // messages, so that the progress lines parse.
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Its own process group, so that `stop` reaches git under gh too.
    cmd.process_group(0);
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().with_context(|| match program.as_str() {
        "gh" => crate::github::NOT_INSTALLED.into(),
        _ => format!("could not start {program}"),
    })?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let ended = Arc::new(AtomicBool::new(false));
    let job = Job {
        pid: child.id() as i32,
        ended: ended.clone(),
    };
    let (tx, rx) = mpsc::unbounded();
    std::thread::spawn(move || {
        let mut reader = Lines::default();
        let mut buf = [0u8; 4096];
        loop {
            match stderr.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    for p in reader.push(&buf[..n]) {
                        let _ = tx.unbounded_send(Event::Progress(p));
                    }
                }
            }
        }
        reader.finish();
        let status = child.wait();
        ended.store(true, Ordering::SeqCst);
        let result = match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(anyhow!(
                "{}",
                reader
                    .error()
                    .unwrap_or_else(|| format!("{program} clone failed ({s})"))
            )),
            Err(e) => Err(e.into()),
        };
        let _ = tx.unbounded_send(Event::Done(result));
    });
    Ok((job, rx))
}

/// Git's stderr cut into lines. Git ends a progress update with `\r` and
/// other lines with `\n`.
#[derive(Default)]
struct Lines {
    partial: Vec<u8>,
    done: f32,
    /// The lines that are no progress, for the error message.
    messages: Vec<String>,
}

impl Lines {
    /// Add output, and return the progress that it reports.
    fn push(&mut self, bytes: &[u8]) -> Vec<Progress> {
        let mut out = vec![];
        for &b in bytes {
            if b == b'\r' || b == b'\n' {
                let line = String::from_utf8_lossy(&std::mem::take(&mut self.partial)).into_owned();
                out.extend(self.line(&line));
            } else {
                self.partial.push(b);
            }
        }
        out
    }

    fn finish(&mut self) {
        let line = String::from_utf8_lossy(&std::mem::take(&mut self.partial)).into_owned();
        self.line(&line);
    }

    fn line(&mut self, line: &str) -> Option<Progress> {
        let line = line.trim_end();
        let short = line.strip_prefix("remote: ").unwrap_or(line).trim();
        if short.is_empty() || line.starts_with("Cloning into") {
            return None;
        }
        if let Some(done) = fraction(short) {
            self.done = self.done.max(done);
        } else if !is_status(short) {
            self.messages.push(line.to_string());
            return None;
        }
        Some(Progress {
            line: short.to_string(),
            done: self.done,
        })
    }

    /// Why the clone failed, in the words of git or gh.
    fn error(&self) -> Option<String> {
        (!self.messages.is_empty()).then(|| self.messages.join("\n"))
    }
}

/// A status line of git: "Enumerating objects: 13, done.", "Total 13
/// (delta 0), reused 0 (delta 0)", or the progress of another phase.
fn is_status(line: &str) -> bool {
    line.ends_with(", done.") || line.starts_with("Total ") || line.contains("% (")
}

/// How much of the whole clone a progress line says is done. Receiving the
/// objects takes the most time.
fn fraction(line: &str) -> Option<f32> {
    let (phase, rest) = line.split_once(':')?;
    let percent: f32 = rest.split_once('%')?.0.trim().parse().ok()?;
    let (start, span) = match phase {
        "Counting objects" => (0.0, 0.05),
        "Compressing objects" => (0.05, 0.05),
        "Receiving objects" => (0.10, 0.70),
        "Resolving deltas" => (0.80, 0.10),
        "Updating files" => (0.90, 0.10),
        _ => return None,
    };
    Some(start + span * percent.clamp(0., 100.) / 100.)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Option<String>, String, String) {
        let s = Source::parse(text).unwrap_or_else(|| panic!("{text} does not parse"));
        (s.github, s.url, s.name)
    }

    #[test]
    fn github_repositories_in_all_spellings() {
        let gibbon = |url: &str| {
            (
                Some("biki/gibbon".to_string()),
                url.to_string(),
                "gibbon".to_string(),
            )
        };
        let https = "https://github.com/biki/gibbon.git";
        assert_eq!(parse("biki/gibbon"), gibbon(https));
        assert_eq!(parse(" https://github.com/biki/gibbon "), gibbon(https));
        assert_eq!(parse("https://github.com/biki/gibbon.git"), gibbon(https));
        assert_eq!(
            parse("https://github.com/biki/gibbon/tree/main/src"),
            gibbon(https)
        );
        assert_eq!(
            parse("git@github.com:biki/gibbon.git"),
            gibbon("git@github.com:biki/gibbon.git")
        );
        assert_eq!(
            parse("ssh://git@github.com/biki/gibbon"),
            gibbon("ssh://git@github.com/biki/gibbon")
        );
    }

    #[test]
    fn other_hosts_and_paths() {
        assert_eq!(
            parse("https://gitlab.com/group/sub/project.git"),
            (
                None,
                "https://gitlab.com/group/sub/project.git".into(),
                "project".into()
            )
        );
        assert_eq!(
            parse("git@example.com:tools.git"),
            (None, "git@example.com:tools.git".into(), "tools".into())
        );
        assert_eq!(parse("/srv/git/app.git/").2, "app");
        assert!(parse("~/src/app").1.ends_with("/src/app"));
    }

    #[test]
    fn a_filter_is_no_source() {
        for text in ["", "gibbon", "two words", "a/b/c", "https://github.com/"] {
            assert_eq!(Source::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn progress_lines_and_errors() {
        let mut lines = Lines::default();
        let out = lines.push(
            b"Cloning into 'x'...\nremote: Enumerating objects: 13, done.        \n\
              Receiving objects:  50% (6/13)\rReceiving objects: 100% (13/13), done.\n\
              Resolving deltas:  50% (1/2)\r",
        );
        let shown: Vec<(&str, i32)> = out
            .iter()
            .map(|p| (p.line.as_str(), (p.done * 100.).round() as i32))
            .collect();
        assert_eq!(
            shown,
            [
                ("Enumerating objects: 13, done.", 0),
                ("Receiving objects:  50% (6/13)", 45),
                ("Receiving objects: 100% (13/13), done.", 80),
                ("Resolving deltas:  50% (1/2)", 85),
            ]
        );
        assert_eq!(lines.error(), None);
        lines.push(b"remote: Repository not found.\nfatal: repository 'x' not found");
        lines.finish();
        assert_eq!(
            lines.error().as_deref(),
            Some("remote: Repository not found.\nfatal: repository 'x' not found")
        );
    }
}
