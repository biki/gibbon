//! Updates from the GitHub releases of Gibbon.
//!
//! A check reads the latest release. When it is newer than the app on disk,
//! Gibbon downloads its `Gibbon.zip` next to the app and unpacks it there.
//! The new app must meet the designated requirement of the running app: the
//! same bundle id and the same signing certificate. Then the two apps swap
//! places, and the new version runs from the next start.
//!
//! `cargo run` and an ad hoc build do not update. The requirement of an ad
//! hoc signature is the hash of its own code, which no other build meets.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, bail};
use gpui_kit::{App, Global};
use serde_json::Value;

const LATEST: &str = "https://api.github.com/repos/biki/gibbon/releases/latest";
/// UI checks: the URL of a release in the JSON of the GitHub API, for
/// example a `file://` URL.
const RELEASES_ENV: &str = "GIBBON_RELEASES";
const ASSET: &str = "Gibbon.zip";
/// The first check waits, so the repositories load first.
const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(12 * 60 * 60);
/// How often the timer compares the clock with the last check. A Mac that
/// sleeps stops the timer, but not the clock.
const TICK: Duration = Duration::from_secs(60 * 60);

const VERSION: &str = env!("CARGO_PKG_VERSION");

const AD_HOC: &str = "This build has an ad hoc signature, so it cannot check an update. \
     Install a release with scripts/install.sh.";

/// What a check found.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// No newer release: the version on disk.
    Current(String),
    /// A newer version is on disk. It runs after a restart.
    Installed(String),
    Failed(String),
}

/// The state of the updates, which the windows show.
#[derive(Default)]
pub struct Updates {
    /// A check runs.
    pub busy: bool,
    pub last: Option<Outcome>,
    /// The user asked for the last check (Check for Updates). Its outcome
    /// shows also when nothing is new.
    pub asked: bool,
}

impl Global for Updates {}

/// Check 10 s after the start, then every 12 hours, while the setting is
/// on.
pub fn start(cx: &mut App) {
    cx.set_global(Updates::default());
    // UI checks stay off the network. `cargo run` and the dev loop do not
    // check: Check for Updates tells why.
    if (crate::background() && std::env::var_os(RELEASES_ENV).is_none()) || bundle().is_none() {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(FIRST_CHECK).await;
        let mut last: Option<SystemTime> = None;
        loop {
            let due = last.is_none_or(|t| t.elapsed().is_ok_and(|e| e >= CHECK_EVERY));
            if due && cx.read_global::<crate::settings::Settings, _>(|s, _| s.auto_update) {
                last = Some(SystemTime::now());
                cx.update(|cx| check(false, cx));
            }
            cx.background_executor().timer(TICK).await;
        }
    })
    .detach();
}

/// Check the latest release, and install it when it is newer. `asked`: the
/// user asked for this check.
pub fn check(asked: bool, cx: &mut App) {
    let updates = cx.global_mut::<Updates>();
    if updates.busy {
        // The check that runs shows its outcome then.
        updates.asked |= asked;
        return;
    }
    updates.busy = true;
    updates.asked = asked;
    cx.spawn(async move |cx| {
        let outcome = cx
            .background_executor()
            .spawn(async move { run().unwrap_or_else(|e| Outcome::Failed(format!("{e:#}"))) })
            .await;
        // A failed automatic check shows no toast.
        if let Outcome::Failed(e) = &outcome {
            eprintln!("update check: {e}");
        }
        cx.update_global::<Updates, _>(|u, _| {
            u.busy = false;
            u.last = Some(outcome);
        });
    })
    .detach();
}

fn run() -> Result<Outcome> {
    let app = bundle().context("Only Gibbon.app updates itself, not a build that cargo runs.")?;
    // macOS runs a quarantined app from a read-only copy until the user
    // moves it.
    if app
        .components()
        .any(|c| c.as_os_str() == "AppTranslocation")
    {
        bail!("Move Gibbon to the Applications folder to get updates.");
    }
    let requirement = requirement(&app)?;
    let on_disk = bundle_version(&app)?;
    let release = latest()?;
    if !newer(&release.version, &on_disk) {
        return Ok(if newer(&on_disk, VERSION) {
            Outcome::Installed(on_disk)
        } else {
            Outcome::Current(on_disk)
        });
    }
    install(&app, &release, &requirement, &on_disk).map(Outcome::Installed)
}

/// The app that runs: `…/Gibbon.app` of `…/Gibbon.app/Contents/MacOS/Gibbon`.
fn bundle() -> Option<PathBuf> {
    bundle_of(&std::env::current_exe().ok()?)
}

fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

