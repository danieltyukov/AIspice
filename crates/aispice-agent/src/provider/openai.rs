//! OpenAI-compatible Chat Completions.
//!
//! One implementation serves OpenAI, Google (Gemini's OpenAI endpoint),
//! OpenRouter, Ollama, LM Studio, vLLM and anything else that speaks the same
//! protocol. Those servers differ in small ways, so the parser is tolerant:
//! `[DONE]` is optional when a finish reason arrived, usage may be missing,
//! tool call ids may come only in the first delta (or never), reasoning
//! arrives as `reasoning_content` or `reasoning`, and a server that ignores
//! `stream: true` and answers with plain JSON still works.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::anthropic::parse_tool_input;
use super::{
    ChatRequest, ChatResponse, Effort, HttpLimits, ModelInfo, Provider, ProviderError, Retrier,
    RetryPolicy, StopReason, StreamEvent, Usage, display_url, http_client, read_limited, read_sse,
    send, trim_base_url,
};

/// Headers that carry nothing secret. Any other custom header value is
/// treated as a credential and scrubbed from errors.
const PUBLIC_HEADERS: &[&str] = &["http-referer", "x-title"];
use crate::message::{ContentBlock, Message, Role};

pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const GOOGLE_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai";
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";

/// Which field carries the output limit. OpenAI's reasoning models reject
/// `max_tokens`; most other servers only know `max_tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenField {
    MaxTokens,
    MaxCompletionTokens,
}

/// How a reasoning effort is requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReasoningStyle {
    /// Top-level `reasoning_effort` (OpenAI, Google, Ollama).
    Effort,
    /// `reasoning: {effort}` (OpenRouter's normalized form).
    Object,
}

#[derive(Clone)]
pub struct OpenAiProvider {
    id: String,
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    headers: Vec<(String, String)>,
    retry: RetryPolicy,
    limits: HttpLimits,
    token_field: TokenField,
    reasoning: ReasoningStyle,
    /// Gemini rejects some JSON Schema keywords the others accept.
    gemini_schemas: bool,
    /// Provider-specific data a server attached to a tool call
    /// (`extra_content`), by call id. Gemini 3 puts each call's
    /// `thought_signature` there and requires it back on the next request.
    call_extras: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>>,
}

/// Gemini's documented value for function calls whose signature is not
/// available (history from another model or an earlier session).
const GEMINI_SKIP_SIGNATURE: &str = "skip_thought_signature_validator";

impl OpenAiProvider {
    fn base(id: &str, base_url: &str, api_key: Option<String>) -> Self {
        Self {
            id: id.to_string(),
            client: http_client(),
            base_url: trim_base_url(base_url),
            api_key: api_key.filter(|k| !k.is_empty()),
            headers: Vec::new(),
            retry: RetryPolicy::default(),
            limits: HttpLimits::default(),
            token_field: TokenField::MaxTokens,
            reasoning: ReasoningStyle::Effort,
            gemini_schemas: false,
            call_extras: Default::default(),
        }
    }

    pub fn with_limits(mut self, limits: HttpLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Errors leave the provider through here, so neither the key nor a
    /// custom credential header ever does.
    fn redact(&self, err: ProviderError) -> ProviderError {
        let mut secrets: Vec<&str> = self.api_key.iter().map(String::as_str).collect();
        secrets.extend(
            self.headers
                .iter()
                .filter(|(name, _)| !PUBLIC_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
                .map(|(_, value)| value.as_str()),
        );
        err.redacted(&secrets)
    }

    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            token_field: TokenField::MaxCompletionTokens,
            ..Self::base("openai", OPENAI_BASE_URL, Some(api_key.into()))
        }
    }

