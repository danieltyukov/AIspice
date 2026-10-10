//! Anthropic Messages API.
//!
//! Streams over SSE, sends tools natively, passes thinking blocks (with their
//! signatures) back unchanged, and marks the stable prefix of every request
//! for prompt caching: the last tool definition and the system prompt get
//! explicit breakpoints, and the growing conversation tail uses automatic
//! caching. In an agent loop that resends the whole history each step, this
//! is what keeps cost roughly linear instead of quadratic.

use std::fmt;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    ChatRequest, ChatResponse, HttpLimits, ModelInfo, Provider, ProviderError, Retrier,
    RetryPolicy, StopReason, StreamEvent, Usage, display_url, error_message, http_client,
    read_limited, read_sse, send, trim_base_url,
};
use crate::message::{ContentBlock, Message, Role};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Model list pages fetched at most. The real list is one page; the cap
/// stops a server that always claims more.
const MAX_MODEL_PAGES: usize = 20;

/// Models that accept `fallbacks: "default"`, which reruns a request declined
/// by a safety classifier on a model that can serve it, inside the same call.
const FALLBACK_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-sonnet-5-5",
];

/// Request features a proxy in front of the API may not understand.
///
/// All on for the official endpoint. [`AnthropicProvider::with_base_url`]
/// turns them off, since an unknown gateway may reject the fields; turn them
/// back on with [`AnthropicProvider::with_features`] when the upstream is the
/// real API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnthropicFeatures {
    /// Stream tool inputs as they are generated rather than buffered per
    /// parameter. The API then skips input validation, which the agent loop
    /// covers by treating unparseable input as a tool error.
    pub eager_input_streaming: bool,
    /// `fallbacks: "default"` on the models that support it.
    pub refusal_fallback: bool,
    /// Top-level automatic cache breakpoint for the conversation tail.
    pub auto_cache: bool,
}

impl AnthropicFeatures {
    pub const ALL: Self = Self {
        eager_input_streaming: true,
        refusal_fallback: true,
        auto_cache: true,
    };
    pub const NONE: Self = Self {
        eager_input_streaming: false,
        refusal_fallback: false,
        auto_cache: false,
    };
}

