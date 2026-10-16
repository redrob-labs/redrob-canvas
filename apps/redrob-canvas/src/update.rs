//! The in-app update check (desktop only).
//!
//! Three seconds after the first frame, off the UI thread, the latest release is read from
//! `https://api.github.com/repos/redrob-labs/redrob-canvas/releases/latest` and its tag compared
//! with `CARGO_PKG_VERSION`. A newer one shows "Update to vX.Y.Z" in the top bar's right slot;
//! clicking downloads this platform's asset (named exactly as `.github/workflows/release.yml`
//! publishes it: `redrob-canvas-<v>-macos-arm64.dmg`, `redrob-canvas-<v>-linux-x86_64.tar.gz`,
//! `redrob-canvas-<v>-windows-x64-setup.exe`) into the OS temp directory, checks its SHA-256
//! against the release's `SHA256SUMS.txt`, and offers "Install and restart", which opens the file
//! with the platform opener (`open` / the setup itself / `xdg-open`) and closes the window.
//!
//! Switched off by `REDROB_CANVAS_NO_UPDATE_CHECK=1` or the "Check for updates automatically"
//! preference (Help menu, `<config dir>/update.json`, which also records the last check: the
//! automatic one runs at most once a day); Help > "Check for updates…" always runs.
//!
//! A failed check is not an alarm: the top bar shows a muted "Update check failed" whose tooltip
//! is the full reason (every cause in the error chain -- `reqwest`'s own message stops at "error
//! sending request"), and the GitHub API's 403 / 429 (its unauthenticated quota is 60 requests an
//! hour per address, shared with every other app on the network) shows nothing until the next
//! day; the check then reads the tag out of the `releases/latest` redirect instead, which has no
//! quota. The client trusts the OS certificate store as well as the bundled roots and uses the OS
//! proxy settings (root `Cargo.toml`), the two things a Windows desktop behind a corporate proxy
//! needs.
//! Under Redrob Launcher the launcher owns updates and the menu says so instead. The launcher
//! records every install in its `installed.json` (`<data-local>/redrob-launcher/installed.json`,
//! `apps["redrob-canvas"].path` -- redrob-launcher `crates/native/src/{manifest,paths}.rs`); this
//! copy is launcher-managed when its executable is that path or lives under it. `REDROB_LAUNCHER=1`
//! forces the same answer for tests and wrappers.
//!
//! Everything lives behind one process-wide [`Updater`] (`OnceLock`) with a `tokio::sync::watch`
//! channel from the worker thread to the UI, so the shell only calls [`menu_items`] and
//! [`top_bar`].
//! The HTTP client is the workspace's async `reqwest` on a current-thread tokio runtime, the way
//! `redrob_ai::agent` spins its own.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::watch;

use crate::services;

/// The repository whose releases are checked.
pub const REPO: &str = "redrob-labs/redrob-canvas";
/// GitHub's "latest release" endpoint for [`REPO`].
pub const LATEST_URL: &str =
    "https://api.github.com/repos/redrob-labs/redrob-canvas/releases/latest";
/// The checksum file the shared redrob-labs publish job writes over every asset.
pub const SUMS_NAME: &str = "SHA256SUMS.txt";
/// `REDROB_CANVAS_NO_UPDATE_CHECK=1`: no automatic check at start.
pub const NO_CHECK_ENV: &str = "REDROB_CANVAS_NO_UPDATE_CHECK";
/// `REDROB_LAUNCHER=1`: started by Redrob Launcher, which owns updates.
pub const LAUNCHER_ENV: &str = "REDROB_LAUNCHER";
/// The launcher's record of what it installed, under its per-user data-local directory.
pub const LAUNCHER_MANIFEST: &str = "redrob-launcher/installed.json";
/// Our id in that record (the launcher's catalog slug for Redrob Canvas).
pub const LAUNCHER_APP_ID: &str = "redrob-canvas";
/// The preference file under the config directory: `{ "auto_check": true, "last_check": <unix s> }`.
pub const PREFS_FILE: &str = "update.json";
/// How long after the first frame the automatic check runs.
pub const START_DELAY: Duration = Duration::from_secs(3);
/// The automatic check runs at most this often.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The release page GitHub redirects to the latest tag: the quota-free way to learn its name.
pub const LATEST_PAGE_URL: &str = "https://github.com/redrob-labs/redrob-canvas/releases/latest";
/// What the menu says under the launcher.
pub const MANAGED_TEXT: &str = "Updates are managed by Redrob Launcher";

/// Seconds since the Unix epoch, now.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Every message in `e`'s cause chain, outermost first: `reqwest`'s Display stops at "error
/// sending request for url", and the reason people need (`invalid peer certificate:
/// UnknownIssuer`, `dns error`, `connection refused`) is two causes down.
pub fn describe(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string()];
    let mut cause = e.source();
    while let Some(c) = cause {
        let text = c.to_string();
        if !parts.iter().any(|p| p.contains(&text)) {
            parts.push(text);
        }
        cause = c.source();
    }
    parts.join(": ")
}

/// `User-Agent` for GitHub (it refuses requests without one).
fn user_agent() -> String {
    format!("redrob-canvas/{}", env!("CARGO_PKG_VERSION"))
}

// ---------------------------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------------------------

/// `MAJOR.MINOR.PATCH`, with a pre-release flag: `1.2.3-rc.1` is older than `1.2.3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: bool,
}

