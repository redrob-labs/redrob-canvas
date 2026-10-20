// SPDX-License-Identifier: MIT OR Apache-2.0

//! Lifecycle of the local IOPaint server: install into an app-owned virtual environment, start on
//! a free loopback port, poll until ready, stop.
//!
//! The server has no authentication of its own; it listens on loopback only and is started with
//! no input folder, so its file manager stays off. The child is killed when the [`Engine`] drops.
//!
//! The server is started through `redrob_shim.py` (written next to the venv), which runs IOPaint's
//! own command line and adds the raw-file routes `super::transport` uses; the shim may only read
//! and write inside [`EnginePaths::exchange_dir`]. A venv without a usable interpreter starts the
//! `iopaint` entry point directly and the client stays on base64.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
    sync::{broadcast, watch},
};

use super::client::{Client, ServerConfig};
use super::models::DEFAULT_REMOVE_BG_MODEL;
use super::transport::{FileExchange, Transport, TransportMode};
use crate::{AiError, Result};

/// A command for an install step or the engine itself. On Windows it runs with no console window
/// (`CREATE_NO_WINDOW`): Canvas is a windowed app, so a console child would pop a cmd window up,
/// and pip / uv under it would inherit that. Their output is read through the pipes either way.
fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)] // only Windows changes it
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd
}

/// The Python file the engine starts IOPaint through.
pub const SHIM_SOURCE: &str = include_str!("redrob_shim.py");

/// The environment variable the shim reads the exchange directory from.
pub const EXCHANGE_DIR_ENV: &str = "REDROB_AI_EXCHANGE_DIR";

/// Lines of engine log kept for a snapshot.
pub const MAX_LOG_LINES: usize = 300;
/// How often readiness is probed.
pub const READY_POLL_INTERVAL: Duration = Duration::from_millis(1500);
/// IOPaint downloads a model on first start; a big diffusion model is gigabytes. Poll for 40 min.
pub const MAX_READY_POLLS: u32 = 1600;

/// Where the engine runs its models.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Device {
    #[default]
    Cpu,
    Cuda,
    Mps,
}

impl Device {
    pub const ALL: [Device; 3] = [Device::Cpu, Device::Cuda, Device::Mps];

    pub fn as_str(self) -> &'static str {
        match self {
            Device::Cpu => "cpu",
            Device::Cuda => "cuda",
            Device::Mps => "mps",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == name)
    }
}

/// IOPaint's optional plugins. Each loads at server start only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Plugin {
    /// Click to select (Segment Anything, `mobile_sam`).
    InteractiveSeg,
    /// Remove background / select subject (rembg).
    RemoveBG,
    AnimeSeg,
    RealESRGAN,
    GFPGAN,
    RestoreFormer,
}

impl Plugin {
    pub const ALL: [Plugin; 6] = [
        Plugin::InteractiveSeg,
        Plugin::RemoveBG,
        Plugin::AnimeSeg,
        Plugin::RealESRGAN,
        Plugin::GFPGAN,
        Plugin::RestoreFormer,
    ];

    /// The plugins on by default.
    pub const DEFAULT: [Plugin; 3] = [Plugin::InteractiveSeg, Plugin::RemoveBG, Plugin::RealESRGAN];

    /// The name IOPaint knows the plugin by (`plugins[].name` in server-config).
    pub fn name(self) -> &'static str {
        match self {
            Plugin::InteractiveSeg => "InteractiveSeg",
            Plugin::RemoveBG => "RemoveBG",
            Plugin::AnimeSeg => "AnimeSeg",
            Plugin::RealESRGAN => "RealESRGAN",
            Plugin::GFPGAN => "GFPGAN",
            Plugin::RestoreFormer => "RestoreFormer",
        }
    }

    /// What the panel calls it.
    pub fn label(self) -> &'static str {
        match self {
            Plugin::InteractiveSeg => "Click to select (Segment Anything)",
            Plugin::RemoveBG => "Remove background / select subject",
            Plugin::AnimeSeg => "Anime segmentation",
            Plugin::RealESRGAN => "Upscale (RealESRGAN)",
            Plugin::GFPGAN => "Face restore (GFPGAN)",
            Plugin::RestoreFormer => "Face restore (RestoreFormer)",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Known names from a list, deduplicated, unknown ones dropped.
    pub fn parse_list<S: AsRef<str>>(names: &[S]) -> Vec<Self> {
        let mut out = Vec::new();
        for name in names {
            if let Some(p) = Self::parse(name.as_ref())
                && !out.contains(&p)
            {
                out.push(p);
            }
        }
        out
    }
}

/// The folders the engine owns under the app's data directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnginePaths {
    /// `<app data>/iopaint`.
    pub root: PathBuf,
    /// `"windows"`, `"macos"`, `"linux"`... (`std::env::consts::OS`); decides the venv layout.
    pub target_os: String,
}

impl EnginePaths {
    /// Paths under `<app_data_dir>/iopaint` for the running platform.
    pub fn new(app_data_dir: impl AsRef<Path>) -> Self {
        Self::for_os(app_data_dir, std::env::consts::OS)
    }

    pub fn for_os(app_data_dir: impl AsRef<Path>, target_os: &str) -> Self {
        Self {
            root: app_data_dir.as_ref().join("iopaint"),
            target_os: target_os.to_owned(),
        }
    }

    pub fn is_windows(&self) -> bool {
        self.target_os == "windows"
    }

    pub fn is_macos(&self) -> bool {
        self.target_os == "macos"
    }

