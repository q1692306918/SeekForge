use crate::common::ChatToolName;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

const REQUEST_ID_HEADER: &str = "x-request-id";

pub fn spawn_chat_completions_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    tool_names: HashMap<String, ChatToolName>,
) -> ResponseStream {
    let upstream_request_id = stream_response
        .headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(process_chat_completions_sse(
        stream_response.bytes,
        tx_event,
        idle_timeout,
        telemetry,
        tool_names,
    ));

    ResponseStream {
        rx_event,
        upstream_request_id,
    }
}

#[derive(Debug, Default)]
struct ChatStreamState {
    response_id: Option<String>,
    assistant_text: String,
    tool_calls: BTreeMap<i64, ToolCallAccumulator>,
    usage: Option<TokenUsage>,
    finish_reason: Option<String>,
    finished_items_emitted: bool,
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    call_id: Option<String>,
    chat_name: Option<String>,
    arguments: String,
    start_emitted: bool,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    id: Option<String>,
    #[serde(default)]
    choices: Vec<ChatCompletionChoice>,
    usage: Option<ChatCompletionUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    #[serde(default)]
    delta: ChatCompletionDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatCompletionDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatCompletionToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionToolCallDelta {
    index: i64,
    id: Option<String>,
    function: Option<ChatCompletionFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionUsage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
    prompt_cache_hit_tokens: Option<i64>,
    prompt_cache_miss_tokens: Option<i64>,
    prompt_tokens_details: Option<ChatCompletionPromptTokensDetails>,
    completion_tokens_details: Option<ChatCompletionCompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionPromptTokensDetails {
    #[serde(default)]
    cached_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionCompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: i64,
}

impl From<ChatCompletionUsage> for TokenUsage {
    fn from(usage: ChatCompletionUsage) -> Self {
        let cached_input_tokens = usage
            .prompt_cache_hit_tokens
            .or_else(|| {
                usage
                    .prompt_tokens_details
                    .map(|details| details.cached_tokens)
            })
            .unwrap_or(0);
        let input_tokens = if usage.prompt_tokens > 0 {
            usage.prompt_tokens
        } else {
            usage.prompt_cache_hit_tokens.unwrap_or(0) + usage.prompt_cache_miss_tokens.unwrap_or(0)
        };

        TokenUsage {
            input_tokens,
            cached_input_tokens,
            output_tokens: usage.completion_tokens,
            reasoning_output_tokens: usage
                .completion_tokens_details
                .map(|details| details.reasoning_tokens)
                .unwrap_or(0),
            total_tokens: usage.total_tokens,
        }
    }
}

pub async fn process_chat_completions_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    tool_names: HashMap<String, ChatToolName>,
) {
    let mut stream = stream.eventsource();
    let mut state = ChatStreamState::default();

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(err))) => {
                debug!("Chat completions SSE error: {err:#}");
                let _ = tx_event.send(Err(ApiError::Stream(err.to_string()))).await;
                return;
            }
            Ok(None) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "stream closed before chat completion finished".into(),
                    )))
                    .await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!("chat completions SSE event: {}", &sse.data);
        if sse.data.trim() == "[DONE]" {
            emit_finished_items(&mut state, &tx_event, &tool_names).await;
            emit_completed(state, &tx_event).await;
            return;
        }

        let chunk: ChatCompletionChunk = match serde_json::from_str(&sse.data) {
            Ok(chunk) => chunk,
            Err(err) => {
                debug!(
                    "failed to parse chat completions SSE event: {err}, data: {}",
                    &sse.data
                );
                continue;
            }
        };
        process_chat_completion_chunk(chunk, &mut state, &tx_event, &tool_names).await;
    }
}

async fn process_chat_completion_chunk(
    chunk: ChatCompletionChunk,
    state: &mut ChatStreamState,
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    tool_names: &HashMap<String, ChatToolName>,
) {
    if let Some(id) = chunk.id {
        state.response_id = Some(id);
    }
    if let Some(usage) = chunk.usage {
        state.usage = Some(usage.into());
    }

    for choice in chunk.choices {
        let ChatCompletionDelta {
            content,
            reasoning_content,
            tool_calls,
        } = choice.delta;

        if let Some(delta) = reasoning_content
            && tx_event
                .send(Ok(ResponseEvent::ReasoningContentDelta {
                    delta,
                    content_index: 0,
                }))
                .await
                .is_err()
        {
            return;
        }

        if let Some(delta) = content {
            state.assistant_text.push_str(&delta);
            if tx_event
                .send(Ok(ResponseEvent::OutputTextDelta(delta)))
                .await
                .is_err()
            {
                return;
            }
        }

        for tool_call in tool_calls {
            process_tool_call_delta(tool_call, state, tx_event, tool_names).await;
        }

        if let Some(finish_reason) = choice.finish_reason {
            state.finish_reason = Some(finish_reason);
            emit_finished_items(state, tx_event, tool_names).await;
        }
    }
}