impl Version {
    /// Parses `vX.Y.Z`, `X.Y.Z` or `X.Y.Z-<suffix>`; anything else is `None`.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let (numbers, suffix) = match text.split_once('-') {
            Some((n, s)) => (n, Some(s)),
            None => (text, None),
        };
        if suffix.is_some_and(str::is_empty) {
            return None;
        }
        let mut parts = numbers.split('.');
        let mut next = || {
            parts
                .next()
                .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|p| p.parse::<u64>().ok())
        };
        let (major, minor, patch) = (next()?, next()?, next()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
            prerelease: suffix.is_some(),
        })
    }

    /// The running binary's version.
    pub fn current() -> Option<Version> {
        Version::parse(env!("CARGO_PKG_VERSION"))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            // A release outranks a pre-release of the same number.
            .then_with(|| other.prerelease.cmp(&self.prerelease))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if self.prerelease {
            f.write_str("-pre")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Assets
// ---------------------------------------------------------------------------------------------

/// The asset `release.yml` publishes for `os` / `arch` (`std::env::consts` names) at `version`
/// (the tag without `v`): the file people install from. `None` for a platform the release does
/// not build (Intel macOS, 32-bit Windows, anything else).
pub fn asset_name_for(version: &str, os: &str, arch: &str) -> Option<String> {
    let suffix = match (os, arch) {
        ("macos", "aarch64") => "macos-arm64.dmg",
        ("linux", "x86_64") => "linux-x86_64.tar.gz",
        ("windows", "x86_64") => "windows-x64-setup.exe",
        _ => return None,
    };
    Some(format!("redrob-canvas-{version}-{suffix}"))
}

/// This machine's asset name at `version`.
pub fn asset_name(version: &str) -> Option<String> {
    asset_name_for(version, std::env::consts::OS, std::env::consts::ARCH)
}

/// `sha256sum` output: one `<hex>  <name>` per line (`*name` for binary mode). Blank lines and
/// malformed ones are skipped; names are returned as written.
pub fn parse_sha256sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end_matches(['\r', '\n']);
            let (hash, rest) = line.split_once(char::is_whitespace)?;
            let hash = hash.trim();
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let name = rest
                .trim_start()
                .strip_prefix('*')
                .unwrap_or(rest.trim_start());
            if name.is_empty() {
                return None;
            }
            Some((hash.to_ascii_lowercase(), name.to_owned()))
        })
        .collect()
}

/// The lower-case SHA-256 `sums` records for `name`.
pub fn expected_sha256(sums: &str, name: &str) -> Option<String> {
    parse_sha256sums(sums)
        .into_iter()
        .find(|(_, n)| n == name)
        .map(|(hash, _)| hash)
}

/// Lower-case hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        // Writing to a String cannot fail.
        let _ = write!(out, "{b:02x}");
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Policy: who updates this install
// ---------------------------------------------------------------------------------------------

/// Whether this process may update itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// The in-app check runs (automatically unless opted out, and from the Help menu).
    InApp,
    /// Redrob Launcher installed this copy and owns its updates.
    LauncherManaged,
}

/// The launcher's per-user data-local directory, as `dirs::data_local_dir()` resolves it in the
/// launcher: `%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on macOS, else
/// `$XDG_DATA_HOME` or `~/.local/share`.
pub fn launcher_manifest_path_with(
    os: &str,
    env: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    let base = match os {
        "windows" => env("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        "macos" => home.map(|h| h.join("Library/Application Support")),
        _ => env("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local/share"))),
    };
    base.map(|b| b.join(LAUNCHER_MANIFEST))
}

/// Whether `exe` is the path the launcher recorded for us (or lives under it, for tree installs
/// and app bundles). `manifest` is the text of the launcher's `installed.json`.
pub fn recorded_by_launcher(manifest: &str, exe: &Path) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(manifest) else {
        return false;
    };
    let Some(recorded) = value
        .get("apps")
        .and_then(|a| a.get(LAUNCHER_APP_ID))
        .and_then(|r| r.get("path"))
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    let recorded = Path::new(recorded);
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    same(exe, recorded) || exe.ancestors().skip(1).any(|dir| same(dir, recorded))
}

/// Launcher-managed when the launcher's `installed.json` records this executable, or the launcher
/// set `REDROB_LAUNCHER=1`.
pub fn launcher_managed_with(
    exe: Option<&Path>,
    manifest: Option<&str>,
    env: impl Fn(&str) -> Option<OsString>,
) -> bool {
    if env(LAUNCHER_ENV).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")) {
        return true;
    }
    match (exe, manifest) {
        (Some(exe), Some(manifest)) => recorded_by_launcher(manifest, exe),
        _ => false,
    }
}

fn launcher_managed() -> bool {
    let exe = std::env::current_exe().ok();
    let manifest = launcher_manifest_path_with(std::env::consts::OS, |key| std::env::var_os(key))
        .and_then(|p| std::fs::read_to_string(p).ok());
    launcher_managed_with(exe.as_deref(), manifest.as_deref(), |key| {
        std::env::var_os(key)
    })
}

/// `REDROB_CANVAS_NO_UPDATE_CHECK=1` (or anything but `0` / empty).
pub fn opted_out_with(env: impl Fn(&str) -> Option<OsString>) -> bool {
    env(NO_CHECK_ENV).is_some_and(|v| !v.is_empty() && v != "0")
}

// ---------------------------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------------------------

fn prefs_path() -> Option<PathBuf> {
    services::config_dir()
        .filter(|_| services::prefs_enabled())
        .map(|d| d.join(PREFS_FILE))
}

/// What `update.json` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// The automatic check at start.
    pub auto_check: bool,
    /// When the last check finished (Unix seconds); `None` for never.
    pub last_check: Option<u64>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            auto_check: true,
            last_check: None,
        }
    }
}