    pub fn google(api_key: impl Into<String>) -> Self {
        Self {
            gemini_schemas: true,
            ..Self::base("google", GOOGLE_BASE_URL, Some(api_key.into()))
        }
    }

    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self {
            reasoning: ReasoningStyle::Object,
            ..Self::base("openrouter", OPENROUTER_BASE_URL, Some(api_key.into()))
        }
        .with_header("HTTP-Referer", "https://danieltyukov.github.io/aispice/")
        .with_header("X-Title", "aispice")
    }

    /// A local Ollama server. No key needed.
    pub fn ollama() -> Self {
        Self::base("ollama", OLLAMA_BASE_URL, None)
    }

    /// Any OpenAI-compatible endpoint. `base_url` is the part before
    /// `/chat/completions`, usually ending in `/v1`.
    pub fn custom(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self::base("custom", &base_url.into(), api_key)
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = trim_base_url(base_url);
        self
    }

    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key.filter(|k| !k.is_empty());
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
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

    /// The JSON body sent to `/chat/completions`.
    /// Put back the `extra_content` each tool call came with. For Gemini, a
    /// call without a stored signature gets the documented skip value so
    /// older history does not make the request fail.
    fn attach_call_extras(&self, messages: &mut Value) {
        let extras = self.call_extras.lock().expect("call extras lock");
        for m in messages.as_array_mut().into_iter().flatten() {
            for call in m
                .get_mut("tool_calls")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                let id = call["id"].as_str().unwrap_or_default().to_string();
                if let Some(extra) = extras.get(&id) {
                    call["extra_content"] = extra.clone();
                } else if self.gemini_schemas {
                    call["extra_content"] =
                        json!({"google": {"thought_signature": GEMINI_SKIP_SIGNATURE}});
                }
            }
        }
    }

    pub fn request_body(&self, req: &ChatRequest) -> Value {
        let mut body = json!({
            "model": req.model,
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": wire_messages(&req.system, &req.messages),
        });
        self.attach_call_extras(&mut body["messages"]);
        let token_field = match self.token_field {
            TokenField::MaxTokens => "max_tokens",
            TokenField::MaxCompletionTokens => "max_completion_tokens",
        };
        body[token_field] = json!(req.max_tokens);
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    let mut schema = t.input_schema.clone();
                    if self.gemini_schemas {
                        strip_keys(&mut schema, &["additionalProperties", "$schema"]);
                    }
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": schema,
                        },
                    })
                })
                .collect();
            body["tools"] = Value::Array(tools);
        }
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t.to_string().parse::<f64>().unwrap_or(f64::from(t)));
        }
        if let Some(effort) = req.thinking.as_ref().and_then(|t| t.effort) {
            // OpenAI-style APIs top out at "high".
            let level = match effort {
                Effort::Low => "low",
                Effort::Medium => "medium",
                Effort::High | Effort::XHigh | Effort::Max => "high",
            };
            match self.reasoning {
                ReasoningStyle::Effort => body["reasoning_effort"] = json!(level),
                ReasoningStyle::Object => body["reasoning"] = json!({"effort": level}),
            }
        }
        body
    }

    fn authorize(&self, mut request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        request
    }

    async fn stream_once(
        &self,
        body: &Value,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, ProviderError> {
        let request = self.authorize(
            self.client
                .post(format!("{}/chat/completions", self.base_url))
                .header("accept", "text/event-stream")
                .timeout(self.limits.request_timeout)
                .json(body),
        );
        let response = send(request, cancel, &self.limits).await?;
        let is_json = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("application/json"));
        let mut state = StreamState::new(&self.limits);
        if is_json {
            let body = read_limited(
                response,
                self.limits.max_stream_bytes,
                Some("the response"),
                &self.limits,
                cancel,
            )
            .await?;
            state.handle(&String::from_utf8_lossy(&body), sink)?;
            state.done = true;
        } else {
            read_sse(response, &self.limits, cancel, |event| {
                state.handle(&event.data, sink)
            })
            .await?;
        }
        let mut extras = self.call_extras.lock().expect("call extras lock");
        for c in &state.calls {
            if let Some(extra) = &c.extra {
                // Bounded: a long session keeps at most the latest few thousand.
                if extras.len() > 4096 {
                    extras.clear();
                }
                extras.insert(c.id.clone(), extra.clone());
            }
        }
        drop(extras);
        state.finish()
    }

    async fn fetch_models(&self, cancel: &CancellationToken) -> Result<Value, ProviderError> {
        let request = self.authorize(
            self.client
                .get(format!("{}/models", self.base_url))
                .timeout(self.limits.list_timeout),
        );
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

impl fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("id", &self.id)
            .field("base_url", &display_url(&self.base_url))
            .field("has_key", &self.api_key.is_some())
            .field("headers", &self.headers.len())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn id(&self) -> &str {
        &self.id
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
                    if !retrier.backoff(&self.id, &err, streamed).await? {
                        return Err(err);
                    }
                }
            }
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let cancel = CancellationToken::new();
        let mut retrier = Retrier::new(&self.retry, &cancel);
        let page: Value = loop {
            match self.fetch_models(&cancel).await {
                Ok(page) => break page,
                Err(err) => {
                    let err = self.redact(err);
                    if !retrier.backoff(&self.id, &err, false).await? {
                        return Err(err);
                    }
                }
            }
        };
        Ok(page["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(model_info)
            .collect())
    }
}

fn model_info(m: &Value) -> Option<ModelInfo> {
    let raw_id = m["id"].as_str()?;
    // Gemini lists models as `models/gemini-...` but takes the bare name.
    let id = raw_id.strip_prefix("models/").unwrap_or(raw_id).to_string();
    let display_name = m["name"]
        .as_str()
        .or(m["display_name"].as_str())
        .unwrap_or(&id)
        .to_string();
    let context_window = m["context_length"]
        .as_u64()
        .or(m["context_window"].as_u64())
        .or(m
            .pointer("/top_provider/context_length")
            .and_then(Value::as_u64))
        .map(|n| n as u32);
    let contains = |list: Option<&Value>, item: &str| {
        list.and_then(Value::as_array)
            .map(|a| a.iter().any(|v| v.as_str() == Some(item)))
    };
    Some(ModelInfo {
        id,
        display_name,
        context_window,
        supports_tools: contains(m.get("supported_parameters"), "tools"),
        supports_vision: contains(m.pointer("/architecture/input_modalities"), "image"),
    })
}