#[derive(Debug, PartialEq)]
struct Release {
    /// `0.2.0` of the tag `v0.2.0`.
    version: String,
    /// The download of `Gibbon.zip`.
    url: String,
}

fn latest() -> Result<Release> {
    let url = std::env::var(RELEASES_ENV).unwrap_or_else(|_| LATEST.to_string());
    let json = run_cmd(
        curl()
            .args(["--max-time", "30"])
            .args(["--header", "Accept: application/vnd.github+json"])
            .arg(url),
    )
    .map_err(|e| {
        if e.to_string().contains("error: 404") {
            anyhow::anyhow!("Gibbon has no release on GitHub yet.")
        } else {
            e.context("Gibbon cannot read the latest release")
        }
    })?;
    parse_release(&json)
}

fn parse_release(json: &str) -> Result<Release> {
    let v: Value = serde_json::from_str(json).context("GitHub sent no release")?;
    let tag = v["tag_name"]
        .as_str()
        .context("The latest release has no tag")?;
    let url = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"] == ASSET)
        .and_then(|a| a["browser_download_url"].as_str())
        .with_context(|| format!("The release {tag} has no {ASSET}"))?;
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: url.to_string(),
    })
}

/// Download `release` next to `app`, check it, and swap it with `app`.
/// Returns the version of the new app.
fn install(app: &Path, release: &Release, requirement: &str, on_disk: &str) -> Result<String> {
    // Next to the app, so the swap stays on one volume.
    let dir = app.with_file_name(".Gibbon-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).with_context(|| {
        let parent = app.parent().unwrap_or(app);
        format!("Gibbon cannot write to {}", parent.display())
    })?;
    let result = (|| {
        let zip = dir.join(ASSET);
        run_cmd(
            curl()
                // Stop when the download stalls for a minute.
                .args(["--speed-limit", "1024", "--speed-time", "60"])
                .arg("--output")
                .arg(&zip)
                .arg(&release.url),
        )
        .context("The download failed")?;
        run_cmd(
            Command::new("/usr/bin/ditto")
                .arg("-x")
                .arg("-k")
                .arg(&zip)
                .arg(&dir),
        )?;
        let new = dir.join("Gibbon.app");
        verify(&new, requirement)?;
        // From the signed app, not from the tag: an old release with a new
        // tag does not go back a version.
        let version = bundle_version(&new)?;
        if !newer(&version, on_disk) {
            bail!("The download is Gibbon {version}, which is not newer than {on_disk}.");
        }
        swap(&new, app).with_context(|| format!("Gibbon cannot replace {}", app.display()))?;
        Ok(version)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// What another app must meet to replace `app`: its designated requirement.
fn requirement(app: &Path) -> Result<String> {
    let out = run_cmd(
        Command::new("/usr/bin/codesign")
            .args(["--display", "-r-"])
            .arg(app),
    )?;
    // codesign marks a requirement that it made up with `# `, as for an ad
    // hoc signature: `# designated => cdhash H"…"`.
    let requirement = out
        .lines()
        .find_map(|l| l.trim_start_matches("# ").strip_prefix("designated => "))
        .context("Gibbon.app has no designated requirement")?;
    if requirement.starts_with("cdhash") {
        bail!(AD_HOC);
    }
    Ok(requirement.to_string())
}

/// `app` meets `requirement`, and no file changed since the signature.
fn verify(app: &Path, requirement: &str) -> Result<()> {
    run_cmd(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg("-R")
            .arg(format!("={requirement}"))
            .arg(app),
    )
    .context("The download does not have the signature of Gibbon")?;
    Ok(())
}

/// `CFBundleShortVersionString` of the app at `app`.
fn bundle_version(app: &Path) -> Result<String> {
    let out = run_cmd(
        Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
            .arg(app.join("Contents/Info.plist")),
    )?;
    Ok(out.trim().to_string())
}

/// Swap the folders `new` and `app` in one step. On a volume that cannot
/// (not APFS), two renames do it, and a failed second rename undoes the
/// first.
fn swap(new: &Path, app: &Path) -> Result<()> {
    let c = |p: &Path| CString::new(p.as_os_str().as_bytes());
    let (from, to) = (c(new)?, c(app)?);
    // SAFETY: two NUL-terminated paths that live past the call.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } == 0 {
        return Ok(());
    }
    let old = new.with_extension("old");
    std::fs::rename(app, &old)?;
    if let Err(e) = std::fs::rename(new, app) {
        let _ = std::fs::rename(&old, app);
        return Err(e.into());
    }
    Ok(())
}

/// `a` is a later version than `b`, for example `0.10.0` after `0.9.1`.
/// False when one of them is not one to three numbers.
fn newer(a: &str, b: &str) -> bool {
    match (numbers(a), numbers(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn numbers(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.trim_start_matches('v').split('.');
    let mut out = [0; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        match parts.next() {
            Some(p) => *slot = p.parse().ok()?,
            None if i > 0 => break,
            None => return None,
        }
    }
    parts.next().is_none().then_some(out)
}

fn curl() -> Command {
    let mut cmd = Command::new("/usr/bin/curl");
    cmd.args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--connect-timeout", "15"])
        .arg("--user-agent")
        .arg(format!("Gibbon/{VERSION}"));
    cmd
}

/// The output of `cmd`, or its error text.
fn run_cmd(cmd: &mut Command) -> Result<String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("{name} did not start"))?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder in the temp folder that goes when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!("gibbon-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An app with an ad hoc signature: a copy of `true` and an Info.plist.
    fn ad_hoc_app(dir: &Path, version: &str) -> PathBuf {
        let app = dir.join("Test.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::copy("/usr/bin/true", app.join("Contents/MacOS/Test")).unwrap();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <plist version=\"1.0\"><dict>\
             <key>CFBundleIdentifier</key><string>dev.gibbon.Test</string>\
             <key>CFBundleExecutable</key><string>Test</string>\
             <key>CFBundleShortVersionString</key><string>{version}</string>\
             </dict></plist>"
        );
        std::fs::write(app.join("Contents/Info.plist"), plist).unwrap();
        run_cmd(
            Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(&app),
        )
        .unwrap();
        app
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.10.0", "0.9.1"));
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("1", "0.9.9"));
        assert!(!newer("0.2", "0.2.0"), "a missing number is 0");
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(!newer("0.3.0-beta", "0.2.0"), "not only numbers");
        assert!(!newer("1.2.3.4", "0.1.0"), "four numbers");
        assert!(!newer("", "0.1.0"));
    }

    #[test]
    fn the_bundle_holds_the_executable() {
        assert_eq!(
            bundle_of(Path::new("/Applications/Gibbon.app/Contents/MacOS/Gibbon")),
            Some(PathBuf::from("/Applications/Gibbon.app"))
        );
        assert_eq!(bundle_of(Path::new("/repo/target/debug/gibbon")), None);
    }

    #[test]
    fn a_release_names_its_zip() {
        let json = r#"{"tag_name": "v0.2.0", "assets": [
            {"name": "notes.txt", "browser_download_url": "https://example.com/notes.txt"},
            {"name": "Gibbon.zip", "browser_download_url": "https://example.com/Gibbon.zip"}
        ]}"#;
        assert_eq!(
            parse_release(json).unwrap(),
            Release {
                version: "0.2.0".into(),
                url: "https://example.com/Gibbon.zip".into(),
            }
        );
        let no_zip = r#"{"tag_name": "v0.2.0", "assets": []}"#;
        assert!(parse_release(no_zip).is_err());
        assert!(parse_release(r#"{"message": "Not Found"}"#).is_err());
    }

    #[test]
    fn swap_exchanges_two_folders() {
        let tmp = TempDir::new("update-swap");
        let (a, b) = (tmp.0.join("a"), tmp.0.join("b"));
        std::fs::create_dir_all(a.join("inner")).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("inner/file"), "new").unwrap();
        std::fs::write(b.join("file"), "old").unwrap();
        swap(&a, &b).unwrap();
        assert_eq!(
            std::fs::read_to_string(b.join("inner/file")).unwrap(),
            "new"
        );
        assert_eq!(std::fs::read_to_string(a.join("file")).unwrap(), "old");
    }

    #[test]
    fn an_ad_hoc_build_cannot_check_updates() {
        let tmp = TempDir::new("update-ad-hoc");
        let app = ad_hoc_app(&tmp.0, "0.4.2");
        assert_eq!(bundle_version(&app).unwrap(), "0.4.2");
        assert_eq!(requirement(&app).unwrap_err().to_string(), AD_HOC);
        // An ad hoc download does not meet the requirement of a release.
        let release = r#"identifier "dev.gibbon.Test" and certificate leaf = H"0123456789abcdef0123456789abcdef01234567""#;
        assert!(verify(&app, release).is_err());
        // An app meets its own requirement until a file changes.
        let out = run_cmd(
            Command::new("/usr/bin/codesign")
                .args(["--display", "-r-"])
                .arg(&app),
        )
        .unwrap();
        let own = out
            .lines()
            .find_map(|l| l.strip_prefix("# designated => "))
            .unwrap();
        assert!(verify(&app, own).is_ok());
        std::fs::write(app.join("Contents/Info.plist"), "changed").unwrap();
        assert!(verify(&app, own).is_err());
    }
}
