//! Coding agents that run on this Mac, and the folders they run in: Claude
//! Code, Codex and others. The Agents view shows which worktree has one.
//!
//! `ps` lists the processes and `lsof` gives their working folders. Both
//! come with macOS. An agent counts for the folder that its own process
//! runs in, not for the folders of the commands that it starts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A coding agent that Gibbon knows by the name of its program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Agent {
    Claude,
    Codex,
    Cursor,
    Gemini,
    OpenCode,
    Aider,
    Goose,
    Amp,
}

impl Agent {
    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Cursor => "Cursor Agent",
            Agent::Gemini => "Gemini CLI",
            Agent::OpenCode => "opencode",
            Agent::Aider => "Aider",
            Agent::Goose => "Goose",
            Agent::Amp => "Amp",
        }
    }

    /// The agent of a program, by the last part of its path.
    fn of_program(name: &str) -> Option<Agent> {
        Some(match name {
            "claude" => Agent::Claude,
            // The npm package starts `codex-aarch64-apple-darwin`.
            n if n == "codex" || n.starts_with("codex-") => Agent::Codex,
            "cursor-agent" => Agent::Cursor,
            "gemini" => Agent::Gemini,
            "opencode" => Agent::OpenCode,
            "aider" => Agent::Aider,
            "goose" => Agent::Goose,
            "amp" => Agent::Amp,
            _ => return None,
        })
    }
}

/// The agents of `agents` in words: "Claude Code", "Claude Code ×2",
/// "Claude Code and Codex".
pub fn names(agents: &[Agent]) -> String {
    let mut kinds: Vec<(Agent, usize)> = vec![];
    for &a in agents {
        match kinds.iter_mut().find(|(k, _)| *k == a) {
            Some((_, n)) => *n += 1,
            None => kinds.push((a, 1)),
        }
    }
    let words: Vec<String> = kinds
        .into_iter()
        .map(|(a, n)| match n {
            1 => a.name().to_string(),
            n => format!("{} ×{n}", a.name()),
        })
        .collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A program that runs the script after it: the script names the agent.
fn is_runtime(program: &str) -> bool {
    matches!(program, "node" | "bun" | "deno") || program.starts_with("python")
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The agent that a command line runs, from the words of `ps -o args`. A
/// program in a folder with a space in its name is not found.
fn agent_of<'a>(mut words: impl Iterator<Item = &'a str>) -> Option<Agent> {
    let program = base_name(words.next()?);
    if is_runtime(program) {
        // `node /opt/homebrew/bin/gemini --yolo`: the first word that is
        // not an option.
        let script = words.find(|w| !w.starts_with('-'))?;
        return Agent::of_program(base_name(script));
    }
    Agent::of_program(program)
}

/// An agent's process, from `ps`.
#[derive(Debug, PartialEq)]
struct Proc {
    pid: u32,
    ppid: u32,
    agent: Agent,
}

/// The agent processes in the output of `ps -o pid=,ppid=,args=`, without
/// the helpers that an agent starts from its own program, such as its MCP
/// server: those are the same agent.
fn parse_ps(out: &str) -> Vec<Proc> {
    let procs: Vec<Proc> = out
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid = words.next()?.parse().ok()?;
            let ppid = words.next()?.parse().ok()?;
            Some(Proc {
                pid,
                ppid,
                agent: agent_of(words)?,
            })
        })
        .collect();
    let by_pid: HashMap<u32, Agent> = procs.iter().map(|p| (p.pid, p.agent)).collect();
    procs
        .into_iter()
        .filter(|p| by_pid.get(&p.ppid) != Some(&p.agent))
        .collect()
}

/// The working folder of each process in the output of `lsof -Fpn`.
fn parse_cwds(out: &str) -> HashMap<u32, PathBuf> {
    let mut cwds = HashMap::new();
    let mut pid = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse().ok();
        } else if let (Some(path), Some(p)) = (line.strip_prefix('n'), pid) {
            cwds.insert(p, PathBuf::from(path));
        }
    }
    cwds
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    // `lsof` fails when one of the processes ended: the others still count.
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The agents that run now, each with its working folder. Empty when `ps`
/// or `lsof` cannot run.
pub fn running() -> Vec<(Agent, PathBuf)> {
    // Full paths: an app started from Finder has a short PATH.
    let Some(ps) = output("/bin/ps", &["-axww", "-o", "pid=,ppid=,args="]) else {
        return vec![];
    };
    let procs = parse_ps(&ps);
    if procs.is_empty() {
        return vec![];
    }
    let pids: Vec<String> = procs.iter().map(|p| p.pid.to_string()).collect();
    let pids = pids.join(",");
    let Some(lsof) = output(
        "/usr/sbin/lsof",
        &["-a", "-d", "cwd", "-p", pids.as_str(), "-Fpn"],
    ) else {
        return vec![];
    };
    let cwds = parse_cwds(&lsof);
    procs
        .into_iter()
        .filter_map(|p| Some((p.agent, cwds.get(&p.pid)?.clone())))
        .collect()
}