/// Canonical messages to Chat Completions messages.
///
/// Tool results become `role: "tool"` messages. Those cannot carry images,
/// so any images a tool returned follow in one user message with a short
/// note naming the call. Thinking blocks are dropped: these APIs either
/// ignore reasoning on input or reject it.
pub fn wire_messages(system: &str, messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    if !system.is_empty() {
        out.push(json!({"role": "system", "content": system}));
    }
    for message in messages {
        match message.role {
            Role::Assistant => out.extend(assistant_message(message)),
            Role::User => user_messages(message, &mut out),
        }
    }
    out
}

fn assistant_message(message: &Message) -> Option<Value> {
    let text = message.text();
    let tool_calls: Vec<Value> = message
        .tool_uses()
        .into_iter()
        .map(|call| {
            json!({
                "id": call.id,
                "type": "function",
                "function": {"name": call.name, "arguments": arguments_text(call.input)},
            })
        })
        .collect();
    if text.is_empty() && tool_calls.is_empty() {
        return None;
    }
    let mut wire = json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { Value::String(text) },
    });
    if !tool_calls.is_empty() {
        wire["tool_calls"] = Value::Array(tool_calls);
    }
    Some(wire)
}

/// Unparseable arguments are kept raw in the history; send them back as the
/// model wrote them.
fn arguments_text(input: &Value) -> String {
    match input {
        Value::String(raw) => raw.clone(),
        other => other.to_string(),
    }
}

fn user_messages(message: &Message, out: &mut Vec<Value>) {
    let mut parts: Vec<Value> = Vec::new();
    for block in &message.content {
        match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let text = content
                    .iter()
                    .filter_map(ContentBlock::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                let images: Vec<&ContentBlock> = content
                    .iter()
                    .filter(|c| matches!(c, ContentBlock::Image { .. }))
                    .collect();
                let mut body = if *is_error {
                    format!("Error: {text}")
                } else {
                    text
                };
                if !images.is_empty() {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(&format!(
                        "[{} image(s) attached in the next user message]",
                        images.len()
                    ));
                    parts.push(text_part(&format!(
                        "Image output of tool call {tool_use_id}:"
                    )));
                    parts.extend(images.into_iter().filter_map(image_part));
                }
                if body.is_empty() {
                    body = "(no output)".into();
                }
                out.push(json!({"role": "tool", "tool_call_id": tool_use_id, "content": body}));
            }
            ContentBlock::Text { text } if !text.is_empty() => parts.push(text_part(text)),
            ContentBlock::Image { .. } => parts.extend(image_part(block)),
            _ => {}
        }
    }
    if parts.is_empty() {
        return;
    }
    // Plain text goes as a string: some servers reject content arrays.
    if parts.iter().all(|p| p["type"] == "text") {
        let text: Vec<&str> = parts.iter().filter_map(|p| p["text"].as_str()).collect();
        out.push(json!({"role": "user", "content": text.join("\n")}));
    } else {
        out.push(json!({"role": "user", "content": parts}));
    }
}

