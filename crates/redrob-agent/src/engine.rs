// SPDX-License-Identifier: MIT OR Apache-2.0

//! The shared Redrob Code daemon as this crate reaches it.
//!
//! One `redrob serve --register` runs per user and every Redrob app attaches to it (redrob-code
//! `docs/LOCAL-ENGINE-API.md`, section 3). The daemon holds the Console key and calls the Console
//! with its own credential; this app sends the daemon's Basic secret, its app name and its session
//! id on every chat request and never sees a Console key on that path. `redrob-engine-client` finds
//! the record and starts the daemon when none answers; this module wraps its blocking calls for
//! the async client and names the two ways the engine can be missing.

use std::{fmt, time::Duration};

use redrob_engine_client::{Record, Unavailable};
use serde_json::Value;

use crate::{Error, Result};

/// Where a person gets Redrob Code when it is not installed: the launcher installs it.
pub const INSTALL_URL: &str = "https://github.com/redrob-labs/redrob-launcher/releases/latest";

/// The sentence shown when no `redrob` binary exists. The link is [`INSTALL_URL`].
pub const NOT_INSTALLED_MESSAGE: &str =
    "Redrob Code is not installed. Install it with Redrob Launcher.";

/// The provider id the daemon lists the Redrob console under (`GET /provider`, `PUT /auth/redrob`).
pub const REDROB_PROVIDER_ID: &str = "redrob";

/// The daemon lists a credential-less Redrob provider with this placeholder; it is not a key.
const PUBLIC_SENTINEL: &str = "public";

/// How long a daemon started by this app stays up with nobody attached.
pub const DEFAULT_IDLE_EXIT_MINUTES: u32 = 10;

/// What the app renders for a missing engine: the install sentence with its link, or the start
/// failure's own reason. No fallback is named because there is none.
pub fn unavailable_message(unavailable: &Unavailable) -> String {
    match unavailable {
        Unavailable::NotInstalled => format!("{NOT_INSTALLED_MESSAGE} {INSTALL_URL}"),
        Unavailable::StartFailed(reason) => {
            format!("The Redrob Code engine could not be started: {reason}")
        }
    }
}

/// How this app presents itself to the shared daemon.
#[derive(Clone)]
pub struct EngineConfig {
    pub(crate) app: String,
    pub(crate) session: Option<String>,
    pub(crate) idle_exit_minutes: u32,
    pub(crate) request_timeout: Duration,
}

impl EngineConfig {
    /// `app` is the name the daemon labels this app's requests with (`x-redrob-app`), the same
    /// id the Console knows the product by.
    pub fn new(app: impl Into<String>) -> Result<Self> {
        let app = app.into();
        if app.trim().is_empty() || app.contains(['\r', '\n']) {
            return Err(Error::InvalidConfiguration(
                "engine app name must be a non-empty single line".into(),
            ));
        }
        Ok(Self {
            app,
            session: None,
            idle_exit_minutes: DEFAULT_IDLE_EXIT_MINUTES,
            request_timeout: Duration::from_secs(60),
        })
    }

    /// The caller's own session id (`x-redrob-session`): a document id, a chat id. The daemon
    /// forwards it so the Console groups this app's requests the way its own agent runner's are.
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        let session = session.into();
        self.session = (!session.trim().is_empty()).then_some(session);
        self
    }

    pub fn with_idle_exit_minutes(mut self, minutes: u32) -> Self {
        self.idle_exit_minutes = minutes;
        self
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }
}

impl fmt::Debug for EngineConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EngineConfig")
            .field("app", &self.app)
            .field("session", &self.session)
            .field("idle_exit_minutes", &self.idle_exit_minutes)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// Whether the daemon is signed in to the Redrob console, from its provider list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountStatus {
    pub connected: bool,
    /// Where the daemon's key came from when it has one: `api` (handed over by an app), `env`,
    /// `config`, or `launcher` (Redrob Launcher's shared sign-in, which the engine reads itself).
    pub source: Option<String>,
}

impl AccountStatus {
    pub const fn signed_out() -> Self {
        Self {
            connected: false,
            source: None,
        }
    }

    /// `true` when the key in use is Redrob Launcher's, so the app says so and offers no sign-out.
    pub fn from_launcher(&self) -> bool {
        self.connected && self.source.as_deref() == Some("launcher")
    }