#[derive(Clone)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    retry: RetryPolicy,
    limits: HttpLimits,
    features: AnthropicFeatures,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: http_client(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            retry: RetryPolicy::default(),
            limits: HttpLimits::default(),
            features: AnthropicFeatures::ALL,
        }
    }

    pub fn with_limits(mut self, limits: HttpLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Errors leave the provider through here, so the key never does.
    fn redact(&self, err: ProviderError) -> ProviderError {
        err.redacted(&[&self.api_key])
    }

    /// Point at a proxy or test server. The URL is the part before `/v1`.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = trim_base_url(base_url);
        if self.base_url != DEFAULT_BASE_URL {
            self.features = AnthropicFeatures::NONE;
        }
        self
    }

    pub fn with_features(mut self, features: AnthropicFeatures) -> Self {
        self.features = features;
        self
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The JSON body sent to `/v1/messages`.
    pub fn request_body(&self, req: &ChatRequest) -> Value {
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "stream": true,
            "messages": wire_messages(&req.messages),
        });
        if !req.system.is_empty() {
            body["system"] = json!([{
                "type": "text",
                "text": req.system,
                "cache_control": {"type": "ephemeral"},
            }]);
        }
        if !req.tools.is_empty() {
            let last = req.tools.len() - 1;
            let tools: Vec<Value> = req
                .tools
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut tool = json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    });
                    if self.features.eager_input_streaming {
                        tool["eager_input_streaming"] = json!(true);
                    }
                    if i == last {
                        tool["cache_control"] = json!({"type": "ephemeral"});
                    }
                    tool
                })
                .collect();
            body["tools"] = Value::Array(tools);
        }
        if let Some(thinking) = &req.thinking {
            body["thinking"] = match thinking.budget_tokens {
                Some(budget) => json!({"type": "enabled", "budget_tokens": budget}),
                None => json!({"type": "adaptive", "display": "summarized"}),
            };
            if let Some(effort) = thinking.effort {
                body["output_config"] = json!({"effort": effort.as_str()});
            }
        }
        if let Some(t) = req.temperature {
            body["temperature"] = json!(short_float(t));
        }
        if self.features.auto_cache {
            body["cache_control"] = json!({"type": "ephemeral"});
        }
        if self.features.refusal_fallback && FALLBACK_MODELS.contains(&req.model.as_str()) {
            body["fallbacks"] = json!("default");
        }
        body
    }

    async fn stream_once(
        &self,
        body: &Value,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, ProviderError> {
        let mut request = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("accept", "text/event-stream")
            .timeout(self.limits.request_timeout)
            .json(body);
        if body.get("fallbacks").is_some() {
            request = request.header("anthropic-beta", FALLBACK_BETA);
        }
        let response = send(request, cancel, &self.limits).await?;
        let mut state = StreamState::new(&self.limits);
        read_sse(response, &self.limits, cancel, |event| {
            state.handle(&event.data, sink)
        })
        .await?;
        state.finish()
    }

    async fn models_page(
        &self,
        after: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Value, ProviderError> {
        let mut url = format!("{}/v1/models?limit=1000", self.base_url);
        if let Some(after) = after {
            url.push_str("&after_id=");
            url.push_str(&query_escape(after));
        }
        let request = self
            .client
            .get(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .timeout(self.limits.list_timeout);
        let response = send(request, cancel, &self.limits).await?;
        let body = read_limited(
            response,
            self.limits.max_json_body,
            Some("the model list"),
            &self.limits,
            cancel,
        )
        .await?;
        serde_json::from_slice(&body).map_err(|e| ProviderError::Parse(e.to_string()))
    }
}

/// Percent-encode a value taken from a response before it goes into a URL.
fn query_escape(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &display_url(&self.base_url))
            .field("features", &self.features)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    async fn stream(
        &self,
        req: &ChatRequest,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, ProviderError> {
        let body = self.request_body(req);
        let mut retrier = Retrier::new(&self.retry, cancel);
        loop {
            let mut streamed = false;
            let result = {
                let mut tracked = |event: StreamEvent| {
                    streamed = true;
                    sink(event);
                };
                self.stream_once(&body, &mut tracked, cancel).await
            };
            match result {
                Ok(response) => return Ok(response),
                Err(err) => {
                    let err = self.redact(err);
                    if !retrier.backoff(self.id(), &err, streamed).await? {
                        return Err(err);
                    }
                }
            }
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let cancel = CancellationToken::new();
        let mut models = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_MODEL_PAGES {
            let mut retrier = Retrier::new(&self.retry, &cancel);
            let page = loop {
                match self.models_page(after.as_deref(), &cancel).await {
                    Ok(page) => break page,
                    Err(err) => {
                        let err = self.redact(err);
                        if !retrier.backoff(self.id(), &err, false).await? {
                            return Err(err);
                        }
                    }
                }
            };
            for m in page["data"].as_array().into_iter().flatten() {
                let Some(id) = m["id"].as_str() else { continue };
                models.push(ModelInfo {
                    id: id.to_string(),
                    display_name: m["display_name"].as_str().unwrap_or(id).to_string(),
                    context_window: m["max_input_tokens"].as_u64().map(|n| n as u32),
                    supports_tools: Some(true),
                    supports_vision: m
                        .pointer("/capabilities/image_input/supported")
                        .and_then(Value::as_bool),
                });
            }
            match (page["has_more"].as_bool(), page["last_id"].as_str()) {
                (Some(true), Some(last)) if after.as_deref() != Some(last) => {
                    after = Some(last.to_string());
                }
                _ => return Ok(models),
            }
        }
        Ok(models)
    }
}

/// Canonical messages to the Messages API shape.
///
/// Unsigned thinking blocks (from another provider) are dropped because the
/// API rejects thinking without a signature. Empty text blocks are dropped
/// because the API rejects those too.
pub fn wire_messages(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .filter_map(|m| {
            let content: Vec<Value> = m.content.iter().filter_map(wire_block).collect();
            if content.is_empty() {
                return None;
            }
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            Some(json!({"role": role, "content": content}))
        })
        .collect()
}

fn wire_block(block: &ContentBlock) -> Option<Value> {
    Some(match block {
        ContentBlock::Text { text } if text.is_empty() => return None,
        ContentBlock::Text { text } => json!({"type": "text", "text": text}),
        ContentBlock::Image {
            media_type,
            data_base64,
        } => json!({
            "type": "image",
            "source": {"type": "base64", "media_type": media_type, "data": data_base64},
        }),
        ContentBlock::ToolUse { id, name, input } => json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            // Unparseable arguments are kept as a string in the canonical
            // history; the API only accepts an object here.
            "input": if input.is_object() { input.clone() } else { json!({}) },
        }),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let mut result = json!({"type": "tool_result", "tool_use_id": tool_use_id});
            let inner: Vec<Value> = content
                .iter()
                .filter(|c| matches!(c, ContentBlock::Text { .. } | ContentBlock::Image { .. }))
                .filter_map(wire_block)
                .collect();
            if !inner.is_empty() {
                result["content"] = Value::Array(inner);
            }
            if *is_error {
                result["is_error"] = json!(true);
            }
            result
        }
        ContentBlock::Thinking {
            thinking,
            signature: Some(signature),
        } => json!({"type": "thinking", "thinking": thinking, "signature": signature}),
        ContentBlock::Thinking {
            signature: None, ..
        } => return None,
        ContentBlock::RedactedThinking { data } => {
            json!({"type": "redacted_thinking", "data": data})
        }
    })
}

