// SPDX-License-Identifier: MIT OR Apache-2.0

//! The mode agent: an [`AgentSession`] from `redrob-agent` on its own tokio runtime thread, so a
//! synchronous UI can send a prompt and poll events each frame. The shell implements
//! [`ModeTools`] once per mode (what the paint, vector and layout editors let the agent do); the
//! adapter turns those into the session's function tools and executes the calls.
//!
//! Chat goes through the shared Redrob Code daemon (`redrob-engine-client`) with model
//! `redrob/auto`; this crate never holds a console key.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

use async_trait::async_trait;
pub use redrob_agent::{
    AgentSession, AgentSessionConfig, AssistantResponse, ChatMessage, ChatProvider, EngineConfig,
    FunctionTool, RedrobClient, ToolCall, ToolOutput,
};
use serde_json::{Value, json};
use tokio::sync::mpsc as tokio_mpsc;

use crate::{AiError, Result};

/// The model name the shared daemon routes to the Redrob console.
pub const DEFAULT_MODEL: &str = "redrob/auto";

/// What one editor mode lets the agent do. Implemented by the shell; kept synchronous so a mode
/// can run the call on its own document (it is invoked off the UI thread, so block as needed).
pub trait ModeTools: Send + Sync + 'static {
    /// The function tools to declare to the model.
    fn list_tools(&self) -> Vec<FunctionTool>;

    /// Runs one tool call. `Ok` is the JSON result handed back to the model; `Err` is a message
    /// the model sees as `{"error": ...}`.
    fn execute(&self, name: &str, arguments: Value) -> std::result::Result<Value, String>;

    /// Whether a tool marked `mutating` may run. Defaults to yes; a mode can ask the user.
    fn approve(&self, _name: &str, _arguments: &Value) -> bool {
        true
    }
}

/// What the agent did since the last poll.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    /// A prompt started running.
    Started,
    /// The assistant's final text for a prompt.
    Text(String),
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        id: String,
        name: String,
        output: String,
    },
    Error(String),
    /// The prompt finished (after `Text` or `Error`).
    Done,
}

/// How the agent presents itself to the daemon and runs its loop.
#[derive(Clone, Debug)]
pub struct AgentOptions {
    /// The `x-redrob-app` name the daemon labels requests with.
    pub app: String,
    /// The caller's own session id (`x-redrob-session`), e.g. a document id.
    pub session: Option<String>,
    pub model: String,
    pub max_tool_rounds: usize,
    pub system_prompt: Option<String>,
    pub request_timeout: Duration,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            app: "redrob-canvas".into(),
            session: None,
            model: DEFAULT_MODEL.into(),
            max_tool_rounds: 8,
            system_prompt: None,
            request_timeout: Duration::from_secs(300),
        }
    }
}

enum Command {
    Prompt(String),
    Shutdown,
}

/// Adapts [`ModeTools`] to the session's executor and approval gate, emitting events.
struct ToolBridge {
    tools: Arc<dyn ModeTools>,
    events: mpsc::Sender<AgentEvent>,
}

#[async_trait]
impl redrob_agent::ToolExecutor for ToolBridge {
    async fn execute(&self, call: &ToolCall) -> redrob_agent::Result<ToolOutput> {
        let _ = self.events.send(AgentEvent::ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        });
        let tools = Arc::clone(&self.tools);
        let name = call.name.clone();
        let arguments = call.arguments.clone();
        let joined = tokio::task::spawn_blocking(move || tools.execute(&name, arguments)).await;
        let output = match joined {
            Ok(Ok(value)) => value.to_string(),
            Ok(Err(message)) => json!({ "error": message }).to_string(),
            Err(join) => json!({ "error": format!("tool panicked: {join}") }).to_string(),
        };
        let _ = self.events.send(AgentEvent::ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            output: output.clone(),
        });
        Ok(ToolOutput::new(output))
    }
}

#[async_trait]
impl redrob_agent::ApprovalGate for ToolBridge {
    async fn approve(&self, call: &ToolCall) -> redrob_agent::Result<bool> {
        Ok(self.tools.approve(&call.name, &call.arguments))
    }
}

/// A running agent. Dropping it stops the thread.
pub struct AgentHandle {
    commands: tokio_mpsc::UnboundedSender<Command>,
    events: Mutex<mpsc::Receiver<AgentEvent>>,
    busy: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for AgentHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentHandle")
            .field("busy", &self.is_busy())
            .finish_non_exhaustive()
    }
}

