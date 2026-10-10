//! Model providers behind one streaming interface.
//!
//! Two wire formats cover every provider aispice supports: the Anthropic
//! Messages API, and OpenAI-compatible Chat Completions (OpenAI, Google,
//! OpenRouter, Ollama, LM Studio, vLLM and friends). Both stream, both speak
//! native tool calling, and both convert to and from the canonical types in
//! [`crate::message`].

pub mod anthropic;
pub mod openai;
mod sse;

use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::message::Message;
use crate::tool::ToolSpec;

pub use anthropic::{AnthropicFeatures, AnthropicProvider};
pub use openai::OpenAiProvider;

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable identifier such as `anthropic` or `openrouter`.
    fn id(&self) -> &str;

    /// Send one request and stream the reply. Deltas go to `sink` as they
    /// arrive; the return value is the complete assistant message.
    async fn stream(
        &self,
        req: &ChatRequest,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, ProviderError>;

    /// Models the account can use, fetched live from the provider.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError>;
}

impl std::fmt::Debug for dyn Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider").field("id", &self.id()).finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    /// Leave `None` on current Claude models, which reject sampling
    /// parameters.
    pub temperature: Option<f32>,
    pub thinking: Option<ThinkingConfig>,
}

/// How much the model should reason before answering.
///
/// Current Claude models (Opus 4.6 and later, Sonnet 5.x, Haiku 5.5, Fable)
/// use adaptive thinking and reject a fixed budget with HTTP 400, so leave
/// `budget_tokens` empty for them and steer depth with `effort`. The budget is
/// for Claude Haiku 4.5 and older models.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThinkingConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
}

impl ThinkingConfig {
    /// Adaptive thinking at the model's default effort.
    pub fn adaptive() -> Self {
        Self::default()
    }

    pub fn with_effort(effort: Effort) -> Self {
        Self {
            budget_tokens: None,
            effort: Some(effort),
        }
    }

    pub fn with_budget(budget_tokens: u32) -> Self {
        Self {
            budget_tokens: Some(budget_tokens),
            effort: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::XHigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

impl std::str::FromStr for Effort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Ok(Effort::Low),
            "medium" => Ok(Effort::Medium),
            "high" => Ok(Effort::High),
            "xhigh" => Ok(Effort::XHigh),
            "max" => Ok(Effort::Max),
            other => Err(format!(
                "unknown effort `{other}` (expected low, medium, high, xhigh or max)"
            )),
        }
    }
}

/// Incremental output while a reply streams. Only for display: the complete
/// message arrives in [`ChatResponse`].
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolUseStart { id: String, name: String },
    ToolUseInputDelta { id: String, partial_json: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    /// The assistant message, with tool inputs already parsed.
    pub message: Message,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// A safety classifier or the model declined. Any partial output in the
    /// message should not be treated as an answer.
    Refusal,
    Other(String),
}

/// Token counts. `input_tokens` excludes cache reads and writes on every
/// provider (OpenAI reports cached tokens inside its prompt count; they are
/// moved to `cache_read_tokens` here).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Usage) {
        self.input_tokens += rhs.input_tokens;
        self.output_tokens += rhs.output_tokens;
        self.cache_read_tokens += rhs.cache_read_tokens;
        self.cache_write_tokens += rhs.cache_write_tokens;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub context_window: Option<u32>,
    pub supports_tools: Option<bool>,
    pub supports_vision: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("authentication failed (HTTP {status}): {message}")]
    Auth { status: u16, message: String },
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after: Option<Duration>,
    },
    /// Overloaded, a 5xx, or another transient server-side failure.
    #[error("provider unavailable (HTTP {status}): {message}")]
    Overloaded { status: u16, message: String },
    #[error("request rejected (HTTP {status}): {message}")]
    BadRequest { status: u16, message: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("cancelled")]
    Cancelled,
    #[error("could not parse the provider response: {0}")]
    Parse(String),
}

impl ProviderError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Overloaded { .. } | Self::Network(_)
        )
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// Map an HTTP error status to an error. `message` is the provider's own
    /// explanation when it sent one.
    pub(crate) fn from_status(status: u16, message: String, retry_after: Option<Duration>) -> Self {
        match status {
            401 | 403 => Self::Auth { status, message },
            429 => Self::RateLimited {
                message,
                retry_after,
            },
            408 | 409 | 500..=599 => Self::Overloaded { status, message },
            _ => Self::BadRequest { status, message },
        }
    }

    pub(crate) fn from_reqwest(err: reqwest::Error) -> Self {
        Self::Network(err.to_string())
    }
}

