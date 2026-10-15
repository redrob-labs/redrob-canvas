// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures_util::{Stream, StreamExt, stream};
use redrob_engine_client::{Record, app_headers, openai_base};
use reqwest::{StatusCode, Url, header::HeaderValue};
use serde_json::Value;

use crate::{
    AccountStatus, ApiKey, AssistantResponse, CancellationToken, ChatEvent, ChatRequest,
    CompletionLimits, DeviceAuthorization, DevicePollOptions, EngineConfig, Error, FinishReason,
    Model, REDROB_PROVIDER_ID, Result, ToolCall, Usage,
    sse::SseParser,
    types::CONSOLE_BASE_URL,
    wire::{
        ChatChunkWire, ChatRequestWire, DeviceAuthorizationWire, DeviceAuthorizeRequestWire,
        DeviceTokenRequestWire, DeviceTokenWire, ModelsWire, finish_reason,
    },
};

const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const ABSOLUTE_MAX_POLL_WAIT: Duration = Duration::from_secs(60 * 60);
/// The daemon's own answer when it holds no Console credential (LOCAL-ENGINE-API.md, Errors).
const ENGINE_NOT_AUTHENTICATED: &str = "engine_not_authenticated";

/// Boxed stream returned by [`RedrobClient::stream_chat`].
pub type ChatEventStream = Pin<Box<dyn Stream<Item = Result<ChatEvent>> + Send>>;

/// Reusable async client for the shared Redrob Code daemon.
///
/// The daemon is found (or started) on the first request and its record kept. The record's secret
/// rotates on every daemon start, so a 401 or a refused connection re-reads the record once and
/// retries once; a second failure is reported. Every chat request carries the daemon's
/// `Authorization`, `x-redrob-app` and, when the app has one, `x-redrob-session`.
#[derive(Clone)]
pub struct RedrobClient {
    http: reqwest::Client,
    config: EngineConfig,
    record: Arc<Mutex<Option<Record>>>,
}

impl fmt::Debug for RedrobClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let record = self
            .record
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        formatter
            .debug_struct("RedrobClient")
            .field("config", &self.config)
            .field("daemon", &record.as_ref().map(|record| record.url.clone()))
            .finish_non_exhaustive()
    }
}