impl Prefs {
    /// Whether the automatic check is due at `now`: never checked, or [`CHECK_INTERVAL`] ago.
    pub fn due(&self, now: u64) -> bool {
        self.last_check
            .is_none_or(|last| now.saturating_sub(last) >= CHECK_INTERVAL.as_secs())
    }
}

/// `update.json`: `auto_check` (`true` when missing) and `last_check`.
pub fn parse_prefs(text: &str) -> Prefs {
    let value = serde_json::from_str::<serde_json::Value>(text).unwrap_or_default();
    Prefs {
        auto_check: value
            .get("auto_check")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        last_check: value.get("last_check").and_then(serde_json::Value::as_u64),
    }
}

/// The `auto_check` flag from `update.json`; `true` when there is no file.
#[cfg(test)]
pub fn parse_auto_check(text: &str) -> bool {
    parse_prefs(text).auto_check
}

fn load_prefs() -> Prefs {
    prefs_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_prefs(&t))
        .unwrap_or_default()
}

fn save_prefs(prefs: Prefs) {
    let Some(path) = prefs_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut value = serde_json::json!({ "auto_check": prefs.auto_check });
    if let Some(last) = prefs.last_check {
        value["last_check"] = serde_json::json!(last);
    }
    let text = value.to_string() + "\n";
    if let Err(e) = std::fs::write(&path, text) {
        log::warn!("couldn't save {}: {e}", path.display());
    }
}

// ---------------------------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------------------------

/// Where the updater is, as the worker thread reports it.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    /// A newer release: its version, this platform's asset URL, the release page.
    Available {
        version: Version,
        url: String,
        sums_url: String,
        release_url: String,
    },
    /// Download in progress, percent of the asset (0 when the size is unknown).
    Downloading(u8),
    /// Downloaded and its SHA-256 verified.
    Ready(PathBuf),
    /// Something went wrong; the reason is shown on hover and in the Help menu.
    Failed(String),
    /// GitHub's API refused (403 / 429: its quota) and the quota-free fallback could not answer
    /// either: nothing is shown until the next day's check.
    RateLimited(String),
}

impl Status {
    /// The newer version, if one was found (Available, Downloading or Ready).
    fn version(&self) -> Option<Version> {
        match self {
            Status::Available { version, .. } => Some(*version),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The updater
// ---------------------------------------------------------------------------------------------

/// The process-wide updater: policy, preference, and the status channel.
pub struct Updater {
    policy: Policy,
    prefs: Mutex<Prefs>,
    started: AtomicBool,
    tx: watch::Sender<Status>,
    rx: watch::Receiver<Status>,
    /// The version the current Downloading / Ready state is for (the status carries the path).
    target: Mutex<Option<(Version, String)>>,
}

static UPDATER: OnceLock<Updater> = OnceLock::new();

impl Updater {
    fn global() -> &'static Updater {
        UPDATER.get_or_init(|| {
            let (tx, rx) = watch::channel(Status::Idle);
            Updater {
                policy: if launcher_managed() {
                    Policy::LauncherManaged
                } else {
                    Policy::InApp
                },
                prefs: Mutex::new(load_prefs()),
                started: AtomicBool::new(false),
                tx,
                rx,
                target: Mutex::new(None),
            }
        })
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    pub fn status(&self) -> Status {
        self.rx.borrow().clone()
    }

    fn set(&self, status: Status) {
        let _ = self.tx.send(status);
    }

    fn prefs(&self) -> Prefs {
        self.prefs.lock().map(|p| *p).unwrap_or_default()
    }

    /// Changes the preferences and writes them.
    fn update_prefs(&self, f: impl FnOnce(&mut Prefs)) {
        let Ok(mut prefs) = self.prefs.lock() else {
            return;
        };
        f(&mut prefs);
        save_prefs(*prefs);
    }

    /// The automatic check, once: after [`START_DELAY`], unless opted out, launcher-managed, or
    /// checked within the last [`CHECK_INTERVAL`].
    fn ensure_started(&'static self) {
        if self.started.swap(true, AtomicOrdering::SeqCst) {
            return;
        }
        let prefs = self.prefs();
        if self.policy != Policy::InApp
            || opted_out_with(|key| std::env::var_os(key))
            || !prefs.auto_check
        {
            return;
        }
        if !prefs.due(now_secs()) {
            log::info!("update check: done within the last day, skipped");
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("update check".into())
            .spawn(move || {
                std::thread::sleep(START_DELAY);
                self.run_check();
            });
        if let Err(e) = spawned {
            log::warn!("couldn't start the update check: {e}");
        }
    }

    /// Help > Check for updates…: now, on a thread of its own.
    fn check_now(&'static self) {
        if matches!(self.status(), Status::Checking | Status::Downloading(_)) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("update check".into())
            .spawn(move || self.run_check());
        if let Err(e) = spawned {
            self.set(Status::Failed(format!("couldn't start the check: {e}")));
        }
    }

    fn run_check(&self) {
        self.set(Status::Checking);
        let outcome = with_runtime(|rt| rt.block_on(fetch_latest()));
        let status = match outcome {
            Ok(status) => status,
            Err(CheckError::Quota(e)) => {
                log::warn!("update check: {e}");
                Status::RateLimited(e)
            }
            Err(CheckError::Other(e)) => {
                log::warn!("update check: {e}");
                Status::Failed(e)
            }
        };
        // A finished check, whatever its outcome, is this day's: a failing network or an exhausted
        // quota is not retried at every start.
        self.update_prefs(|p| p.last_check = Some(now_secs()));
        self.set(status);
    }

    /// "Update to vX.Y.Z": download and verify, then Ready.
    fn download(&'static self) {
        let Status::Available {
            version,
            url,
            sums_url,
            ..
        } = self.status()
        else {
            return;
        };
        let Some(name) = asset_name(&version.to_string()) else {
            self.set(Status::Failed("no release asset for this platform".into()));
            return;
        };
        if let Ok(mut t) = self.target.lock() {
            *t = Some((version, name.clone()));
        }
        let spawned = std::thread::Builder::new()
            .name("update download".into())
            .spawn(move || {
                self.set(Status::Downloading(0));
                let outcome = with_runtime(|rt| {
                    rt.block_on(download_verified(&url, &sums_url, &name, |pct| {
                        self.set(Status::Downloading(pct));
                    }))
                });
                match outcome {
                    Ok(path) => self.set(Status::Ready(path)),
                    Err(e) => {
                        log::warn!("update download: {e}");
                        self.set(Status::Failed(e));
                    }
                }
            });
        if let Err(e) = spawned {
            self.set(Status::Failed(format!("couldn't start the download: {e}")));
        }
    }

    /// The version a Downloading / Ready status is for.
    fn target_version(&self) -> Option<Version> {
        self.target
            .lock()
            .ok()
            .and_then(|t| t.as_ref().map(|(v, _)| *v))
    }
}

/// Runs `f` with a current-thread tokio runtime (built here, on the calling worker thread).
fn with_runtime<T, E: From<String>>(
    f: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, E>,
) -> Result<T, E> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| E::from(format!("tokio runtime: {e}")))?;
    f(&rt)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(60))
        // The `releases/latest` probe reads the redirect itself; API and download URLs are final
        // (release assets redirect once, to the object store, and that one is followed).
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("HTTP client: {}", describe(&e)))
}