fn text_part(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn image_part(block: &ContentBlock) -> Option<Value> {
    match block {
        ContentBlock::Image {
            media_type,
            data_base64,
        } => Some(json!({
            "type": "image_url",
            "image_url": {"url": format!("data:{media_type};base64,{data_base64}")},
        })),
        _ => None,
    }
}

fn strip_keys(value: &mut Value, keys: &[&str]) {
    match value {
        Value::Object(map) => {
            for key in keys {
                map.remove(*key);
            }
            map.values_mut().for_each(|v| strip_keys(v, keys));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| strip_keys(v, keys)),
        _ => {}
    }
}

struct Call {
    index: Option<u64>,
    id: String,
    name: String,
    arguments: String,
    started: bool,
    /// The call's `extra_content`, kept to send back (Gemini signatures).
    extra: Option<Value>,
}

struct StreamState {
    text: String,
    reasoning: String,
    calls: Vec<Call>,
    finish_reason: Option<String>,
    usage: Usage,
    done: bool,
    max_blocks: usize,
    max_tool_input: usize,
}

impl StreamState {
    fn new(limits: &HttpLimits) -> Self {
        Self {
            text: String::new(),
            reasoning: String::new(),
            calls: Vec::new(),
            finish_reason: None,
            usage: Usage::default(),
            done: false,
            max_blocks: limits.max_blocks,
            max_tool_input: limits.max_tool_input,
        }
    }

    /// Returns `Ok(false)` at `[DONE]`.
    fn handle(
        &mut self,
        data: &str,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<bool, ProviderError> {
        let data = data.trim();
        if data.is_empty() {
            return Ok(true);
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(false);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| ProviderError::Parse(format!("{e} in chunk {data:.200}")))?;
        if v["error"].is_object() {
            return Err(stream_error(&v["error"]));
        }
        if v["usage"].is_object() {
            self.update_usage(&v["usage"]);
        }
        for choice in v["choices"].as_array().into_iter().flatten() {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                continue;
            }
            // `message` is the non-streaming shape.
            let delta = if choice["delta"].is_object() {
                &choice["delta"]
            } else {
                &choice["message"]
            };
            if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
                self.text.push_str(text);
                sink(StreamEvent::TextDelta(text.to_string()));
            }
            let reasoning = delta["reasoning_content"]
                .as_str()
                .or(delta["reasoning"].as_str())
                .filter(|t| !t.is_empty());
            if let Some(text) = reasoning {
                self.reasoning.push_str(text);
                sink(StreamEvent::ThinkingDelta(text.to_string()));
            }
            for (pos, call) in delta["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                self.tool_call_delta(call, pos, sink)?;
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                self.finish_reason = Some(reason.to_string());
            }
        }
        Ok(true)
    }

    fn tool_call_delta(
        &mut self,
        delta: &Value,
        pos: usize,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        let index = delta["index"].as_u64();
        let id = delta["id"].as_str().filter(|s| !s.is_empty());
        let existing = match (index, id) {
            (Some(i), _) => self.calls.iter().position(|c| c.index == Some(i)),
            (None, Some(id)) => self.calls.iter().position(|c| c.id == id),
            // No index and no id: a continuation of the last call, unless it
            // is a second call in the same chunk.
            (None, None) if pos == 0 => self.calls.len().checked_sub(1),
            (None, None) => None,
        };
        let slot = match existing {
            Some(slot) => slot,
            None if self.calls.len() >= self.max_blocks => {
                return Err(ProviderError::TooLarge(format!(
                    "more than {} tool calls in one response",
                    self.max_blocks
                )));
            }
            None => {
                self.calls.push(Call {
                    index,
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                    started: false,
                    extra: None,
                });
                self.calls.len() - 1
            }
        };
        let max_tool_input = self.max_tool_input;
        let call = &mut self.calls[slot];
        if call.id.is_empty()
            && let Some(id) = id
        {
            call.id = id.to_string();
        }
        if call.name.is_empty()
            && let Some(name) = delta["function"]["name"].as_str()
        {
            call.name = name.to_string();
        }
        if !delta["extra_content"].is_null() {
            call.extra = Some(delta["extra_content"].clone());
        }
        let fragment = match &delta["function"]["arguments"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            // Some servers send the arguments as an object instead of text.
            other => other.to_string(),
        };
        if call.arguments.len() + fragment.len() > max_tool_input {
            return Err(ProviderError::too_large(
                "the input of one tool call",
                max_tool_input,
            ));
        }
        call.arguments.push_str(&fragment);
        if !call.started && !call.name.is_empty() {
            call.started = true;
            if call.id.is_empty() {
                call.id = generated_call_id(slot);
            }
            sink(StreamEvent::ToolUseStart {
                id: call.id.clone(),
                name: call.name.clone(),
            });
            if !call.arguments.is_empty() {
                sink(StreamEvent::ToolUseInputDelta {
                    id: call.id.clone(),
                    partial_json: call.arguments.clone(),
                });
            }
        } else if call.started && !fragment.is_empty() {
            sink(StreamEvent::ToolUseInputDelta {
                id: call.id.clone(),
                partial_json: fragment,
            });
        }
        Ok(())
    }

    fn update_usage(&mut self, usage: &Value) {
        let prompt = usage["prompt_tokens"].as_u64().unwrap_or(0);
        let cached = usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .or(usage["prompt_cache_hit_tokens"].as_u64())
            .unwrap_or(0);
        self.usage = Usage {
            input_tokens: prompt.saturating_sub(cached),
            output_tokens: usage["completion_tokens"].as_u64().unwrap_or(0),
            cache_read_tokens: cached,
            cache_write_tokens: 0,
        };
    }

    fn finish(self) -> Result<ChatResponse, ProviderError> {
        if !self.done && self.finish_reason.is_none() {
            return Err(ProviderError::Network(
                "the stream ended before the response finished".into(),
            ));
        }
        let mut content = Vec::new();
        if !self.reasoning.is_empty() {
            content.push(ContentBlock::Thinking {
                thinking: self.reasoning,
                signature: None,
            });
        }
        if !self.text.is_empty() {
            content.push(ContentBlock::Text { text: self.text });
        }
        let has_calls = self.calls.iter().any(|c| !c.name.is_empty());
        for (slot, call) in self.calls.into_iter().enumerate() {
            if call.name.is_empty() {
                continue;
            }
            content.push(ContentBlock::ToolUse {
                id: if call.id.is_empty() {
                    generated_call_id(slot)
                } else {
                    call.id
                },
                name: call.name,
                input: parse_tool_input(&call.arguments),
            });
        }
        // Several servers report "stop" even when the turn ends in tool
        // calls, so the calls themselves decide.
        let stop_reason = match self.finish_reason.as_deref() {
            Some("length") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::Refusal,
            _ if has_calls => StopReason::ToolUse,
            None | Some("stop") => StopReason::EndTurn,
            Some(other) => StopReason::Other(other.to_string()),
        };
        Ok(ChatResponse {
            message: Message::assistant(content),
            stop_reason,
            usage: self.usage,
        })
    }
}

fn generated_call_id(slot: usize) -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_usize(slot);
    format!("call_{:016x}", hasher.finish())
}