impl RedrobClient {
    /// Takes no network action: the daemon is looked up when the first request needs it.
    pub fn new(config: EngineConfig) -> Result<Self> {
        if config.request_timeout.is_zero() {
            return Err(Error::InvalidConfiguration(
                "request timeout must be greater than zero".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()?;
        Self::with_http_client(config, http)
    }

    /// Creates a client around an existing reqwest client.
    pub fn with_http_client(config: EngineConfig, http: reqwest::Client) -> Result<Self> {
        Ok(Self {
            http,
            config,
            record: Arc::new(Mutex::new(None)),
        })
    }

    /// Starts from a record already in hand (tests, or an app that resolved the daemon itself).
    /// A stale one is replaced on the first 401 exactly like a cached one.
    pub fn with_record(config: EngineConfig, record: Record) -> Result<Self> {
        let client = Self::new(config)?;
        *client.lock_record() = Some(record);
        Ok(client)
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Finds or starts the daemon now, so an app can show "not installed" before the first chat.
    pub async fn connect(&self) -> Result<Record> {
        if let Some(record) = self.lock_record().clone() {
            return Ok(record);
        }
        self.reconnect().await
    }

    async fn reconnect(&self) -> Result<Record> {
        let record = crate::engine::require(&self.config).await?;
        *self.lock_record() = Some(record.clone());
        Ok(record)
    }

    fn lock_record(&self) -> std::sync::MutexGuard<'_, Option<Record>> {
        self.record
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sends one request to the daemon, retrying once with a fresh record when the daemon has
    /// evidently restarted (connection refused, or 401 against the secret in hand).
    async fn send<F>(&self, build: F) -> Result<reqwest::Response>
    where
        F: Fn(&Record) -> Result<reqwest::RequestBuilder>,
    {
        let record = self.connect().await?;
        let first = build(&record)?.send().await;
        let retry = match &first {
            Ok(response) => response.status() == StatusCode::UNAUTHORIZED,
            Err(error) => error.is_connect() || error.is_request(),
        };
        if !retry {
            return Ok(first?);
        }
        *self.lock_record() = None;
        let record = self.reconnect().await?;
        Ok(build(&record)?.send().await?)
    }

    fn headers(
        &self,
        record: &Record,
        builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        let mut builder = builder;
        for (name, value) in app_headers(record, &self.config.app, self.config.session.as_deref()) {
            let Ok(mut header) = HeaderValue::from_str(&value) else {
                continue;
            };
            if name == "Authorization" {
                header.set_sensitive(true);
            }
            builder = builder.header(name, header);
        }
        builder
    }

    /// `<daemon>/v1/<path>`: the OpenAI-compatible route.
    fn endpoint(record: &Record, path: &str) -> Result<Url> {
        normalize_base_url(&openai_base(record))?
            .join(path)
            .map_err(|error| Error::InvalidConfiguration(format!("invalid endpoint URL: {error}")))
    }

    /// `<daemon>/<path>`: the daemon's own routes (`/provider`, `/auth/<provider>`).
    fn daemon_endpoint(record: &Record, path: &str) -> Result<Url> {
        normalize_base_url(&record.url)?
            .join(path)
            .map_err(|error| Error::InvalidConfiguration(format!("invalid endpoint URL: {error}")))
    }

    pub async fn models(&self) -> Result<Vec<Model>> {
        let response = self
            .send(|record| {
                Ok(self.headers(record, self.http.get(Self::endpoint(record, "models")?)))
            })
            .await?;
        let response = ensure_success(response).await?;
        let models: ModelsWire = response.json().await?;
        Ok(models.data.into_iter().map(Into::into).collect())
    }

    pub async fn stream_chat(&self, request: ChatRequest) -> Result<ChatEventStream> {
        if request.model.trim().is_empty() {
            return Err(Error::InvalidConfiguration(
                "chat model must not be empty".into(),
            ));
        }
        let wire = ChatRequestWire::from(request);
        let response = self
            .send(|record| {
                Ok(self.headers(
                    record,
                    self.http
                        .post(Self::endpoint(record, "chat/completions")?)
                        .json(&wire),
                ))
            })
            .await?;
        let response = ensure_success(response).await?;

        let body = Box::pin(response.bytes_stream());
        let output = stream::try_unfold(
            (
                body,
                SseParser::default(),
                VecDeque::<ChatEvent>::new(),
                false,
            ),
            |(mut body, mut parser, mut queued, mut done)| async move {
                loop {
                    if let Some(event) = queued.pop_front() {
                        return Ok(Some((event, (body, parser, queued, done))));
                    }
                    if done {
                        return Ok(None);
                    }

                    match body.next().await {
                        Some(Ok(chunk)) => {
                            for data in parser.feed(&chunk)? {
                                decode_sse_data(&data, &mut queued, &mut done)?;
                                if done {
                                    break;
                                }
                            }
                        }
                        Some(Err(error)) => return Err(Error::Http(error)),
                        None => {
                            for data in parser.finish()? {
                                decode_sse_data(&data, &mut queued, &mut done)?;
                                if done {
                                    break;
                                }
                            }
                            if queued.is_empty() && !done {
                                return Err(Error::Protocol(
                                    "chat stream ended without [DONE]".into(),
                                ));
                            }
                        }
                    }
                }
            },
        );
        Ok(Box::pin(output))
    }

    pub async fn complete_chat(&self, request: ChatRequest) -> Result<AssistantResponse> {
        self.complete_chat_with_limits(request, CompletionLimits::default())
            .await
    }

    /// Accumulates a streaming chat response under explicit finite limits.
    pub async fn complete_chat_with_limits(
        &self,
        request: ChatRequest,
        limits: CompletionLimits,
    ) -> Result<AssistantResponse> {
        let mut stream = self.stream_chat(request).await?;
        let mut accumulator = ChatAccumulator::new(limits);
        while let Some(event) = stream.next().await {
            let event = event?;
            let finished = matches!(event, ChatEvent::Done);
            accumulator.apply(event)?;
            if finished {
                return accumulator.finish();
            }
        }
        Err(Error::Protocol(
            "chat stream ended before completion".into(),
        ))
    }

    /// Whether the daemon is signed in to the Redrob console (`GET /provider`), and with whose key.
    pub async fn account_status(&self) -> Result<AccountStatus> {
        let response = self
            .send(|record| {
                Ok(self.headers(
                    record,
                    self.http.get(Self::daemon_endpoint(record, "provider")?),
                ))
            })
            .await?;
        let response = ensure_success(response).await?;
        let list: Value = response.json().await?;
        Ok(AccountStatus::from_provider_list(&list))
    }

    /// Hands a console key to the daemon (`PUT /auth/redrob`), which keeps it from here on. A
    /// second call rotates it. The key is consumed: nothing in this app holds it afterwards.
    pub async fn sign_in(&self, key: ApiKey) -> Result<()> {
        let body = serde_json::json!({"type": "api", "key": key.expose()});
        let response = self
            .send(|record| {
                Ok(self.headers(
                    record,
                    self.http
                        .put(Self::daemon_endpoint(
                            record,
                            &format!("auth/{REDROB_PROVIDER_ID}"),
                        )?)
                        .json(&body),
                ))
            })
            .await?;
        // The daemon's failure body could echo the payload; only the status is kept.
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                status: response.status(),
                body: "the engine did not accept the Redrob key".into(),
            });
        }
        Ok(())
    }

    /// Removes the console key from the daemon (`DELETE /auth/redrob`). Already gone is fine.
    pub async fn sign_out(&self) -> Result<()> {
        let response = self
            .send(|record| {
                Ok(self.headers(
                    record,
                    self.http.delete(Self::daemon_endpoint(
                        record,
                        &format!("auth/{REDROB_PROVIDER_ID}"),
                    )?),
                ))
            })
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        ensure_success(response).await.map(drop)
    }
}

/// The Redrob console's device authorization flow: the one place this crate still talks to the
/// console, because that is where a person approves a code and where the workspace key comes
/// from. The key it yields goes to the daemon through [`RedrobClient::sign_in`], never onto a chat
/// request and never into a file of this app's.
#[derive(Clone)]
pub struct DeviceFlowClient {
    http: reqwest::Client,
    base_url: Url,
}

impl fmt::Debug for DeviceFlowClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceFlowClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl DeviceFlowClient {
    /// Against the Redrob console.
    pub fn new() -> Result<Self> {
        Self::with_base_url(CONSOLE_BASE_URL, Duration::from_secs(60))
    }