/// `0.2f32` as `0.2` rather than `0.20000000298023224`.
fn short_float(v: f32) -> f64 {
    v.to_string().parse().unwrap_or(f64::from(v))
}

enum Partial {
    Text(String),
    Thinking {
        thinking: String,
        signature: String,
    },
    Redacted(String),
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
    Ignored,
}

/// Assembles a message from stream events.
struct StreamState {
    blocks: Vec<(u64, Partial)>,
    usage: Usage,
    stop_reason: Option<String>,
    done: bool,
    max_blocks: usize,
    max_tool_input: usize,
}

impl StreamState {
    fn new(limits: &HttpLimits) -> Self {
        Self {
            blocks: Vec::new(),
            usage: Usage::default(),
            stop_reason: None,
            done: false,
            max_blocks: limits.max_blocks,
            max_tool_input: limits.max_tool_input,
        }
    }

    /// Returns `Ok(false)` once the message is complete.
    fn handle(
        &mut self,
        data: &str,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<bool, ProviderError> {
        if data.trim().is_empty() {
            return Ok(true);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| ProviderError::Parse(format!("{e} in event {data:.200}")))?;
        match v["type"].as_str().unwrap_or("") {
            "message_start" => self.update_usage(&v["message"]["usage"]),
            "content_block_start" => self.block_start(&v, sink)?,
            "content_block_delta" => self.block_delta(&v, sink)?,
            "message_delta" => {
                if let Some(reason) = v["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                self.update_usage(&v["usage"]);
            }
            "message_stop" => {
                self.done = true;
                return Ok(false);
            }
            "error" => return Err(stream_error(&v["error"])),
            _ => {}
        }
        Ok(true)
    }

    fn block_start(
        &mut self,
        v: &Value,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        if self.blocks.len() >= self.max_blocks {
            return Err(ProviderError::TooLarge(format!(
                "more than {} content blocks in one response",
                self.max_blocks
            )));
        }
        let index = v["index"].as_u64().unwrap_or(self.blocks.len() as u64);
        let block = &v["content_block"];
        let str_field = |key: &str| block[key].as_str().unwrap_or("").to_string();
        let partial = match block["type"].as_str().unwrap_or("") {
            "text" => {
                let text = str_field("text");
                if !text.is_empty() {
                    sink(StreamEvent::TextDelta(text.clone()));
                }
                Partial::Text(text)
            }
            "thinking" => {
                let thinking = str_field("thinking");
                if !thinking.is_empty() {
                    sink(StreamEvent::ThinkingDelta(thinking.clone()));
                }
                Partial::Thinking {
                    thinking,
                    signature: str_field("signature"),
                }
            }
            "redacted_thinking" => Partial::Redacted(str_field("data")),
            "tool_use" => {
                let (id, name) = (str_field("id"), str_field("name"));
                sink(StreamEvent::ToolUseStart {
                    id: id.clone(),
                    name: name.clone(),
                });
                let json = match &block["input"] {
                    Value::Object(m) if !m.is_empty() => block["input"].to_string(),
                    _ => String::new(),
                };
                Partial::ToolUse { id, name, json }
            }
            "fallback" => {
                // A server-side fallback took over mid-output. The next model
                // sees only the earlier text, so thinking and tool calls from
                // before the switch must not be echoed back.
                for (i, p) in &mut self.blocks {
                    if *i < index
                        && matches!(
                            p,
                            Partial::Thinking { .. }
                                | Partial::Redacted(_)
                                | Partial::ToolUse { .. }
                        )
                    {
                        *p = Partial::Ignored;
                    }
                }
                Partial::Ignored
            }
            _ => Partial::Ignored,
        };
        self.blocks.push((index, partial));
        Ok(())
    }

    fn block_delta(
        &mut self,
        v: &Value,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        let index = v["index"].as_u64();
        let max_tool_input = self.max_tool_input;
        let Some((_, block)) = self
            .blocks
            .iter_mut()
            .rev()
            .find(|(i, _)| index.is_none_or(|x| x == *i))
        else {
            return Ok(());
        };
        let delta = &v["delta"];
        let field = |key: &str| delta[key].as_str().unwrap_or("");
        match (delta["type"].as_str().unwrap_or(""), block) {
            ("text_delta", Partial::Text(text)) => {
                text.push_str(field("text"));
                sink(StreamEvent::TextDelta(field("text").to_string()));
            }
            ("thinking_delta", Partial::Thinking { thinking, .. }) => {
                thinking.push_str(field("thinking"));
                sink(StreamEvent::ThinkingDelta(field("thinking").to_string()));
            }
            ("signature_delta", Partial::Thinking { signature, .. }) => {
                signature.push_str(field("signature"));
            }
            ("input_json_delta", Partial::ToolUse { id, json, .. }) => {
                let partial = field("partial_json");
                if json.len() + partial.len() > max_tool_input {
                    return Err(ProviderError::too_large(
                        "the input of one tool call",
                        max_tool_input,
                    ));
                }
                json.push_str(partial);
                if !partial.is_empty() {
                    sink(StreamEvent::ToolUseInputDelta {
                        id: id.clone(),
                        partial_json: partial.to_string(),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn update_usage(&mut self, usage: &Value) {
        let get = |key: &str| usage[key].as_u64();
        if let Some(n) = get("input_tokens") {
            self.usage.input_tokens = n;
        }
        if let Some(n) = get("output_tokens") {
            self.usage.output_tokens = n;
        }
        if let Some(n) = get("cache_read_input_tokens") {
            self.usage.cache_read_tokens = n;
        }
        if let Some(n) = get("cache_creation_input_tokens") {
            self.usage.cache_write_tokens = n;
        }
    }

    fn finish(mut self) -> Result<ChatResponse, ProviderError> {
        if !self.done {
            return Err(ProviderError::Network(
                "the stream ended before message_stop".into(),
            ));
        }
        self.blocks.sort_by_key(|(i, _)| *i);
        let content = self
            .blocks
            .into_iter()
            .filter_map(|(_, p)| match p {
                Partial::Text(text) if text.is_empty() => None,
                Partial::Text(text) => Some(ContentBlock::Text { text }),
                Partial::Thinking {
                    thinking,
                    signature,
                } => Some(ContentBlock::Thinking {
                    thinking,
                    signature: (!signature.is_empty()).then_some(signature),
                }),
                Partial::Redacted(data) => Some(ContentBlock::RedactedThinking { data }),
                Partial::ToolUse { id, name, json } => Some(ContentBlock::ToolUse {
                    id,
                    name,
                    input: parse_tool_input(&json),
                }),
                Partial::Ignored => None,
            })
            .collect();
        let stop_reason = match self.stop_reason.as_deref() {
            Some("end_turn") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some("refusal") => StopReason::Refusal,
            Some(other) => StopReason::Other(other.to_string()),
            None => StopReason::Other("unknown".into()),
        };
        Ok(ChatResponse {
            message: Message::assistant(content),
            stop_reason,
            usage: self.usage,
        })
    }
}

/// Parse accumulated tool arguments. Anything that is not a JSON object is
/// kept as the raw string so the agent can return it to the model as an
/// error rather than run the tool on a guess.
pub(crate) fn parse_tool_input(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(v @ Value::Object(_)) => v,
        _ => Value::String(raw.to_string()),
    }
}

fn stream_error(err: &Value) -> ProviderError {
    let kind = err["type"].as_str().unwrap_or("error");
    let message = err["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| error_message(&err.to_string()));
    match kind {
        "overloaded_error" => ProviderError::Overloaded {
            status: 529,
            message,
        },
        "api_error" => ProviderError::Overloaded {
            status: 500,
            message,
        },
        "rate_limit_error" => ProviderError::RateLimited {
            message,
            retry_after: None,
        },
        "authentication_error" => ProviderError::Auth {
            status: 401,
            message,
        },
        "permission_error" => ProviderError::Auth {
            status: 403,
            message,
        },
        "not_found_error" => ProviderError::BadRequest {
            status: 404,
            message,
        },
        "request_too_large" => ProviderError::BadRequest {
            status: 413,
            message,
        },
        _ => ProviderError::BadRequest {
            status: 400,
            message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Effort, ThinkingConfig};
    use crate::tool::ToolSpec;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn tool(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: format!("{name} tool"),
            input_schema: json!({"type": "object", "properties": {"circuit": {"type": "string"}}}),
        }
    }

    fn request(messages: Vec<Message>) -> ChatRequest {
        ChatRequest {
            model: "claude-opus-5-5".into(),
            system: "You design circuits.".into(),
            messages,
            tools: vec![tool("read_schematic"), tool("simulate")],
            max_tokens: 64000,
            temperature: None,
            thinking: Some(ThinkingConfig::with_effort(Effort::High)),
        }
    }

    fn fast_retry() -> RetryPolicy {
        RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(50),
            ..RetryPolicy::default()
        }
    }

    async fn provider(server: &MockServer) -> AnthropicProvider {
        AnthropicProvider::new("test-key")
            .with_base_url(server.uri())
            .with_retry(fast_retry())
    }

    async fn mount_sse(server: &MockServer, body: &str) {
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_string(), "text/event-stream"),
            )
            .mount(server)
            .await;
    }

    async fn run(
        p: &AnthropicProvider,
        req: &ChatRequest,
    ) -> (Result<ChatResponse, ProviderError>, Vec<StreamEvent>) {
        let mut events = Vec::new();
        let result = p
            .stream(req, &mut |e| events.push(e), &CancellationToken::new())
            .await;
        (result, events)
    }

    // ---- canonical to wire ----

    #[test]
    fn converts_history_with_images_tool_results_and_thinking() {
        let history = vec![
            Message::user(vec![
                ContentBlock::text("Why does this ring?"),
                ContentBlock::image("image/png", "iVBORw0KGgo="),
            ]),
            Message::assistant(vec![
                ContentBlock::Thinking {
                    thinking: "Check the phase margin.".into(),
                    signature: Some("EqQBCkYIBxgCKkA=".into()),
                },
                ContentBlock::RedactedThinking {
                    data: "EmwKAhgBEgy3va3pzix/LafPsn4a".into(),
                },
                ContentBlock::text(""),
                ContentBlock::ToolUse {
                    id: "toolu_01".into(),
                    name: "render_schematic".into(),
                    input: json!({"circuit": "amp.asc"}),
                },
                ContentBlock::ToolUse {
                    id: "toolu_02".into(),
                    name: "simulate".into(),
                    input: Value::String("{\"circuit\": \"amp".into()),
                },
            ]),
            Message::user(vec![
                ContentBlock::tool_result(
                    "toolu_01",
                    vec![
                        ContentBlock::text("Rendered amp.asc"),
                        ContentBlock::image("image/png", "AAAA"),
                    ],
                    false,
                ),
                ContentBlock::tool_result(
                    "toolu_02",
                    vec![ContentBlock::text("{\"INVALID_JSON\":\"...\"}")],
                    true,
                ),
            ]),
            Message::assistant(vec![ContentBlock::Thinking {
                thinking: "from another provider".into(),
                signature: None,
            }]),
        ];
        assert_eq!(
            Value::Array(wire_messages(&history)),
            json!([
                {"role": "user", "content": [
                    {"type": "text", "text": "Why does this ring?"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBORw0KGgo="}}
                ]},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "Check the phase margin.", "signature": "EqQBCkYIBxgCKkA="},
                    {"type": "redacted_thinking", "data": "EmwKAhgBEgy3va3pzix/LafPsn4a"},
                    {"type": "tool_use", "id": "toolu_01", "name": "render_schematic", "input": {"circuit": "amp.asc"}},
                    {"type": "tool_use", "id": "toolu_02", "name": "simulate", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_01", "content": [
                        {"type": "text", "text": "Rendered amp.asc"},
                        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}}
                    ]},
                    {"type": "tool_result", "tool_use_id": "toolu_02", "is_error": true, "content": [
                        {"type": "text", "text": "{\"INVALID_JSON\":\"...\"}"}
                    ]}
                ]}
            ])
        );
    }