fn stream_error(err: &Value) -> ProviderError {
    let message = err["message"]
        .as_str()
        .unwrap_or("error in the response stream")
        .to_string();
    if let Some(code) = err["code"].as_u64().or(err["status"].as_u64()) {
        return ProviderError::from_status(code as u16, message, None);
    }
    let kind = err["type"].as_str().or(err["code"].as_str()).unwrap_or("");
    if kind.contains("rate_limit") {
        ProviderError::RateLimited {
            message,
            retry_after: None,
        }
    } else if kind.contains("overloaded") || kind.contains("server_error") {
        ProviderError::Overloaded {
            status: 500,
            message,
        }
    } else {
        ProviderError::BadRequest {
            status: 400,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ThinkingConfig;
    use crate::tool::ToolSpec;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn request(messages: Vec<Message>) -> ChatRequest {
        ChatRequest {
            model: "gpt-test".into(),
            system: "You design circuits.".into(),
            messages,
            tools: vec![ToolSpec {
                name: "simulate".into(),
                description: "Run a simulation".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"circuit": {"type": "string"}},
                    "additionalProperties": false,
                }),
            }],
            max_tokens: 16384,
            temperature: None,
            thinking: None,
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

    async fn mount_sse(server: &MockServer, body: &str) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_string(), "text/event-stream"),
            )
            .mount(server)
            .await;
    }

    async fn run(
        p: &OpenAiProvider,
        req: &ChatRequest,
    ) -> (Result<ChatResponse, ProviderError>, Vec<StreamEvent>) {
        let mut events = Vec::new();
        let result = p
            .stream(req, &mut |e| events.push(e), &CancellationToken::new())
            .await;
        (result, events)
    }

    fn openai(server: &MockServer) -> OpenAiProvider {
        OpenAiProvider::openai("sk-test")
            .with_base_url(format!("{}/v1", server.uri()))
            .with_retry(fast_retry())
    }

    // ---- canonical to wire ----

    #[test]
    fn converts_tool_results_with_images_into_tool_and_user_messages() {
        let history = vec![
            Message::user(vec![
                ContentBlock::text("Render it"),
                ContentBlock::image("image/png", "QUJD"),
            ]),
            Message::assistant(vec![
                ContentBlock::Thinking {
                    thinking: "need the render".into(),
                    signature: Some("sig".into()),
                },
                ContentBlock::text("Rendering."),
                ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "render_schematic".into(),
                    input: json!({"circuit": "amp.asc"}),
                },
                ContentBlock::ToolUse {
                    id: "call_2".into(),
                    name: "simulate".into(),
                    input: Value::String("{\"circuit\": ".into()),
                },
            ]),
            Message::user(vec![
                ContentBlock::tool_result(
                    "call_1",
                    vec![
                        ContentBlock::text("Rendered."),
                        ContentBlock::image("image/png", "AAAA"),
                    ],
                    false,
                ),
                ContentBlock::tool_result("call_2", vec![ContentBlock::text("bad json")], true),
                ContentBlock::text("Also check the bias."),
            ]),
            Message::assistant(vec![ContentBlock::Thinking {
                thinking: "only thinking".into(),
                signature: None,
            }]),
        ];
        assert_eq!(
            Value::Array(wire_messages("sys", &history)),
            json!([
                {"role": "system", "content": "sys"},
                {"role": "user", "content": [
                    {"type": "text", "text": "Render it"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJD"}}
                ]},
                {"role": "assistant", "content": "Rendering.", "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "render_schematic", "arguments": "{\"circuit\":\"amp.asc\"}"}},
                    {"id": "call_2", "type": "function", "function": {"name": "simulate", "arguments": "{\"circuit\": "}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "Rendered.\n[1 image(s) attached in the next user message]"},
                {"role": "tool", "tool_call_id": "call_2", "content": "Error: bad json"},
                {"role": "user", "content": [
                    {"type": "text", "text": "Image output of tool call call_1:"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
                    {"type": "text", "text": "Also check the bias."}
                ]}
            ])
        );
    }

    #[test]
    fn text_only_user_content_is_a_string_and_empty_results_say_so() {
        let history = vec![
            Message::user_text("hello"),
            Message::assistant(vec![ContentBlock::ToolUse {
                id: "c".into(),
                name: "t".into(),
                input: json!({}),
            }]),
            Message::user(vec![ContentBlock::tool_result("c", vec![], false)]),
        ];
        assert_eq!(
            Value::Array(wire_messages("", &history)),
            json!([
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "c", "type": "function", "function": {"name": "t", "arguments": "{}"}}
                ]},
                {"role": "tool", "tool_call_id": "c", "content": "(no output)"}
            ])
        );
    }

    #[test]
    fn request_bodies_per_preset() {
        let mut req = request(vec![Message::user_text("hi")]);
        req.thinking = Some(ThinkingConfig::with_effort(Effort::Max));
        req.temperature = Some(0.3);

        let body = OpenAiProvider::openai("k").request_body(&req);
        assert_eq!(body["max_completion_tokens"], 16384);
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["temperature"], json!(0.3));
        assert_eq!(body["stream_options"], json!({"include_usage": true}));
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(
            body["tools"][0]["function"]["parameters"]["additionalProperties"],
            false
        );

        let body = OpenAiProvider::google("k").request_body(&req);
        assert_eq!(body["max_tokens"], 16384);
        assert!(
            body["tools"][0]["function"]["parameters"]
                .get("additionalProperties")
                .is_none()
        );

        let body = OpenAiProvider::openrouter("k").request_body(&req);
        assert_eq!(body["reasoning"], json!({"effort": "high"}));
        assert!(body.get("reasoning_effort").is_none());

        req.thinking = None;
        req.tools.clear();
        let body = OpenAiProvider::ollama().request_body(&req);
        assert!(body.get("tools").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn preset_base_urls() {
        assert_eq!(OpenAiProvider::openai("k").base_url(), OPENAI_BASE_URL);
        assert_eq!(OpenAiProvider::google("k").base_url(), GOOGLE_BASE_URL);
        assert_eq!(
            OpenAiProvider::openrouter("k").base_url(),
            OPENROUTER_BASE_URL
        );
        assert_eq!(OpenAiProvider::ollama().base_url(), OLLAMA_BASE_URL);
        let custom = OpenAiProvider::custom("http://localhost:1234/v1/", None);
        assert_eq!(custom.base_url(), "http://localhost:1234/v1");
        assert_eq!(custom.id(), "custom");
        assert!(!format!("{:?}", OpenAiProvider::openai("sk-secret")).contains("secret"));
    }

    // ---- recorded-shape streams ----

    const TEXT: &str = r#"data: {"id":"chatcmpl-B9MBs8CjcvOU2jLn4n570S5qMJKcT","object":"chat.completion.chunk","created":1741569952,"model":"gpt-test","service_tier":"default","system_fingerprint":"fp_1","choices":[{"index":0,"delta":{"role":"assistant","content":"","refusal":null},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-B9MBs8CjcvOU2jLn4n570S5qMJKcT","object":"chat.completion.chunk","created":1741569952,"model":"gpt-test","service_tier":"default","system_fingerprint":"fp_1","choices":[{"index":0,"delta":{"content":"The cutoff"},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-B9MBs8CjcvOU2jLn4n570S5qMJKcT","object":"chat.completion.chunk","created":1741569952,"model":"gpt-test","service_tier":"default","system_fingerprint":"fp_1","choices":[{"index":0,"delta":{"content":" is 1.59 kHz."},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-B9MBs8CjcvOU2jLn4n570S5qMJKcT","object":"chat.completion.chunk","created":1741569952,"model":"gpt-test","service_tier":"default","system_fingerprint":"fp_1","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"stop"}],"usage":null}

data: {"id":"chatcmpl-B9MBs8CjcvOU2jLn4n570S5qMJKcT","object":"chat.completion.chunk","created":1741569952,"model":"gpt-test","service_tier":"default","system_fingerprint":"fp_1","choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":12,"total_tokens":1212,"prompt_tokens_details":{"cached_tokens":1024,"audio_tokens":0},"completion_tokens_details":{"reasoning_tokens":0,"audio_tokens":0}}}