impl AgentHandle {
    /// Starts an agent over the shared Redrob Code daemon. Takes no network action: the daemon is
    /// looked up (and started) when the first prompt needs it.
    pub fn spawn(tools: Arc<dyn ModeTools>, options: AgentOptions) -> Result<Self> {
        let mut config =
            EngineConfig::new(options.app.clone())?.with_request_timeout(options.request_timeout);
        if let Some(session) = &options.session {
            config = config.with_session(session.clone());
        }
        let provider: Arc<dyn ChatProvider> = Arc::new(RedrobClient::new(config)?);
        Self::spawn_with_provider(provider, tools, options)
    }

    /// Starts an agent over any [`ChatProvider`] (tests, or an app that resolved its own client).
    pub fn spawn_with_provider(
        provider: Arc<dyn ChatProvider>,
        tools: Arc<dyn ModeTools>,
        options: AgentOptions,
    ) -> Result<Self> {
        let (event_tx, event_rx) = mpsc::channel();
        let bridge = Arc::new(ToolBridge {
            tools: Arc::clone(&tools),
            events: event_tx.clone(),
        });
        let mut session = AgentSession::new(
            provider,
            Arc::clone(&bridge) as Arc<dyn redrob_agent::ToolExecutor>,
            bridge as Arc<dyn redrob_agent::ApprovalGate>,
            AgentSessionConfig {
                model: options.model.clone(),
                tools: tools.list_tools(),
                max_tool_rounds: options.max_tool_rounds,
            },
        )?;
        if let Some(system) = &options.system_prompt {
            session.push_message(ChatMessage::system(system.clone()));
        }
        let (command_tx, mut command_rx) = tokio_mpsc::unbounded_channel::<Command>();
        let busy = Arc::new(AtomicBool::new(false));
        let busy_flag = Arc::clone(&busy);
        let thread = std::thread::Builder::new()
            .name("redrob-ai-agent".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = event_tx.send(AgentEvent::Error(format!(
                            "could not start the agent runtime: {e}"
                        )));
                        return;
                    }
                };
                runtime.block_on(async move {
                    while let Some(command) = command_rx.recv().await {
                        match command {
                            Command::Shutdown => break,
                            Command::Prompt(text) => {
                                busy_flag.store(true, Ordering::SeqCst);
                                let _ = event_tx.send(AgentEvent::Started);
                                match session.run(text).await {
                                    Ok(response) => {
                                        let _ = event_tx.send(AgentEvent::Text(
                                            response.content.unwrap_or_default(),
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(AgentEvent::Error(e.to_string()));
                                    }
                                }
                                busy_flag.store(false, Ordering::SeqCst);
                                let _ = event_tx.send(AgentEvent::Done);
                            }
                        }
                    }
                });
            })?;
        Ok(Self {
            commands: command_tx,
            events: Mutex::new(event_rx),
            busy,
            thread: Some(thread),
        })
    }

    /// Queues a user prompt. Prompts run one after another on the same conversation.
    pub fn prompt(&self, text: impl Into<String>) -> Result<()> {
        self.commands
            .send(Command::Prompt(text.into()))
            .map_err(|_| AiError::Process("the agent thread has stopped".into()))
    }

    /// Everything that happened since the last poll (never blocks).
    pub fn poll(&self) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        if let Ok(rx) = self.events.lock() {
            while let Ok(event) = rx.try_recv() {
                out.push(event);
            }
        }
        out
    }

    /// Waits up to `timeout` for the next event (tests and headless callers).
    pub fn next_event(&self, timeout: Duration) -> Option<AgentEvent> {
        self.events.lock().ok()?.recv_timeout(timeout).ok()
    }

    /// Whether a prompt is running.
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Whether the agent thread is still alive.
    pub fn is_alive(&self) -> bool {
        !self.commands.is_closed()
    }
}