/// The agents of each worktree in `worktrees`. An agent belongs to the
/// deepest worktree that holds its folder: Claude Code puts its worktrees
/// inside the main one.
pub fn by_worktree(
    running: &[(Agent, PathBuf)],
    worktrees: &[&Path],
) -> HashMap<PathBuf, Vec<Agent>> {
    let mut map: HashMap<PathBuf, Vec<Agent>> = HashMap::new();
    for (agent, cwd) in running {
        let deepest = worktrees
            .iter()
            .filter(|w| cwd.starts_with(w))
            .max_by_key(|w| w.components().count());
        if let Some(w) = deepest {
            map.entry(w.to_path_buf()).or_default().push(*agent);
        }
    }
    for agents in map.values_mut() {
        agents.sort();
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(args: &str) -> Option<Agent> {
        agent_of(args.split_whitespace())
    }

    #[test]
    fn programs_and_scripts_name_the_agent() {
        assert_eq!(agent("claude"), Some(Agent::Claude));
        assert_eq!(
            agent("/Users/a/.local/bin/claude --resume"),
            Some(Agent::Claude)
        );
        assert_eq!(
            agent("/opt/node_modules/@openai/codex/vendor/codex-aarch64-apple-darwin exec"),
            Some(Agent::Codex)
        );
        assert_eq!(
            agent("node --no-warnings /opt/homebrew/bin/gemini -y"),
            Some(Agent::Gemini)
        );
        assert_eq!(
            agent("/usr/bin/python3.12 /Users/a/.local/bin/aider"),
            Some(Agent::Aider)
        );
        assert_eq!(agent("node /Users/a/app/server.js"), None);
        assert_eq!(agent("/bin/zsh -l"), None);
        // A program whose name only starts like an agent's.
        assert_eq!(agent("claudette"), None);
    }

    #[test]
    fn helpers_of_an_agent_are_left_out() {
        let ps = "\
              1     0 /sbin/launchd
            100     1 claude
            101   100 claude mcp serve
            102   100 /bin/zsh -c cargo test
            200     1 node /opt/homebrew/bin/codex
            201   200 /opt/codex/codex-aarch64-apple-darwin
            300   100 codex";
        assert_eq!(
            parse_ps(ps),
            [
                Proc {
                    pid: 100,
                    ppid: 1,
                    agent: Agent::Claude
                },
                Proc {
                    pid: 200,
                    ppid: 1,
                    agent: Agent::Codex
                },
                // Codex that Claude Code started is an agent of its own.
                Proc {
                    pid: 300,
                    ppid: 100,
                    agent: Agent::Codex
                },
            ]
        );
    }

    #[test]
    fn lsof_gives_each_folder() {
        let out = "p100\nfcwd\nn/Users/a/repo\np200\nfcwd\nn/private/tmp/x y\n";
        let cwds = parse_cwds(out);
        assert_eq!(cwds[&100], PathBuf::from("/Users/a/repo"));
        assert_eq!(cwds[&200], PathBuf::from("/private/tmp/x y"));
    }

    #[test]
    fn an_agent_belongs_to_the_deepest_worktree() {
        let main = Path::new("/r");
        let inner = Path::new("/r/.claude/worktrees/one");
        let other = Path::new("/r-other");
        let running = vec![
            (Agent::Claude, PathBuf::from("/r/src")),
            (Agent::Claude, PathBuf::from("/r/.claude/worktrees/one")),
            (Agent::Codex, PathBuf::from("/r/.claude/worktrees/one/src")),
            (Agent::Codex, PathBuf::from("/elsewhere")),
        ];
        let map = by_worktree(&running, &[main, inner, other]);
        assert_eq!(map[main], [Agent::Claude]);
        assert_eq!(map[inner], [Agent::Claude, Agent::Codex]);
        // `/r-other` does not hold `/r/src`: paths compare by part.
        assert!(!map.contains_key(other));
    }

    #[test]
    fn names_count_each_agent() {
        assert_eq!(names(&[]), "");
        assert_eq!(names(&[Agent::Claude]), "Claude Code");
        assert_eq!(names(&[Agent::Claude, Agent::Claude]), "Claude Code ×2");
        assert_eq!(
            names(&[Agent::Claude, Agent::Codex, Agent::Amp]),
            "Claude Code, Codex and Amp"
        );
    }
}