/// Bounded retry for transient failures.
///
/// A request is retried only when nothing has been streamed to the caller
/// yet: replaying a half-shown answer would duplicate text in the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(1000),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            ..Self::default()
        }
    }

    /// Exponential backoff with jitter in the upper half of each step, so
    /// concurrent clients spread out without any retry coming back instantly.
    /// A server's `retry-after` wins when present.
    pub fn delay(&self, attempt: u32, err: &ProviderError) -> Duration {
        if let Some(after) = err.retry_after() {
            return after.min(self.max_delay);
        }
        let step = self
            .base_delay
            .saturating_mul(1u32 << attempt.min(16))
            .min(self.max_delay);
        let half = step / 2;
        half + half.mul_f64(jitter_fraction())
    }
}

fn jitter_fraction() -> f64 {
    // RandomState is seeded per instance from the OS, which is all the
    // randomness backoff jitter needs.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(0);
    (hasher.finish() >> 11) as f64 / (1u64 << 53) as f64
}

/// Tracks retries across attempts of one request.
pub(crate) struct Retrier<'a> {
    policy: &'a RetryPolicy,
    cancel: &'a CancellationToken,
    attempt: u32,
}

impl<'a> Retrier<'a> {
    pub(crate) fn new(policy: &'a RetryPolicy, cancel: &'a CancellationToken) -> Self {
        Self {
            policy,
            cancel,
            attempt: 0,
        }
    }

    /// Wait before the next attempt if `err` deserves one. Returns false when
    /// the caller should give up and return `err`.
    pub(crate) async fn backoff(
        &mut self,
        provider: &str,
        err: &ProviderError,
        streamed: bool,
    ) -> Result<bool, ProviderError> {
        if streamed || !err.is_retryable() || self.attempt >= self.policy.max_retries {
            return Ok(false);
        }
        if let Some(after) = err.retry_after()
            && after > self.policy.max_delay
        {
            return Ok(false);
        }
        let delay = self.policy.delay(self.attempt, err);
        self.attempt += 1;
        tracing::warn!(
            provider,
            attempt = self.attempt,
            delay_ms = delay.as_millis() as u64,
            error = %err,
            "retrying model request"
        );
        tokio::select! {
            _ = self.cancel.cancelled() => Err(ProviderError::Cancelled),
            _ = tokio::time::sleep(delay) => Ok(true),
        }
    }
}

/// Send a request, racing it against cancellation, and turn an HTTP error
/// status into a `ProviderError` with the provider's message.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, ProviderError> {
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
        r = request.send() => r.map_err(ProviderError::from_reqwest)?,
    };
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    let retry_after = retry_after(response.headers());
    let body = tokio::select! {
        _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
        b = response.text() => b.unwrap_or_default(),
    };
    Err(ProviderError::from_status(
        status,
        error_message(&body),
        retry_after,
    ))
}

/// Pull the human-readable message out of an error body. Anthropic, OpenAI
/// and most compatible servers nest it as `{"error": {"message": ...}}`;
/// some put a string in `error` or `message`.
pub(crate) fn error_message(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let found = parsed.as_ref().and_then(|v| {
        v.pointer("/error/message")
            .or_else(|| v.get("error").filter(|e| e.is_string()))
            .or_else(|| v.get("message"))
            .or_else(|| v.get("detail"))
            .and_then(|m| m.as_str())
            .map(str::to_string)
    });
    match found {
        Some(message) => message,
        None if body.trim().is_empty() => "no details".to_string(),
        None => body.chars().take(500).collect(),
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(ms) = get("retry-after-ms").and_then(|v| v.trim().parse::<f64>().ok()) {
        return Some(Duration::from_secs_f64(ms.max(0.0) / 1000.0));
    }
    get("retry-after")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|s| Duration::from_secs_f64(s.max(0.0)))
}

