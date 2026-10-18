//! File › Open Location…, File › Create › From Screenshot… / From Web Page…, and File › Send by
//! Email… (Photo mode, v0.11 parity). Open Location fetches a URL through the shell's HTTP
//! client ([`Session::fetch_url`]) or, without one, the system's `curl`; the screenshot and
//! web-page captures run the platform's own tools (`screencapture`, `grim` / `gnome-screenshot` /
//! `spectacle` / `import` / `scrot` / `maim`, PowerShell; a headless Chromium or Firefox for a
//! page) and open the picture they write; Send by Email exports a copy and hands it to the
//! system mailer (`xdg-email`, Mail.app, or the default `mailto:` handler with the file revealed).

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The shell's HTTP fetch: URL in, bytes out.
pub type FetchUrlFn = Box<dyn Fn(&str) -> std::result::Result<Vec<u8>, String> + Send + Sync>;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Largest download Open Location accepts.
pub const MAX_DOWNLOAD: usize = 512 << 20;

fn url_param<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    let url = p.get("url").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| bad(cmd, "pass `url`"))?;
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://")) {
        return Err(bad(cmd, "`url` must start with http://, https:// or file://"));
    }
    if url.chars().any(|c| c.is_control()) {
        return Err(bad(cmd, "`url` holds control characters"));
    }
    Ok(url)
}