    pub fn with_base_url(base_url: &str, request_timeout: Duration) -> Result<Self> {
        if request_timeout.is_zero() {
            return Err(Error::InvalidConfiguration(
                "request timeout must be greater than zero".into(),
            ));
        }
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(request_timeout)
                .build()?,
            base_url: normalize_base_url(base_url)?,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path)
            .map_err(|error| Error::InvalidConfiguration(format!("invalid endpoint URL: {error}")))
    }

    /// Starts the device authorization flow as `product`, one of the ids the console accepts
    /// (`apps/api/src/device/device-products.ts`). The console names the app by this id on its
    /// confirm screen, so each app must send its own.
    pub async fn authorize_device(&self, product: &'static str) -> Result<DeviceAuthorization> {
        let response = self
            .http
            .post(self.endpoint("device/authorize")?)
            .json(&DeviceAuthorizeRequestWire { product })
            .send()
            .await?;
        let response = ensure_success(response).await?;
        let authorization: DeviceAuthorizationWire = response.json().await?;
        if authorization.device_code.is_empty()
            || authorization.user_code.is_empty()
            || authorization.verification_uri.is_empty()
            || authorization.expires_in == 0
        {
            return Err(Error::Protocol(
                "device authorization response is missing required fields".into(),
            ));
        }
        Ok(DeviceAuthorization {
            user_code: authorization.user_code,
            verification_uri: authorization.verification_uri,
            verification_uri_complete: authorization.verification_uri_complete,
            expires_in: Duration::from_secs(authorization.expires_in),
            interval: Duration::from_secs(authorization.interval.unwrap_or(5)),
            device_code: authorization.device_code.into_boxed_str(),
            received_at: Instant::now(),
        })
    }

