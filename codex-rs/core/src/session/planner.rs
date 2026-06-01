use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::stream_events_utils::last_assistant_message_from_item;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use futures::StreamExt;
use futures::future::BoxFuture;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;
use tracing::debug;
use tracing::trace_span;

const PLANNER_PROMPT: &str = r#"You are the optional DeepSeek-native planner for this Codex turn.

Read the conversation and the newest user request, then produce concise executor guidance.
Do not call tools. Do not ask the user questions. Do not include implementation details that depend on unavailable tool output.
Focus on risks, intended order of work, and any constraints the executor should preserve.
Keep the guidance short and structured."#;

const PLANNER_GUIDANCE_PREFIX: &str = "<deepseek_planner_guidance>";
const PLANNER_GUIDANCE_SUFFIX: &str = "</deepseek_planner_guidance>";

pub(crate) fn run_deepseek_planner(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    input: Vec<ResponseItem>,
    cancellation_token: CancellationToken,
) -> BoxFuture<'static, CodexResult<Option<ResponseItem>>> {
    Box::pin(async move {
        debug!(
            executor_model = %turn_context.model_info.slug,
            planner_model = %turn_context.config.deepseek_native.planner_model,
            "running DeepSeek planner"
        );
        let planner_context = turn_context
            .with_model(
                turn_context.config.deepseek_native.planner_model.clone(),
                &sess.services.models_manager,
            )
            .await;
        let guidance =
            run_planner_request(sess.as_ref(), &planner_context, input, cancellation_token)
                .await?
                .and_then(|message| planner_guidance_message(message.as_str()));
        debug!(
            planner_model = %planner_context.model_info.slug,
            guidance_emitted = guidance.is_some(),
            "completed DeepSeek planner"
        );
        Ok(guidance)
    })
}

fn planner_prompt(input: Vec<ResponseItem>) -> Prompt {
    Prompt {
        input,
        tools: Vec::new(),
        parallel_tool_calls: false,
        base_instructions: BaseInstructions {
            text: PLANNER_PROMPT.to_string(),
        },
        personality: None,
        output_schema: None,
        output_schema_strict: true,
    }
}

async fn run_planner_request(
    sess: &Session,
    planner_context: &TurnContext,
    input: Vec<ResponseItem>,
    cancellation_token: CancellationToken,
) -> CodexResult<Option<String>> {
    let prompt = planner_prompt(input);
    let mut client_session = sess.services.model_client.new_session();
    let window_id = sess.services.model_client.current_window_id();
    let turn_metadata_header = planner_context
        .turn_metadata_state
        .current_header_value_for_model_request(&window_id);
    let inference_trace = sess.services.rollout_thread_trace.inference_trace_context(
        planner_context.sub_id.as_str(),
        planner_context.model_info.slug.as_str(),
        planner_context.provider.info().name.as_str(),
    );
    let stream_result = tokio::select! {
        _ = cancellation_token.cancelled() => return Err(CodexErr::TurnAborted),
        result = client_session.stream(
            &prompt,
            &planner_context.model_info,
            &planner_context.session_telemetry,
            planner_context.reasoning_effort,
            planner_context.reasoning_summary,
            planner_context.config.service_tier.clone(),
            turn_metadata_header.as_deref(),
            &inference_trace,
        ).instrument(trace_span!("stream_planner_request")) => result,
    };
    let mut stream = stream_result?;
    let mut last_message = None;
    let response_span = trace_span!("planner_response");

    loop {
        let event = tokio::select! {
            _ = cancellation_token.cancelled() => return Err(CodexErr::TurnAborted),
            event = stream.next() => event,
        };
        let Some(event) = event else {
            return Err(CodexErr::Stream(
                "planner stream closed before response.completed".to_string(),
                None,
            ));
        };
        let event = event?;
        sess.services
            .session_telemetry
            .record_responses(&response_span, &event);
        match event {
            ResponseEvent::Created
            | ResponseEvent::OutputItemAdded(_)
            | ResponseEvent::OutputTextDelta(_)
            | ResponseEvent::ToolCallInputDelta { .. }
            | ResponseEvent::ReasoningSummaryDelta { .. }
            | ResponseEvent::ReasoningSummaryPartAdded { .. }
            | ResponseEvent::ReasoningContentDelta { .. }
            | ResponseEvent::RateLimits(_)
            | ResponseEvent::ModelsEtag(_)
            | ResponseEvent::ServerModel(_)
            | ResponseEvent::ModelVerifications(_)
            | ResponseEvent::ServerReasoningIncluded(_) => {}
            ResponseEvent::OutputItemDone(item) => {
                if let Some(message) =
                    last_assistant_message_from_item(&item, /*plan_mode*/ false)
                {
                    last_message = Some(message);
                }
            }
            ResponseEvent::Completed { .. } => return Ok(last_message),
        }
    }
}

fn planner_guidance_message(message: &str) -> Option<ResponseItem> {
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    Some(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: format!("{PLANNER_GUIDANCE_PREFIX}\n{message}\n{PLANNER_GUIDANCE_SUFFIX}"),
        }],
        phase: None,
    })
}