/// The file name a URL names (its last path segment, without the query), else `fallback`.
pub fn name_of_url(url: &str, fallback: &str) -> String {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = no_query.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let decoded = percent_decode(last);
    if decoded.is_empty() || !decoded.contains('.') || decoded.contains("://") { fallback.to_string() } else { decoded }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch(s: &Session, url: &str) -> Result<Vec<u8>> {
    if let Some(local) = url.strip_prefix("file://") {
        let path = percent_decode(local);
        let bytes = std::fs::read(&path).map_err(|e| other(format!("{path}: {e}")))?;
        return Ok(bytes);
    }
    if let Some(f) = s.fetch_url.as_ref() {
        let bytes = f(url).map_err(other)?;
        if bytes.len() > MAX_DOWNLOAD {
            return Err(other("the download is larger than 512 MiB"));
        }
        return Ok(bytes);
    }
    // No HTTP client in this session: the system's curl (shipped with macOS and Windows 10+).
    let out = std::process::Command::new("curl")
        .args(["-L", "--silent", "--show-error", "--fail", "--max-filesize", &MAX_DOWNLOAD.to_string(), "--max-time", "120", "--", url])
        .output()
        .map_err(|e| other(format!("curl: {e} (no HTTP client is available in this session)")))?;
    if !out.status.success() {
        return Err(other(format!("{url}: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(out.stdout)
}

#[cfg(target_arch = "wasm32")]
fn fetch(_s: &Session, _url: &str) -> Result<Vec<u8>> {
    Err(other("Open Location needs the desktop app"))
}

/// `file.openLocation`: opens the image a URL points at.
fn open_location(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.openLocation";
    let url = url_param(p, CMD)?.to_string();
    let bytes = fetch(s, &url)?;
    if bytes.is_empty() {
        return Err(other(format!("{url}: the download is empty")));
    }
    let name = name_of_url(&url, "Untitled.png");
    let r = crate::file_cmds::open_bytes_as(s, &name, &bytes, None, None)?;
    Ok(json!({ "url": url, "name": name, "bytes": bytes.len(), "opened": r }))
}

/// A fresh file in the temporary directory.
fn temp_file(ext: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("redrob-canvas-{}-{nanos}.{ext}", std::process::id()))
}

#[cfg(not(target_arch = "wasm32"))]
fn which(program: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| {
        let p = d.join(program);
        p.is_file() || (cfg!(windows) && (d.join(format!("{program}.exe")).is_file() || d.join(format!("{program}.cmd")).is_file()))
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_tool(program: &str, args: &[String]) -> Result<()> {
    let out = std::process::Command::new(program).args(args).output().map_err(|e| other(format!("{program}: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(other(format!("{program} failed: {}", err.trim())));
    }
    Ok(())
}

/// What a screenshot covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShotMode {
    Screen,
    Window,
    Region,
}

/// The screenshot tool invocation for this platform: (program, args) writing a PNG to `file`.
/// `None` when no tool is installed.
#[cfg(not(target_arch = "wasm32"))]
pub fn screenshot_command(mode: ShotMode, delay_s: u32, cursor: bool, file: &str) -> Option<(String, Vec<String>)> {
    let s = |v: &str| v.to_string();
    if cfg!(target_os = "macos") {
        let mut a = vec![s("-x")];
        if cursor {
            a.push(s("-C"));
        }
        if delay_s > 0 {
            a.push(s("-T"));
            a.push(delay_s.to_string());
        }
        match mode {
            ShotMode::Screen => {}
            ShotMode::Window => {
                a.push(s("-i"));
                a.push(s("-W"));
            }
            ShotMode::Region => a.push(s("-i")),
        }
        a.push(s(file));
        return Some((s("screencapture"), a));
    }
    if cfg!(target_os = "windows") {
        // PowerShell: the whole virtual screen, or the foreground window's rectangle.
        let window = mode != ShotMode::Screen;
        let script = format!(
            "Start-Sleep -Seconds {delay_s}; Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
             Add-Type -Name W -Namespace U -MemberDefinition '[DllImport(\"user32.dll\")] public static extern System.IntPtr GetForegroundWindow(); [DllImport(\"user32.dll\")] public static extern bool GetWindowRect(System.IntPtr h, out RECT r); public struct RECT {{ public int L; public int T; public int R; public int B; }}'; \
             $b = [System.Windows.Forms.SystemInformation]::VirtualScreen; \
             if ({window}) {{ $r = New-Object U.W+RECT; [void][U.W]::GetWindowRect([U.W]::GetForegroundWindow(), [ref]$r); $b = New-Object System.Drawing.Rectangle($r.L, $r.T, ($r.R - $r.L), ($r.B - $r.T)) }}; \
             $bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height); $g = [System.Drawing.Graphics]::FromImage($bmp); \
             $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $b.Size); $bmp.Save('{file}', [System.Drawing.Imaging.ImageFormat]::Png)",
            window = if window { "$true" } else { "$false" },
            file = file.replace('\'', "''")
        );
        return Some((s("powershell"), vec![s("-NoProfile"), s("-NonInteractive"), s("-Command"), script]));
    }
    // Linux and the BSDs: Wayland tools first when a Wayland session is running, then X11 ones.
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let delay = delay_s.to_string();
    if wayland && which("grim") {
        return Some(match mode {
            ShotMode::Region if which("slurp") => (s("sh"), vec![s("-c"), format!("sleep {delay}; grim -g \"$(slurp)\" '{}'", file.replace('\'', "'\\''"))]),
            _ => (s("sh"), vec![s("-c"), format!("sleep {delay}; grim {} '{}'", if cursor { "-c" } else { "" }, file.replace('\'', "'\\''"))]),
        });
    }
    if which("gnome-screenshot") {
        let mut a = vec![s("-f"), s(file), s("-d"), delay];
        match mode {
            ShotMode::Window => a.push(s("-w")),
            ShotMode::Region => a.push(s("-a")),
            ShotMode::Screen => {}
        }
        if cursor {
            a.push(s("-p"));
        }
        return Some((s("gnome-screenshot"), a));
    }
    if which("spectacle") {
        let mut a = vec![s("-b"), s("-n"), s("-o"), s(file), s("-d"), (delay_s * 1000).to_string()];
        a.push(s(match mode {
            ShotMode::Screen => "-f",
            ShotMode::Window => "-a",
            ShotMode::Region => "-r",
        }));
        if cursor {
            a.push(s("-p"));
        }
        return Some((s("spectacle"), a));
    }
    if which("maim") {
        if mode == ShotMode::Window {
            return Some((s("sh"), vec![s("-c"), format!("sleep {}; maim -i \"$(xdotool getactivewindow)\" '{}'", delay_s, file.replace('\'', "'\\''"))]));
        }
        let mut a = vec![s("-d"), delay];
        if mode == ShotMode::Region {
            a.push(s("-s"));
        }
        a.push(s(file));
        return Some((s("maim"), a));
    }
    if which("scrot") {
        let mut a = vec![s("-d"), delay];
        match mode {
            ShotMode::Window => a.push(s("-u")),
            ShotMode::Region => a.push(s("-s")),
            ShotMode::Screen => {}
        }
        if cursor {
            a.push(s("-p"));
        }
        a.push(s(file));
        return Some((s("scrot"), a));
    }
    if which("import") {
        // ImageMagick: `-window root` is the screen; without it the user clicks a window or
        // drags a region.
        let mut a = Vec::new();
        if delay_s > 0 {
            a.push(s("-pause"));
            a.push(delay);
        }
        if mode == ShotMode::Screen {
            a.push(s("-window"));
            a.push(s("root"));
        }
        a.push(s(file));
        return Some((s("import"), a));
    }
    None
}

/// `file.create.screenshot`: a new document from a screenshot.
#[cfg(not(target_arch = "wasm32"))]
fn screenshot(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.create.screenshot";
    let mode = match p.get("mode").and_then(Value::as_str).unwrap_or("screen") {
        "screen" => ShotMode::Screen,
        "window" => ShotMode::Window,
        "region" => ShotMode::Region,
        other => return Err(bad(CMD, format!("`mode` must be screen, window or region (got `{other}`)"))),
    };
    let delay = p.get("delay").and_then(Value::as_f64).map_or(0, |d| d.clamp(0.0, 60.0).round() as u32);
    let cursor = p.get("cursor").and_then(Value::as_bool).unwrap_or(false);
    let file = temp_file("png");
    let file_s = file.to_string_lossy().into_owned();
    if p.get("dryRun").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(json!({ "command": screenshot_command(mode, delay, cursor, &file_s).map(|(p, a)| json!({"program": p, "args": a})) }));
    }
    let (program, args) = screenshot_command(mode, delay, cursor, &file_s)
        .ok_or_else(|| other("no screenshot tool found (install gnome-screenshot, spectacle, grim, maim, scrot or ImageMagick)"))?;
    run_tool(&program, &args)?;
    let bytes = std::fs::read(&file).map_err(|e| other(format!("the screenshot was not written ({e}); the capture may have been cancelled")))?;
    let _ = std::fs::remove_file(&file);
    let r = crate::file_cmds::open_bytes_as(s, "Screenshot.png", &bytes, None, None)?;
    if let Some(st) = s.active_mut() {
        st.saved_revision = 0;
    }
    Ok(json!({ "mode": format!("{mode:?}").to_lowercase(), "delay": delay, "opened": r }))
}

#[cfg(target_arch = "wasm32")]
fn screenshot(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(other("screenshots need the desktop app"))
}

/// The headless browser invocation that renders `url` to a PNG at `file`, `width` pixels wide.
#[cfg(not(target_arch = "wasm32"))]
pub fn web_page_command(url: &str, width: u32, height: u32, file: &str) -> Option<(String, Vec<String>)> {
    let s = |v: &str| v.to_string();
    let chromium = [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
        "chrome",
        "brave-browser",
        "microsoft-edge",
        "msedge",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    ];
    for c in chromium {
        if which(c) || std::path::Path::new(c).is_file() {
            return Some((
                s(c),
                vec![
                    s("--headless=new"),
                    s("--disable-gpu"),
                    s("--hide-scrollbars"),
                    format!("--window-size={width},{height}"),
                    format!("--screenshot={file}"),
                    s(url),
                ],
            ));
        }
    }
    for f in ["firefox", "/Applications/Firefox.app/Contents/MacOS/firefox", "C:\\Program Files\\Mozilla Firefox\\firefox.exe"] {
        if which(f) || std::path::Path::new(f).is_file() {
            return Some((s(f), vec![s("--headless"), s("--screenshot"), s(file), format!("--window-size={width},{height}"), s(url)]));
        }
    }
    None
}

/// `file.create.webPage`: a new document from a rendered web page.
#[cfg(not(target_arch = "wasm32"))]
fn web_page(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.create.webPage";
    let url = url_param(p, CMD)?.to_string();
    let width = p.get("width").and_then(Value::as_u64).map_or(1280, |w| w.clamp(320, 8192) as u32);
    let height = p.get("height").and_then(Value::as_u64).map_or(1024, |h| h.clamp(240, 16384) as u32);
    let file = temp_file("png");
    let file_s = file.to_string_lossy().into_owned();
    if p.get("dryRun").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(json!({ "command": web_page_command(&url, width, height, &file_s).map(|(p, a)| json!({"program": p, "args": a})) }));
    }
    let (program, args) = web_page_command(&url, width, height, &file_s)
        .ok_or_else(|| other("no browser that can render a page headlessly was found (Chromium, Chrome, Edge, Brave or Firefox)"))?;
    run_tool(&program, &args)?;
    let bytes = std::fs::read(&file).map_err(|e| other(format!("the page was not rendered ({e})")))?;
    let _ = std::fs::remove_file(&file);
    let name = name_of_url(&url, "Web Page.png");
    let r = crate::file_cmds::open_bytes_as(s, &format!("{}.png", name.rsplit_once('.').map_or(name.as_str(), |(a, _)| a)), &bytes, None, None)?;
    if let Some(st) = s.active_mut() {
        st.saved_revision = 0;
    }
    Ok(json!({ "url": url, "width": width, "height": height, "opened": r }))
}

#[cfg(target_arch = "wasm32")]
fn web_page(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(other("rendering a web page needs the desktop app"))
}

/// The mailer invocation attaching `file` with `subject`: (program, args). `None` when none fits.
#[cfg(not(target_arch = "wasm32"))]
pub fn mail_command(file: &str, subject: &str) -> Option<(String, Vec<String>)> {
    let s = |v: &str| v.to_string();
    if cfg!(target_os = "macos") {
        // Mail.app: a new message with the file attached.
        let script = format!(
            "tell application \"Mail\"\nset m to make new outgoing message with properties {{subject:\"{}\", visible:true}}\ntell m to make new attachment with properties {{file name:POSIX file \"{}\"}} at after the last paragraph\nactivate\nend tell",
            subject.replace('"', "\\\""),
            file.replace('"', "\\\"")
        );
        return Some((s("osascript"), vec![s("-e"), script]));
    }
    if cfg!(target_os = "windows") {
        // The default mail client through MAPI is not reachable from a shell; open a message and
        // reveal the attachment so it can be dropped in.
        return Some((
            s("powershell"),
            vec![
                s("-NoProfile"),
                s("-Command"),
                format!("Start-Process 'mailto:?subject={}'; Start-Process explorer.exe -ArgumentList '/select,\"{}\"'", percent_encode(subject), file),
            ],
        ));
    }
    if which("xdg-email") {
        return Some((s("xdg-email"), vec![s("--attach"), s(file), s("--subject"), s(subject)]));
    }
    if which("thunderbird") {
        return Some((s("thunderbird"), vec![s("-compose"), format!("subject='{}',attachment='file://{}'", subject.replace('\'', ""), file)]));
    }
    None
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `file.sendByEmail`: exports a copy (the given format, default PNG) and opens it in a new mail.
#[cfg(not(target_arch = "wasm32"))]
fn send_by_email(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png").trim_start_matches('.').to_ascii_lowercase();
    if format.is_empty() || format.len() > 8 || !format.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(bad("file.sendByEmail", "`format` must be a file extension such as png or jpg"));
    }
    let stem = d.doc.name.rsplit_once('.').map_or(d.doc.name.as_str(), |(a, _)| a).to_string();
    let file = std::env::temp_dir()
        .join(format!("{}.{format}", stem.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect::<String>()));
    let file_s = file.to_string_lossy().into_owned();
    let save = crate::file_cmds::SaveOpts::from_params(s, p, "file.sendByEmail")?;
    let (bytes, warnings) = crate::file_cmds::encode(&d.doc, &file_s, save)?;
    crate::file_cmds::write_file(&file_s, &bytes)?;
    let subject = p.get("subject").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{stem}.{format}"));
    if p.get("dryRun").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(json!({ "file": file_s, "command": mail_command(&file_s, &subject).map(|(p, a)| json!({"program": p, "args": a})) }));
    }
    let (program, args) = mail_command(&file_s, &subject).ok_or_else(|| other("no mail client found (install xdg-email or Thunderbird)"))?;
    std::process::Command::new(&program).args(&args).spawn().map_err(|e| other(format!("{program}: {e}")))?;
    Ok(json!({ "file": file_s, "subject": subject, "warnings": warnings }))
}

#[cfg(target_arch = "wasm32")]
fn send_by_email(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(other("Send by Email needs the desktop app"))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.openLocation",
            label: "Open Location…",
            menu: &["File"],
            shortcut: None,
            params: r##"{"url":"http(s):// or file:// address of an image"} → {url, name, bytes, opened}"##,
            enabled: always,
            run: open_location,
            journal: true,
        },
        CommandSpec {
            id: "file.create.screenshot",
            label: "From Screenshot…",
            menu: &["File", "Create"],
            shortcut: None,
            params: r##"{"mode":"screen|window|region"="screen","delay":seconds=0,"cursor":bool=false} → {mode, delay, opened}"##,
            enabled: always,
            run: screenshot,
            journal: true,
        },
        CommandSpec {
            id: "file.create.webPage",
            label: "From Web Page…",
            menu: &["File", "Create"],
            shortcut: None,
            params: r##"{"url":"http(s):// address","width":px=1280,"height":px=1024} → {url, width, height, opened}"##,
            enabled: always,
            run: web_page,
            journal: true,
        },
        CommandSpec {
            id: "file.sendByEmail",
            label: "Send by Email…",
            menu: &["File"],
            shortcut: None,
            params: r##"{"format":"png|jpg|…"="png","subject":string?,"quality":0-12?} → {file, subject, warnings}"##,
            enabled: has_doc,
            run: send_by_email,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_names_and_decoding() {
        assert_eq!(name_of_url("https://x.org/a/b/photo%20one.jpg?x=1", "u.png"), "photo one.jpg");
        assert_eq!(name_of_url("https://x.org/", "u.png"), "u.png");
        assert_eq!(percent_encode("a b&c"), "a%20b%26c");
    }
}
