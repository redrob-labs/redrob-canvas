// SPDX-License-Identifier: MIT OR Apache-2.0

//! The client against a fake shared daemon: a tiny HTTP server on 127.0.0.1, a `server.json`
//! record in a private `REDROB_RUN_DIR`, and a stub `redrob` binary (`REDROB_CODE_BIN`) that
//! prints a record the way `redrob service start --json` does. These prove the contract an app
//! must keep (redrob-code docs/LOCAL-ENGINE-API.md section 3): chat hits
//! `<daemon>/v1/chat/completions` with the daemon's Authorization and `x-redrob-app`; a 401 against
//! a stale secret re-reads the record once and retries; a missing engine renders the install
//! sentence; and no request ever carries a console key.
//!
//! The environment is process-wide, so every test here takes one lock.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::Duration,
};

use redrob_agent::{
    AccountStatus, ApiKey, ChatMessage, ChatRequest, EngineConfig, Error, INSTALL_URL,
    NOT_INSTALLED_MESSAGE, RedrobClient,
};
use redrob_engine_client::{Record, Unavailable};
use serde_json::{Value, json};

static ENVIRONMENT: Mutex<()> = Mutex::new(());

/// One request as the fake daemon saw it.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// What the fake daemon answers with: `(status, content-type, body)`.
type Answer = (u16, &'static str, String);

struct FakeDaemon {
    url: String,
    secret: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeDaemon {
    /// Serves `/api/health` and, with the right secret, whatever `answer` says; the wrong secret
    /// is 401 everywhere, as the real daemon does.
    fn start(secret: &str, answer: impl Fn(&Seen) -> Answer + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let expected = redrob_engine_client::authorization(secret);
        let log = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                log.lock().unwrap().push(request.clone());
                let (status, content_type, body) =
                    if request.header("authorization") != Some(expected.as_str()) {
                        (
                            401,
                            "application/json",
                            json!({"error": {"message": "unauthorized"}}).to_string(),
                        )
                    } else if request.path == "/api/health" {
                        (
                            200,
                            "application/json",
                            json!({"healthy": true}).to_string(),
                        )
                    } else {
                        answer(&request)
                    };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.flush();
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            secret: secret.into(),
            seen,
        }
    }

    fn record(&self) -> Record {
        Record {
            id: format!("id-{}", self.secret),
            version: "0.6.2".into(),
            url: self.url.clone(),
            port: self.url.rsplit(':').next().unwrap().parse().unwrap(),
            pid: std::process::id(),
            secret: self.secret.clone(),
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Seen {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn sse_reply(text: &str) -> Answer {
    let chunk = json!({"choices": [{"delta": {"content": text}, "finish_reason": "stop"}]});
    (
        200,
        "text/event-stream",
        format!("data: {chunk}\n\ndata: [DONE]\n\n"),
    )
}

/// A private run dir and engine binary for one test; the lock is held for its lifetime.
struct Environment {
    _lock: MutexGuard<'static, ()>,
    dir: PathBuf,
}

impl Environment {
    fn new(name: &str) -> Self {
        let lock = ENVIRONMENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!(
            "redrob-agent-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: the lock above makes this the only test touching the environment.
        unsafe {
            std::env::set_var("REDROB_RUN_DIR", &dir);
            std::env::set_var("REDROB_CODE_BIN", dir.join("missing-redrob"));
        }
        Self { _lock: lock, dir }
    }

    fn write_record(&self, record: &Record) {
        std::fs::write(
            self.dir.join("server.json"),
            serde_json::to_string(record).unwrap(),
        )
        .unwrap();
    }

    /// A stand-in for `redrob` whose `service start --json` prints `record`, so the lookup that
    /// follows a stale record goes through the engine's own start path.
    #[cfg(unix)]
    fn install_stub_engine(&self, record: &Record) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.dir.join("redrob");
        let json = serde_json::to_string(record)
            .unwrap()
            .replace('\'', "'\\''");
        let script = format!("{}\nprintf '%s\\n' '{json}'\n", "#!".to_owned() + "/bin/sh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: see `new`.
        unsafe { std::env::set_var("REDROB_CODE_BIN", &path) };
    }

    fn record_on_disk(&self) -> Option<Record> {
        redrob_engine_client::read_record_at(&self.dir.join("server.json"))
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        // SAFETY: still under the lock.
        unsafe {
            std::env::remove_var("REDROB_RUN_DIR");
            std::env::remove_var("REDROB_CODE_BIN");
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn config() -> EngineConfig {
    EngineConfig::new("canvas")
        .unwrap()
        .with_request_timeout(Duration::from_secs(5))
}

fn chat() -> ChatRequest {
    ChatRequest::new(vec![ChatMessage::user("hello")])
}

#[cfg(unix)]
#[test]
fn chat_hits_the_daemon_v1_route_with_its_authorization_app_and_session() {
    let env = Environment::new("chat");
    let daemon = FakeDaemon::start("s3cret", |seen| {
        assert_eq!(seen.method, "POST");
        sse_reply("hi")
    });
    env.write_record(&daemon.record());
    // `require_as` asks whether an engine is installed before it reads the record.
    env.install_stub_engine(&daemon.record());

    let client = RedrobClient::new(config().with_session("doc-42")).unwrap();
    let response = runtime().block_on(client.complete_chat(chat())).unwrap();
    assert_eq!(response.content.as_deref(), Some("hi"));

    let seen = daemon.seen();
    let health = &seen[0];
    assert_eq!(
        health.path, "/api/health",
        "the record is probed before it is trusted"
    );
    let request = seen.last().unwrap();
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(
        request.header("authorization"),
        Some(redrob_engine_client::authorization("s3cret").as_str())
    );
    assert_eq!(request.header("x-redrob-app"), Some("canvas"));
    assert_eq!(request.header("x-redrob-session"), Some("doc-42"));
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(body["model"], "auto");
    assert_eq!(body["stream"], true);
    for request in &seen {
        assert!(
            !request
                .headers
                .values()
                .any(|value| value.starts_with("Bearer ")),
            "no request may carry a console key"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_401_against_a_stale_secret_re_reads_the_record_once_and_retries() {
    let env = Environment::new("rotate");
    let daemon = FakeDaemon::start("fresh", |_| sse_reply("after restart"));
    // The record on disk and in the client's hand are both stale: the daemon restarted with a new
    // secret. The engine's own `service start` is what yields the fresh record.
    let stale = Record {
        secret: "stale".into(),
        ..daemon.record()
    };
    env.write_record(&stale);
    env.install_stub_engine(&daemon.record());

    let client = RedrobClient::with_record(config(), stale).unwrap();
    let response = runtime().block_on(client.complete_chat(chat())).unwrap();
    assert_eq!(response.content.as_deref(), Some("after restart"));

    let seen = daemon.seen();
    let chat_requests: Vec<&Seen> = seen
        .iter()
        .filter(|seen| seen.path == "/v1/chat/completions")
        .collect();
    assert_eq!(chat_requests.len(), 2, "one failed try, one retry");
    assert_eq!(
        chat_requests[0].header("authorization"),
        Some(redrob_engine_client::authorization("stale").as_str())
    );
    assert_eq!(
        chat_requests[1].header("authorization"),
        Some(redrob_engine_client::authorization("fresh").as_str())
    );
}

#[test]
fn a_401_that_persists_is_reported_not_retried_forever() {
    let env = Environment::new("persist");
    let daemon = FakeDaemon::start("right", |_| sse_reply("never"));
    env.write_record(&daemon.record());
    // No stub engine: `REDROB_CODE_BIN` points at nothing, so after the 401 the re-read reports
    // the engine missing rather than looping.
    let client = RedrobClient::with_record(
        config(),
        Record {
            secret: "wrong".into(),
            ..daemon.record()
        },
    )
    .unwrap();
    let error = runtime()
        .block_on(client.complete_chat(chat()))
        .unwrap_err();
    assert!(matches!(
        error,
        Error::EngineUnavailable(Unavailable::NotInstalled)
    ));
    let chats = daemon
        .seen()
        .iter()
        .filter(|seen| seen.path == "/v1/chat/completions")
        .count();
    assert_eq!(chats, 1);
}

#[test]
fn a_missing_engine_renders_the_install_sentence_with_the_launcher_link() {
    let env = Environment::new("missing");
    assert!(!env.dir.join("missing-redrob").exists());
    let client = RedrobClient::new(config()).unwrap();
    let error = runtime()
        .block_on(client.complete_chat(chat()))
        .unwrap_err();
    assert!(matches!(
        error,
        Error::EngineUnavailable(Unavailable::NotInstalled)
    ));
    let message = error.to_string();
    assert!(message.starts_with(NOT_INSTALLED_MESSAGE), "{message}");
    assert!(message.contains(INSTALL_URL), "{message}");
    assert!(env.record_on_disk().is_none(), "nothing was written");
}

#[cfg(unix)]
#[test]
fn a_start_that_fails_reports_the_engine_reason() {
    use std::os::unix::fs::PermissionsExt;
    let env = Environment::new("startfail");
    let path = env.dir.join("redrob");
    std::fs::write(
        &path,
        format!(
            "{}\necho 'port 4096 is busy' >&2\nexit 7\n",
            "#!".to_owned() + "/bin/sh"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: under the environment lock.
    unsafe { std::env::set_var("REDROB_CODE_BIN", &path) };

    let client = RedrobClient::new(config()).unwrap();
    let error = runtime().block_on(client.connect()).unwrap_err();
    match error {
        Error::EngineUnavailable(Unavailable::StartFailed(reason)) => {
            assert!(reason.contains("port 4096 is busy"), "{reason}");
        }
        other => panic!("expected StartFailed, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn the_daemon_without_a_console_key_is_told_apart_by_its_code() {
    let env = Environment::new("notauth");
    let daemon = FakeDaemon::start("s", |_| {
        (
            401,
            "application/json",
            json!({"error": {"message": "sign in first", "code": "engine_not_authenticated"}})
                .to_string(),
        )
    });
    env.write_record(&daemon.record());
    env.install_stub_engine(&daemon.record());
    let client = RedrobClient::new(config()).unwrap();
    let error = runtime()
        .block_on(client.complete_chat(chat()))
        .unwrap_err();
    assert!(matches!(error, Error::EngineNotAuthenticated), "{error:?}");
}

#[cfg(unix)]
#[test]
fn sign_in_puts_the_key_to_the_daemon_and_status_reads_the_provider_list() {
    let env = Environment::new("signin");
    let daemon = FakeDaemon::start("s", |seen| {
        match (seen.method.as_str(), seen.path.as_str()) {
            ("PUT", "/auth/redrob") => (200, "application/json", "true".into()),
            ("DELETE", "/auth/redrob") => (200, "application/json", "true".into()),
            ("GET", "/provider") => (
                200,
                "application/json",
                json!({"all": [{"id": "redrob", "source": "launcher"}]}).to_string(),
            ),
            _ => (404, "text/plain", String::new()),
        }
    });
    env.write_record(&daemon.record());
    env.install_stub_engine(&daemon.record());
    let client = RedrobClient::new(config()).unwrap();
    let runtime = runtime();

    runtime
        .block_on(client.sign_in(ApiKey::new("rrk_test_secret").unwrap()))
        .unwrap();
    let put = daemon
        .seen()
        .into_iter()
        .find(|seen| seen.method == "PUT")
        .unwrap();
    assert_eq!(put.path, "/auth/redrob");
    assert_eq!(put.header("x-redrob-app"), Some("canvas"));
    let body: Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body, json!({"type": "api", "key": "rrk_test_secret"}));

    let status = runtime.block_on(client.account_status()).unwrap();
    assert_eq!(
        status,
        AccountStatus {
            connected: true,
            source: Some("launcher".into())
        }
    );
    assert!(status.from_launcher());

    runtime.block_on(client.sign_out()).unwrap();
    assert!(daemon.seen().iter().any(|seen| seen.method == "DELETE"));
}