    /// Polls until authorization succeeds, reaches a terminal OAuth device-flow
    /// error, times out, or is cancelled.
    pub async fn poll_device_token(
        &self,
        authorization: &DeviceAuthorization,
        options: DevicePollOptions,
        cancellation: &CancellationToken,
    ) -> Result<ApiKey> {
        validate_poll_options(options)?;
        let allowed_wait = authorization
            .expires_in
            .min(options.max_wait)
            .min(ABSOLUTE_MAX_POLL_WAIT);
        let deadline = authorization
            .received_at
            .checked_add(allowed_wait)
            .ok_or(Error::AuthorizationTimedOut)?;
        let deadline_is_expiration = allowed_wait == authorization.expires_in;
        let mut interval = authorization
            .interval
            .max(options.min_interval)
            .min(options.max_interval);

        loop {
            if cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(deadline_error(deadline_is_expiration));
            }

            let token_result = tokio::select! {
                result = self.request_device_token(&authorization.device_code) => result,
                () = cancellation.cancelled() => return Err(Error::Cancelled),
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                    return Err(deadline_error(deadline_is_expiration));
                }
            };
            match token_result {
                Ok(key) => return Ok(key),
                Err(Error::AuthorizationPending) => {}
                Err(Error::AuthorizationSlowDown) => {
                    interval = interval
                        .saturating_add(options.slow_down_increment)
                        .min(options.max_interval);
                }
                Err(error) => return Err(error),
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(deadline_error(deadline_is_expiration));
            }
            tokio::select! {
                () = tokio::time::sleep(interval.min(remaining)) => {},
                () = cancellation.cancelled() => return Err(Error::Cancelled),
            }
        }
    }

    async fn request_device_token(&self, device_code: &str) -> Result<ApiKey> {
        let response = self
            .http
            .post(self.endpoint("device/token")?)
            .json(&DeviceTokenRequestWire { device_code })
            .send()
            .await?;
        let status = response.status();
        let body = read_limited_body(response).await?;
        decode_device_token_response(status, &body)
    }
}

fn normalize_base_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value)
        .map_err(|error| Error::InvalidConfiguration(format!("invalid base URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::InvalidConfiguration(
            "base URL must use http or https".into(),
        ));
    }
    if url.cannot_be_a_base()
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidConfiguration(
            "base URL must be an absolute URL without query or fragment".into(),
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = read_limited_body(response).await?;
    if status == StatusCode::UNAUTHORIZED && names_engine_not_authenticated(&body) {
        return Err(Error::EngineNotAuthenticated);
    }
    Err(Error::HttpStatus { status, body })
}

/// Branches on the daemon's error `code`, never on its message, which is localized.
fn names_engine_not_authenticated(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/code")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|code| code == ENGINE_NOT_AUTHENTICATED)
}

async fn read_limited_body(response: reqwest::Response) -> Result<String> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
        if bytes.len() == MAX_ERROR_BODY_BYTES {
            truncated = true;
            break;
        }
    }
    let mut body = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        body.push_str("…[truncated]");
    }
    Ok(body)
}

fn decode_sse_data(data: &str, queued: &mut VecDeque<ChatEvent>, done: &mut bool) -> Result<()> {
    if data.trim() == "[DONE]" {
        queued.push_back(ChatEvent::Done);
        *done = true;
        return Ok(());
    }
    let chunk: ChatChunkWire = serde_json::from_str(data)?;
    if let Some(error) = chunk.error {
        let detail = if let Some(code) = error.code {
            format!("{code}: {}", error.message)
        } else {
            error.message
        };
        return Err(Error::Protocol(detail));
    }
    for choice in chunk.choices {
        if let Some(content) = choice.delta.content
            && !content.is_empty()
        {
            queued.push_back(ChatEvent::TextDelta(content));
        }
        for call in choice.delta.tool_calls {
            let (name, arguments_delta) = call
                .function
                .map(|function| (function.name, function.arguments))
                .unwrap_or_default();
            queued.push_back(ChatEvent::ToolCallDelta {
                index: call.index,
                id: call.id,
                name,
                arguments_delta,
            });
        }
        if let Some(reason) = choice.finish_reason {
            queued.push_back(ChatEvent::FinishReason(finish_reason(reason)));
        }
    }
    if let Some(usage) = chunk.usage {
        queued.push_back(ChatEvent::Usage(usage.into()));
    }
    Ok(())
}