/// Read a server-sent event stream, handing each event to `on_event` until it
/// returns `Ok(false)` or the body ends. Returns whether `on_event` asked to
/// stop (as opposed to the body simply ending).
pub(crate) async fn read_sse(
    response: reqwest::Response,
    cancel: &CancellationToken,
    mut on_event: impl FnMut(sse::SseEvent) -> Result<bool, ProviderError>,
) -> Result<bool, ProviderError> {
    let mut parser = sse::SseParser::default();
    let mut body = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
            chunk = body.next() => chunk,
        };
        let (events, ended) = match chunk {
            Some(Ok(bytes)) => (parser.push(&bytes), false),
            Some(Err(err)) => return Err(ProviderError::from_reqwest(err)),
            None => (parser.finish(), true),
        };
        for event in events {
            if !on_event(event)? {
                return Ok(true);
            }
        }
        if ended {
            return Ok(false);
        }
    }
}

/// The shared HTTP client settings: generous read timeout because local
/// models can take minutes to load before the first byte.
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(600))
        .user_agent(concat!("aispice/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

pub(crate) fn trim_base_url(url: impl Into<String>) -> String {
    url.into().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        let e = |s| ProviderError::from_status(s, "m".into(), None);
        assert!(matches!(e(401), ProviderError::Auth { .. }));
        assert!(matches!(e(403), ProviderError::Auth { .. }));
        assert!(matches!(e(429), ProviderError::RateLimited { .. }));
        assert!(matches!(e(529), ProviderError::Overloaded { .. }));
        assert!(matches!(e(503), ProviderError::Overloaded { .. }));
        assert!(matches!(e(400), ProviderError::BadRequest { .. }));
        assert!(matches!(e(404), ProviderError::BadRequest { .. }));
        assert!(e(429).is_retryable());
        assert!(e(500).is_retryable());
        assert!(!e(400).is_retryable());
        assert!(!e(401).is_retryable());
        assert!(ProviderError::Network("reset".into()).is_retryable());
        assert!(!ProviderError::Cancelled.is_retryable());
    }

    #[test]
    fn backoff_grows_and_stays_bounded() {
        let policy = RetryPolicy {
            max_retries: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(1000),
        };
        let err = ProviderError::Overloaded {
            status: 529,
            message: String::new(),
        };
        for attempt in 0..8 {
            let d = policy.delay(attempt, &err);
            let step = (100u64 << attempt).min(1000);
            assert!(d >= Duration::from_millis(step / 2), "{attempt}: {d:?}");
            assert!(d <= Duration::from_millis(step), "{attempt}: {d:?}");
        }
        let limited = ProviderError::RateLimited {
            message: String::new(),
            retry_after: Some(Duration::from_millis(250)),
        };
        assert_eq!(policy.delay(0, &limited), Duration::from_millis(250));
    }

    #[test]
    fn error_message_extraction() {
        assert_eq!(
            error_message(
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad model"}}"#
            ),
            "bad model"
        );
        assert_eq!(error_message(r#"{"error":"nope"}"#), "nope");
        assert_eq!(error_message(r#"{"message":"m"}"#), "m");
        assert_eq!(error_message("plain text"), "plain text");
        assert_eq!(error_message(""), "no details");
    }

    #[tokio::test]
    async fn retrier_respects_limits_and_streaming() {
        let policy = RetryPolicy {
            max_retries: 1,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        };
        let cancel = CancellationToken::new();
        let err = ProviderError::Network("x".into());
        let mut r = Retrier::new(&policy, &cancel);
        assert!(!r.backoff("t", &err, true).await.unwrap(), "streamed");
        assert!(r.backoff("t", &err, false).await.unwrap());
        assert!(!r.backoff("t", &err, false).await.unwrap(), "exhausted");
        let mut r = Retrier::new(&policy, &cancel);
        let long = ProviderError::RateLimited {
            message: String::new(),
            retry_after: Some(Duration::from_secs(60)),
        };
        assert!(!r.backoff("t", &long, false).await.unwrap(), "too long");
        let fatal = ProviderError::BadRequest {
            status: 400,
            message: String::new(),
        };
        assert!(!r.backoff("t", &fatal, false).await.unwrap());
    }

    #[test]
    fn effort_parses_and_serializes() {
        assert_eq!("XHigh".parse::<Effort>().unwrap(), Effort::XHigh);
        assert!("extreme".parse::<Effort>().is_err());
        assert_eq!(serde_json::to_string(&Effort::XHigh).unwrap(), "\"xhigh\"");
    }

    #[test]
    fn usage_adds() {
        let mut u = Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: 4,
        };
        u += u;
        assert_eq!(u.cache_write_tokens, 8);
    }
}
