//! Development loop: rebuild Gibbon and restart it whenever a source file
//! changes. Run it with `scripts/dev.sh [repo]`.
//!
//! - Builds first, then restarts, so the old window stays up during a build.
//! - A failed build keeps the running app.
//! - Restarts do not take focus from your editor; Gibbon reopens on the same
//!   screen and at the same window place (see `src/session.rs`).
//! - Quitting Gibbon (⌘Q) ends the loop. After a crash it waits for a fix.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};

fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let args: Vec<String> = std::env::args().skip(1).collect();

    let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res
            && !matches!(event.kind, EventKind::Access(_))
        {
            let _ = tx.send(event.paths);
        }
    })?;
    watcher.watch(&root.join("src"), RecursiveMode::Recursive)?;
    watcher.watch(&root.join("assets"), RecursiveMode::Recursive)?;
    watcher.watch(&root.join("Cargo.toml"), RecursiveMode::NonRecursive)?;

    let mut app: Option<Child> = None;
    let mut first = true;
    loop {
        if build(&root) {
            if let Some(mut old) = app.take() {
                let _ = old.kill();
                let _ = old.wait();
            }
            app = Some(start(&root, &args, first)?);
            first = false;
        } else if app.is_some() {
            println!("✗ Build failed. The running app stays; save a fix to try again.");
        } else {
            println!("✗ Build failed. Save a fix to try again.");
        }

        // Wait for the next change; stop when the user quits Gibbon.
        loop {
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(paths) => {
                    if let Some(p) = paths.iter().find(|p| relevant(&root, p)) {
                        let shown = p.strip_prefix(&root).unwrap_or(p);
                        println!("↻ {}", shown.display());
                        settle(&rx);
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    let Some(child) = app.as_mut() else { continue };
                    if let Ok(Some(status)) = child.try_wait() {
                        if status.success() {
                            println!("Gibbon quit. Dev loop stopped.");
                            return Ok(());
                        }
                        println!("✗ Gibbon stopped ({status}). Save a fix to restart it.");
                        app = None;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }
}

/// Source files that change the app; not this runner, not editor temp files.
fn relevant(root: &Path, path: &Path) -> bool {
    if path.starts_with(root.join("src").join("bin")) {
        return false;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with('.') || name.ends_with('~') || name.ends_with(".swp") {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("rs" | "toml" | "ttf" | "json" | "png" | "svg")
    )
}

/// Editors write several events per save: let them arrive, then drop them.
fn settle(rx: &Receiver<Vec<PathBuf>>) {
    let until = Instant::now() + Duration::from_millis(200);
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        if rx.recv_timeout(left).is_err() {
            break;
        }
    }
}

fn build(root: &Path) -> bool {
    let t = Instant::now();
    let ok = Command::new("cargo")
        .args(["build", "--quiet", "--bin", "gibbon"])
        .current_dir(root)
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        println!("✓ Built in {:.1} s", t.elapsed().as_secs_f32());
    }
    ok
}

fn start(root: &Path, args: &[String], first: bool) -> anyhow::Result<Child> {
    let mut cmd = Command::new(root.join("target/debug/gibbon"));
    cmd.args(args);
    if !first {
        cmd.env("GIBBON_NO_ACTIVATE", "1");
    }
    Ok(cmd.spawn()?)
}