/// Why a check did not produce an answer.
#[derive(Debug, PartialEq, Eq)]
enum CheckError {
    /// GitHub refused for its quota (403 / 429) and the fallback did not help: stay quiet.
    Quota(String),
    /// Anything else: shown muted, with the reason on hover.
    Other(String),
}

impl From<String> for CheckError {
    fn from(e: String) -> Self {
        CheckError::Other(e)
    }
}

/// Reads the latest release and decides between UpToDate and Available. The API answers with
/// the asset list; when it refuses for its quota, the `releases/latest` redirect gives the tag
/// and the asset URLs follow from the release's naming (`release.yml`).
async fn fetch_latest() -> Result<Status, CheckError> {
    let client = client()?;
    let response = client
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| CheckError::Other(format!("GitHub: {}", describe(&e))))?;
    let status = response.status();
    if status.is_success() {
        let release: serde_json::Value = response
            .json()
            .await
            .map_err(|e| CheckError::Other(format!("GitHub: invalid JSON: {}", describe(&e))))?;
        return decide(&release, Version::current()).map_err(CheckError::Other);
    }
    let answered = format!("GitHub API answered {status}");
    if !is_quota(status.as_u16()) {
        return Err(CheckError::Other(answered));
    }
    // The quota: read the tag out of the redirect instead (no API, no quota).
    match latest_tag_from_redirect().await {
        Ok(tag) => decide(&release_from_tag(&tag), Version::current()).map_err(CheckError::Other),
        Err(e) => Err(CheckError::Quota(format!("{answered}; {e}"))),
    }
}

/// GitHub's quota answers: 403 (unauthenticated rate limit) and 429 (secondary rate limit).
pub fn is_quota(status: u16) -> bool {
    matches!(status, 403 | 429)
}

/// The tag `releases/latest` redirects to (`.../releases/tag/v0.8.1` -> `v0.8.1`), read without
/// following the redirect.
async fn latest_tag_from_redirect() -> Result<String, String> {
    let probe = reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("HTTP client: {}", describe(&e)))?;
    let response = probe
        .head(LATEST_PAGE_URL)
        .send()
        .await
        .map_err(|e| format!("releases page: {}", describe(&e)))?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            format!(
                "releases page answered {} without a redirect",
                response.status()
            )
        })?;
    tag_from_release_url(location)
        .ok_or_else(|| format!("releases page redirected to {location}, not a tag"))
}

/// `v0.8.1` out of `https://github.com/<repo>/releases/tag/v0.8.1` (or the path alone).
pub fn tag_from_release_url(url: &str) -> Option<String> {
    let (_, tag) = url.rsplit_once("/releases/tag/")?;
    let tag = tag
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    (!tag.is_empty() && Version::parse(tag).is_some()).then(|| tag.to_owned())
}

/// A release object like the API's, built from its tag alone: `release.yml` names every asset
/// from the version, and GitHub serves them at `releases/download/<tag>/<name>`.
pub fn release_from_tag(tag: &str) -> serde_json::Value {
    let plain = tag.strip_prefix('v').unwrap_or(tag);
    let download = |name: &str| format!("https://github.com/{REPO}/releases/download/{tag}/{name}");
    let mut assets = vec![serde_json::json!({
        "name": SUMS_NAME,
        "browser_download_url": download(SUMS_NAME),
    })];
    for (os, arch) in [
        ("macos", "aarch64"),
        ("linux", "x86_64"),
        ("windows", "x86_64"),
    ] {
        if let Some(name) = asset_name_for(plain, os, arch) {
            assets.push(serde_json::json!({
                "name": name,
                "browser_download_url": download(&name),
            }));
        }
    }
    serde_json::json!({
        "tag_name": tag,
        "html_url": format!("https://github.com/{REPO}/releases/tag/{tag}"),
        "assets": assets,
    })
}