    /// Reads `GET /provider`: a `redrob` entry with a credential means signed in. The shape is the
    /// one Cowork and Design read (`{"all": [{"id", "source", "key", "options": {"apiKey"}}]}`).
    pub fn from_provider_list(list: &Value) -> Self {
        let Some(entry) = list.get("all").and_then(Value::as_array).and_then(|all| {
            all.iter()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(REDROB_PROVIDER_ID))
        }) else {
            return Self::signed_out();
        };
        if entry
            .pointer("/options/apiKey")
            .and_then(Value::as_str)
            .is_some_and(|key| key == PUBLIC_SENTINEL)
        {
            return Self::signed_out();
        }
        let source = entry.get("source").and_then(Value::as_str);
        let has_key = entry
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| !key.trim().is_empty());
        if !(matches!(source, Some("api" | "env" | "launcher")) || has_key) {
            return Self::signed_out();
        }
        Self {
            connected: true,
            source: Some(
                source
                    .filter(|known| matches!(*known, "api" | "env" | "config" | "launcher"))
                    .unwrap_or("api")
                    .to_owned(),
            ),
        }
    }
}

/// Finds the daemon or starts it, off the async runtime: the client crate shells out to
/// `redrob service start` and probes with blocking HTTP.
pub(crate) async fn require(config: &EngineConfig) -> Result<Record> {
    let app = config.app.clone();
    let idle = config.idle_exit_minutes;
    tokio::task::spawn_blocking(move || redrob_engine_client::require_as(&app, idle))
        .await
        .map_err(|_| Error::Protocol("the engine lookup task was cancelled".into()))?
        .map_err(Error::EngineUnavailable)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AccountStatus, EngineConfig, INSTALL_URL, unavailable_message};
    use redrob_engine_client::Unavailable;

    #[test]
    fn a_missing_engine_renders_the_install_sentence_and_the_launcher_link() {
        let message = unavailable_message(&Unavailable::NotInstalled);
        assert!(
            message.starts_with("Redrob Code is not installed. Install it with Redrob Launcher.")
        );
        assert!(message.ends_with(INSTALL_URL));
        assert_eq!(
            unavailable_message(&Unavailable::StartFailed("port busy".into())),
            "The Redrob Code engine could not be started: port busy"
        );
    }

    #[test]
    fn the_provider_list_tells_signed_in_from_public_and_absent() {
        assert_eq!(
            AccountStatus::from_provider_list(&json!({"all": []})),
            AccountStatus::signed_out()
        );
        assert_eq!(
            AccountStatus::from_provider_list(
                &json!({"all": [{"id": "redrob", "options": {"apiKey": "public"}}]})
            ),
            AccountStatus::signed_out()
        );
        assert_eq!(
            AccountStatus::from_provider_list(
                &json!({"all": [{"id": "redrob", "source": "config"}]})
            ),
            AccountStatus::signed_out(),
            "a config entry without a key is not a sign-in"
        );
        let launcher = AccountStatus::from_provider_list(
            &json!({"all": [{"id": "redrob", "source": "launcher"}]}),
        );
        assert!(launcher.connected && launcher.from_launcher());
        let api = AccountStatus::from_provider_list(
            &json!({"all": [{"id": "openai"}, {"id": "redrob", "source": "api", "key": "rrk_x"}]}),
        );
        assert_eq!(api.source.as_deref(), Some("api"));
        assert!(!api.from_launcher());
        let keyed =
            AccountStatus::from_provider_list(&json!({"all": [{"id": "redrob", "key": "rrk_x"}]}));
        assert_eq!(keyed.source.as_deref(), Some("api"));
    }

    #[test]
    fn a_blank_session_sends_nothing_and_a_blank_app_is_refused() {
        let config = EngineConfig::new("canvas").unwrap().with_session("  ");
        assert_eq!(config.session(), None);
        assert_eq!(
            EngineConfig::new("canvas")
                .unwrap()
                .with_session("doc-1")
                .session(),
            Some("doc-1")
        );
        assert!(EngineConfig::new(" ").is_err());
        assert!(EngineConfig::new("a\nb").is_err());
    }
}
