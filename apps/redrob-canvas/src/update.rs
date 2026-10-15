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
//! preference (Help menu, `<config dir>/update.json`); Help > "Check for updates…" still works.
//! Under Redrob Launcher (`redrob-launcher.json` beside the binary, or `REDROB_LAUNCHER=1`) the
//! launcher owns updates and the menu says so instead.
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
/// The launcher's sentinel beside the binary (the launcher writes it into every install).
pub const LAUNCHER_SENTINEL: &str = "redrob-launcher.json";
/// The preference file under the config directory: `{ "auto_check": true }`.
pub const PREFS_FILE: &str = "update.json";
/// How long after the first frame the automatic check runs.
pub const START_DELAY: Duration = Duration::from_secs(3);
/// What the menu says under the launcher.
pub const MANAGED_TEXT: &str = "Updates are managed by Redrob Launcher";

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

/// Launcher-managed when `redrob-launcher.json` sits beside the binary (`exe_dir`) or the
/// launcher set `REDROB_LAUNCHER=1`.
pub fn launcher_managed_with(
    exe_dir: Option<&Path>,
    env: impl Fn(&str) -> Option<OsString>,
) -> bool {
    if env(LAUNCHER_ENV).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")) {
        return true;
    }
    exe_dir.is_some_and(|dir| dir.join(LAUNCHER_SENTINEL).is_file())
}

fn launcher_managed() -> bool {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    launcher_managed_with(exe_dir.as_deref(), |key| std::env::var_os(key))
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

/// The `auto_check` flag from `update.json`; `true` when there is no file.
pub fn parse_auto_check(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("auto_check").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

fn load_auto_check() -> bool {
    prefs_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_auto_check(&t))
        .unwrap_or(true)
}

fn save_auto_check(on: bool) {
    let Some(path) = prefs_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::json!({ "auto_check": on }).to_string() + "\n";
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
    Failed(String),
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
    auto_check: AtomicBool,
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
                auto_check: AtomicBool::new(load_auto_check()),
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

    /// The automatic check, once: after [`START_DELAY`], unless opted out or launcher-managed.
    fn ensure_started(&'static self) {
        if self.started.swap(true, AtomicOrdering::SeqCst) {
            return;
        }
        if self.policy != Policy::InApp
            || opted_out_with(|key| std::env::var_os(key))
            || !self.auto_check.load(AtomicOrdering::Relaxed)
        {
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
        match outcome {
            Ok(status) => self.set(status),
            Err(e) => {
                log::warn!("update check: {e}");
                self.set(Status::Failed(e));
            }
        }
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
        self.target.lock().ok().and_then(|t| t.map(|(v, _)| v))
    }
}

/// Runs `f` with a current-thread tokio runtime (built here, on the calling worker thread).
fn with_runtime<T>(
    f: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String>,
) -> Result<T, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;
    f(&rt)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))
}

/// Reads the latest release and decides between UpToDate and Available.
async fn fetch_latest() -> Result<Status, String> {
    let client = client()?;
    let response = client
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("GitHub answered {}", response.status()));
    }
    let release: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("GitHub: invalid JSON: {e}"))?;
    decide(&release, Version::current())
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
        .map_err(|e| format!("{SUMS_NAME}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("{SUMS_NAME}: {e}"))?
        .text()
        .await
        .map_err(|e| format!("{SUMS_NAME}: {e}"))?;
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
        .map_err(|e| format!("download: {e}"))?
        .error_for_status()
        .map_err(|e| format!("download: {e}"))?;
    let total = response.content_length().unwrap_or(0);
    let mut received: u64 = 0;
    let mut hasher = sha2::Sha256::new();
    let mut last_pct = u8::MAX;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("download: {e}"))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("{}: {e}", part.display()))?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        let pct = if total > 0 {
            (received.saturating_mul(100) / total).min(100) as u8
        } else {
            0
        };
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