/// [`Status`] for a GitHub release object (`tag_name`, `assets[]`, `html_url`) against `current`.
pub fn decide(release: &serde_json::Value, current: Option<Version>) -> Result<Status, String> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or("GitHub: release has no tag_name")?;
    let latest = Version::parse(tag).ok_or_else(|| format!("GitHub: tag {tag:?} is not vX.Y.Z"))?;
    let current = current.ok_or("this build has no parsable version")?;
    if latest <= current {
        return Ok(Status::UpToDate);
    }
    let plain = tag.strip_prefix('v').unwrap_or(tag);
    let name = asset_name(plain).ok_or("no release asset for this platform")?;
    let assets = release["assets"]
        .as_array()
        .ok_or("GitHub: release has no assets")?;
    let download_url = |wanted: &str| {
        assets.iter().find_map(|a| {
            (a["name"].as_str() == Some(wanted))
                .then(|| a["browser_download_url"].as_str().map(str::to_owned))
                .flatten()
        })
    };
    let url = download_url(&name).ok_or_else(|| format!("release {tag} has no {name}"))?;
    let sums_url =
        download_url(SUMS_NAME).ok_or_else(|| format!("release {tag} has no {SUMS_NAME}"))?;
    Ok(Status::Available {
        version: latest,
        url,
        sums_url,
        release_url: release["html_url"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/{tag}")),
    })
}

/// Downloads `url` to `<temp>/redrob-canvas-update/<name>`, verifying it against `sums_url`.
async fn download_verified(
    url: &str,
    sums_url: &str,
    name: &str,
    mut progress: impl FnMut(u8),
) -> Result<PathBuf, String> {
    use sha2::Digest as _;
    use std::io::Write as _;

    let client = client()?;
    let sums = client
        .get(sums_url)
        .send()
        .await
        .map_err(|e| format!("{SUMS_NAME}: {}", describe(&e)))?
        .error_for_status()
        .map_err(|e| format!("{SUMS_NAME}: {}", describe(&e)))?
        .text()
        .await
        .map_err(|e| format!("{SUMS_NAME}: {}", describe(&e)))?;
    let expected = expected_sha256(&sums, name)
        .ok_or_else(|| format!("{SUMS_NAME} has no entry for {name}"))?;

    let dir = std::env::temp_dir().join("redrob-canvas-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(name);
    let part = dir.join(format!("{name}.part"));
    let mut file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download: {}", describe(&e)))?
        .error_for_status()
        .map_err(|e| format!("download: {}", describe(&e)))?;
    let total = response.content_length().unwrap_or(0);
    let mut received: u64 = 0;
    let mut hasher = sha2::Sha256::new();
    let mut last_pct = u8::MAX;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("download: {}", describe(&e)))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("{}: {e}", part.display()))?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        let pct = received
            .saturating_mul(100)
            .checked_div(total)
            .map_or(0, |p| p.min(100) as u8);
        if pct != last_pct {
            last_pct = pct;
            progress(pct);
        }
    }
    file.flush()
        .map_err(|e| format!("{}: {e}", part.display()))?;
    drop(file);

    let actual = hex(&hasher.finalize());
    if actual != expected {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "{name}: SHA-256 mismatch (the download is not the published file)"
        ));
    }
    std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// The command that opens `path` the way a user would double-click it. Windows runs the setup
/// itself; macOS mounts the disk image; Linux hands the archive to the desktop.
pub fn opener_for(os: &str, path: &Path) -> (String, Vec<OsString>) {
    match os {
        "macos" => ("open".into(), vec![path.as_os_str().to_owned()]),
        "windows" => (path.to_string_lossy().into_owned(), Vec::new()),
        _ => ("xdg-open".into(), vec![path.as_os_str().to_owned()]),
    }
}

