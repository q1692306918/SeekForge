use crate::error::ApiError;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::config_types::Verbosity as VerbosityConfig;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::ModelVerification;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::protocol::W3cTraceContext;
use futures::Stream;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::sync::mpsc;

pub const WS_REQUEST_HEADER_TRACEPARENT_CLIENT_METADATA_KEY: &str = "ws_request_header_traceparent";
pub const WS_REQUEST_HEADER_TRACESTATE_CLIENT_METADATA_KEY: &str = "ws_request_header_tracestate";

/// Canonical input payload for the compaction endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct CompactionInput<'a> {
    pub model: &'a str,
    pub input: &'a [ResponseItem],
    #[serde(skip_serializing_if = "str::is_empty")]
    pub instructions: &'a str,
    pub tools: Vec<Value>,
    pub parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextControls>,
}

/// Canonical input payload for the memory summarize endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct MemorySummarizeInput {
    pub model: String,
    #[serde(rename = "traces")]
    pub raw_memories: Vec<RawMemory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawMemory {
    pub id: String,
    pub metadata: RawMemoryMetadata,
    pub items: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawMemoryMetadata {
    pub source_path: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MemorySummarizeOutput {
    #[serde(rename = "trace_summary", alias = "raw_memory")]
    pub raw_memory: String,
    pub memory_summary: String,
}

#[derive(Debug)]
pub enum ResponseEvent {
    Created,
    OutputItemDone(ResponseItem),
    OutputItemAdded(ResponseItem),
    /// Emitted when the server includes `OpenAI-Model` on the stream response.
    /// This can differ from the requested model when backend safety routing applies.
    ServerModel(String),
    /// Emitted when the server recommends additional account verification.
    ModelVerifications(Vec<ModelVerification>),
    /// Emitted when `X-Reasoning-Included: true` is present on the response,
    /// meaning the server already accounted for past reasoning tokens and the
    /// client should not re-estimate them.
    ServerReasoningIncluded(bool),
    Completed {
        response_id: String,
        token_usage: Option<TokenUsage>,
        /// Did the model affirmatively end its turn? Some providers do not set this,
        /// so we rely on fallback logic when this is `None`.
        end_turn: Option<bool>,
    },
    OutputTextDelta(String),
    ToolCallInputDelta {
        item_id: String,
        call_id: Option<String>,
        delta: String,
    },
    ReasoningSummaryDelta {
        delta: String,
        summary_index: i64,
    },
    ReasoningContentDelta {
        delta: String,
        content_index: i64,
    },
    ReasoningSummaryPartAdded {
        summary_index: i64,
    },
    RateLimits(RateLimitSnapshot),
    ModelsEtag(String),
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Reasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffortConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReasoningSummaryConfig>,
}

#[derive(Debug, Serialize, Default, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TextFormatType {
    #[default]
    JsonSchema,
}

#[derive(Debug, Serialize, Default, Clone, PartialEq)]
pub struct TextFormat {
    /// Format type used by the OpenAI text controls.
    pub r#type: TextFormatType,
    /// When true, the server is expected to strictly validate responses.
    pub strict: bool,
    /// JSON schema for the desired output.
    pub schema: Value,
    /// Friendly name for the format, used in telemetry/debugging.
    pub name: String,
}

/// Controls the `text` field for the Responses API, combining verbosity and
/// optional JSON schema output formatting.
#[derive(Debug, Serialize, Default, Clone, PartialEq)]
pub struct TextControls {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<OpenAiVerbosity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<TextFormat>,
}

#[derive(Debug, Serialize, Default, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiVerbosity {
    Low,
    #[default]
    Medium,
    High,
}

