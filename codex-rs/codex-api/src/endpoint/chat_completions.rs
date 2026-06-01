use crate::auth::SharedAuthProvider;
use crate::common::ChatCompletionsApiRequest;
use crate::common::ResponseStream;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::Compression;
use crate::requests::headers::build_session_headers;
use crate::requests::headers::insert_header;
use crate::requests::headers::subagent_header;
use crate::sse::chat_completions::spawn_chat_completions_stream;
use crate::telemetry::SseTelemetry;
use codex_client::HttpTransport;
use codex_client::RequestCompression;
use codex_client::RequestTelemetry;
use codex_protocol::protocol::SessionSource;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use serde_json::Value;
use std::sync::Arc;
use tracing::instrument;

pub struct ChatCompletionsClient<T: HttpTransport> {
    session: EndpointSession<T>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

#[derive(Default)]
pub struct ChatCompletionsOptions {
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub session_source: Option<SessionSource>,
    pub extra_headers: HeaderMap,
    pub compression: Compression,
}

impl<T: HttpTransport> ChatCompletionsClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
            sse_telemetry: None,
        }
    }

    pub fn with_telemetry(
        self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
            sse_telemetry: sse,
        }
    }

    #[instrument(
        name = "chat_completions.stream_request",
        level = "info",
        skip_all,
        fields(
            transport = "chat_completions_http",
            http.method = "POST",
            api.path = "chat/completions"
        )
    )]
    pub async fn stream_request(
        &self,
        request: ChatCompletionsApiRequest,
        options: ChatCompletionsOptions,
    ) -> Result<ResponseStream, ApiError> {
        let ChatCompletionsOptions {
            session_id,
            thread_id,
            session_source,
            extra_headers,
            compression,
        } = options;
        let tool_names = request.tool_names();
        let body = serde_json::to_value(&request).map_err(|err| {
            ApiError::Stream(format!("failed to encode chat completions request: {err}"))
        })?;

        let mut headers = extra_headers;
        if let Some(ref thread_id) = thread_id {
            insert_header(&mut headers, "x-client-request-id", thread_id);
        }
        headers.extend(build_session_headers(session_id, thread_id));
        if let Some(subagent) = subagent_header(&session_source) {
            insert_header(&mut headers, "x-openai-subagent", &subagent);
        }

        self.stream(body, headers, compression, tool_names).await
    }

    fn path() -> &'static str {
        "chat/completions"
    }

    #[instrument(
        name = "chat_completions.stream",
        level = "info",
        skip_all,
        fields(
            transport = "chat_completions_http",
            http.method = "POST",
            api.path = "chat/completions"
        )
    )]
    async fn stream(
        &self,
        body: Value,
        extra_headers: HeaderMap,
        compression: Compression,
        tool_names: std::collections::HashMap<String, crate::common::ChatToolName>,
    ) -> Result<ResponseStream, ApiError> {
        let request_compression = match compression {
            Compression::None => RequestCompression::None,
            Compression::Zstd => RequestCompression::Zstd,
        };

        let stream_response = self
            .session
            .stream_with(
                Method::POST,
                Self::path(),
                extra_headers,
                Some(body),
                |req| {
                    req.headers.insert(
                        http::header::ACCEPT,
                        HeaderValue::from_static("text/event-stream"),
                    );
                    req.compression = request_compression;
                },
            )
            .await?;

        Ok(spawn_chat_completions_stream(
            stream_response,
            self.session.provider().stream_idle_timeout,
            self.sse_telemetry.clone(),
            tool_names,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::AuthProvider;
    use crate::ChatCompletionsApiRequest;
    use crate::ChatCompletionsClient;
    use crate::ChatCompletionsOptions;
    use crate::Provider;
    use crate::RetryConfig;
    use crate::common::ResponsesApiRequest;
    use codex_client::ReqwestTransport;
    use futures::StreamExt;
    use http::HeaderMap;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::body_json;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    #[derive(Debug)]
    struct NoAuth;

    impl AuthProvider for NoAuth {
        fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
    }

    #[tokio::test]
    async fn stream_request_posts_chat_completions_body() {
        let server = MockServer::start().await;
        let request = ChatCompletionsApiRequest::from_responses_request(ResponsesApiRequest {
            model: "deepseek-v4-flash".to_string(),
            instructions: "base".to_string(),
            input: Vec::new(),
            tools: Vec::new(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: false,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            text: None,
            client_metadata: None,
        });

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_json(json!({
                "model": "deepseek-v4-flash",
                "messages": [{"role": "system", "content": "base"}],
                "stream": true,
                "stream_options": {"include_usage": true}
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string("data: [DONE]\n\n"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let provider = Provider {
            name: "DeepSeek".to_string(),
            base_url: server.uri(),
            query_params: None,
            headers: HeaderMap::new(),
            retry: RetryConfig {
                max_attempts: 1,
                base_delay: Duration::from_millis(1),
                retry_429: false,
                retry_5xx: false,
                retry_transport: false,
            },
            stream_idle_timeout: Duration::from_secs(5),
        };
        let client = ChatCompletionsClient::new(
            ReqwestTransport::new(reqwest::Client::new()),
            provider,
            Arc::new(NoAuth),
        );

        let mut stream = client
            .stream_request(request, ChatCompletionsOptions::default())
            .await
            .expect("stream should start");

        let event = stream.next().await.expect("completed event");
        assert!(matches!(event, Ok(crate::ResponseEvent::Completed { .. })));
    }
}