    #[test]
    fn request_body_marks_cache_breakpoints_and_features() {
        let p = AnthropicProvider::new("k");
        let body = p.request_body(&request(vec![Message::user_text("hi")]));
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 64000);
        assert_eq!(
            body["system"],
            json!([{"type": "text", "text": "You design circuits.", "cache_control": {"type": "ephemeral"}}])
        );
        let tools = body["tools"].as_array().unwrap();
        assert!(tools[0].get("cache_control").is_none());
        assert_eq!(tools[1]["cache_control"], json!({"type": "ephemeral"}));
        assert!(tools.iter().all(|t| t["eager_input_streaming"] == true));
        assert_eq!(
            body["thinking"],
            json!({"type": "adaptive", "display": "summarized"})
        );
        assert_eq!(body["output_config"], json!({"effort": "high"}));
        assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(body["fallbacks"], "default");
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn request_body_for_older_models_and_proxies() {
        let p = AnthropicProvider::new("k").with_base_url("https://proxy.example/anthropic/");
        assert_eq!(p.base_url(), "https://proxy.example/anthropic");
        let mut req = request(vec![Message::user_text("hi")]);
        req.model = "claude-haiku-4-5".into();
        req.system.clear();
        req.tools.clear();
        req.temperature = Some(0.2);
        req.thinking = Some(ThinkingConfig::with_budget(4096));
        let body = p.request_body(&req);
        assert_eq!(
            body["thinking"],
            json!({"type": "enabled", "budget_tokens": 4096})
        );
        assert_eq!(body["temperature"], json!(0.2));
        for key in [
            "system",
            "tools",
            "cache_control",
            "fallbacks",
            "output_config",
        ] {
            assert!(body.get(key).is_none(), "{key}");
        }
    }