async fn process_tool_call_delta(
    delta: ChatCompletionToolCallDelta,
    state: &mut ChatStreamState,
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    tool_names: &HashMap<String, ChatToolName>,
) {
    let accumulator = state.tool_calls.entry(delta.index).or_default();
    if let Some(call_id) = delta.id {
        accumulator.call_id = Some(call_id);
    }
    if let Some(function) = delta.function {
        if let Some(name) = function.name {
            accumulator.chat_name = Some(name);
        }
        if !accumulator.start_emitted
            && accumulator.call_id.is_some()
            && accumulator.chat_name.is_some()
        {
            let item = response_item_for_tool_call(accumulator, tool_names, String::new());
            accumulator.start_emitted = true;
            if tx_event
                .send(Ok(ResponseEvent::OutputItemAdded(item)))
                .await
                .is_err()
            {
                return;
            }
        }
        if let Some(arguments) = function.arguments {
            accumulator.arguments.push_str(&arguments);
            if let Some(call_id) = accumulator.call_id.as_ref()
                && tx_event
                    .send(Ok(ResponseEvent::ToolCallInputDelta {
                        item_id: call_id.clone(),
                        call_id: Some(call_id.clone()),
                        delta: arguments,
                    }))
                    .await
                    .is_err()
            {}
        }
    }
}

async fn emit_finished_items(
    state: &mut ChatStreamState,
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    tool_names: &HashMap<String, ChatToolName>,
) {
    if state.finished_items_emitted {
        return;
    }

    if !state.tool_calls.is_empty() {
        for accumulator in state.tool_calls.values() {
            let item =
                response_item_for_tool_call(accumulator, tool_names, accumulator.arguments.clone());
            if tx_event
                .send(Ok(ResponseEvent::OutputItemDone(item)))
                .await
                .is_err()
            {
                return;
            }
        }
        state.finished_items_emitted = true;
        return;
    }

    if !state.assistant_text.is_empty() {
        let item = ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: state.assistant_text.clone(),
            }],
            phase: None,
        };
        let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
        state.finished_items_emitted = true;
    }
}

fn response_item_for_tool_call(
    accumulator: &ToolCallAccumulator,
    tool_names: &HashMap<String, ChatToolName>,
    arguments: String,
) -> ResponseItem {
    let chat_name = accumulator.chat_name.clone().unwrap_or_default();
    let tool_name = tool_names.get(&chat_name).cloned().unwrap_or(ChatToolName {
        namespace: None,
        name: chat_name,
    });
    ResponseItem::FunctionCall {
        id: None,
        name: tool_name.name,
        namespace: tool_name.namespace,
        arguments,
        call_id: accumulator.call_id.clone().unwrap_or_default(),
    }
}

async fn emit_completed(
    state: ChatStreamState,
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
) {
    let end_turn = state
        .finish_reason
        .as_deref()
        .map(|reason| reason != "tool_calls");
    let response_id = state
        .response_id
        .unwrap_or_else(|| "chatcmpl-unknown".to_string());
    let _ = tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id,
            token_usage: state.usage,
            end_turn,
        }))
        .await;
}

#[cfg(test)]
mod tests {
    use super::process_chat_completions_sse;
    use crate::common::ChatToolName;
    use crate::common::ResponseEvent;
    use crate::error::ApiError;
    use assert_matches::assert_matches;
    use bytes::Bytes;
    use codex_client::TransportError;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use futures::stream;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    async fn collect_events(
        events: Vec<serde_json::Value>,
        tool_names: HashMap<String, ChatToolName>,
    ) -> Vec<Result<ResponseEvent, ApiError>> {
        let sse = events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>()
            + "data: [DONE]\n\n";
        let stream = stream::iter(vec![Ok::<_, TransportError>(Bytes::from(sse))]);
        let (tx, mut rx) = mpsc::channel::<Result<ResponseEvent, ApiError>>(16);
        tokio::spawn(process_chat_completions_sse(
            Box::pin(stream),
            tx,
            std::time::Duration::from_secs(5),
            /*telemetry*/ None,
            tool_names,
        ));

        let mut collected = Vec::new();
        while let Some(event) = rx.recv().await {
            collected.push(event);
        }
        collected
    }

