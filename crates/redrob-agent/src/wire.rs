// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ChatMessage, ChatRequest, FinishReason, FunctionTool, Model, ToolCall, Usage};

#[derive(Serialize)]
pub(crate) struct ChatRequestWire {
    model: String,
    messages: Vec<ChatMessageWire>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolWire>,
    stream: bool,
    stream_options: StreamOptionsWire,
}

impl From<ChatRequest> for ChatRequestWire {
    fn from(request: ChatRequest) -> Self {
        Self {
            model: request.model,
            messages: request.messages.into_iter().map(Into::into).collect(),
            tools: request.tools.into_iter().map(Into::into).collect(),
            stream: true,
            stream_options: StreamOptionsWire {
                include_usage: true,
            },
        }
    }
}

#[derive(Serialize)]
struct StreamOptionsWire {
    include_usage: bool,
}

#[derive(Serialize)]
struct ChatMessageWire {
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ToolCallWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl From<ChatMessage> for ChatMessageWire {
    fn from(message: ChatMessage) -> Self {
        match message {
            ChatMessage::System { content } => Self::plain("system", content),
            ChatMessage::User { content } => Self::plain("user", content),
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => Self {
                role: "assistant",
                content,
                tool_calls: tool_calls.into_iter().map(Into::into).collect(),
                tool_call_id: None,
            },
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => Self {
                role: "tool",
                content: Some(content),
                tool_calls: Vec::new(),
                tool_call_id: Some(tool_call_id),
            },
        }
    }
}

impl ChatMessageWire {
    fn plain(role: &'static str, content: String) -> Self {
        Self {
            role,
            content: Some(content),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

#[derive(Serialize)]
struct ToolCallWire {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionCallWire,
}

impl From<ToolCall> for ToolCallWire {
    fn from(call: ToolCall) -> Self {
        Self {
            id: call.id,
            kind: "function",
            function: FunctionCallWire {
                name: call.name,
                arguments: call.arguments.to_string(),
            },
        }
    }
}

#[derive(Serialize)]
struct FunctionCallWire {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct ToolWire {
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionToolWire,
}

impl From<FunctionTool> for ToolWire {
    fn from(tool: FunctionTool) -> Self {
        Self {
            kind: "function",
            function: FunctionToolWire {
                name: tool.name,
                description: tool.description,
                parameters: tool.parameters,
            },
        }
    }
}

#[derive(Serialize)]
struct FunctionToolWire {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: Value,
}

#[derive(Deserialize)]
pub(crate) struct ChatChunkWire {
    #[serde(default)]
    pub choices: Vec<ChoiceWire>,
    pub usage: Option<UsageWire>,
    pub error: Option<ProviderErrorWire>,
}

#[derive(Deserialize)]
pub(crate) struct ChoiceWire {
    #[serde(default)]
    pub delta: DeltaWire,
    pub finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
pub(crate) struct DeltaWire {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallDeltaWire>,
}

#[derive(Deserialize)]
pub(crate) struct ToolCallDeltaWire {
    pub index: usize,
    pub id: Option<String>,
    pub function: Option<FunctionDeltaWire>,
}

#[derive(Deserialize)]
pub(crate) struct FunctionDeltaWire {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UsageWire {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

impl From<UsageWire> for Usage {
    fn from(usage: UsageWire) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ProviderErrorWire {
    #[serde(default)]
    pub message: String,
    pub code: Option<String>,
}

pub(crate) fn finish_reason(value: String) -> FinishReason {
    match value.as_str() {
        "stop" => FinishReason::Stop,
        "tool_calls" => FinishReason::ToolCalls,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Other(value),
    }
}

#[derive(Deserialize)]
pub(crate) struct ModelsWire {
    #[serde(default, alias = "models")]
    pub data: Vec<ModelWire>,
}

#[derive(Deserialize)]
pub(crate) struct ModelWire {
    id: String,
    name: Option<String>,
    description: Option<String>,
    owned_by: Option<String>,
    created: Option<u64>,
}

impl From<ModelWire> for Model {
    fn from(model: ModelWire) -> Self {
        Self {
            id: model.id,
            name: model.name,
            description: model.description,
            owned_by: model.owned_by,
            created: model.created,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct DeviceAuthorizeRequestWire {
    pub product: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceAuthorizationWire {
    #[serde(alias = "device_code")]
    pub device_code: String,
    #[serde(alias = "user_code")]
    pub user_code: String,
    #[serde(alias = "verification_uri")]
    pub verification_uri: String,
    #[serde(alias = "verification_uri_complete")]
    pub verification_uri_complete: Option<String>,
    #[serde(alias = "expires_in")]
    pub expires_in: u64,
    pub interval: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceTokenRequestWire<'a> {
    pub device_code: &'a str,
}

#[derive(Deserialize)]
pub(crate) struct DeviceTokenWire {
    #[serde(default, rename = "accessToken", alias = "access_token")]
    access_token: Option<String>,
    #[serde(default, rename = "apiKey", alias = "api_key")]
    api_key: Option<String>,
    #[serde(default)]
    token: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

impl DeviceTokenWire {
    pub(crate) fn into_parts(self) -> (Option<String>, Option<String>, Option<String>) {
        (
            self.access_token.or(self.api_key).or(self.token),
            self.error,
            self.error_description,
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{ChatRequestWire, DeviceAuthorizeRequestWire, DeviceTokenRequestWire, ModelsWire};
    use crate::{ChatMessage, ChatRequest, Model};

    #[test]
    fn chat_request_forces_stream_and_usage() {
        let wire = ChatRequestWire::from(ChatRequest::new(vec![ChatMessage::user("hello")]));
        let value = serde_json::to_value(wire).unwrap();
        assert_eq!(value["model"], "auto");
        assert_eq!(value["stream"], true);
        assert_eq!(value["stream_options"]["include_usage"], true);
    }

    #[test]
    fn authorization_payloads_use_authoritative_casing() {
        assert_eq!(
            serde_json::to_value(DeviceAuthorizeRequestWire { product: "code" }).unwrap(),
            json!({"product": "code"})
        );
        assert_eq!(
            serde_json::to_value(DeviceTokenRequestWire {
                device_code: "secret-device-code"
            })
            .unwrap(),
            json!({"deviceCode": "secret-device-code"})
        );
    }

    #[test]
    fn decodes_model_list_to_public_dtos() {
        let wire: ModelsWire = serde_json::from_value(json!({
            "object": "list",
            "data": [{
                "id": "redrob/auto",
                "name": "Automatic",
                "description": "Chooses a model",
                "owned_by": "redrob",
                "created": 42,
                "extra": true
            }]
        }))
        .unwrap();
        let models: Vec<Model> = wire.data.into_iter().map(Into::into).collect();
        assert_eq!(models[0].id, "redrob/auto");
        assert_eq!(models[0].name.as_deref(), Some("Automatic"));
        assert_eq!(models[0].created, Some(42));
    }

    #[test]
    fn mutating_flag_is_not_sent_to_provider() {
        let tool = crate::FunctionTool::new("save", json!({"type": "object"}))
            .unwrap()
            .mutating(true);
        let value: Value = serde_json::to_value(ChatRequestWire::from(
            ChatRequest::new(vec![]).with_tools(vec![tool]),
        ))
        .unwrap();
        assert!(value["tools"][0]["function"].get("mutating").is_none());
    }
}