    #[test]
    fn tool_input_parsing_keeps_bad_json_raw() {
        assert_eq!(parse_tool_input(""), json!({}));
        assert_eq!(parse_tool_input("{\"a\": 1}"), json!({"a": 1}));
        assert_eq!(parse_tool_input("{\"a\": "), json!("{\"a\": "));
        assert_eq!(parse_tool_input("[1]"), json!("[1]"));
    }

    // ---- recorded-shape streams ----

    const TEXT_ONLY: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01XFDUDYJgAACzvnptvVoYEL","type":"message","role":"assistant","model":"claude-opus-5-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":25,"cache_creation_input_tokens":0,"cache_read_input_tokens":1800,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: ping
data: {"type": "ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The cutoff is "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"1.59 kHz."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":12}}

event: message_stop
data: {"type":"message_stop"}

"#;

    #[tokio::test]
    async fn streams_text() {
        let server = MockServer::start().await;
        mount_sse(&server, TEXT_ONLY).await;
        let p = provider(&server).await;
        let (result, events) = run(&p, &request(vec![Message::user_text("cutoff?")])).await;
        assert_eq!(
            result.unwrap(),
            ChatResponse {
                message: Message::assistant_text("The cutoff is 1.59 kHz."),
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 25,
                    output_tokens: 12,
                    cache_read_tokens: 1800,
                    cache_write_tokens: 0,
                },
            }
        );
        assert_eq!(
            events,
            vec![
                StreamEvent::TextDelta("The cutoff is ".into()),
                StreamEvent::TextDelta("1.59 kHz.".into()),
            ]
        );
        let sent = &server.received_requests().await.unwrap()[0];
        assert_eq!(sent.headers["x-api-key"], "test-key");
        assert_eq!(sent.headers["anthropic-version"], API_VERSION);
        assert!(sent.headers.get("anthropic-beta").is_none());
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(
            body["messages"],
            json!([{"role": "user", "content": [{"type": "text", "text": "cutoff?"}]}])
        );
    }