impl Drop for AgentHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take()
            && std::thread::current().id() != thread.thread().id()
        {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use redrob_agent::ChatRequest;

    use super::*;

    /// Answers the first request with one tool call, the second with text.
    struct FakeProvider {
        calls: AtomicUsize,
        seen: Mutex<Vec<ChatRequest>>,
    }

    #[async_trait]
    impl ChatProvider for FakeProvider {
        async fn complete(&self, request: ChatRequest) -> redrob_agent::Result<AssistantResponse> {
            self.seen.lock().unwrap().push(request);
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(if n == 0 {
                AssistantResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "fill".into(),
                        arguments: json!({ "color": "#ff0000" }),
                    }],
                    usage: None,
                    finish_reason: None,
                }
            } else {
                AssistantResponse {
                    content: Some("Filled it red.".into()),
                    tool_calls: Vec::new(),
                    usage: None,
                    finish_reason: None,
                }
            })
        }
    }

    struct PaintTools {
        fills: Mutex<Vec<String>>,
    }

    impl ModeTools for PaintTools {
        fn list_tools(&self) -> Vec<FunctionTool> {
            vec![
                FunctionTool::new(
                    "fill",
                    json!({ "type": "object", "properties": { "color": { "type": "string" } } }),
                )
                .unwrap()
                .with_description("Fill the selection")
                .mutating(true),
            ]
        }

        fn execute(&self, name: &str, arguments: Value) -> std::result::Result<Value, String> {
            match name {
                "fill" => {
                    let color = arguments["color"].as_str().unwrap_or_default().to_owned();
                    self.fills.lock().unwrap().push(color.clone());
                    Ok(json!({ "ok": true, "color": color }))
                }
                other => Err(format!("unknown tool {other}")),
            }
        }
    }

    fn collect(handle: &AgentHandle) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        while let Some(event) = handle.next_event(Duration::from_secs(10)) {
            let done = event == AgentEvent::Done;
            events.push(event);
            if done {
                break;
            }
        }
        events
    }

    #[test]
    fn prompt_runs_tools_and_reports_events() {
        let provider = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        let tools = Arc::new(PaintTools {
            fills: Mutex::new(Vec::new()),
        });
        let handle = AgentHandle::spawn_with_provider(
            Arc::clone(&provider) as Arc<dyn ChatProvider>,
            Arc::clone(&tools) as Arc<dyn ModeTools>,
            AgentOptions {
                system_prompt: Some("You edit pictures.".into()),
                ..AgentOptions::default()
            },
        )
        .unwrap();
        assert!(handle.is_alive());
        handle.prompt("make it red").unwrap();
        let events = collect(&handle);
        assert_eq!(
            events,
            vec![
                AgentEvent::Started,
                AgentEvent::ToolCall {
                    id: "call_1".into(),
                    name: "fill".into(),
                    arguments: json!({ "color": "#ff0000" }),
                },
                AgentEvent::ToolResult {
                    id: "call_1".into(),
                    name: "fill".into(),
                    output: json!({ "ok": true, "color": "#ff0000" }).to_string(),
                },
                AgentEvent::Text("Filled it red.".into()),
                AgentEvent::Done,
            ]
        );
        assert_eq!(*tools.fills.lock().unwrap(), vec!["#ff0000"]);
        assert!(!handle.is_busy());
        let seen = provider.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].model, DEFAULT_MODEL);
        assert_eq!(seen[0].tools.len(), 1);
        assert!(matches!(seen[0].messages[0], ChatMessage::System { .. }));
        drop(seen);
        drop(handle);
    }

    #[test]
    fn tool_errors_reach_the_model_as_json() {
        struct Failing;
        impl ModeTools for Failing {
            fn list_tools(&self) -> Vec<FunctionTool> {
                vec![FunctionTool::new("fill", json!({ "type": "object" })).unwrap()]
            }
            fn execute(&self, _: &str, _: Value) -> std::result::Result<Value, String> {
                Err("no selection".into())
            }
        }
        let provider = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        let handle =
            AgentHandle::spawn_with_provider(provider, Arc::new(Failing), AgentOptions::default())
                .unwrap();
        handle.prompt("go").unwrap();
        let events = collect(&handle);
        assert!(events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolResult { output, .. } if output == &json!({ "error": "no selection" }).to_string()
        )));
    }

    #[test]
    fn denied_approval_is_an_error_event() {
        struct Denying;
        impl ModeTools for Denying {
            fn list_tools(&self) -> Vec<FunctionTool> {
                vec![
                    FunctionTool::new("fill", json!({ "type": "object" }))
                        .unwrap()
                        .mutating(true),
                ]
            }
            fn execute(&self, _: &str, _: Value) -> std::result::Result<Value, String> {
                Ok(json!({}))
            }
            fn approve(&self, _: &str, _: &Value) -> bool {
                false
            }
        }
        let provider = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        let handle =
            AgentHandle::spawn_with_provider(provider, Arc::new(Denying), AgentOptions::default())
                .unwrap();
        handle.prompt("go").unwrap();
        let events = collect(&handle);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Error(_))));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCall { .. }))
        );
    }
}
