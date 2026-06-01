pub(crate) mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;
pub mod test_support;

pub use codex_app_server_protocol::AuthMode;
use codex_protocol::models::BASE_INSTRUCTIONS_DEFAULT;
use codex_protocol::openai_models::ApplyPatchToolType;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::openai_models::TruncationPolicyConfig;
use codex_protocol::openai_models::WebSearchToolType;
pub use config::ModelsManagerConfig;

pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-v4-flash";
pub const DEEPSEEK_REVIEW_MODEL: &str = "deepseek-v4-pro";

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    let mut response: ModelsResponse = serde_json::from_str(include_str!("../models.json"))?;
    response.models.extend(deepseek_models());
    Ok(response)
}

/// Convert the client version string to a whole version string (e.g. "1.2.3-alpha.4" -> "1.2.3").
pub fn client_version_to_whole() -> String {
    format!(
        "{}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    )
}

fn deepseek_models() -> Vec<ModelInfo> {
    vec![
        deepseek_model(
            DEEPSEEK_DEFAULT_MODEL,
            "DeepSeek V4 Flash",
            "Default fast executor model for the DeepSeek-native Codex runtime.",
            /*priority*/ -100,
        ),
        deepseek_model(
            DEEPSEEK_REVIEW_MODEL,
            "DeepSeek V4 Pro",
            "Higher-capability DeepSeek model for review and planning workflows.",
            /*priority*/ -90,
        ),
    ]
}

fn deepseek_model(slug: &str, display_name: &str, description: &str, priority: i32) -> ModelInfo {
    ModelInfo {
        slug: slug.to_string(),
        display_name: display_name.to_string(),
        description: Some(description.to_string()),
        default_reasoning_level: None,
        supported_reasoning_levels: vec![ReasoningEffortPreset {
            effort: ReasoningEffort::None,
            description: "Use the provider default reasoning behavior".to_string(),
        }],
        shell_type: ConfigShellToolType::ShellCommand,
        visibility: ModelVisibility::List,
        supported_in_api: true,
        priority,
        additional_speed_tiers: Vec::new(),
        service_tiers: Vec::new(),
        default_service_tier: None,
        availability_nux: None,
        upgrade: None,
        base_instructions: BASE_INSTRUCTIONS_DEFAULT.to_string(),
        model_messages: None,
        supports_reasoning_summaries: false,
        default_reasoning_summary: codex_protocol::config_types::ReasoningSummary::None,
        support_verbosity: false,
        default_verbosity: None,
        apply_patch_tool_type: Some(ApplyPatchToolType::Freeform),
        web_search_tool_type: WebSearchToolType::Text,
        truncation_policy: TruncationPolicyConfig::tokens(10_000),
        supports_parallel_tool_calls: true,
        supports_image_detail_original: false,
        context_window: Some(128_000),
        max_context_window: Some(128_000),
        auto_compact_token_limit: None,
        effective_context_window_percent: 95,
        experimental_supported_tools: Vec::new(),
        input_modalities: vec![InputModality::Text],
        used_fallback_model_metadata: false,
        supports_search_tool: false,
        tool_mode: None,
    }
}