impl From<VerbosityConfig> for OpenAiVerbosity {
    fn from(v: VerbosityConfig) -> Self {
        match v {
            VerbosityConfig::Low => OpenAiVerbosity::Low,
            VerbosityConfig::Medium => OpenAiVerbosity::Medium,
            VerbosityConfig::High => OpenAiVerbosity::High,
        }
    }
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ResponsesApiRequest {
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    pub input: Vec<ResponseItem>,
    pub tools: Vec<serde_json::Value>,
    pub tool_choice: String,
    pub parallel_tool_calls: bool,
    pub reasoning: Option<Reasoning>,
    pub store: bool,
    pub stream: bool,
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextControls>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatToolName {
    pub namespace: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatCompletionsApiRequest {
    pub model: String,
    pub messages: Vec<ChatCompletionMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ChatCompletionTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub parallel_tool_calls: bool,
    pub stream: bool,
    pub stream_options: ChatCompletionStreamOptions,
    #[serde(skip)]
    tool_names: HashMap<String, ChatToolName>,
}

impl ChatCompletionsApiRequest {
    pub fn from_responses_request(request: ResponsesApiRequest) -> Self {
        let (tools, tool_names) = chat_tools_from_responses_tools(&request.tools);
        let mut messages = Vec::new();
        if !request.instructions.is_empty() {
            messages.push(ChatCompletionMessage::text("system", request.instructions));
        }
        for item in request.input {
            append_chat_messages_for_response_item(&mut messages, item);
        }

        Self {
            model: request.model,
            messages,
            tool_choice: (!tools.is_empty()).then_some(request.tool_choice),
            parallel_tool_calls: !tools.is_empty() && request.parallel_tool_calls,
            stream: true,
            stream_options: ChatCompletionStreamOptions {
                include_usage: true,
            },
            tools,
            tool_names,
        }
    }

    pub fn tool_name_for_chat_name(&self, chat_name: &str) -> Option<ChatToolName> {
        self.tool_names.get(chat_name).cloned()
    }

    pub fn tool_names(&self) -> HashMap<String, ChatToolName> {
        self.tool_names.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatCompletionStreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatCompletionMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatCompletionMessageToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatCompletionMessage {
    fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn tool_output(call_id: String, content: String) -> Self {
        Self {
            role: "tool".to_string(),
            content,
            tool_calls: None,
            tool_call_id: Some(call_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatCompletionMessageToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ChatCompletionFunctionCall,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatCompletionFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatCompletionTool {
    pub r#type: String,
    pub function: ChatCompletionToolFunction,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatCompletionToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

fn chat_tools_from_responses_tools(
    tools: &[Value],
) -> (Vec<ChatCompletionTool>, HashMap<String, ChatToolName>) {
    let mut function_tools = Vec::new();
    for tool in tools {
        let Some(tool_type) = tool.get("type").and_then(Value::as_str) else {
            continue;
        };
        match tool_type {
            "function" => {
                append_chat_function_tool(&mut function_tools, None, None, tool);
            }
            "namespace" => {
                let namespace = tool.get("name").and_then(Value::as_str);
                let namespace_description = tool.get("description").and_then(Value::as_str);
                if let Some(namespace_tools) = tool.get("tools").and_then(Value::as_array) {
                    for namespace_tool in namespace_tools {
                        append_chat_function_tool(
                            &mut function_tools,
                            namespace,
                            namespace_description,
                            namespace_tool,
                        );
                    }
                }
            }
            "tool_search" => {
                append_chat_function_tool(
                    &mut function_tools,
                    None,
                    None,
                    &json_like_tool_search(tool),
                );
            }
            "web_search" | "image_generation" | "custom" => {}
            _ => {}
        }
    }

    function_tools.sort_by(|left, right| left.tool.function.name.cmp(&right.tool.function.name));

    let mut chat_tools = Vec::with_capacity(function_tools.len());
    let mut tool_names = HashMap::with_capacity(function_tools.len());
    for function_tool in function_tools {
        tool_names.insert(
            function_tool.tool.function.name.clone(),
            function_tool.tool_name,
        );
        chat_tools.push(function_tool.tool);
    }

    (chat_tools, tool_names)
}

#[derive(Debug)]
struct ChatFunctionTool {
    tool_name: ChatToolName,
    tool: ChatCompletionTool,
}

fn append_chat_function_tool(
    function_tools: &mut Vec<ChatFunctionTool>,
    namespace: Option<&str>,
    namespace_description: Option<&str>,
    tool: &Value,
) {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return;
    }
    let Some(name) = tool.get("name").and_then(Value::as_str) else {
        return;
    };
    let chat_name = chat_name_for_tool_name(namespace, name);
    let description = chat_tool_description(namespace_description, tool);
    let parameters = tool
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));

    function_tools.push(ChatFunctionTool {
        tool_name: ChatToolName {
            namespace: namespace.map(str::to_string),
            name: name.to_string(),
        },
        tool: ChatCompletionTool {
            r#type: "function".to_string(),
            function: ChatCompletionToolFunction {
                name: chat_name,
                description,
                parameters,
            },
        },
    });
}

fn json_like_tool_search(tool: &Value) -> Value {
    serde_json::json!({
        "type": "function",
        "name": "tool_search",
        "description": tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "parameters": tool
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
    })
}

fn chat_tool_description(namespace_description: Option<&str>, tool: &Value) -> String {
    let tool_description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match namespace_description.filter(|description| !description.trim().is_empty()) {
        Some(namespace_description) if tool_description.trim().is_empty() => {
            namespace_description.to_string()
        }
        Some(namespace_description) => format!("{namespace_description}\n\n{tool_description}"),
        None => tool_description.to_string(),
    }
}

fn append_chat_messages_for_response_item(
    messages: &mut Vec<ChatCompletionMessage>,
    item: ResponseItem,
) {
    match item {
        ResponseItem::Message { role, content, .. } => {
            messages.push(ChatCompletionMessage::text(
                role,
                content_items_to_chat_text(&content),
            ));
        }
        ResponseItem::FunctionCall {
            name,
            namespace,
            arguments,
            call_id,
            ..
        } => messages.push(ChatCompletionMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![ChatCompletionMessageToolCall {
                id: call_id,
                r#type: "function".to_string(),
                function: ChatCompletionFunctionCall {
                    name: chat_name_for_tool_name(namespace.as_deref(), &name),
                    arguments,
                },
            }]),
            tool_call_id: None,
        }),
        ResponseItem::FunctionCallOutput { call_id, output } => {
            messages.push(ChatCompletionMessage::tool_output(
                call_id,
                output.to_string(),
            ));
        }
        ResponseItem::CustomToolCall {
            call_id,
            name,
            input,
            ..
        } => messages.push(ChatCompletionMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![ChatCompletionMessageToolCall {
                id: call_id,
                r#type: "function".to_string(),
                function: ChatCompletionFunctionCall {
                    name,
                    arguments: input,
                },
            }]),
            tool_call_id: None,
        }),
        ResponseItem::CustomToolCallOutput {
            call_id, output, ..
        } => messages.push(ChatCompletionMessage::tool_output(
            call_id,
            output.to_string(),
        )),
        ResponseItem::ToolSearchOutput {
            call_id: Some(call_id),
            execution,
            ..
        } => messages.push(ChatCompletionMessage::tool_output(call_id, execution)),
        ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::ToolSearchOutput { call_id: None, .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::CompactionTrigger
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => {}
    }
}

pub fn chat_name_for_tool_name(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(namespace) if namespace.ends_with("__") || namespace.ends_with('/') => {
            format!("{namespace}{name}")
        }
        Some(namespace) => format!("{namespace}__{name}"),
        None => name.to_string(),
    }
}

fn content_items_to_chat_text(content: &[ContentItem]) -> String {
    content
        .iter()
        .map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => text.clone(),
            ContentItem::InputImage { image_url, .. } => format!("[image: {image_url}]"),
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl From<&ResponsesApiRequest> for ResponseCreateWsRequest {
    fn from(request: &ResponsesApiRequest) -> Self {
        Self {
            model: request.model.clone(),
            instructions: request.instructions.clone(),
            previous_response_id: None,
            input: request.input.clone(),
            tools: request.tools.clone(),
            tool_choice: request.tool_choice.clone(),
            parallel_tool_calls: request.parallel_tool_calls,
            reasoning: request.reasoning.clone(),
            store: request.store,
            stream: request.stream,
            include: request.include.clone(),
            service_tier: request.service_tier.clone(),
            prompt_cache_key: request.prompt_cache_key.clone(),
            text: request.text.clone(),
            generate: None,
            client_metadata: request.client_metadata.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ResponseCreateWsRequest {
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    pub input: Vec<ResponseItem>,
    pub tools: Vec<Value>,
    pub tool_choice: String,
    pub parallel_tool_calls: bool,
    pub reasoning: Option<Reasoning>,
    pub store: bool,
    pub stream: bool,
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextControls>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct ResponseProcessedWsRequest {
    pub response_id: String,
}

pub fn response_create_client_metadata(
    client_metadata: Option<HashMap<String, String>>,
    trace: Option<&W3cTraceContext>,
) -> Option<HashMap<String, String>> {
    let mut client_metadata = client_metadata.unwrap_or_default();

    if let Some(traceparent) = trace.and_then(|trace| trace.traceparent.as_deref()) {
        client_metadata.insert(
            WS_REQUEST_HEADER_TRACEPARENT_CLIENT_METADATA_KEY.to_string(),
            traceparent.to_string(),
        );
    }
    if let Some(tracestate) = trace.and_then(|trace| trace.tracestate.as_deref()) {
        client_metadata.insert(
            WS_REQUEST_HEADER_TRACESTATE_CLIENT_METADATA_KEY.to_string(),
            tracestate.to_string(),
        );
    }

    (!client_metadata.is_empty()).then_some(client_metadata)
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum ResponsesWsRequest {
    #[serde(rename = "response.create")]
    ResponseCreate(ResponseCreateWsRequest),
    #[serde(rename = "response.processed")]
    ResponseProcessed(ResponseProcessedWsRequest),
}

pub fn create_text_param_for_request(
    verbosity: Option<VerbosityConfig>,
    output_schema: &Option<Value>,
    output_schema_strict: bool,
) -> Option<TextControls> {
    if verbosity.is_none() && output_schema.is_none() {
        return None;
    }

    Some(TextControls {
        verbosity: verbosity.map(std::convert::Into::into),
        format: output_schema.as_ref().map(|schema| TextFormat {
            r#type: TextFormatType::JsonSchema,
            strict: output_schema_strict,
            schema: schema.clone(),
            name: "codex_output_schema".to_string(),
        }),
    })
}

pub struct ResponseStream {
    pub rx_event: mpsc::Receiver<Result<ResponseEvent, ApiError>>,
    /// Server-assigned `x-request-id` response header, when present.
    pub upstream_request_id: Option<String>,
}

impl Stream for ResponseStream {
    type Item = Result<ResponseEvent, ApiError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx_event.poll_recv(cx)
    }
}

#[cfg(test)]
mod chat_completions_tests {
    use super::*;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::models::ReasoningItemContent;
    use codex_protocol::models::ReasoningItemReasoningSummary;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn base_responses_request() -> ResponsesApiRequest {
        ResponsesApiRequest {
            model: "deepseek-v4-flash".to_string(),
            instructions: "base instructions".to_string(),
            input: Vec::new(),
            tools: Vec::new(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: Some("stable-cache-key".to_string()),
            text: None,
            client_metadata: None,
        }
    }

    #[test]
    fn chat_completions_request_serializes_messages_tools_and_filters_reasoning_content() {
        let mut request = base_responses_request();
        request.input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hello".to_string(),
                }],
                phase: None,
            },
            ResponseItem::Reasoning {
                id: "rs_1".to_string(),
                summary: vec![ReasoningItemReasoningSummary::SummaryText {
                    text: "summary".to_string(),
                }],
                content: Some(vec![ReasoningItemContent::ReasoningText {
                    text: "hidden reasoning_content".to_string(),
                }]),
                encrypted_content: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "lookup".to_string(),
                namespace: Some("mcp__demo__".to_string()),
                arguments: "{\"q\":\"x\"}".to_string(),
                call_id: "call_1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call_1".to_string(),
                output: FunctionCallOutputPayload::from_text("ok".to_string()),
            },
        ];
        request.tools = vec![json!({
            "type": "namespace",
            "name": "mcp__demo__",
            "description": "Demo tools.",
            "tools": [{
                "type": "function",
                "name": "lookup",
                "description": "Look up a value.",
                "parameters": {"type": "object", "properties": {}}
            }]
        })];

        let chat = ChatCompletionsApiRequest::from_responses_request(request);
        let serialized = serde_json::to_value(&chat).expect("chat request should serialize");

        assert_eq!(
            serialized,
            json!({
                "model": "deepseek-v4-flash",
                "messages": [
                    {"role": "system", "content": "base instructions"},
                    {"role": "user", "content": "hello"},
                    {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "mcp__demo__lookup",
                                "arguments": "{\"q\":\"x\"}"
                            }
                        }]
                    },
                    {"role": "tool", "content": "ok", "tool_call_id": "call_1"}
                ],
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "mcp__demo__lookup",
                        "description": "Demo tools.\n\nLook up a value.",
                        "parameters": {"type": "object", "properties": {}}
                    }
                }],
                "tool_choice": "auto",
                "parallel_tool_calls": true,
                "stream": true,
                "stream_options": {"include_usage": true}
            })
        );
        assert!(!serialized.to_string().contains("hidden reasoning_content"));
        assert_eq!(
            chat.tool_name_for_chat_name("mcp__demo__lookup"),
            Some(ChatToolName {
                namespace: Some("mcp__demo__".to_string()),
                name: "lookup".to_string(),
            })
        );
    }

    #[test]
    fn chat_completions_request_keeps_serialized_prefix_stable_when_appending_tail() {
        let mut first = base_responses_request();
        first.input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "first stable user turn".to_string(),
                }],
                phase: None,
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "first stable assistant turn".to_string(),
                }],
                phase: None,
            },
        ];

        let mut second = first.clone();
        second.input.push(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "new tail turn".to_string(),
            }],
            phase: None,
        });

        let first_chat = ChatCompletionsApiRequest::from_responses_request(first);
        let second_chat = ChatCompletionsApiRequest::from_responses_request(second);
        let first_prefix_messages =
            serde_json::to_vec(&first_chat.messages).expect("first chat messages should serialize");
        let second_prefix_messages =
            serde_json::to_vec(&second_chat.messages[..first_chat.messages.len()])
                .expect("second chat prefix messages should serialize");

        assert_eq!(first_prefix_messages, second_prefix_messages);
    }

    #[test]
    fn chat_completions_request_sorts_tools_for_stable_prefix() {
        let mut first = base_responses_request();
        first.tools = vec![
            json!({
                "type": "namespace",
                "name": "mcp__zeta__",
                "description": "Zeta namespace.",
                "tools": [{
                    "type": "function",
                    "name": "search",
                    "description": "Search zeta.",
                    "parameters": {"type": "object", "properties": {"query": {"type": "string"}}}
                }]
            }),
            json!({
                "type": "tool_search",
                "description": "Find deferred tools.",
                "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}
            }),
            json!({
                "type": "function",
                "name": "alpha",
                "description": "Alpha local tool.",
                "parameters": {"type": "object", "properties": {}}
            }),
            json!({
                "type": "namespace",
                "name": "mcp__alpha__",
                "description": "Alpha namespace.",
                "tools": [{
                    "type": "function",
                    "name": "lookup",
                    "description": "Look up alpha.",
                    "parameters": {"type": "object", "properties": {"id": {"type": "string"}}}
                }]
            }),
        ];

        let mut second = base_responses_request();
        second.tools = first.tools.iter().rev().cloned().collect();

        let first_chat = ChatCompletionsApiRequest::from_responses_request(first);
        let second_chat = ChatCompletionsApiRequest::from_responses_request(second);
        let first_tools =
            serde_json::to_vec(&first_chat.tools).expect("first tools should serialize");
        let second_tools =
            serde_json::to_vec(&second_chat.tools).expect("second tools should serialize");

        assert_eq!(first_tools, second_tools);
        assert_eq!(
            first_chat
                .tools
                .iter()
                .map(|tool| tool.function.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "alpha",
                "mcp__alpha__lookup",
                "mcp__zeta__search",
                "tool_search",
            ]
        );
        assert_eq!(
            first_chat.tool_name_for_chat_name("mcp__alpha__lookup"),
            Some(ChatToolName {
                namespace: Some("mcp__alpha__".to_string()),
                name: "lookup".to_string(),
            })
        );
    }

    #[tokio::test]
    #[ignore = "requires DEEPSEEK_API_KEY and live DeepSeek network access"]
    async fn live_deepseek_cache_probe_reports_hit_miss_and_reasoning_usage() -> anyhow::Result<()>
    {
        let api_key = match std::env::var("DEEPSEEK_API_KEY") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                eprintln!("skipping live DeepSeek cache probe: DEEPSEEK_API_KEY is not set");
                return Ok(());
            }
        };
        let mut request = base_responses_request();
        request.instructions =
            "You are a deterministic cache probe. Reply only with cache-probe-ok.".to_string();
        request.input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Return exactly: cache-probe-ok".to_string(),
                }],
                phase: None,
            },
            ResponseItem::Reasoning {
                id: "rs_live_probe".to_string(),
                summary: Vec::new(),
                content: Some(vec![ReasoningItemContent::ReasoningText {
                    text: "reasoning_content_must_not_be_sent_to_deepseek".to_string(),
                }]),
                encrypted_content: None,
            },
        ];

        let chat = ChatCompletionsApiRequest::from_responses_request(request);
        let serialized = serde_json::to_value(&chat).expect("chat request should serialize");
        assert!(
            !serialized
                .to_string()
                .contains("reasoning_content_must_not_be_sent_to_deepseek")
        );

        let client = reqwest::Client::new();
        let first = post_live_deepseek_probe(&client, &api_key, &serialized).await?;
        let second = post_live_deepseek_probe(&client, &api_key, &serialized).await?;

        eprintln!(
            "first probe usage: hit={} miss={} reasoning={} total={}",
            first.prompt_cache_hit_tokens,
            first.prompt_cache_miss_tokens,
            first.reasoning_tokens,
            first.total_tokens
        );
        eprintln!(
            "second probe usage: hit={} miss={} reasoning={} total={}",
            second.prompt_cache_hit_tokens,
            second.prompt_cache_miss_tokens,
            second.reasoning_tokens,
            second.total_tokens
        );
        assert!(
            first.total_tokens > 0 || second.total_tokens > 0,
            "live probe should report DeepSeek usage"
        );
        Ok(())
    }

    #[derive(Debug, Default)]
    struct DeepSeekLiveProbeUsage {
        prompt_cache_hit_tokens: i64,
        prompt_cache_miss_tokens: i64,
        reasoning_tokens: i64,
        total_tokens: i64,
    }

    async fn post_live_deepseek_probe(
        client: &reqwest::Client,
        api_key: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<DeepSeekLiveProbeUsage> {
        let response_text = client
            .post("https://api.deepseek.com/chat/completions")
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(parse_live_deepseek_usage(&response_text))
    }

    fn parse_live_deepseek_usage(response_text: &str) -> DeepSeekLiveProbeUsage {
        let mut usage = DeepSeekLiveProbeUsage::default();
        for line in response_text.lines() {
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };
            if let Some(raw_usage) = value.get("usage") {
                usage.prompt_cache_hit_tokens = raw_usage
                    .get("prompt_cache_hit_tokens")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(usage.prompt_cache_hit_tokens);
                usage.prompt_cache_miss_tokens = raw_usage
                    .get("prompt_cache_miss_tokens")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(usage.prompt_cache_miss_tokens);
                usage.reasoning_tokens = raw_usage
                    .pointer("/completion_tokens_details/reasoning_tokens")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(usage.reasoning_tokens);
                usage.total_tokens = raw_usage
                    .get("total_tokens")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(usage.total_tokens);
            }
        }
        usage
    }

    #[test]
    fn parse_live_deepseek_usage_extracts_cache_and_reasoning_fields() {
        let usage = parse_live_deepseek_usage(
            r#"data: {"usage":{"prompt_cache_hit_tokens":11,"prompt_cache_miss_tokens":7,"completion_tokens_details":{"reasoning_tokens":3},"total_tokens":29}}

data: [DONE]
"#,
        );

        assert_eq!(usage.prompt_cache_hit_tokens, 11);
        assert_eq!(usage.prompt_cache_miss_tokens, 7);
        assert_eq!(usage.reasoning_tokens, 3);
        assert_eq!(usage.total_tokens, 29);
    }
}