    const TOOL_USE: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_014p7gG3wDgGV9EUtLvnow3U","type":"message","role":"assistant","model":"claude-opus-5-5","stop_sequence":null,"usage":{"input_tokens":472,"output_tokens":2},"content":[],"stop_reason":null}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Running an AC sweep."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01T1x1fJ34qAmk2tNTrN7Up6","name":"simulate","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"circuit\": \"rc"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":".asc\", \"analysis\""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":": {\"type\": \"ac\", \"points\": 101}}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":89}}

event: message_stop
data: {"type":"message_stop"}
"#;

    #[tokio::test]
    async fn streams_tool_use_with_input_json_deltas() {
        let server = MockServer::start().await;
        mount_sse(&server, TOOL_USE).await;
        let p = provider(&server).await;
        let (result, events) = run(&p, &request(vec![Message::user_text("sweep it")])).await;
        assert_eq!(
            result.unwrap(),
            ChatResponse {
                message: Message::assistant(vec![
                    ContentBlock::text("Running an AC sweep."),
                    ContentBlock::ToolUse {
                        id: "toolu_01T1x1fJ34qAmk2tNTrN7Up6".into(),
                        name: "simulate".into(),
                        input: json!({"circuit": "rc.asc", "analysis": {"type": "ac", "points": 101}}),
                    },
                ]),
                stop_reason: StopReason::ToolUse,
                usage: Usage {
                    input_tokens: 472,
                    output_tokens: 89,
                    ..Usage::default()
                },
            }
        );
        assert_eq!(
            events[1],
            StreamEvent::ToolUseStart {
                id: "toolu_01T1x1fJ34qAmk2tNTrN7Up6".into(),
                name: "simulate".into(),
            }
        );
        let deltas: String = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolUseInputDelta { partial_json, .. } => Some(partial_json.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            deltas,
            r#"{"circuit": "rc.asc", "analysis": {"type": "ac", "points": 101}}"#
        );
    }

    const THINKING_TOOL_USE: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-opus-5-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":12,"cache_creation_input_tokens":2048,"cache_read_input_tokens":0,"output_tokens":3}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"The divider sets the bias; "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"read the schematic first."}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCgIYAhIM1gbcDa9GJwZA2b3h"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"GgwKuA9tXV5OfJ8h4XoQ=="}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_02","name":"read_schematic","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"circuit\":\"bias.asc\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"input_tokens":12,"cache_creation_input_tokens":2048,"cache_read_input_tokens":0,"output_tokens":154}}