data: [DONE]

"#;

    #[tokio::test]
    async fn streams_text() {
        let server = MockServer::start().await;
        mount_sse(&server, TEXT).await;
        let (result, events) = run(
            &openai(&server),
            &request(vec![Message::user_text("cutoff?")]),
        )
        .await;
        assert_eq!(
            result.unwrap(),
            ChatResponse {
                message: Message::assistant_text("The cutoff is 1.59 kHz."),
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 176,
                    output_tokens: 12,
                    cache_read_tokens: 1024,
                    cache_write_tokens: 0,
                },
            }
        );
        assert_eq!(
            events,
            vec![
                StreamEvent::TextDelta("The cutoff".into()),
                StreamEvent::TextDelta(" is 1.59 kHz.".into()),
            ]
        );
        let sent = &server.received_requests().await.unwrap()[0];
        assert_eq!(sent.headers["authorization"], "Bearer sk-test");
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(
            body["messages"][0],
            json!({"role": "system", "content": "You design circuits."})
        );
        assert_eq!(body["stream"], true);
    }

    const PARALLEL_TOOL_CALLS: &str = r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-test","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_DdmO9pD3xa9XTPNJ32zg2hcA","type":"function","function":{"name":"read_schematic","arguments":""}}],"refusal":null},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-test","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"circ"}}]},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-test","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_ABgVZ1a8mVb3BLQOCvr27Hsj","type":"function","function":{"name":"simulate","arguments":""}}]},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-test","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"circuit\": \"rc.asc\", "}}]},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-test","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"uit\": \"rc.asc\"}"}}]},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-test","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"analysis\": {\"type\": \"tran\", \"stop\": \"5m\"}}"}}]},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-test","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"tool_calls"}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-test","choices":[],"usage":{"prompt_tokens":300,"completion_tokens":40,"total_tokens":340}}

data: [DONE]