/// "Install and restart": opens the verified download and closes the window.
fn install_and_restart(ctx: &egui::Context, path: &Path) -> Result<(), String> {
    let (program, args) = opener_for(std::env::consts::OS, path);
    std::process::Command::new(&program)
        .args(&args)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------------------------

/// `Last checked 3 h ago`, from the recorded time; `None` for never.
pub fn last_checked_text(last: Option<u64>, now: u64) -> Option<String> {
    use crate::i18n::tf;
    let ago = now.saturating_sub(last?);
    let text = if ago < 60 {
        tf("just now", &[])
    } else if ago < 3600 {
        tf("{n} min ago", &[("n", &(ago / 60).to_string())])
    } else if ago < 86_400 {
        tf("{n} h ago", &[("n", &(ago / 3600).to_string())])
    } else {
        tf("{n} d ago", &[("n", &(ago / 86_400).to_string())])
    };
    Some(tf("Last checked {ago}", &[("ago", &text)]))
}

/// The Help menu's update items: "Check for updates…" (or the launcher notice), the
/// automatic-check preference, the last outcome and when it was checked. Drawn inside the shell's
/// Help menu.
pub fn menu_items(ui: &mut egui::Ui) {
    use crate::i18n::{t, tf};
    let updater = Updater::global();
    updater.ensure_started();
    ui.separator();
    match updater.policy() {
        Policy::LauncherManaged => {
            ui.add_enabled(false, egui::Button::new(t("Check for updates…")))
                .on_disabled_hover_text(t(MANAGED_TEXT));
            ui.label(egui::RichText::new(t(MANAGED_TEXT)).weak());
        }
        Policy::InApp => {
            let status = updater.status();
            let busy = matches!(status, Status::Checking | Status::Downloading(_));
            if ui
                .add_enabled(!busy, egui::Button::new(t("Check for updates…")))
                .clicked()
            {
                ui.close();
                updater.check_now();
            }
            let prefs = updater.prefs();
            let mut auto = prefs.auto_check;
            if ui
                .checkbox(&mut auto, t("Check for updates automatically"))
                .on_hover_text(tf(
                    "At start, once a day. Also off with {env}=1.",
                    &[("env", NO_CHECK_ENV)],
                ))
                .changed()
            {
                updater.update_prefs(|p| p.auto_check = auto);
            }
            let line = match &status {
                Status::Idle => None,
                Status::Checking => Some(t("Checking…").to_owned()),
                Status::UpToDate => Some(tf(
                    "Up to date (v{version})",
                    &[("version", env!("CARGO_PKG_VERSION"))],
                )),
                Status::Available { version, .. } => Some(tf(
                    "v{version} is available",
                    &[("version", &version.to_string())],
                )),
                Status::Downloading(pct) => {
                    Some(tf("Downloading… {pct}%", &[("pct", &pct.to_string())]))
                }
                Status::Ready(_) => Some(t("Ready to install").to_owned()),
                Status::Failed(e) => Some(tf("Update check failed: {error}", &[("error", e)])),
                Status::RateLimited(e) => {
                    Some(tf("Update check skipped: {error}", &[("error", e)]))
                }
            };
            if let Some(line) = line {
                ui.label(egui::RichText::new(line).weak());
            }
            if let Some(line) = last_checked_text(prefs.last_check, now_secs()) {
                ui.label(egui::RichText::new(line).weak().small());
            }
        }
    }
}

/// The top bar's right slot: the update button while an update is available, downloading or
/// ready; a muted note, with the reason on hover, after a failed check. Nothing while there is
/// none, nothing after a quota refusal, and nothing under the launcher.
pub fn top_bar(ui: &mut egui::Ui) {
    use crate::i18n::{t, tf};
    let updater = Updater::global();
    updater.ensure_started();
    if updater.policy() != Policy::InApp {
        return;
    }
    let status = updater.status();
    match &status {
        Status::Available {
            version,
            release_url,
            ..
        } => {
            let version = version.to_string();
            let button = ui
                .button(tf("Update to v{version}", &[("version", &version)]))
                .on_hover_text(tf(
                    "Download v{version} and verify it ({url})",
                    &[("version", &version), ("url", release_url)],
                ));
            if button.clicked() {
                updater.download();
            }
        }
        Status::Downloading(pct) => {
            let pct = pct.to_string();
            let line = match updater.target_version().or_else(|| status.version()) {
                Some(v) => tf(
                    "Downloading v{version}… {pct}%",
                    &[("version", &v.to_string()), ("pct", &pct)],
                ),
                None => tf("Downloading… {pct}%", &[("pct", &pct)]),
            };
            ui.label(egui::RichText::new(line).weak());
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        Status::Ready(path) => {
            let shown = match updater.target_version() {
                Some(v) => format!("v{v}: {}", path.display()),
                None => path.display().to_string(),
            };
            let button = ui
                .button(t("Install and restart"))
                .on_hover_text(tf("Opens {path}", &[("path", &shown)]));
            if button.clicked()
                && let Err(e) = install_and_restart(ui.ctx(), path)
            {
                updater.set(Status::Failed(e));
            }
        }
        Status::Failed(e) => {
            // Muted, not an error colour: a missed check is not something to act on.
            ui.label(egui::RichText::new(t("Update check failed")).weak().small())
                .on_hover_text(e);
        }
        Status::Idle | Status::Checking | Status::UpToDate | Status::RateLimited(_) => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).expect(text)
    }

    #[test]
    fn versions_parse_with_or_without_the_v_and_compare_numerically() {
        assert_eq!(v("v0.7.0"), v("0.7.0"));
        assert!(v("v0.7.1") > v("v0.7.0"));
        assert!(v("v0.10.0") > v("v0.9.9"), "numeric, not lexical");
        assert!(v("v1.0.0") > v("v0.99.99"));
        assert_eq!(v("v0.7.0").cmp(&v("0.7.0")), Ordering::Equal);
        assert!(
            v("v0.8.0-rc.1") < v("v0.8.0"),
            "a pre-release is older than the release"
        );
        assert!(v("v0.8.0-rc.1") > v("v0.7.9"));
        assert_eq!(v("v1.2.3").to_string(), "1.2.3");
        for bad in [
            "", "v", "1.2", "1.2.3.4", "v1.2.x", "1.2.3-", "latest", "v-1.2.3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?} must not parse");
        }
        assert!(Version::current().is_some(), "CARGO_PKG_VERSION parses");
    }

    #[test]
    fn the_asset_is_the_installer_release_yml_publishes_for_each_target() {
        assert_eq!(
            asset_name_for("0.8.0", "macos", "aarch64").as_deref(),
            Some("redrob-canvas-0.8.0-macos-arm64.dmg")
        );
        assert_eq!(
            asset_name_for("0.8.0", "linux", "x86_64").as_deref(),
            Some("redrob-canvas-0.8.0-linux-x86_64.tar.gz")
        );
        assert_eq!(
            asset_name_for("0.8.0", "windows", "x86_64").as_deref(),
            Some("redrob-canvas-0.8.0-windows-x64-setup.exe")
        );
        // Not built: Intel macOS, 32-bit and ARM Windows, ARM Linux, anything else.
        for (os, arch) in [
            ("macos", "x86_64"),
            ("windows", "x86"),
            ("windows", "aarch64"),
            ("linux", "aarch64"),
            ("freebsd", "x86_64"),
        ] {
            assert_eq!(asset_name_for("0.8.0", os, arch), None, "{os}/{arch}");
        }
    }

    #[test]
    fn sha256sums_lines_parse_and_the_wanted_name_is_found() {
        let sums = "\
0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  redrob-canvas-0.8.0-linux-x86_64.tar.gz
ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789 *redrob-canvas-0.8.0-windows-x64-setup.exe

not a line
deadbeef  too-short.zip
0000000000000000000000000000000000000000000000000000000000000000  redrob-launcher.json\r
";
        let parsed = parse_sha256sums(sums);
        assert_eq!(parsed.len(), 3, "{parsed:?}");
        assert_eq!(
            expected_sha256(sums, "redrob-canvas-0.8.0-linux-x86_64.tar.gz").as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            expected_sha256(sums, "redrob-canvas-0.8.0-windows-x64-setup.exe").as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"),
            "binary-mode `*` is stripped and the hash is lower-cased"
        );
        assert_eq!(
            expected_sha256(sums, "redrob-launcher.json").as_deref(),
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            "a trailing CR does not stick to the name"
        );
        assert_eq!(expected_sha256(sums, "too-short.zip"), None);
        assert_eq!(expected_sha256(sums, "missing"), None);
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
    }

    #[test]
    fn launcher_managed_is_the_launcher_record_or_the_environment() {
        let none = |_: &str| None::<OsString>;
        let exe =
            Path::new("/home/u/.local/share/redrob-launcher/apps/redrob-canvas/redrob-canvas");
        let tree = r#"{ "apps": { "redrob-canvas": { "version": "0.7.0", "kind": "TarGz",
            "path": "/home/u/.local/share/redrob-launcher/apps/redrob-canvas", "installed_at": 1 } } }"#;
        assert!(launcher_managed_with(Some(exe), Some(tree), none));
        let exact = r#"{ "apps": { "redrob-canvas": { "version": "0.7.0", "kind": "AppImage",
            "path": "/home/u/.local/share/redrob-launcher/apps/redrob-canvas/redrob-canvas", "installed_at": 1 } } }"#;
        assert!(launcher_managed_with(Some(exe), Some(exact), none));
        let other = r#"{ "apps": { "redrob-cad": { "version": "0.0.3", "kind": "TarGz",
            "path": "/home/u/.local/share/redrob-launcher/apps/redrob-cad", "installed_at": 1 } } }"#;
        assert!(!launcher_managed_with(Some(exe), Some(other), none));
        assert!(!launcher_managed_with(
            Some(Path::new("/opt/redrob-canvas/redrob-canvas")),
            Some(tree),
            none
        ));
        assert!(!launcher_managed_with(Some(exe), Some("not json"), none));
        assert!(!launcher_managed_with(Some(exe), None, none));
        assert!(!launcher_managed_with(None, Some(tree), none));
        let env_on = |key: &str| (key == LAUNCHER_ENV).then(|| OsString::from("1"));
        assert!(launcher_managed_with(None, None, env_on));
        let env_off = |key: &str| (key == LAUNCHER_ENV).then(|| OsString::from("0"));
        assert!(!launcher_managed_with(None, None, env_off));
        // The opt-out is a separate switch.
        assert!(opted_out_with(
            |key| (key == NO_CHECK_ENV).then(|| OsString::from("1"))
        ));
        assert!(!opted_out_with(
            |key| (key == NO_CHECK_ENV).then(|| OsString::from("0"))
        ));
        assert!(!opted_out_with(none));
    }

    #[test]
    fn a_release_decides_up_to_date_or_available_with_both_urls() {
        let release = serde_json::json!({
            "tag_name": "v9.0.0",
            "html_url": "https://github.com/redrob-labs/redrob-canvas/releases/tag/v9.0.0",
            "assets": [
                { "name": asset_name("9.0.0").unwrap(), "browser_download_url": "https://dl/asset" },
                { "name": "SHA256SUMS.txt", "browser_download_url": "https://dl/sums" },
            ]
        });
        match decide(&release, Version::current()).unwrap() {
            Status::Available {
                version,
                url,
                sums_url,
                release_url,
            } => {
                assert_eq!(version, v("9.0.0"));
                assert_eq!(url, "https://dl/asset");
                assert_eq!(sums_url, "https://dl/sums");
                assert!(release_url.ends_with("v9.0.0"));
            }
            other => assert!(matches!(other, Status::Available { .. }), "{other:?}"),
        }
        let same = serde_json::json!({ "tag_name": format!("v{}", env!("CARGO_PKG_VERSION")), "assets": [] });
        assert_eq!(decide(&same, Version::current()).unwrap(), Status::UpToDate);
        let older = serde_json::json!({ "tag_name": "v0.0.1", "assets": [] });
        assert_eq!(
            decide(&older, Version::current()).unwrap(),
            Status::UpToDate
        );
        let no_sums = serde_json::json!({
            "tag_name": "v9.0.0",
            "assets": [{ "name": asset_name("9.0.0").unwrap(), "browser_download_url": "https://dl/asset" }]
        });
        assert!(
            decide(&no_sums, Version::current())
                .unwrap_err()
                .contains(SUMS_NAME)
        );
        let bad_tag = serde_json::json!({ "tag_name": "latest", "assets": [] });
        assert!(decide(&bad_tag, Version::current()).is_err());
    }

    #[test]
    fn preferences_default_to_automatic_checks() {
        assert!(parse_auto_check(""));
        assert!(parse_auto_check("{}"));
        assert!(parse_auto_check("{ \"auto_check\": true }"));
        assert!(!parse_auto_check("{ \"auto_check\": false }"));
        assert!(parse_auto_check("not json"));
        assert_eq!(parse_prefs("not json"), Prefs::default());
        let prefs = parse_prefs("{ \"auto_check\": false, \"last_check\": 1000 }");
        assert_eq!(
            prefs,
            Prefs {
                auto_check: false,
                last_check: Some(1000)
            }
        );
    }

    #[test]
    fn the_automatic_check_runs_at_most_once_a_day() {
        let day = CHECK_INTERVAL.as_secs();
        assert!(Prefs::default().due(0), "never checked");
        let checked = Prefs {
            auto_check: true,
            last_check: Some(1_000_000),
        };
        assert!(!checked.due(1_000_000), "just now");
        assert!(!checked.due(1_000_000 + day - 1));
        assert!(checked.due(1_000_000 + day));
        assert!(checked.due(1_000_000 + 3 * day));
        assert!(!checked.due(999_000), "a clock set back is not due");
        assert_eq!(last_checked_text(None, 5), None);
        assert_eq!(
            last_checked_text(Some(100), 130).as_deref(),
            Some("Last checked just now")
        );
        assert_eq!(
            last_checked_text(Some(0), 7_200).as_deref(),
            Some("Last checked 2 h ago")
        );
        assert_eq!(
            last_checked_text(Some(0), 3 * day).as_deref(),
            Some("Last checked 3 d ago")
        );
    }

    #[test]
    fn a_quota_refusal_falls_back_to_the_release_redirect() {
        assert!(is_quota(403) && is_quota(429));
        assert!(!is_quota(404) && !is_quota(500) && !is_quota(200));
        assert_eq!(
            tag_from_release_url(
                "https://github.com/redrob-labs/redrob-canvas/releases/tag/v0.8.1"
            )
            .as_deref(),
            Some("v0.8.1")
        );
        assert_eq!(
            tag_from_release_url("/redrob-labs/redrob-canvas/releases/tag/v0.9.0/?x=1").as_deref(),
            Some("v0.9.0")
        );
        assert_eq!(
            tag_from_release_url("https://github.com/redrob-labs/redrob-canvas/releases"),
            None
        );
        assert_eq!(tag_from_release_url("/releases/tag/latest"), None);
        // The synthesized release decides like the API's: the asset for this platform and the sums.
        let release = release_from_tag("v9.0.0");
        assert_eq!(release["tag_name"], "v9.0.0");
        match decide(&release, Version::current()).unwrap() {
            Status::Available {
                url,
                sums_url,
                release_url,
                ..
            } => {
                assert_eq!(
                    url,
                    format!(
                        "https://github.com/{REPO}/releases/download/v9.0.0/{}",
                        asset_name("9.0.0").unwrap()
                    )
                );
                assert_eq!(
                    sums_url,
                    format!("https://github.com/{REPO}/releases/download/v9.0.0/{SUMS_NAME}")
                );
                assert!(release_url.ends_with("/releases/tag/v9.0.0"));
            }
            other => assert!(matches!(other, Status::Available { .. }), "{other:?}"),
        }
        assert_eq!(
            decide(
                &release_from_tag(&format!("v{}", env!("CARGO_PKG_VERSION"))),
                Version::current()
            )
            .unwrap(),
            Status::UpToDate
        );
    }

    #[test]
    fn error_chains_are_spelled_out() {
        #[derive(Debug)]
        struct Outer(std::io::Error);
        impl fmt::Display for Outer {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("error sending request")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let e = Outer(std::io::Error::other(
            "invalid peer certificate: UnknownIssuer",
        ));
        assert_eq!(
            describe(&e),
            "error sending request: invalid peer certificate: UnknownIssuer"
        );
    }

    #[test]
    fn the_opener_is_the_platform_double_click() {
        let path = Path::new("/tmp/redrob-canvas-0.8.0-macos-arm64.dmg");
        assert_eq!(opener_for("macos", path).0, "open");
        assert_eq!(opener_for("linux", path).0, "xdg-open");
        let (program, args) = opener_for("windows", Path::new("C:\\x\\setup.exe"));
        assert_eq!(program, "C:\\x\\setup.exe");
        assert!(args.is_empty(), "the setup runs itself");
    }
}

