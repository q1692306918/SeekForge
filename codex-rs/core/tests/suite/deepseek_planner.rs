use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::built_in_model_providers;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::chat_completions_sse;
use core_test_support::responses::mount_chat_completions_sse_sequence;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use wiremock::MockServer;

const FIRST_USER: &str = "implement planner split";
const SECOND_USER: &str = "continue after planner split";
const PLAN_GUIDANCE_ONE: &str = "PLAN_GUIDANCE_ONE";
const PLAN_GUIDANCE_TWO: &str = "PLAN_GUIDANCE_TWO";
const EXECUTOR_REPLY_ONE: &str = "EXECUTOR_REPLY_ONE";
const EXECUTOR_REPLY_TWO: &str = "EXECUTOR_REPLY_TWO";

fn deepseek_model_provider(server: &MockServer) -> ModelProviderInfo {
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["deepseek"].clone();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.request_max_retries = Some(0);
    provider.stream_max_retries = Some(0);
    provider
}

fn chat_text_response(id: &str, text: &str, total_tokens: i64) -> String {
    chat_completions_sse(vec![json!({
        "id": id,
        "choices": [{
            "delta": {"content": text},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": total_tokens,
            "completion_tokens": 0,
            "total_tokens": total_tokens
        }
    })])
}

fn message_texts(body: &Value, role: &str) -> Vec<String> {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["role"].as_str() == Some(role))
        .filter_map(|message| message["content"].as_str().map(str::to_string))
        .collect()
}

async fn submit_text_turn(codex: &codex_core::CodexThread, text: &str) -> anyhow::Result<()> {
    codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: text.into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: Default::default(),
        })
        .await?;
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_planner_runs_in_separate_session_and_injects_ephemeral_guidance()
-> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let request_log = mount_chat_completions_sse_sequence(
        &server,
        vec![
            chat_text_response(
                "chatcmpl-plan-1",
                PLAN_GUIDANCE_ONE,
                /*total_tokens*/ 80,
            ),
            chat_text_response(
                "chatcmpl-exec-1",
                EXECUTOR_REPLY_ONE,
                /*total_tokens*/ 100,
            ),
            chat_text_response(
                "chatcmpl-plan-2",
                PLAN_GUIDANCE_TWO,
                /*total_tokens*/ 90,
            ),
            chat_text_response(
                "chatcmpl-exec-2",
                EXECUTOR_REPLY_TWO,
                /*total_tokens*/ 110,
            ),
        ],
    )
    .await;

    let model_provider = deepseek_model_provider(&server);
    let mut builder = test_codex().with_config(move |config| {
        config.model_provider_id = "deepseek".to_string();
        config.model_provider = model_provider;
        config.model = Some("deepseek-v4-flash".to_string());
        config.deepseek_native.planner_enabled = true;
        config.deepseek_native.planner_model = "deepseek-v4-pro".to_string();
    });
    let codex = builder.build(&server).await?.codex;

    submit_text_turn(&codex, FIRST_USER).await?;
    submit_text_turn(&codex, SECOND_USER).await?;

    let requests = request_log.requests();
    assert_eq!(requests.len(), 4);
    let bodies: Vec<Value> = requests
        .iter()
        .map(core_test_support::responses::ResponsesRequest::body_json)
        .collect();

    let first_planner = &bodies[0];
    assert_eq!(first_planner["model"].as_str(), Some("deepseek-v4-pro"));
    assert!(
        first_planner
            .get("tools")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty),
        "planner should not receive executor tools: {first_planner}"
    );
    assert!(
        message_texts(first_planner, "system")
            .iter()
            .any(|text| text.contains("optional DeepSeek-native planner")),
        "planner should use planner-specific instructions"
    );
    assert!(
        message_texts(first_planner, "user")
            .iter()
            .any(|text| text.contains(FIRST_USER)),
        "planner should receive the user request"
    );

    let first_executor = &bodies[1];
    assert_eq!(first_executor["model"].as_str(), Some("deepseek-v4-flash"));
    assert!(
        first_executor
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty()),
        "executor should retain normal Codex tools"
    );
    let first_executor_users = message_texts(first_executor, "user");
    assert!(
        first_executor_users
            .iter()
            .any(|text| text.contains("<deepseek_planner_guidance>")
                && text.contains(PLAN_GUIDANCE_ONE)),
        "executor should receive planner guidance as a tail message: {first_executor_users:?}"
    );

    let second_planner = &bodies[2];
    assert_eq!(second_planner["model"].as_str(), Some("deepseek-v4-pro"));
    assert!(
        !second_planner.to_string().contains(PLAN_GUIDANCE_ONE),
        "prior planner guidance should not persist into later planner requests"
    );

    let second_executor = &bodies[3];
    assert_eq!(second_executor["model"].as_str(), Some("deepseek-v4-flash"));
    let second_executor_text = second_executor.to_string();
    assert!(second_executor_text.contains(PLAN_GUIDANCE_TWO));
    assert!(
        !second_executor_text.contains(PLAN_GUIDANCE_ONE),
        "prior planner guidance should not persist into later executor requests"
    );

    Ok(())
}