event: message_stop
data: {"type":"message_stop"}
"#;

    #[tokio::test]
    async fn streams_thinking_with_signature_then_tool_use() {
        let server = MockServer::start().await;
        mount_sse(&server, THINKING_TOOL_USE).await;
        let p = provider(&server).await;
        let (result, events) = run(&p, &request(vec![Message::user_text("bias?")])).await;
        let response = result.unwrap();
        assert_eq!(
            response,
            ChatResponse {
                message: Message::assistant(vec![
                    ContentBlock::Thinking {
                        thinking: "The divider sets the bias; read the schematic first.".into(),
                        signature: Some(
                            "EqQBCgIYAhIM1gbcDa9GJwZA2b3hGgwKuA9tXV5OfJ8h4XoQ==".into()
                        ),
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_02".into(),
                        name: "read_schematic".into(),
                        input: json!({"circuit": "bias.asc"}),
                    },
                ]),
                stop_reason: StopReason::ToolUse,
                usage: Usage {
                    input_tokens: 12,
                    output_tokens: 154,
                    cache_read_tokens: 0,
                    cache_write_tokens: 2048,
                },
            }
        );
        assert_eq!(
            events[0],
            StreamEvent::ThinkingDelta("The divider sets the bias; ".into())
        );
        // The signed block goes back verbatim on the next turn.
        let next = wire_messages(&[response.message]);
        assert_eq!(
            next[0]["content"][0]["signature"],
            "EqQBCgIYAhIM1gbcDa9GJwZA2b3hGgwKuA9tXV5OfJ8h4XoQ=="
        );
    }

    const ERROR_MID_STREAM: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-opus-5-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Partial"}}

event: error
data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}