    /// The virtual environment.
    pub fn env_dir(&self) -> PathBuf {
        self.root.join("env")
    }

    /// Where IOPaint caches downloaded models (`--model-dir`).
    pub fn model_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    fn bin_dir(&self) -> PathBuf {
        self.env_dir()
            .join(if self.is_windows() { "Scripts" } else { "bin" })
    }

    /// The venv's `iopaint` entry point.
    pub fn iopaint_executable(&self) -> PathBuf {
        self.bin_dir().join(if self.is_windows() {
            "iopaint.exe"
        } else {
            "iopaint"
        })
    }

    /// Where the shim is written before a start.
    pub fn shim_path(&self) -> PathBuf {
        self.root.join("redrob_shim.py")
    }

    /// Where the client and the shim exchange raw pixel files: `/dev/shm` (memory) on Linux when
    /// it exists, else `<root>/exchange`. Per process, so two running apps never share a file.
    pub fn exchange_dir(&self) -> PathBuf {
        let shm = Path::new("/dev/shm");
        if self.target_os == "linux" && shm.is_dir() {
            return shm.join(format!("redrob-canvas-{}", std::process::id()));
        }
        self.root.join("exchange")
    }

    /// The venv's interpreter.
    pub fn python(&self) -> PathBuf {
        self.bin_dir().join(if self.is_windows() {
            "python.exe"
        } else {
            "python"
        })
    }

    pub fn is_installed(&self) -> bool {
        self.iopaint_executable().is_file()
    }
}

/// How the virtual environment is provisioned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallMethod {
    /// `uv venv --python 3.11` then `uv pip install`.
    Uv(PathBuf),
    /// `<python> -m venv` then `pip install`; the command may be several words (`py -3.11`).
    Python(Vec<String>),
}

impl InstallMethod {
    /// The label the panel shows: `"uv"` or `"python 3.11"`.
    pub fn label(&self) -> &'static str {
        match self {
            InstallMethod::Uv(_) => "uv",
            InstallMethod::Python(_) => "python 3.11",
        }
    }

    /// `uv` from PATH (also `~/.local/bin`, `~/.cargo/bin`), else Python 3.11 (3.10 accepted:
    /// IOPaint 1.6.0 pins Pillow 9.5.0, which has no wheels past 3.11), else `None`.
    pub fn detect() -> Option<Self> {
        if let Some(uv) = find_uv() {
            return Some(InstallMethod::Uv(uv));
        }
        find_python311().map(InstallMethod::Python)
    }
}

fn path_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
    }
    dirs
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    path_dirs().into_iter().find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{name}{ext}")))
            .find(|p| p.is_file())
    })
}

fn find_uv() -> Option<PathBuf> {
    find_executable("uv")
}

fn find_python311() -> Option<Vec<String>> {
    for name in ["python3.11", "python3.10"] {
        if let Some(found) = find_executable(name) {
            return Some(vec![found.to_string_lossy().into_owned()]);
        }
    }
    if cfg!(windows)
        && let Some(py) = find_executable("py")
    {
        return Some(vec![py.to_string_lossy().into_owned(), "-3.11".into()]);
    }
    None
}

/// IOPaint 1.6.0 is the last release (the project is archived). rembg is its RemoveBG plugin's
/// optional dependency; it is pinned to a release that accepts IOPaint's own pins (Pillow 9.5,
/// NumPy 1.x), since a newer rembg silently upgrades both.
pub fn pinned_packages() -> [&'static str; 4] {
    [
        "iopaint==1.6.0",
        "rembg[cpu]==2.0.57",
        "pillow==9.5.0",
        "numpy<2",
    ]
}

/// The PyTorch pins, installed first and from their own index.
pub fn torch_packages() -> [&'static str; 2] {
    ["torch==2.5.1", "torchvision==0.20.1"]
}

/// PyTorch's default Linux and Windows wheels carry CUDA (gigabytes); use the CPU build unless the
/// user picked CUDA. macOS wheels are CPU/Metal already, so they take no index.
pub fn torch_index_url(device: Device, target_os: &str) -> Option<&'static str> {
    if target_os == "macos" {
        return None;
    }
    Some(if device == Device::Cuda {
        "https://download.pytorch.org/whl/cu121"
    } else {
        "https://download.pytorch.org/whl/cpu"
    })
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The commands `install` runs, in order (each `Vec` is argv). Pure, so a test can read it
/// without installing anything.
pub fn install_steps(
    method: &InstallMethod,
    paths: &EnginePaths,
    device: Device,
) -> Vec<Vec<String>> {
    let env = s(&paths.env_dir());
    let python = s(&paths.python());
    let mut torch: Vec<String> = Vec::new();
    if let Some(index) = torch_index_url(device, &paths.target_os) {
        torch.push("--index-url".into());
        torch.push(index.into());
    }
    torch.extend(torch_packages().iter().map(|p| (*p).to_owned()));
    let pinned: Vec<String> = pinned_packages().iter().map(|p| (*p).to_owned()).collect();
    match method {
        InstallMethod::Uv(uv) => {
            let uv = s(uv);
            let pip = || {
                vec![
                    uv.clone(),
                    "pip".to_owned(),
                    "install".to_owned(),
                    "--python".to_owned(),
                    python.clone(),
                ]
            };
            vec![
                vec![
                    uv.clone(),
                    "venv".into(),
                    "--python".into(),
                    "3.11".into(),
                    env,
                ],
                [pip(), torch].concat(),
                [pip(), pinned].concat(),
            ]
        }
        InstallMethod::Python(base) => {
            let pip = || {
                vec![
                    python.clone(),
                    "-m".to_owned(),
                    "pip".to_owned(),
                    "install".to_owned(),
                ]
            };
            vec![
                [base.clone(), vec!["-m".into(), "venv".into(), env]].concat(),
                [pip(), vec!["--upgrade".into(), "pip".into()]].concat(),
                [pip(), torch].concat(),
                [pip(), pinned].concat(),
            ]
        }
    }
}

