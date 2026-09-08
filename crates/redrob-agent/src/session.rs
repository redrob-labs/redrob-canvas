// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fmt, sync::Arc};

use async_trait::async_trait;

use crate::{
    AgentSessionConfig, AssistantResponse, ChatMessage, ChatRequest, Error, RedrobClient, Result,
    ToolCall, ToolOutput,
};

/// Provider-neutral interface consumed by [`AgentSession`].
#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn complete(&self, request: ChatRequest) -> Result<AssistantResponse>;
}

#[async_trait]
impl ChatProvider for RedrobClient {
    async fn complete(&self, request: ChatRequest) -> Result<AssistantResponse> {
        self.complete_chat(request).await
    }
}

/// Executes a declared function tool. It intentionally has no shell-specific
/// operation; applications decide which typed tools they expose.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, call: &ToolCall) -> Result<ToolOutput>;
}

/// Authorizes each mutating tool invocation before it reaches the executor.
#[async_trait]
pub trait ApprovalGate: Send + Sync {
    async fn approve(&self, call: &ToolCall) -> Result<bool>;
}

/// Provider-neutral conversation loop that executes assistant tool calls.
pub struct AgentSession {
    provider: Arc<dyn ChatProvider>,
    executor: Arc<dyn ToolExecutor>,
    approval: Arc<dyn ApprovalGate>,
    config: AgentSessionConfig,
    messages: Vec<ChatMessage>,
}

impl fmt::Debug for AgentSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentSession")
            .field("config", &self.config)
            .field("messages", &self.messages)
            .finish_non_exhaustive()
    }
}

impl AgentSession {
    pub fn new(
        provider: Arc<dyn ChatProvider>,
        executor: Arc<dyn ToolExecutor>,
        approval: Arc<dyn ApprovalGate>,
        config: AgentSessionConfig,
    ) -> Result<Self> {
        if config.model.trim().is_empty() {
            return Err(Error::InvalidConfiguration(
                "agent model must not be empty".into(),
            ));
        }
        if config.max_tool_rounds == 0 {
            return Err(Error::InvalidConfiguration(
                "max_tool_rounds must be greater than zero".into(),
            ));
        }
        for (index, tool) in config.tools.iter().enumerate() {
            if config.tools[..index]
                .iter()
                .any(|existing| existing.name == tool.name)
            {
                return Err(Error::InvalidConfiguration(format!(
                    "duplicate tool name `{}`",
                    tool.name
                )));
            }
        }
        Ok(Self {
            provider,
            executor,
            approval,
            config,
            messages: Vec::new(),
        })
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn push_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
    }

    /// Adds a user message, then continues calling the provider until it
    /// returns a final assistant response or the tool-round bound is reached.
    pub async fn run(&mut self, user_message: impl Into<String>) -> Result<AssistantResponse> {
        self.messages.push(ChatMessage::user(user_message));
        let mut tool_rounds = 0usize;

        loop {
            let request = ChatRequest::new(self.messages.clone())
                .with_model(self.config.model.clone())
                .with_tools(self.config.tools.clone());
            let response = self.provider.complete(request).await?;
            self.messages.push(ChatMessage::Assistant {
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
            });

            if response.tool_calls.is_empty() {
                return Ok(response);
            }
            if tool_rounds >= self.config.max_tool_rounds {
                return Err(Error::ToolRoundLimit(self.config.max_tool_rounds));
            }
            tool_rounds += 1;

            for call in &response.tool_calls {
                let tool = self
                    .config
                    .tools
                    .iter()
                    .find(|tool| tool.name == call.name)
                    .ok_or_else(|| Error::UnknownTool(call.name.clone()))?;
                if tool.mutating && !self.approval.approve(call).await? {
                    return Err(Error::ApprovalDenied(call.name.clone()));
                }
                let output = self.executor.execute(call).await?;
                self.messages
                    .push(ChatMessage::tool(call.id.clone(), output.content));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::json;

    use super::{AgentSession, ApprovalGate, ChatProvider, ToolExecutor};
    use crate::{
        AgentSessionConfig, AssistantResponse, ChatRequest, Error, FunctionTool, Result, ToolCall,
        ToolOutput,
    };

    struct ScriptedProvider(Mutex<Vec<AssistantResponse>>);

    #[async_trait]
    impl ChatProvider for ScriptedProvider {
        async fn complete(&self, _request: ChatRequest) -> Result<AssistantResponse> {
            Ok(self.0.lock().unwrap().remove(0))
        }
    }

    #[derive(Default)]
    struct RecordingExecutor(Mutex<Vec<String>>);

    #[async_trait]
    impl ToolExecutor for RecordingExecutor {
        async fn execute(&self, call: &ToolCall) -> Result<ToolOutput> {
            self.0.lock().unwrap().push(call.name.clone());
            Ok(ToolOutput::new("saved"))
        }
    }

    struct Gate {
        approved: bool,
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl ApprovalGate for Gate {
        async fn approve(&self, _call: &ToolCall) -> Result<bool> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.approved)
        }
    }

    fn response(content: Option<&str>, tool_calls: Vec<ToolCall>) -> AssistantResponse {
        AssistantResponse {
            content: content.map(str::to_owned),
            tool_calls,
            usage: None,
            finish_reason: None,
        }
    }

    #[tokio::test]
    async fn mutating_tools_require_approval_before_execution() {
        let provider = Arc::new(ScriptedProvider(Mutex::new(vec![response(
            None,
            vec![ToolCall {
                id: "call-1".into(),
                name: "save".into(),
                arguments: json!({"path": "image.png"}),
            }],
        )])));
        let executor = Arc::new(RecordingExecutor::default());
        let gate = Arc::new(Gate {
            approved: false,
            calls: Mutex::new(0),
        });
        let config = AgentSessionConfig {
            tools: vec![
                FunctionTool::new("save", json!({"type": "object"}))
                    .unwrap()
                    .mutating(true),
            ],
            ..AgentSessionConfig::default()
        };
        let mut session =
            AgentSession::new(provider, executor.clone(), gate.clone(), config).unwrap();
        let error = session.run("save it").await.unwrap_err();
        assert!(matches!(error, Error::ApprovalDenied(name) if name == "save"));
        assert_eq!(*gate.calls.lock().unwrap(), 1);
        assert!(executor.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn loops_tool_results_back_to_provider() {
        let provider = Arc::new(ScriptedProvider(Mutex::new(vec![
            response(
                None,
                vec![ToolCall {
                    id: "call-1".into(),
                    name: "inspect".into(),
                    arguments: json!({}),
                }],
            ),
            response(Some("done"), vec![]),
        ])));
        let executor = Arc::new(RecordingExecutor::default());
        let gate = Arc::new(Gate {
            approved: true,
            calls: Mutex::new(0),
        });
        let config = AgentSessionConfig {
            tools: vec![FunctionTool::new("inspect", json!({"type": "object"})).unwrap()],
            ..AgentSessionConfig::default()
        };
        let mut session =
            AgentSession::new(provider, executor.clone(), gate.clone(), config).unwrap();
        let final_response = session.run("inspect").await.unwrap();
        assert_eq!(final_response.content.as_deref(), Some("done"));
        assert_eq!(&*executor.0.lock().unwrap(), &["inspect"]);
        assert_eq!(*gate.calls.lock().unwrap(), 0);
        assert_eq!(session.messages().len(), 4);
    }
}