"#;

    #[tokio::test]
    async fn error_event_after_output_is_returned_without_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(ERROR_MID_STREAM, "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let p = provider(&server).await;
        let (result, events) = run(&p, &request(vec![Message::user_text("hi")])).await;
        match result {
            Err(ProviderError::Overloaded { status, message }) => {
                assert_eq!(status, 529);
                assert_eq!(message, "Overloaded");
            }
            other => panic!("expected overloaded, got {other:?}"),
        }
        assert_eq!(events, vec![StreamEvent::TextDelta("Partial".into())]);
    }

    #[tokio::test]
    async fn error_event_before_output_is_retried() {
        let server = MockServer::start().await;
        let early_error = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\nevent: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(early_error, "text/event-stream"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mount_sse(&server, TEXT_ONLY).await;
        let p = provider(&server).await;
        let (result, _) = run(&p, &request(vec![Message::user_text("hi")])).await;
        assert_eq!(result.unwrap().message.text(), "The cutoff is 1.59 kHz.");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn retries_529_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(529).set_body_json(
                json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
            ))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mount_sse(&server, TEXT_ONLY).await;
        let p = provider(&server).await;
        let (result, _) = run(&p, &request(vec![Message::user_text("hi")])).await;
        assert!(result.is_ok());
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn auth_errors_are_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(
                json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let p = provider(&server).await;
        let (result, _) = run(&p, &request(vec![Message::user_text("hi")])).await;
        match result {
            Err(ProviderError::Auth {
                status: 401,
                message,
            }) => assert_eq!(message, "invalid x-api-key"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn truncated_stream_is_a_network_error() {
        let server = MockServer::start().await;
        let cut = TEXT_ONLY
            .split("event: message_delta")
            .next()
            .unwrap()
            .to_string();
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(cut, "text/event-stream"))
            .mount(&server)
            .await;
        let p = provider(&server).await;
        let (result, _) = run(&p, &request(vec![Message::user_text("hi")])).await;
        assert!(
            matches!(result, Err(ProviderError::Network(_))),
            "{result:?}"
        );
        // Text had streamed, so no retry.
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    const MID_OUTPUT_FALLBACK: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-opus-5-5","content":[],"stop_reason":null,"usage":{"input_tokens":40,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sigA"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":"Let me"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: content_block_start
data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_cut","name":"simulate","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"circ"}}

event: content_block_start
data: {"type":"content_block_start","index":3,"content_block":{"type":"fallback","from":{"model":"claude-opus-5-5"},"to":{"model":"claude-opus-4-8"}}}

event: content_block_stop
data: {"type":"content_block_stop","index":3}

event: content_block_start
data: {"type":"content_block_start","index":4,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":4,"delta":{"type":"text_delta","text":" check the bias."}}

event: content_block_stop
data: {"type":"content_block_stop","index":4}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":30}}

event: message_stop
data: {"type":"message_stop"}
"#;

    #[tokio::test]
    async fn mid_output_fallback_drops_pre_switch_thinking_and_tool_calls() {
        let server = MockServer::start().await;
        mount_sse(&server, MID_OUTPUT_FALLBACK).await;
        let p = provider(&server)
            .await
            .with_features(AnthropicFeatures::ALL);
        let (result, _) = run(&p, &request(vec![Message::user_text("hi")])).await;
        assert_eq!(
            result.unwrap().message,
            Message::assistant(vec![
                ContentBlock::text("Let me"),
                ContentBlock::text(" check the bias."),
            ])
        );
        let sent = &server.received_requests().await.unwrap()[0];
        assert_eq!(sent.headers["anthropic-beta"], FALLBACK_BETA);
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["tools"][0]["eager_input_streaming"], true);
    }

    #[tokio::test]
    async fn cancellation_aborts_a_slow_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(TEXT_ONLY, "text/event-stream")
                    .set_delay(Duration::from_secs(10)),
            )
            .mount(&server)
            .await;
        let p = provider(&server).await;
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger.cancel();
        });
        let started = std::time::Instant::now();
        let result = p
            .stream(
                &request(vec![Message::user_text("hi")]),
                &mut |_| {},
                &cancel,
            )
            .await;
        assert!(matches!(result, Err(ProviderError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn lists_models_across_pages() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(query_param("after_id", "claude-sonnet-5-5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5", "max_input_tokens": 200000,
                          "capabilities": {"image_input": {"supported": true}}}],
                "has_more": false, "first_id": "claude-haiku-4-5", "last_id": "claude-haiku-4-5"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5", "max_input_tokens": 1000000, "max_tokens": 128000,
                     "capabilities": {"image_input": {"supported": true}}},
                    {"id": "claude-sonnet-5-5", "display_name": "Claude Sonnet 5.5"}
                ],
                "has_more": true, "first_id": "claude-opus-5-5", "last_id": "claude-sonnet-5-5"
            })))
            .mount(&server)
            .await;
        let p = provider(&server).await;
        let models = p.list_models().await.unwrap();
        assert_eq!(
            models,
            vec![
                ModelInfo {
                    id: "claude-opus-5-5".into(),
                    display_name: "Claude Opus 5.5".into(),
                    context_window: Some(1_000_000),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                },
                ModelInfo {
                    id: "claude-sonnet-5-5".into(),
                    display_name: "Claude Sonnet 5.5".into(),
                    context_window: None,
                    supports_tools: Some(true),
                    supports_vision: None,
                },
                ModelInfo {
                    id: "claude-haiku-4-5".into(),
                    display_name: "Claude Haiku 4.5".into(),
                    context_window: Some(200_000),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                },
            ]
        );
    }

    #[test]
    fn debug_never_shows_the_key() {
        let p = AnthropicProvider::new("sk-ant-secret");
        assert!(!format!("{p:?}").contains("secret"));
    }
}