/// `iopaint start ...` arguments. Loopback only: the server has no authentication. No `--input`,
/// so its file manager is off. Plugin flags exactly as IOPaint 1.6.0 names them;
/// `remove_bg_model` is one of [`super::models::REMOVE_BG_MODELS`] (default isnet).
pub fn server_arguments(
    port: u16,
    model: &str,
    device: Device,
    model_dir: &Path,
    plugins: &[Plugin],
    remove_bg_model: &str,
) -> Vec<String> {
    let d = device.as_str();
    let mut args: Vec<String> = [
        "start",
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "--model",
        model,
        "--device",
        d,
        "--model-dir",
        &s(model_dir),
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect();
    let mut push = |flags: &[&str]| args.extend(flags.iter().map(|f| (*f).to_owned()));
    if plugins.contains(&Plugin::InteractiveSeg) {
        push(&[
            "--enable-interactive-seg",
            "--interactive-seg-model",
            "mobile_sam",
            "--interactive-seg-device",
            d,
        ]);
    }
    if plugins.contains(&Plugin::RemoveBG) {
        push(&[
            "--enable-remove-bg",
            "--remove-bg-device",
            d,
            "--remove-bg-model",
            remove_bg_model,
        ]);
    }
    if plugins.contains(&Plugin::AnimeSeg) {
        push(&["--enable-anime-seg"]);
    }
    if plugins.contains(&Plugin::RealESRGAN) {
        push(&["--enable-realesrgan", "--realesrgan-device", d]);
    }
    if plugins.contains(&Plugin::GFPGAN) {
        push(&["--enable-gfpgan", "--gfpgan-device", d]);
    }
    if plugins.contains(&Plugin::RestoreFormer) {
        push(&["--enable-restoreformer", "--restoreformer-device", d]);
    }
    args
}

/// `iopaint run ...` arguments: every image in `images` with a same-named mask in `masks` (a
/// folder, or one mask file for all), results into `output`.
pub fn batch_arguments(
    model: &str,
    device: Device,
    images: &Path,
    masks: &Path,
    output: &Path,
    model_dir: &Path,
) -> Vec<String> {
    [
        "run",
        "--model",
        model,
        "--device",
        device.as_str(),
        "--image",
        &s(images),
        "--mask",
        &s(masks),
        "--output",
        &s(output),
        "--model-dir",
        &s(model_dir),
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect()
}

/// A free TCP port on 127.0.0.1 (bound to port 0 and released).
pub fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

/// Where the engine is. `Error` carries the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineState {
    /// No `iopaint` executable in the venv.
    Missing,
    Installing,
    Stopped,
    Starting,
    Ready,
    /// A request is in flight.
    Busy,
    Error(String),
}

impl EngineState {
    /// The lowercase name the Qt panel used (`error` for any `Error(_)`).
    pub fn as_str(&self) -> &'static str {
        match self {
            EngineState::Missing => "missing",
            EngineState::Installing => "installing",
            EngineState::Stopped => "stopped",
            EngineState::Starting => "starting",
            EngineState::Ready => "ready",
            EngineState::Busy => "busy",
            EngineState::Error(_) => "error",
        }
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            EngineState::Busy | EngineState::Starting | EngineState::Installing
        )
    }
}

struct Running {
    child: Child,
    port: u16,
}

struct Inner {
    paths: EnginePaths,
    state: watch::Sender<EngineState>,
    message: watch::Sender<String>,
    log_tx: broadcast::Sender<String>,
    log: Mutex<VecDeque<String>>,
    running: Mutex<Option<Running>>,
    config: Mutex<Option<ServerConfig>>,
    install_running: Mutex<bool>,
    /// The user's transport setting.
    transport_mode: Mutex<TransportMode>,
    /// Whether the running server answered `/api/v1/redrob/transport`.
    shim_has_files: Mutex<bool>,
    /// The RemoveBG model the next start passes (and a running server is switched to).
    remove_bg_model: Mutex<String>,
    /// The model the running server was started with or switched to.
    current_model: Mutex<Option<String>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(mut running) = self.running.lock()
            && let Some(mut r) = running.take()
        {
            // Best effort; `kill_on_drop` covers the rest.
            let _ = r.child.start_kill();
        }
    }
}