#[cfg(test)]
mod launcher_paths {
    use super::*;

    #[test]
    fn launcher_manifest_follows_the_launchers_data_local_dir() {
        let env = |key: &str| match key {
            "HOME" => Some(OsString::from("/home/u")),
            "LOCALAPPDATA" => Some(OsString::from("C:\\Users\\u\\AppData\\Local")),
            _ => None,
        };
        assert_eq!(
            launcher_manifest_path_with("linux", env),
            Some(PathBuf::from(
                "/home/u/.local/share/redrob-launcher/installed.json"
            ))
        );
        assert_eq!(
            launcher_manifest_path_with("macos", env),
            Some(PathBuf::from(
                "/home/u/Library/Application Support/redrob-launcher/installed.json"
            ))
        );
        assert_eq!(
            launcher_manifest_path_with("windows", env),
            Some(PathBuf::from(
                "C:\\Users\\u\\AppData\\Local/redrob-launcher/installed.json"
            ))
        );
        let xdg = |key: &str| match key {
            "XDG_DATA_HOME" => Some(OsString::from("/data")),
            "HOME" => Some(OsString::from("/home/u")),
            _ => None,
        };
        assert_eq!(
            launcher_manifest_path_with("linux", xdg),
            Some(PathBuf::from("/data/redrob-launcher/installed.json"))
        );
    }
}