fn decode_device_token_response(status: StatusCode, body: &str) -> Result<ApiKey> {
    let response: DeviceTokenWire = serde_json::from_str(body)?;
    let (token, error, description) = response.into_parts();
    if let Some(error) = error {
        return match error.as_str() {
            "authorization_pending" | "pending" => Err(Error::AuthorizationPending),
            "slow_down" => Err(Error::AuthorizationSlowDown),
            "access_denied" => Err(Error::AccessDenied),
            "expired_token" => Err(Error::ExpiredToken),
            _ => Err(Error::HttpStatus {
                status,
                body: description.unwrap_or_else(|| "device authorization failed".into()),
            }),
        };
    }
    if !status.is_success() {
        return Err(Error::HttpStatus {
            status,
            body: "device token request failed".into(),
        });
    }
    let token = token.ok_or_else(|| {
        Error::Protocol("device token response did not contain an API key".into())
    })?;
    ApiKey::new(token)
}

fn deadline_error(is_expiration: bool) -> Error {
    if is_expiration {
        Error::ExpiredToken
    } else {
        Error::AuthorizationTimedOut
    }
}

fn validate_poll_options(options: DevicePollOptions) -> Result<()> {
    if options.max_wait.is_zero()
        || options.min_interval.is_zero()
        || options.max_interval < options.min_interval
        || options.slow_down_increment.is_zero()
    {
        return Err(Error::InvalidConfiguration(
            "device polling durations must be non-zero and max_interval >= min_interval".into(),
        ));
    }
    Ok(())
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

struct ChatAccumulator {
    limits: CompletionLimits,
    content: String,
    calls: BTreeMap<usize, PartialToolCall>,
    total_argument_bytes: usize,
    usage: Option<Usage>,
    finish_reason: Option<FinishReason>,
}

impl Default for ChatAccumulator {
    fn default() -> Self {
        Self::new(CompletionLimits::default())
    }
}

impl ChatAccumulator {
    fn new(limits: CompletionLimits) -> Self {
        Self {
            limits,
            content: String::new(),
            calls: BTreeMap::new(),
            total_argument_bytes: 0,
            usage: None,
            finish_reason: None,
        }
    }

    fn checked_size(
        current: usize,
        additional: usize,
        limit: usize,
        resource: &'static str,
    ) -> Result<usize> {
        let size = current
            .checked_add(additional)
            .ok_or(Error::CompletionLimitExceeded { resource, limit })?;
        if size > limit {
            return Err(Error::CompletionLimitExceeded { resource, limit });
        }
        Ok(size)
    }

    fn apply(&mut self, event: ChatEvent) -> Result<()> {
        match event {
            ChatEvent::TextDelta(text) => {
                Self::checked_size(
                    self.content.len(),
                    text.len(),
                    self.limits.max_assistant_text_bytes,
                    "assistant text",
                )?;
                self.content.push_str(&text);
            }
            ChatEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                let existing = self.calls.get(&index);
                if existing.is_none() && self.calls.len() >= self.limits.max_tool_calls {
                    return Err(Error::CompletionLimitExceeded {
                        resource: "tool call count",
                        limit: self.limits.max_tool_calls,
                    });
                }
                let id_bytes = Self::checked_size(
                    existing.map_or(0, |call| call.id.len()),
                    id.as_ref().map_or(0, String::len),
                    self.limits.max_tool_call_id_bytes,
                    "tool call id",
                )?;
                let name_bytes = Self::checked_size(
                    existing.map_or(0, |call| call.name.len()),
                    name.as_ref().map_or(0, String::len),
                    self.limits.max_tool_name_bytes,
                    "tool name",
                )?;
                let argument_delta_bytes = arguments_delta.as_ref().map_or(0, String::len);
                let argument_bytes = Self::checked_size(
                    existing.map_or(0, |call| call.arguments.len()),
                    argument_delta_bytes,
                    self.limits.max_tool_argument_bytes,
                    "tool call arguments",
                )?;
                let total_argument_bytes = Self::checked_size(
                    self.total_argument_bytes,
                    argument_delta_bytes,
                    self.limits.max_total_tool_argument_bytes,
                    "total tool call arguments",
                )?;

                let call = self.calls.entry(index).or_default();
                if let Some(id) = id {
                    call.id.push_str(&id);
                }
                if let Some(name) = name {
                    call.name.push_str(&name);
                }
                if let Some(arguments) = arguments_delta {
                    call.arguments.push_str(&arguments);
                }
                debug_assert_eq!(call.id.len(), id_bytes);
                debug_assert_eq!(call.name.len(), name_bytes);
                debug_assert_eq!(call.arguments.len(), argument_bytes);
                self.total_argument_bytes = total_argument_bytes;
            }
            ChatEvent::Usage(usage) => self.usage = Some(usage),
            ChatEvent::FinishReason(reason) => self.finish_reason = Some(reason),
            ChatEvent::Done => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<AssistantResponse> {
        let mut tool_calls = Vec::with_capacity(self.calls.len());
        for (index, call) in self.calls {
            if call.id.is_empty() || call.name.is_empty() {
                return Err(Error::Protocol(format!(
                    "tool call at index {index} is missing its id or function name"
                )));
            }
            let arguments = if call.arguments.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&call.arguments).map_err(|_| {
                    Error::Protocol(format!(
                        "tool call at index {index} has invalid JSON arguments"
                    ))
                })?
            };
            tool_calls.push(ToolCall {
                id: call.id,
                name: call.name,
                arguments,
            });
        }
        Ok(AssistantResponse {
            content: (!self.content.is_empty()).then_some(self.content),
            tool_calls,
            usage: self.usage,
            finish_reason: self.finish_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        time::{Duration, Instant},
    };

    use redrob_engine_client::Record;
    use reqwest::StatusCode;
    use serde_json::json;

    use super::{
        ChatAccumulator, DeviceFlowClient, decode_device_token_response, decode_sse_data,
        names_engine_not_authenticated, normalize_base_url,
    };
    use crate::{
        CancellationToken, ChatEvent, CompletionLimits, DeviceAuthorization, DevicePollOptions,
        EngineConfig, Error, FinishReason, RedrobClient,
    };

    fn record(url: &str, secret: &str) -> Record {
        Record {
            id: "daemon-1".into(),
            version: "0.6.2".into(),
            url: url.into(),
            port: 4096,
            pid: 1,
            secret: secret.into(),
        }
    }

    #[test]
    fn constructs_endpoints_under_the_daemon_v1_base_with_or_without_trailing_slash() {
        for url in ["http://127.0.0.1:4096", "http://127.0.0.1:4096/"] {
            let daemon = record(url, "s");
            assert_eq!(
                RedrobClient::endpoint(&daemon, "chat/completions")
                    .unwrap()
                    .as_str(),
                "http://127.0.0.1:4096/v1/chat/completions"
            );
            assert_eq!(
                RedrobClient::endpoint(&daemon, "models").unwrap().as_str(),
                "http://127.0.0.1:4096/v1/models"
            );
            assert_eq!(
                RedrobClient::daemon_endpoint(&daemon, "provider")
                    .unwrap()
                    .as_str(),
                "http://127.0.0.1:4096/provider"
            );
            assert_eq!(
                RedrobClient::daemon_endpoint(&daemon, "auth/redrob")
                    .unwrap()
                    .as_str(),
                "http://127.0.0.1:4096/auth/redrob"
            );
        }
        assert!(normalize_base_url("file:///tmp/api").is_err());
        let console = DeviceFlowClient::new().unwrap();
        assert_eq!(
            console.endpoint("device/authorize").unwrap().as_str(),
            "https://console.redrob.ai/api/backend/v1/device/authorize"
        );
    }

    #[test]
    fn the_daemon_sign_in_code_is_read_from_the_error_code_not_the_message() {
        assert!(names_engine_not_authenticated(
            r#"{"error":{"message":"로그인이 필요합니다","code":"engine_not_authenticated"}}"#
        ));
        assert!(!names_engine_not_authenticated(
            r#"{"error":{"message":"engine_not_authenticated"}}"#
        ));
        assert!(!names_engine_not_authenticated("not json"));
    }

    #[test]
    fn the_client_debug_output_names_no_secret() {
        let client = RedrobClient::with_record(
            EngineConfig::new("canvas").unwrap(),
            record("http://127.0.0.1:4096", "top-secret"),
        )
        .unwrap();
        let shown = format!("{client:?}");
        assert!(shown.contains("http://127.0.0.1:4096"));
        assert!(!shown.contains("top-secret"));
    }

    #[test]
    fn accumulates_interleaved_tool_calls_and_parses_only_at_finish() {
        let chunks = [
            json!({"choices":[{"delta":{"tool_calls":[
                {"index":1,"id":"call-b","function":{"name":"lo","arguments":"{\"b\":"}},
                {"index":0,"id":"call-a","function":{"name":"sa","arguments":"{\"a\":"}}
            ]},"finish_reason":null}]}),
            json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"name":"ve","arguments":"1}"}},
                {"index":1,"function":{"name":"ok","arguments":"2}"}}
            ]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}),
        ];
        let mut queued = VecDeque::new();
        let mut done = false;
        for chunk in chunks {
            decode_sse_data(&chunk.to_string(), &mut queued, &mut done).unwrap();
        }
        decode_sse_data("[DONE]", &mut queued, &mut done).unwrap();
        let mut accumulator = ChatAccumulator::default();
        for event in queued {
            accumulator.apply(event).unwrap();
        }
        let response = accumulator.finish().unwrap();
        assert_eq!(response.tool_calls[0].name, "save");
        assert_eq!(response.tool_calls[0].arguments, json!({"a": 1}));
        assert_eq!(response.tool_calls[1].name, "look");
        assert_eq!(response.tool_calls[1].arguments, json!({"b": 2}));
        assert_eq!(response.finish_reason, Some(FinishReason::ToolCalls));
        assert_eq!(response.usage.unwrap().total_tokens, 7);
    }

    fn limits() -> CompletionLimits {
        CompletionLimits {
            max_assistant_text_bytes: 4,
            max_tool_calls: 2,
            max_tool_argument_bytes: 6,
            max_total_tool_argument_bytes: 8,
            max_tool_call_id_bytes: 4,
            max_tool_name_bytes: 5,
        }
    }

    fn tool_delta(
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) -> ChatEvent {
        ChatEvent::ToolCallDelta {
            index,
            id: id.map(str::to_owned),
            name: name.map(str::to_owned),
            arguments_delta: arguments.map(str::to_owned),
        }
    }

    #[test]
    fn assistant_text_limit_accepts_exact_boundary_and_rejects_one_over() {
        let mut accumulator = ChatAccumulator::new(limits());
        accumulator
            .apply(ChatEvent::TextDelta("four".into()))
            .unwrap();
        assert!(matches!(
            accumulator.apply(ChatEvent::TextDelta("!".into())),
            Err(Error::CompletionLimitExceeded { .. })
        ));
        assert_eq!(accumulator.content, "four");
    }

    #[test]
    fn fragmented_id_and_name_limits_are_incremental_and_exact() {
        let mut accumulator = ChatAccumulator::new(limits());
        accumulator
            .apply(tool_delta(7, Some("ab"), Some("sa"), Some("{}")))
            .unwrap();
        accumulator
            .apply(tool_delta(7, Some("cd"), Some("ve!"), None))
            .unwrap();
        assert_eq!(accumulator.calls[&7].id, "abcd");
        assert_eq!(accumulator.calls[&7].name, "save!");

        assert!(
            accumulator
                .apply(tool_delta(7, Some("x"), None, None))
                .is_err()
        );
        assert!(
            accumulator
                .apply(tool_delta(7, None, Some("x"), None))
                .is_err()
        );
        assert_eq!(accumulator.calls[&7].id, "abcd");
        assert_eq!(accumulator.calls[&7].name, "save!");
    }

    #[test]
    fn per_call_argument_limit_accepts_exact_boundary_and_rejects_one_over() {
        let mut accumulator = ChatAccumulator::new(limits());
        accumulator
            .apply(tool_delta(0, Some("a"), Some("tool"), Some("{\"x\":")))
            .unwrap();
        accumulator
            .apply(tool_delta(0, None, None, Some("1")))
            .unwrap();
        assert_eq!(accumulator.calls[&0].arguments, "{\"x\":1");
        assert!(
            accumulator
                .apply(tool_delta(0, None, None, Some("}")))
                .is_err()
        );
        assert_eq!(accumulator.calls[&0].arguments, "{\"x\":1");
        assert_eq!(accumulator.total_argument_bytes, 6);
    }

    #[test]
    fn aggregate_argument_limit_handles_interleaved_calls_transactionally() {
        let mut accumulator = ChatAccumulator::new(limits());
        accumulator
            .apply(tool_delta(10, Some("a"), Some("one"), Some("1234")))
            .unwrap();
        accumulator
            .apply(tool_delta(2, Some("b"), Some("two"), Some("5678")))
            .unwrap();
        assert_eq!(accumulator.total_argument_bytes, 8);
        assert!(
            accumulator
                .apply(tool_delta(10, None, None, Some("9")))
                .is_err()
        );
        assert_eq!(accumulator.calls[&10].arguments, "1234");
        assert_eq!(accumulator.total_argument_bytes, 8);
    }

    #[test]
    fn call_count_allows_existing_sparse_index_at_boundary() {
        let mut accumulator = ChatAccumulator::new(limits());
        accumulator
            .apply(tool_delta(usize::MAX, Some("a"), Some("one"), Some("1")))
            .unwrap();
        accumulator
            .apply(tool_delta(4, Some("b"), Some("two"), Some("2")))
            .unwrap();
        accumulator
            .apply(tool_delta(usize::MAX, Some("c"), None, Some("3")))
            .unwrap();
        assert_eq!(accumulator.calls.len(), 2);
        assert_eq!(accumulator.calls[&usize::MAX].id, "ac");
        assert!(
            accumulator
                .apply(tool_delta(9, Some("c"), Some("three"), Some("4")))
                .is_err()
        );
        assert!(!accumulator.calls.contains_key(&9));
    }

    #[test]
    fn rejected_multi_field_delta_never_partially_appends() {
        let mut accumulator = ChatAccumulator::new(limits());
        accumulator
            .apply(tool_delta(0, Some("a"), Some("save"), Some("12345")))
            .unwrap();
        assert!(
            accumulator
                .apply(tool_delta(0, Some("b"), None, Some("67")))
                .is_err()
        );
        let call = &accumulator.calls[&0];
        assert_eq!(call.id, "a");
        assert_eq!(call.name, "save");
        assert_eq!(call.arguments, "12345");
        assert_eq!(accumulator.total_argument_bytes, 5);

        assert!(
            accumulator
                .apply(tool_delta(1, Some("x"), Some("name!!"), Some("{}")))
                .is_err()
        );
        assert!(!accumulator.calls.contains_key(&1));
        assert_eq!(accumulator.total_argument_bytes, 5);
    }

    #[test]
    fn emits_text_usage_finish_and_done_events() {
        let mut events = VecDeque::new();
        let mut done = false;
        decode_sse_data(
            r#"{"choices":[{"delta":{"content":"hello"},"finish_reason":"stop"}],"usage":{"total_tokens":2}}"#,
            &mut events,
            &mut done,
        )
        .unwrap();
        decode_sse_data("[DONE]", &mut events, &mut done).unwrap();
        assert!(matches!(events.pop_front(), Some(ChatEvent::TextDelta(text)) if text == "hello"));
        assert_eq!(
            events.pop_front(),
            Some(ChatEvent::FinishReason(FinishReason::Stop))
        );
        assert!(matches!(events.pop_front(), Some(ChatEvent::Usage(_))));
        assert_eq!(events.pop_front(), Some(ChatEvent::Done));
    }

    #[test]
    fn maps_authorization_errors_without_exposing_device_codes() {
        let cases = [
            ("authorization_pending", Error::AuthorizationPending),
            ("slow_down", Error::AuthorizationSlowDown),
            ("access_denied", Error::AccessDenied),
            ("expired_token", Error::ExpiredToken),
        ];
        for (code, expected) in cases {
            let error = decode_device_token_response(
                StatusCode::BAD_REQUEST,
                &json!({"error": code}).to_string(),
            )
            .unwrap_err();
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&expected)
            );
        }
        let key = decode_device_token_response(StatusCode::OK, r#"{"accessToken":"top-secret"}"#)
            .unwrap();
        assert_eq!(format!("{key:?}"), "ApiKey([REDACTED])");
    }

    #[tokio::test]
    async fn polling_honors_preexisting_cancellation_without_network_access() {
        let client =
            DeviceFlowClient::with_base_url("https://example.test/api/v1", Duration::from_secs(5))
                .unwrap();
        let authorization = DeviceAuthorization {
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://example.test/activate".into(),
            verification_uri_complete: None,
            expires_in: Duration::from_secs(600),
            interval: Duration::from_secs(5),
            device_code: "private-device-code".into(),
            received_at: Instant::now(),
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = client
            .poll_device_token(&authorization, DevicePollOptions::default(), &cancellation)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        assert!(!format!("{authorization:?}").contains("private-device-code"));
    }
}