/// The local IOPaint server: install, start, poll ready, stop. Cheap to clone (shared handle).
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("paths", &self.inner.paths)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another task panicked while holding it; the data is plain.
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Engine {
    /// An engine over `paths`: `Stopped` when the venv has `iopaint`, else `Missing`.
    pub fn new(paths: EnginePaths) -> Self {
        let (state, message) = if paths.is_installed() {
            (
                EngineState::Stopped,
                "The AI engine is installed and stopped",
            )
        } else {
            (
                EngineState::Missing,
                "The AI engine (IOPaint) is not installed",
            )
        };
        let (log_tx, _) = broadcast::channel(512);
        Self {
            inner: Arc::new(Inner {
                paths,
                state: watch::Sender::new(state),
                message: watch::Sender::new(message.to_owned()),
                log_tx,
                log: Mutex::new(VecDeque::new()),
                running: Mutex::new(None),
                config: Mutex::new(None),
                install_running: Mutex::new(false),
                transport_mode: Mutex::new(TransportMode::default()),
                shim_has_files: Mutex::new(false),
                remove_bg_model: Mutex::new(DEFAULT_REMOVE_BG_MODEL.to_owned()),
                current_model: Mutex::new(None),
            }),
        }
    }

    pub fn paths(&self) -> &EnginePaths {
        &self.inner.paths
    }

    pub fn state(&self) -> EngineState {
        self.inner.state.borrow().clone()
    }

    /// Changes as they happen; `borrow_and_update()` / `changed().await` on the receiver.
    pub fn subscribe_state(&self) -> watch::Receiver<EngineState> {
        self.inner.state.subscribe()
    }

    /// The one-line status the panel shows.
    pub fn message(&self) -> String {
        self.inner.message.borrow().clone()
    }

    pub fn subscribe_message(&self) -> watch::Receiver<String> {
        self.inner.message.subscribe()
    }

    /// Log lines as they arrive (install output, server output). A slow reader sees `Lagged`.
    pub fn subscribe_log(&self) -> broadcast::Receiver<String> {
        self.inner.log_tx.subscribe()
    }

    /// The last [`MAX_LOG_LINES`] log lines.
    pub fn log(&self) -> Vec<String> {
        lock(&self.inner.log).iter().cloned().collect()
    }

    /// The server's configuration once it became ready.
    pub fn config(&self) -> Option<ServerConfig> {
        lock(&self.inner.config).clone()
    }

    /// The port the server listens on while it runs.
    pub fn port(&self) -> Option<u16> {
        lock(&self.inner.running).as_ref().map(|r| r.port)
    }

    /// A client for the running server, or `None` when it is not running. Pixels go over the
    /// file transport when the user asked for it and the shim is up, else as base64.
    pub fn client(&self) -> Option<Client> {
        let port = self.port()?;
        let base_url = format!("http://127.0.0.1:{port}");
        let transport = Transport::choose(
            self.transport_mode(),
            &base_url,
            Some(&self.inner.paths.exchange_dir()),
            *lock(&self.inner.shim_has_files),
        );
        Client::with_transport(base_url, transport).ok()
    }

    /// The user's transport setting (`file` by default).
    pub fn transport_mode(&self) -> TransportMode {
        *lock(&self.inner.transport_mode)
    }

    /// Changes the transport setting; the next request uses it (no restart needed).
    pub fn set_transport_mode(&self, mode: TransportMode) {
        *lock(&self.inner.transport_mode) = mode;
    }

    /// The transport requests actually take right now: `File` only while the shim's routes are
    /// up and the setting asks for it.
    pub fn transport_in_use(&self) -> TransportMode {
        self.client()
            .map(|c| c.transport().mode())
            .unwrap_or(TransportMode::Base64)
    }

    /// The RemoveBG model the next start loads.
    pub fn remove_bg_model(&self) -> String {
        lock(&self.inner.remove_bg_model).clone()
    }

    /// Sets the RemoveBG model for the next start; a running server with the plugin is switched
    /// in place.
    pub async fn set_remove_bg_model(&self, model: &str) -> Result<()> {
        *lock(&self.inner.remove_bg_model) = model.to_owned();
        let has_plugin = self.config().is_some_and(|c| c.has_plugin("RemoveBG"));
        if self.state() != EngineState::Ready || !has_plugin {
            return Ok(());
        }
        let model = model.to_owned();
        self.request(
            "Subject model",
            format!("Loading {model}…"),
            |client| async move { client.switch_plugin_model("RemoveBG", &model).await },
        )
        .await?;
        if let Some(mut config) = self.config()
            && let Some(entry) = config.plugin_models.get_mut("RemoveBG")
        {
            entry.current = self.remove_bg_model();
            *lock(&self.inner.config) = Some(config);
        }
        Ok(())
    }

    /// The model the running server holds, once it is ready.
    pub fn current_model(&self) -> Option<String> {
        lock(&self.inner.current_model).clone()
    }

    pub fn is_running(&self) -> bool {
        self.port().is_some()
    }

    /// How `install` would provision Python, or `None` when neither uv nor Python 3.11 exists.
    pub fn install_method(&self) -> Option<InstallMethod> {
        InstallMethod::detect()
    }

    fn set_state(&self, state: EngineState) {
        self.inner.state.send_if_modified(|current| {
            if *current == state {
                false
            } else {
                *current = state;
                true
            }
        });
    }

    fn set_message(&self, message: impl Into<String>) {
        let message = message.into();
        self.inner.message.send_if_modified(|current| {
            if *current == message {
                false
            } else {
                *current = message;
                true
            }
        });
    }

    fn append_log(&self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        // Download progress bars redraw with \r; keep only the last frame.
        let last = trimmed.rsplit('\r').next().unwrap_or(trimmed).trim();
        let line: String = last.chars().take(300).collect();
        {
            let mut log = lock(&self.inner.log);
            log.push_back(line.clone());
            while log.len() > MAX_LOG_LINES {
                log.pop_front();
            }
        }
        let _ = self.inner.log_tx.send(line);
    }

    fn forward_output(&self, reader: impl AsyncRead + Unpin + Send + 'static) {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                engine.append_log(&line);
            }
        });
    }

    /// Runs one install step to completion, streaming its output to the log.
    async fn run_step(&self, argv: &[String]) -> Result<()> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| AiError::Process("empty command".into()))?;
        self.append_log(&format!("$ {}", argv.join(" ")));
        let mut child = quiet_command(program)
            .args(args)
            .env("PYTHONUTF8", "1")
            .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                self.append_log(&format!("could not start {program}"));
                AiError::Process(format!("could not start {program}: {e}"))
            })?;
        if let Some(out) = child.stdout.take() {
            self.forward_output(out);
        }
        if let Some(err) = child.stderr.take() {
            self.forward_output(err);
        }
        let status = child.wait().await?;
        if !status.success() {
            let code = status.code().unwrap_or(-1);
            self.append_log(&format!("step failed with exit code {code}"));
            return Err(AiError::Process(format!(
                "{program} exited with code {code}"
            )));
        }
        Ok(())
    }

    /// Installs IOPaint 1.6.0 and PyTorch into the venv (about 1.5 GB), streaming progress to the
    /// log. Stops a running server first. Ends `Stopped` on success, `Missing` on failure.
    pub async fn install(&self, device: Device) -> Result<()> {
        {
            let mut flag = lock(&self.inner.install_running);
            if *flag {
                return Err(AiError::Busy("Install".into()));
            }
            *flag = true;
        }
        let result = self.install_inner(device).await;
        *lock(&self.inner.install_running) = false;
        result
    }

    async fn install_inner(&self, device: Device) -> Result<()> {
        self.stop().await;
        let paths = &self.inner.paths;
        let Some(method) = InstallMethod::detect() else {
            self.set_state(EngineState::Missing);
            self.set_message("Install uv (docs.astral.sh/uv) or Python 3.11 first, then try again");
            return Err(AiError::NoInstaller);
        };
        std::fs::create_dir_all(&paths.root)?;
        std::fs::create_dir_all(paths.model_dir())?;
        lock(&self.inner.log).clear();
        self.set_state(EngineState::Installing);
        self.set_message("Installing the AI engine (about 1.5 GB)…");
        let mut outcome = Ok(());
        for step in install_steps(&method, paths, device) {
            if let Err(e) = self.run_step(&step).await {
                outcome = Err(e);
                break;
            }
        }
        if outcome.is_ok() && paths.is_installed() {
            self.set_state(EngineState::Stopped);
            self.set_message("AI engine installed");
            Ok(())
        } else {
            self.set_state(EngineState::Missing);
            self.set_message("Installing the AI engine failed; see the log");
            outcome.and(Err(AiError::NotInstalled))
        }
    }

    /// Starts `iopaint start` with `model` on a free loopback port and waits until
    /// `/api/v1/server-config` answers (the first start of a model downloads it). Returns the
    /// server's configuration; the engine is then `Ready`. A second call while running is a no-op
    /// that returns the current configuration.
    pub async fn start(
        &self,
        model: &str,
        device: Device,
        plugins: &[Plugin],
    ) -> Result<ServerConfig> {
        if self.is_running() {
            if let Some(config) = self.config() {
                return Ok(config);
            }
            return Err(AiError::Busy("Start".into()));
        }
        let paths = &self.inner.paths;
        if !paths.is_installed() {
            self.set_state(EngineState::Missing);
            self.set_message("The AI engine (IOPaint) is not installed");
            return Err(AiError::NotInstalled);
        }
        let port = match free_port() {
            Ok(port) => port,
            Err(e) => {
                self.set_state(EngineState::Error(
                    "No free local port for the AI engine".into(),
                ));
                self.set_message("No free local port for the AI engine");
                return Err(e.into());
            }
        };
        std::fs::create_dir_all(paths.model_dir())?;
        let remove_bg_model = self.remove_bg_model();
        let args = server_arguments(
            port,
            model,
            device,
            &paths.model_dir(),
            plugins,
            &remove_bg_model,
        );
        let exchange = FileExchange::new(paths.exchange_dir());
        let (program, args) = match self.write_shim() {
            Ok(shim) => {
                let mut argv = vec![s(&shim)];
                argv.extend(args);
                (paths.python(), argv)
            }
            Err(e) => {
                self.append_log(&format!("file transport off: {e}"));
                (paths.iopaint_executable(), args)
            }
        };
        *lock(&self.inner.shim_has_files) = false;
        let mut child = quiet_command(program)
            .args(&args)
            .env("PYTHONUTF8", "1")
            .env("PYTHONUNBUFFERED", "1")
            .env(EXCHANGE_DIR_ENV, exchange.dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                let reason = format!("could not start the AI engine: {e}");
                self.set_state(EngineState::Error(reason.clone()));
                self.set_message(reason.clone());
                AiError::Process(reason)
            })?;
        if let Some(out) = child.stdout.take() {
            self.forward_output(out);
        }
        if let Some(err) = child.stderr.take() {
            self.forward_output(err);
        }
        *lock(&self.inner.running) = Some(Running { child, port });
        *lock(&self.inner.config) = None;
        self.set_state(EngineState::Starting);
        self.set_message(format!(
            "Starting the AI engine with {model} (the first start downloads it)…"
        ));

        let client = Client::for_port(port)?;
        for _ in 0..MAX_READY_POLLS {
            tokio::time::sleep(READY_POLL_INTERVAL).await;
            if let Some(status) = self.try_wait() {
                self.finish_stopped(status);
                return Err(AiError::NotReadyInTime);
            }
            if self.state() != EngineState::Starting {
                // Stopped from elsewhere while we waited.
                return Err(AiError::NotReadyInTime);
            }
            let probe = tokio::time::timeout(Duration::from_secs(5), client.server_config()).await;
            if let Ok(Ok(config)) = probe {
                let has_files =
                    tokio::time::timeout(Duration::from_secs(5), client.has_file_transport())
                        .await
                        .unwrap_or(false);
                *lock(&self.inner.shim_has_files) = has_files;
                *lock(&self.inner.config) = Some(config.clone());
                *lock(&self.inner.current_model) = Some(model.to_owned());
                self.set_state(EngineState::Ready);
                self.set_message(format!(
                    "AI engine ready · {model} · {} transport",
                    if has_files && self.transport_mode() == TransportMode::File {
                        "file"
                    } else {
                        "base64"
                    }
                ));
                self.watch_exit();
                return Ok(config);
            }
        }
        self.stop().await;
        self.set_state(EngineState::Error(
            "The AI engine did not become ready".into(),
        ));
        self.set_message("The AI engine did not become ready");
        Err(AiError::NotReadyInTime)
    }

    /// Writes the shim next to the venv so the start can run IOPaint through it. Fails when the
    /// venv has no interpreter or the folder is not writable, in which case the plain entry point
    /// is used.
    fn write_shim(&self) -> Result<PathBuf> {
        let paths = &self.inner.paths;
        if !paths.python().is_file() {
            return Err(AiError::Process(
                "the AI engine's interpreter is missing".into(),
            ));
        }
        let shim = paths.shim_path();
        if std::fs::read_to_string(&shim).ok().as_deref() != Some(SHIM_SOURCE) {
            std::fs::write(&shim, SHIM_SOURCE)?;
        }
        Ok(shim)
    }

    /// Exit status if the child has exited.
    fn try_wait(&self) -> Option<Option<i32>> {
        let mut running = lock(&self.inner.running);
        let r = running.as_mut()?;
        match r.child.try_wait() {
            Ok(Some(status)) => {
                running.take();
                Some(status.code())
            }
            Ok(None) => None,
            Err(_) => {
                running.take();
                Some(None)
            }
        }
    }

    fn finish_stopped(&self, code: Option<i32>) {
        *lock(&self.inner.config) = None;
        *lock(&self.inner.current_model) = None;
        *lock(&self.inner.shim_has_files) = false;
        FileExchange::new(self.inner.paths.exchange_dir()).clear();
        if self.state() == EngineState::Installing {
            return;
        }
        self.set_state(EngineState::Stopped);
        match code {
            Some(0) | None => {}
            Some(code) => self.set_message(format!(
                "The AI engine stopped (exit code {code}); see the log"
            )),
        }
    }

    /// After ready: notice the server dying on its own.
    fn watch_exit(&self) {
        let engine = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if !engine.is_running() {
                    break;
                }
                if let Some(code) = engine.try_wait() {
                    engine.finish_stopped(code);
                    break;
                }
            }
        });
    }

    /// Terminates the server (3 s grace, then kill). `Stopped` afterwards unless installing or
    /// missing.
    pub async fn stop(&self) {
        let running = lock(&self.inner.running).take();
        if let Some(mut r) = running {
            let _ = r.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(3), r.child.wait()).await;
        }
        *lock(&self.inner.config) = None;
        *lock(&self.inner.current_model) = None;
        *lock(&self.inner.shim_has_files) = false;
        FileExchange::new(self.inner.paths.exchange_dir()).clear();
        if !matches!(self.state(), EngineState::Installing | EngineState::Missing) {
            self.set_state(EngineState::Stopped);
        }
    }

    /// Runs one request against the server, holding `Busy` while it is in flight and showing
    /// `busy_text`. Fails with [`AiError::NotReady`] / [`AiError::Busy`] (naming `action`) when the
    /// engine cannot take a request.
    pub async fn request<T, F>(
        &self,
        action: &str,
        busy_text: impl Into<String>,
        f: impl FnOnce(Client) -> F,
    ) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        match self.state() {
            EngineState::Ready => {}
            EngineState::Busy => {
                self.set_message(format!("{action}: the AI engine is still working"));
                return Err(AiError::Busy(action.to_owned()));
            }
            _ => {
                self.set_message(format!("{action}: start the AI engine first"));
                return Err(AiError::NotReady(action.to_owned()));
            }
        }
        let Some(client) = self.client() else {
            return Err(AiError::NotReady(action.to_owned()));
        };
        self.set_state(EngineState::Busy);
        self.set_message(busy_text);
        let result = f(client).await;
        if self.state() == EngineState::Busy {
            self.set_state(if self.is_running() {
                EngineState::Ready
            } else {
                EngineState::Stopped
            });
        }
        if let Err(e) = &result {
            self.set_message(e.to_string());
        }
        result
    }

    /// Switches the server to a downloaded model in place; a model the server has not downloaded
    /// is loaded by restarting with it (`iopaint start` downloads the model it starts with).
    pub async fn set_model(&self, name: &str, device: Device, plugins: &[Plugin]) -> Result<()> {
        let downloaded = self.config().is_some_and(|c| c.has_model(name));
        if self.state() == EngineState::Ready && downloaded {
            self.request("Model", format!("Loading {name}…"), |client| async move {
                client.set_model(name).await
            })
            .await?;
            *lock(&self.inner.current_model) = Some(name.to_owned());
            self.set_message(format!("AI engine ready · {name}"));
            return Ok(());
        }
        if self.is_running() {
            self.stop().await;
            self.start(name, device, plugins).await?;
        }
        Ok(())
    }

    /// `iopaint run` over a folder: every image with a same-named mask in `masks`. Runs the
    /// installed CLI as a step (the server need not be running). Log lines stream as usual.
    pub async fn run_batch(
        &self,
        model: &str,
        device: Device,
        images: &Path,
        masks: &Path,
        output: &Path,
    ) -> Result<()> {
        let paths = &self.inner.paths;
        if !paths.is_installed() {
            self.set_message("Batch: install the AI engine first");
            return Err(AiError::NotInstalled);
        }
        if !images.is_dir() || !masks.exists() || output.as_os_str().is_empty() {
            self.set_message(
                "Batch: choose an image folder, a mask folder or file, and an output folder",
            );
            return Err(AiError::Geometry(
                "batch needs an image folder, a mask folder or file, and an output folder".into(),
            ));
        }
        std::fs::create_dir_all(output)?;
        let mut argv = vec![s(&paths.iopaint_executable())];
        argv.extend(batch_arguments(
            model,
            device,
            images,
            masks,
            output,
            &paths.model_dir(),
        ));
        self.set_message(format!("Batch: erasing with {model}…"));
        match self.run_step(&argv).await {
            Ok(()) => {
                self.set_message(format!("Batch finished · results in {}", output.display()));
                Ok(())
            }
            Err(e) => {
                self.set_message("Batch failed; see the log");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn paths(os: &str) -> EnginePaths {
        EnginePaths::for_os("/data/app", os)
    }

    #[test]
    fn server_arguments_table() {
        let model_dir = PathBuf::from("/data/app/iopaint/models");
        let base = |device: &str, port: &str| {
            vec![
                "start",
                "--host",
                "127.0.0.1",
                "--port",
                port,
                "--model",
                "lama",
                "--device",
                device,
                "--model-dir",
                "/data/app/iopaint/models",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        };
        let cases: Vec<(&[Plugin], Device, Vec<&str>)> = vec![
            (&[], Device::Cpu, vec![]),
            (
                &[Plugin::InteractiveSeg],
                Device::Cpu,
                vec![
                    "--enable-interactive-seg",
                    "--interactive-seg-model",
                    "mobile_sam",
                    "--interactive-seg-device",
                    "cpu",
                ],
            ),
            (
                &[Plugin::RemoveBG],
                Device::Cuda,
                vec![
                    "--enable-remove-bg",
                    "--remove-bg-device",
                    "cuda",
                    "--remove-bg-model",
                    "isnet-general-use",
                ],
            ),
            (&[Plugin::AnimeSeg], Device::Cpu, vec!["--enable-anime-seg"]),
            (
                &[Plugin::RealESRGAN],
                Device::Mps,
                vec!["--enable-realesrgan", "--realesrgan-device", "mps"],
            ),
            (
                &[Plugin::GFPGAN],
                Device::Cpu,
                vec!["--enable-gfpgan", "--gfpgan-device", "cpu"],
            ),
            (
                &[Plugin::RestoreFormer],
                Device::Cpu,
                vec!["--enable-restoreformer", "--restoreformer-device", "cpu"],
            ),
        ];
        for (plugins, device, extra) in cases {
            let args = server_arguments(
                8123,
                "lama",
                device,
                &model_dir,
                plugins,
                "isnet-general-use",
            );
            let mut expected = base(device.as_str(), "8123");
            expected.extend(extra.iter().map(|s| (*s).to_owned()));
            assert_eq!(args, expected, "{plugins:?}");
        }
        // Order is fixed by the plugin order, not the caller's.
        let all = server_arguments(1, "lama", Device::Cpu, &model_dir, &Plugin::ALL, "u2net");
        let seg = all.iter().position(|a| a == "--enable-interactive-seg");
        let rf = all.iter().position(|a| a == "--enable-restoreformer");
        assert!(seg < rf);
        assert!(!all.iter().any(|a| a == "--input"));
        assert!(all.iter().any(|a| a == "u2net"));
    }

    #[test]
    fn batch_arguments_shape() {
        let args = batch_arguments(
            "lama",
            Device::Cpu,
            Path::new("/in"),
            Path::new("/masks"),
            Path::new("/out"),
            Path::new("/models"),
        );
        assert_eq!(
            args,
            vec![
                "run",
                "--model",
                "lama",
                "--device",
                "cpu",
                "--image",
                "/in",
                "--mask",
                "/masks",
                "--output",
                "/out",
                "--model-dir",
                "/models"
            ]
        );
    }

    #[test]
    fn install_steps_uv() {
        let steps = install_steps(
            &InstallMethod::Uv(PathBuf::from("/usr/bin/uv")),
            &paths("linux"),
            Device::Cpu,
        );
        assert_eq!(steps.len(), 3);
        assert_eq!(
            steps[0],
            vec![
                "/usr/bin/uv",
                "venv",
                "--python",
                "3.11",
                "/data/app/iopaint/env"
            ]
        );
        assert_eq!(
            steps[1],
            vec![
                "/usr/bin/uv",
                "pip",
                "install",
                "--python",
                "/data/app/iopaint/env/bin/python",
                "--index-url",
                "https://download.pytorch.org/whl/cpu",
                "torch==2.5.1",
                "torchvision==0.20.1"
            ]
        );
        assert_eq!(
            &steps[2][5..],
            &[
                "iopaint==1.6.0",
                "rembg[cpu]==2.0.57",
                "pillow==9.5.0",
                "numpy<2"
            ]
        );
    }

    #[test]
    fn install_steps_python_fallback_and_cuda() {
        let steps = install_steps(
            &InstallMethod::Python(vec!["py".into(), "-3.11".into()]),
            &paths("windows"),
            Device::Cuda,
        );
        assert_eq!(steps.len(), 4);
        let env = paths("windows").env_dir().to_string_lossy().into_owned();
        assert_eq!(steps[0], vec!["py", "-3.11", "-m", "venv", env.as_str()]);
        let python = paths("windows").python().to_string_lossy().into_owned();
        assert!(python.ends_with("python.exe"));
        assert_eq!(
            steps[1][..4],
            [python.clone(), "-m".into(), "pip".into(), "install".into()]
        );
        assert_eq!(&steps[1][4..], &["--upgrade", "pip"]);
        assert!(steps[2].contains(&"https://download.pytorch.org/whl/cu121".to_owned()));
        assert!(steps[3].contains(&"iopaint==1.6.0".to_owned()));
    }

    #[test]
    fn macos_takes_no_torch_index() {
        assert_eq!(torch_index_url(Device::Cpu, "macos"), None);
        assert_eq!(torch_index_url(Device::Cuda, "macos"), None);
        assert_eq!(
            torch_index_url(Device::Cpu, "linux"),
            Some("https://download.pytorch.org/whl/cpu")
        );
        let steps = install_steps(
            &InstallMethod::Uv(PathBuf::from("uv")),
            &paths("macos"),
            Device::Mps,
        );
        assert!(!steps[1].iter().any(|a| a == "--index-url"));
    }

    #[test]
    fn paths_follow_the_platform() {
        let linux = paths("linux");
        assert!(
            linux
                .iopaint_executable()
                .ends_with("iopaint/env/bin/iopaint")
        );
        assert!(linux.model_dir().ends_with("iopaint/models"));
        let win = paths("windows");
        assert!(win.iopaint_executable().ends_with("iopaint.exe"));
        assert!(win.python().to_string_lossy().contains("Scripts"));
        assert!(!linux.is_installed());
        assert!(linux.shim_path().ends_with("iopaint/redrob_shim.py"));
        assert!(win.exchange_dir().ends_with("iopaint/exchange"));
        let shm = linux.exchange_dir();
        assert!(
            shm.starts_with("/dev/shm") || shm.ends_with("iopaint/exchange"),
            "{shm:?}"
        );
        assert!(SHIM_SOURCE.contains("/api/v1/redrob/transport"));
    }

    #[test]
    fn free_port_is_bindable() {
        let port = free_port().unwrap();
        assert!(port > 0);
        // Released: we can bind it again right away.
        std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    #[test]
    fn plugin_and_device_names_round_trip() {
        for p in Plugin::ALL {
            assert_eq!(Plugin::parse(p.name()), Some(p));
        }
        for d in Device::ALL {
            assert_eq!(Device::parse(d.as_str()), Some(d));
        }
        assert_eq!(Device::parse("tpu"), None);
        assert_eq!(
            Plugin::parse_list(&["RemoveBG", "Nope", "RemoveBG", "GFPGAN"]),
            vec![Plugin::RemoveBG, Plugin::GFPGAN]
        );
        assert_eq!(InstallMethod::Uv(PathBuf::new()).label(), "uv");
        assert_eq!(EngineState::Error("x".into()).as_str(), "error");
        assert!(EngineState::Starting.is_busy());
    }

    #[tokio::test]
    async fn engine_states_without_an_install() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(EnginePaths::new(dir.path()));
        assert_eq!(engine.state(), EngineState::Missing);
        assert!(engine.client().is_none());
        assert_eq!(engine.transport_mode(), TransportMode::File);
        assert_eq!(engine.transport_in_use(), TransportMode::Base64);
        engine.set_transport_mode(TransportMode::Base64);
        assert_eq!(engine.transport_mode(), TransportMode::Base64);
        assert_eq!(engine.remove_bg_model(), "isnet-general-use");
        engine.set_remove_bg_model("u2net").await.unwrap();
        assert_eq!(engine.remove_bg_model(), "u2net");
        assert_eq!(engine.current_model(), None);
        assert!(matches!(
            engine.start("lama", Device::Cpu, &[]).await,
            Err(AiError::NotInstalled)
        ));
        assert!(matches!(
            engine.request("AI fill", "x", |_| async { Ok(()) }).await,
            Err(AiError::NotReady(_))
        ));
        assert_eq!(engine.message(), "AI fill: start the AI engine first");
        let mut log_rx = engine.subscribe_log();
        engine.append_log("downloading 10%\rdownloading 20%\r  downloading 30%  ");
        engine.append_log("   ");
        assert_eq!(log_rx.try_recv().unwrap(), "downloading 30%");
        assert_eq!(engine.log(), vec!["downloading 30%"]);
    }

    #[tokio::test]
    async fn a_failing_step_is_reported_with_its_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(EnginePaths::new(dir.path()));
        let err = engine
            .run_step(&["/definitely/not/a/program".to_owned()])
            .await
            .unwrap_err();
        assert!(matches!(err, AiError::Process(_)));
        assert!(
            engine
                .log()
                .iter()
                .any(|l| l.starts_with("could not start"))
        );
    }
}