/// The Help menu's update items: "Check for updates…" (or the launcher notice), the
/// automatic-check preference, and the last outcome. Drawn inside the shell's Help menu.
pub fn menu_items(ui: &mut egui::Ui) {
    let updater = Updater::global();
    updater.ensure_started();
    ui.separator();
    match updater.policy() {
        Policy::LauncherManaged => {
            ui.add_enabled(false, egui::Button::new("Check for updates…"))
                .on_disabled_hover_text(MANAGED_TEXT);
            ui.label(egui::RichText::new(MANAGED_TEXT).weak());
        }
        Policy::InApp => {
            let status = updater.status();
            let busy = matches!(status, Status::Checking | Status::Downloading(_));
            if ui
                .add_enabled(!busy, egui::Button::new("Check for updates…"))
                .clicked()
            {
                ui.close();
                updater.check_now();
            }
            let mut auto = updater.auto_check.load(AtomicOrdering::Relaxed);
            if ui
                .checkbox(&mut auto, "Check for updates automatically")
                .on_hover_text(format!(
                    "At start, after {} s. Also off with {NO_CHECK_ENV}=1.",
                    START_DELAY.as_secs()
                ))
                .changed()
            {
                updater.auto_check.store(auto, AtomicOrdering::Relaxed);
                save_auto_check(auto);
            }
            let line = match &status {
                Status::Idle => None,
                Status::Checking => Some("Checking…".to_owned()),
                Status::UpToDate => Some(format!("Up to date (v{})", env!("CARGO_PKG_VERSION"))),
                Status::Available { version, .. } => Some(format!("v{version} is available")),
                Status::Downloading(pct) => Some(format!("Downloading… {pct}%")),
                Status::Ready(_) => Some("Ready to install".to_owned()),
                Status::Failed(e) => Some(format!("Update check failed: {e}")),
            };
            if let Some(line) = line {
                ui.label(egui::RichText::new(line).weak());
            }
        }
    }
}

/// The top bar's right slot: the update button while an update is available, downloading or
/// ready. Nothing while there is none (and nothing under the launcher).
pub fn top_bar(ui: &mut egui::Ui) {
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
            let button = ui
                .button(format!("Update to v{version}"))
                .on_hover_text(format!("Download v{version} and verify it ({release_url})"));
            if button.clicked() {
                updater.download();
            }
        }
        Status::Downloading(pct) => {
            let version = updater
                .target_version()
                .or_else(|| status.version())
                .map(|v| format!(" v{v}"))
                .unwrap_or_default();
            ui.label(egui::RichText::new(format!("Downloading{version}… {pct}%")).weak());
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        Status::Ready(path) => {
            let version = updater
                .target_version()
                .map(|v| format!(" v{v}"))
                .unwrap_or_default();
            let button = ui
                .button("Install and restart")
                .on_hover_text(format!("Opens{version}: {}", path.display()));
            if button.clicked()
                && let Err(e) = install_and_restart(ui.ctx(), path)
            {
                updater.set(Status::Failed(e));
            }
        }
        Status::Failed(e) => {
            ui.label(egui::RichText::new("Update failed").color(ui.visuals().error_fg_color))
                .on_hover_text(e);
        }
        Status::Idle | Status::Checking | Status::UpToDate => {}
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
    fn launcher_managed_is_the_sentinel_beside_the_binary_or_the_environment() {
        let none = |_: &str| None::<OsString>;
        let dir =
            std::env::temp_dir().join(format!("redrob-canvas-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!launcher_managed_with(Some(&dir), none));
        assert!(!launcher_managed_with(None, none));
        std::fs::write(dir.join(LAUNCHER_SENTINEL), "{ \"schema\": 1 }\n").unwrap();
        assert!(launcher_managed_with(Some(&dir), none));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!launcher_managed_with(Some(&dir), none));
        let env_on = |key: &str| (key == LAUNCHER_ENV).then(|| OsString::from("1"));
        assert!(launcher_managed_with(None, env_on));
        let env_off = |key: &str| (key == LAUNCHER_ENV).then(|| OsString::from("0"));
        assert!(!launcher_managed_with(None, env_off));
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
            other => panic!("{other:?}"),
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