    #[tokio::test]
    async fn streams_text_reasoning_and_deepseek_usage() {
        let events = collect_events(
            vec![
                json!({
                    "id": "chatcmpl-1",
                    "choices": [{
                        "delta": {"reasoning_content": "thinking"},
                        "finish_reason": null
                    }]
                }),
                json!({
                    "id": "chatcmpl-1",
                    "choices": [{
                        "delta": {"content": "hello"},
                        "finish_reason": null
                    }]
                }),
                json!({
                    "id": "chatcmpl-1",
                    "choices": [{
                        "delta": {},
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 5,
                        "total_tokens": 15,
                        "prompt_cache_hit_tokens": 7,
                        "prompt_cache_miss_tokens": 3,
                        "completion_tokens_details": {"reasoning_tokens": 2}
                    }
                }),
            ],
            HashMap::new(),
        )
        .await;

        assert_matches!(
            &events[0],
            Ok(ResponseEvent::ReasoningContentDelta {
                delta,
                content_index: 0
            }) if delta == "thinking"
        );
        assert_matches!(
            &events[1],
            Ok(ResponseEvent::OutputTextDelta(delta)) if delta == "hello"
        );
        assert_matches!(
            &events[2],
            Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
                role,
                content,
                phase: None,
                ..
            })) if role == "assistant"
                && content == &vec![ContentItem::OutputText {
                    text: "hello".to_string(),
                }]
        );
        assert_matches!(
            &events[3],
            Ok(ResponseEvent::Completed {
                response_id,
                token_usage: Some(usage),
                end_turn: Some(true),
            }) if response_id == "chatcmpl-1"
                && usage.input_tokens == 10
                && usage.cached_input_tokens == 7
                && usage.output_tokens == 5
                && usage.reasoning_output_tokens == 2
                && usage.total_tokens == 15
        );
        assert_eq!(events.len(), 4);
    }

    #[tokio::test]
    async fn deepseek_usage_falls_back_to_cache_hit_and_miss_tokens() {
        let events = collect_events(
            vec![json!({
                "id": "chatcmpl-cache",
                "choices": [{
                    "delta": {},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "completion_tokens": 2,
                    "total_tokens": 14,
                    "prompt_cache_hit_tokens": 9,
                    "prompt_cache_miss_tokens": 3
                }
            })],
            HashMap::new(),
        )
        .await;

        assert_matches!(
            &events[0],
            Ok(ResponseEvent::Completed {
                response_id,
                token_usage: Some(usage),
                end_turn: Some(true),
            }) if response_id == "chatcmpl-cache"
                && usage.input_tokens == 12
                && usage.cached_input_tokens == 9
                && usage.non_cached_input() == 3
                && usage.output_tokens == 2
                && usage.total_tokens == 14
        );
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn accumulates_streamed_tool_call_deltas_by_index() {
        let events = collect_events(
            vec![
                json!({
                    "id": "chatcmpl-2",
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "mcp__demo__lookup",
                                    "arguments": "{\"q\":\""
                                }
                            }]
                        },
                        "finish_reason": null
                    }]
                }),
                json!({
                    "id": "chatcmpl-2",
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "function": {"arguments": "x\"}"}
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }]
                }),
            ],
            HashMap::from([(
                "mcp__demo__lookup".to_string(),
                ChatToolName {
                    namespace: Some("mcp__demo__".to_string()),
                    name: "lookup".to_string(),
                },
            )]),
        )
        .await;

        assert_matches!(
            &events[0],
            Ok(ResponseEvent::OutputItemAdded(ResponseItem::FunctionCall {
                name,
                namespace: Some(namespace),
                arguments,
                call_id,
                ..
            })) if name == "lookup"
                && namespace == "mcp__demo__"
                && arguments.is_empty()
                && call_id == "call_1"
        );
        assert_matches!(
            &events[1],
            Ok(ResponseEvent::ToolCallInputDelta {
                item_id,
                call_id: Some(call_id),
                delta,
            }) if item_id == "call_1" && call_id == "call_1" && delta == "{\"q\":\""
        );
        assert_matches!(
            &events[2],
            Ok(ResponseEvent::ToolCallInputDelta {
                item_id,
                call_id: Some(call_id),
                delta,
            }) if item_id == "call_1" && call_id == "call_1" && delta == "x\"}"
        );
        assert_matches!(
            &events[3],
            Ok(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                name,
                namespace: Some(namespace),
                arguments,
                call_id,
                ..
            })) if name == "lookup"
                && namespace == "mcp__demo__"
                && arguments == "{\"q\":\"x\"}"
                && call_id == "call_1"
        );
        assert_matches!(
            &events[4],
            Ok(ResponseEvent::Completed {
                response_id,
                token_usage: None,
                end_turn: Some(false),
            }) if response_id == "chatcmpl-2"
        );
        assert_eq!(events.len(), 5);
    }
}