"#;

    #[tokio::test]
    async fn streams_parallel_tool_calls_with_split_arguments() {
        let server = MockServer::start().await;
        mount_sse(&server, PARALLEL_TOOL_CALLS).await;
        let (result, events) =
            run(&openai(&server), &request(vec![Message::user_text("go")])).await;
        assert_eq!(
            result.unwrap(),
            ChatResponse {
                message: Message::assistant(vec![
                    ContentBlock::ToolUse {
                        id: "call_DdmO9pD3xa9XTPNJ32zg2hcA".into(),
                        name: "read_schematic".into(),
                        input: json!({"circuit": "rc.asc"}),
                    },
                    ContentBlock::ToolUse {
                        id: "call_ABgVZ1a8mVb3BLQOCvr27Hsj".into(),
                        name: "simulate".into(),
                        input: json!({"circuit": "rc.asc", "analysis": {"type": "tran", "stop": "5m"}}),
                    },
                ]),
                stop_reason: StopReason::ToolUse,
                usage: Usage {
                    input_tokens: 300,
                    output_tokens: 40,
                    ..Usage::default()
                },
            }
        );
        let starts: Vec<&StreamEvent> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolUseStart { .. }))
            .collect();
        assert_eq!(starts.len(), 2);
        assert!(events.contains(&StreamEvent::ToolUseInputDelta {
            id: "call_DdmO9pD3xa9XTPNJ32zg2hcA".into(),
            partial_json: "uit\": \"rc.asc\"}".into(),
        }));
    }

    #[tokio::test]
    async fn retries_after_429() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "0")
                    .set_body_json(json!({"error": {"message": "Rate limit reached for requests", "type": "requests", "code": "rate_limit_exceeded"}})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mount_sse(&server, TEXT).await;
        let (result, _) = run(&openai(&server), &request(vec![Message::user_text("hi")])).await;
        assert_eq!(result.unwrap().message.text(), "The cutoff is 1.59 kHz.");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn gives_up_after_bounded_retries() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
            .mount(&server)
            .await;
        let (result, _) = run(&openai(&server), &request(vec![Message::user_text("hi")])).await;
        match result {
            Err(ProviderError::Overloaded {
                status: 503,
                message,
            }) => assert_eq!(message, "upstream down"),
            other => panic!("{other:?}"),
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    const OLLAMA: &str = r#"data: {"id":"chatcmpl-417","object":"chat.completion.chunk","created":1760000000,"model":"qwen3:8b","system_fingerprint":"fp_ollama","choices":[{"index":0,"delta":{"role":"assistant","content":"","reasoning":"Need the netlist."},"finish_reason":null}]}

data: {"id":"chatcmpl-417","object":"chat.completion.chunk","created":1760000000,"model":"qwen3:8b","system_fingerprint":"fp_ollama","choices":[{"index":0,"delta":{"role":"assistant","content":"Checking."},"finish_reason":null}]}

data: {"id":"chatcmpl-417","object":"chat.completion.chunk","created":1760000000,"model":"qwen3:8b","system_fingerprint":"fp_ollama","choices":[{"index":0,"delta":{"role":"assistant","content":"","tool_calls":[{"id":"call_q0pb3bsz","index":0,"type":"function","function":{"name":"netlist","arguments":"{\"circuit\":\"rc.asc\"}"}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-417","object":"chat.completion.chunk","created":1760000000,"model":"qwen3:8b","system_fingerprint":"fp_ollama","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":"stop"}]}

data: [DONE]

"#;

    #[tokio::test]
    async fn ollama_stream_without_usage_or_key() {
        let server = MockServer::start().await;
        mount_sse(&server, OLLAMA).await;
        let p = OpenAiProvider::ollama()
            .with_base_url(format!("{}/v1", server.uri()))
            .with_retry(fast_retry());
        let (result, events) = run(&p, &request(vec![Message::user_text("netlist?")])).await;
        assert_eq!(
            result.unwrap(),
            ChatResponse {
                message: Message::assistant(vec![
                    ContentBlock::Thinking {
                        thinking: "Need the netlist.".into(),
                        signature: None,
                    },
                    ContentBlock::text("Checking."),
                    ContentBlock::ToolUse {
                        id: "call_q0pb3bsz".into(),
                        name: "netlist".into(),
                        input: json!({"circuit": "rc.asc"}),
                    },
                ]),
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            }
        );
        assert_eq!(
            events[0],
            StreamEvent::ThinkingDelta("Need the netlist.".into())
        );
        let sent = &server.received_requests().await.unwrap()[0];
        assert!(sent.headers.get("authorization").is_none());
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(body["max_tokens"], 16384);
    }

    #[tokio::test]
    async fn tolerates_missing_done_missing_ids_and_reasoning_content() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hm\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"name\":\"lint\",\"arguments\":\"{\\\"circuit\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"arguments\":\"\\\"a.asc\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        );
        let server = MockServer::start().await;
        mount_sse(&server, stream).await;
        let (result, events) =
            run(&openai(&server), &request(vec![Message::user_text("lint")])).await;
        let response = result.unwrap();
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(
            response.message.content[0],
            ContentBlock::Thinking {
                thinking: "hm".into(),
                signature: None,
            }
        );
        let ContentBlock::ToolUse { id, name, input } = &response.message.content[1] else {
            panic!("{:?}", response.message);
        };
        assert!(id.starts_with("call_"));
        assert_eq!(name, "lint");
        assert_eq!(input, &json!({"circuit": "a.asc"}));
        assert!(events.contains(&StreamEvent::ToolUseStart {
            id: id.clone(),
            name: "lint".into(),
        }));
    }

    #[tokio::test]
    async fn bad_arguments_stay_raw_and_length_maps_to_max_tokens() {
        let stream = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"simulate\",\"arguments\":\"{\\\"circuit\\\": \\\"rc\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let server = MockServer::start().await;
        mount_sse(&server, stream).await;
        let (result, _) = run(&openai(&server), &request(vec![Message::user_text("x")])).await;
        let response = result.unwrap();
        assert_eq!(response.stop_reason, StopReason::MaxTokens);
        assert_eq!(
            response.message.content,
            vec![ContentBlock::ToolUse {
                id: "c1".into(),
                name: "simulate".into(),
                input: Value::String("{\"circuit\": \"rc".into()),
            }]
        );
    }

    #[tokio::test]
    async fn error_object_mid_stream() {
        let stream = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            "data: {\"error\":{\"code\":502,\"message\":\"Provider returned error\"}}\n\n",
        );
        let server = MockServer::start().await;
        mount_sse(&server, stream).await;
        let (result, _) = run(&openai(&server), &request(vec![Message::user_text("x")])).await;
        match result {
            Err(ProviderError::Overloaded {
                status: 502,
                message,
            }) => assert_eq!(message, "Provider returned error"),
            other => panic!("{other:?}"),
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn accepts_a_plain_json_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "x", "object": "chat.completion",
                "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "Done."}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2}
            })))
            .mount(&server)
            .await;
        let (result, _) = run(&openai(&server), &request(vec![Message::user_text("x")])).await;
        let response = result.unwrap();
        assert_eq!(response.message, Message::assistant_text("Done."));
        assert_eq!(response.usage.input_tokens, 5);
    }

    #[tokio::test]
    async fn openrouter_sends_attribution_headers_and_lists_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/models"))
            .and(header("authorization", "Bearer or-key"))
            .and(header("http-referer", "https://danieltyukov.github.io/aispice/"))
            .and(header("x-title", "aispice"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
                {"id": "anthropic/claude-opus-5-5", "name": "Anthropic: Claude Opus 5.5", "context_length": 1000000,
                 "architecture": {"input_modalities": ["text", "image"]}, "supported_parameters": ["tools", "reasoning"]},
                {"id": "some/text-model", "name": "Text Model", "context_length": 8192,
                 "architecture": {"input_modalities": ["text"]}, "supported_parameters": ["temperature"]}
            ]})))
            .mount(&server)
            .await;
        let p =
            OpenAiProvider::openrouter("or-key").with_base_url(format!("{}/api/v1", server.uri()));
        assert_eq!(
            p.list_models().await.unwrap(),
            vec![
                ModelInfo {
                    id: "anthropic/claude-opus-5-5".into(),
                    display_name: "Anthropic: Claude Opus 5.5".into(),
                    context_window: Some(1_000_000),
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                },
                ModelInfo {
                    id: "some/text-model".into(),
                    display_name: "Text Model".into(),
                    context_window: Some(8192),
                    supports_tools: Some(false),
                    supports_vision: Some(false),
                },
            ]
        );
    }

    #[tokio::test]
    async fn gemini_and_ollama_model_lists() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"object": "list", "data": [
                    {"id": "models/gemini-2.5-pro", "object": "model", "owned_by": "google"},
                    {"id": "qwen3:8b", "object": "model", "created": 1, "owned_by": "library"}
                ]}),
            ))
            .mount(&server)
            .await;
        let p = OpenAiProvider::google("g").with_base_url(format!("{}/v1", server.uri()));
        let models = p.list_models().await.unwrap();
        assert_eq!(models[0].id, "gemini-2.5-pro");
        assert_eq!(models[0].display_name, "gemini-2.5-pro");
        assert_eq!(models[0].supports_tools, None);
        assert_eq!(models[1].id, "qwen3:8b");
    }

    #[tokio::test]
    async fn gemini_thought_signatures_round_trip() {
        let server = wiremock::MockServer::start().await;
        let stream = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"fc_1\",\"type\":\"function\",\"function\":{\"name\":\"lint\",\"arguments\":\"{}\"},\"extra_content\":{\"google\":{\"thought_signature\":\"SIG123\"}}}]}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(stream),
            )
            .mount(&server)
            .await;
        let p = OpenAiProvider::google("k").with_base_url(server.uri());
        let req_with = |messages: Vec<Message>| ChatRequest {
            model: "gemini-3".into(),
            system: String::new(),
            messages,
            tools: vec![],
            max_tokens: 100,
            temperature: None,
            thinking: None,
        };
        let req = req_with(vec![Message::user(vec![ContentBlock::text("go")])]);
        let cancel = CancellationToken::new();
        let resp = p.stream(&req, &mut |_| {}, &cancel).await.unwrap();
        // The next request carries the signature back on that call, and a
        // call with no stored signature gets Gemini's skip value.
        let mut history = vec![resp.message.clone()];
        history.push(Message::assistant(vec![ContentBlock::ToolUse {
            id: "old".into(),
            name: "lint".into(),
            input: json!({}),
        }]));
        let body = p.request_body(&req_with(history));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(
            msgs[0]["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
            "SIG123"
        );
        assert_eq!(
            msgs[1]["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
            GEMINI_SKIP_SIGNATURE
        );
    }
}
