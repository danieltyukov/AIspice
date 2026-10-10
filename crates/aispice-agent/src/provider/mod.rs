//! Model providers behind one streaming interface.
//!
//! Two wire formats cover every provider aispice supports: the Anthropic
//! Messages API, and OpenAI-compatible Chat Completions (OpenAI, Google,
//! OpenRouter, Ollama, LM Studio, vLLM and friends). Both stream, both speak
//! native tool calling, and both convert to and from the canonical types in
//! [`crate::message`].

pub mod anthropic;
pub mod openai;
mod redact;
#[cfg(test)]
mod security_tests;
mod sse;

use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use async_trait::async_trait;
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
    /// The response broke one of the [`HttpLimits`].
    #[error("the provider response was too large: {0}")]
    TooLarge(String),
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

    /// A transport error, without the request URL (a custom endpoint may
    /// carry a key in it) but with the underlying cause, which reqwest's own
    /// message leaves out.
    pub(crate) fn from_reqwest(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            return Self::Network("the request timed out".into());
        }
        let err = err.without_url();
        let mut message = err.to_string();
        let mut source = std::error::Error::source(&err);
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        Self::Network(redact::scrub(&message, &[]))
    }

    pub(crate) fn too_large(what: &str, limit: usize) -> Self {
        Self::TooLarge(format!("{what} exceeded {}", byte_size(limit)))
    }

    /// The same error with `secrets` and anything key-shaped removed from its
    /// message. Every error leaves a provider through here.
    pub(crate) fn redacted(self, secrets: &[&str]) -> Self {
        let clean = |m: String| redact::scrub(&m, secrets);
        match self {
            Self::Auth { status, message } => Self::Auth {
                status,
                message: clean(message),
            },
            Self::RateLimited {
                message,
                retry_after,
            } => Self::RateLimited {
                message: clean(message),
                retry_after,
            },
            Self::Overloaded { status, message } => Self::Overloaded {
                status,
                message: clean(message),
            },
            Self::BadRequest { status, message } => Self::BadRequest {
                status,
                message: clean(message),
            },
            Self::Network(message) => Self::Network(clean(message)),
            Self::Parse(message) => Self::Parse(clean(message)),
            Self::TooLarge(message) => Self::TooLarge(clean(message)),
            Self::Cancelled => Self::Cancelled,
        }
    }
}

fn byte_size(n: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * 1024;
    if n >= MIB && n.is_multiple_of(MIB) {
        format!("{} MiB", n / MIB)
    } else if n >= KIB && n.is_multiple_of(KIB) {
        format!("{} KiB", n / KIB)
    } else {
        format!("{n} bytes")
    }
}

/// Hard limits on everything read from the network, so a broken or hostile
/// server (or a proxy in between) cannot exhaust memory or hang a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpLimits {
    /// Bytes of an error body read for its message; the rest is ignored.
    pub max_error_body: usize,
    /// One line of a server-sent event stream, and one event's data.
    pub max_line: usize,
    /// Everything streamed in one response.
    pub max_stream_bytes: usize,
    /// The JSON arguments of one tool call.
    pub max_tool_input: usize,
    /// Content blocks or tool calls in one response.
    pub max_blocks: usize,
    /// A model list or other non-streamed JSON body.
    pub max_json_body: usize,
    /// Longest wait for response headers or for the next chunk of a body.
    pub idle_timeout: Duration,
    /// Longest a whole chat request may take, streaming included. Long, as
    /// hard turns on capable models can run for many minutes.
    pub request_timeout: Duration,
    /// Longest a model listing may take.
    pub list_timeout: Duration,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_error_body: 64 * 1024,
            max_line: 1024 * 1024,
            max_stream_bytes: 32 * 1024 * 1024,
            max_tool_input: 4 * 1024 * 1024,
            max_blocks: 512,
            max_json_body: 8 * 1024 * 1024,
            idle_timeout: Duration::from_secs(120),
            request_timeout: Duration::from_secs(30 * 60),
            list_timeout: Duration::from_secs(60),
        }
    }
}

/// Bounded retry for transient failures.
///
/// A request is retried only when nothing has been streamed to the caller
/// yet: replaying a half-shown answer would duplicate text in the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    /// Retries after the first attempt, never more than [`MAX_RETRIES`].
    pub max_retries: u32,
    pub base_delay: Duration,
    /// Cap on one backoff step.
    pub max_delay: Duration,
    /// Longest `retry-after` honoured. A server asking for more gets its
    /// error returned at once instead of a long silent wait.
    pub max_retry_after: Duration,
    /// Cap on the total time spent waiting across all retries.
    pub max_total_delay: Duration,
}

/// Ceiling on [`RetryPolicy::max_retries`], whatever the configuration says.
pub const MAX_RETRIES: u32 = 10;

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(1000),
            max_delay: Duration::from_secs(30),
            max_retry_after: Duration::from_secs(60),
            max_total_delay: Duration::from_secs(120),
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
            return after.min(self.max_retry_after);
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
    waited: Duration,
}

impl<'a> Retrier<'a> {
    pub(crate) fn new(policy: &'a RetryPolicy, cancel: &'a CancellationToken) -> Self {
        Self {
            policy,
            cancel,
            attempt: 0,
            waited: Duration::ZERO,
        }
    }

    /// Wait before the next attempt if `err` deserves one. Returns false when
    /// the caller should give up and return `err`. `err` must already be
    /// redacted, since it is logged.
    pub(crate) async fn backoff(
        &mut self,
        provider: &str,
        err: &ProviderError,
        streamed: bool,
    ) -> Result<bool, ProviderError> {
        let max_retries = self.policy.max_retries.min(MAX_RETRIES);
        if streamed || !err.is_retryable() || self.attempt >= max_retries {
            return Ok(false);
        }
        if let Some(after) = err.retry_after()
            && after > self.policy.max_retry_after
        {
            return Ok(false);
        }
        let delay = self.policy.delay(self.attempt, err);
        if self.waited + delay > self.policy.max_total_delay {
            return Ok(false);
        }
        self.waited += delay;
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

/// Send a request, racing it against cancellation and the idle timeout, and
/// turn an HTTP error status into a `ProviderError` with the provider's
/// message. Only the first [`HttpLimits::max_error_body`] bytes of an error
/// body are read.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
    limits: &HttpLimits,
) -> Result<reqwest::Response, ProviderError> {
    let response = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
        r = tokio::time::timeout(limits.idle_timeout, request.send()) => match r {
            Ok(r) => r.map_err(ProviderError::from_reqwest)?,
            Err(_) => return Err(idle_error(limits.idle_timeout)),
        },
    };
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    let retry_after = retry_after(response.headers());
    let body = match read_limited(response, limits.max_error_body, None, limits, cancel).await {
        Ok(body) => body,
        Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
        // The status is what matters; a body that fails to arrive only
        // costs the detail.
        Err(_) => Vec::new(),
    };
    Err(ProviderError::from_status(
        status,
        error_message(&String::from_utf8_lossy(&body)),
        retry_after,
    ))
}

fn idle_error(idle: Duration) -> ProviderError {
    ProviderError::Network(format!(
        "no data from the provider for {} s",
        idle.as_secs()
    ))
}

/// The next chunk of a body, or `None` at its end.
async fn next_chunk(
    response: &mut reqwest::Response,
    limits: &HttpLimits,
    cancel: &CancellationToken,
) -> Result<Option<impl AsRef<[u8]> + use<>>, ProviderError> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ProviderError::Cancelled),
        r = tokio::time::timeout(limits.idle_timeout, response.chunk()) => match r {
            Ok(chunk) => chunk.map_err(ProviderError::from_reqwest),
            Err(_) => Err(idle_error(limits.idle_timeout)),
        },
    }
}

/// Read a whole body, at most `limit` bytes. With `fail_as` set, a longer
/// body is an error naming it; without, it is cut at the limit.
pub(crate) async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
    fail_as: Option<&str>,
    limits: &HttpLimits,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ProviderError> {
    let mut body = Vec::new();
    while let Some(chunk) = next_chunk(&mut response, limits, cancel).await? {
        let chunk = chunk.as_ref();
        if body.len() + chunk.len() > limit {
            if let Some(what) = fail_as {
                return Err(ProviderError::too_large(what, limit));
            }
            body.extend_from_slice(&chunk[..limit - body.len()]);
            break;
        }
        body.extend_from_slice(chunk);
    }
    Ok(body)
}

/// Pull the human-readable message out of an error body. Anthropic, OpenAI
/// and most compatible servers nest it as `{"error": {"message": ...}}`;
/// some put a string in `error` or `message`. The result is scrubbed of
/// key-shaped text and truncated.
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
        Some(message) => redact::scrub(&message, &[]),
        None if body.trim().is_empty() => "no details".to_string(),
        None => redact::scrub(body.trim(), &[]),
    }
}

/// `retry-after-ms` or `retry-after` in seconds. Values that do not fit a
/// `Duration` (`inf`, `1e300`) count as "longer than any cap" instead of
/// panicking; negative values count as zero; garbage is ignored.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    get("retry-after-ms")
        .and_then(|v| parse_seconds(v, 1000.0))
        .or_else(|| get("retry-after").and_then(|v| parse_seconds(v, 1.0)))
}

fn parse_seconds(value: &str, per_second: f64) -> Option<Duration> {
    let n: f64 = value.trim().parse().ok()?;
    if n.is_nan() {
        return None;
    }
    let seconds = (n / per_second).max(0.0);
    Some(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
}

/// Read a server-sent event stream, handing each event to `on_event` until it
/// returns `Ok(false)` or the body ends. Returns whether `on_event` asked to
/// stop (as opposed to the body simply ending). Enforces the line, event,
/// total size and idle limits.
pub(crate) async fn read_sse(
    mut response: reqwest::Response,
    limits: &HttpLimits,
    cancel: &CancellationToken,
    mut on_event: impl FnMut(sse::SseEvent) -> Result<bool, ProviderError>,
) -> Result<bool, ProviderError> {
    let mut parser = sse::SseParser::new(limits.max_line);
    let mut total = 0usize;
    loop {
        let (events, ended) = match next_chunk(&mut response, limits, cancel).await? {
            Some(chunk) => {
                let chunk = chunk.as_ref();
                total += chunk.len();
                if total > limits.max_stream_bytes {
                    return Err(ProviderError::too_large(
                        "the response stream",
                        limits.max_stream_bytes,
                    ));
                }
                (parser.push(chunk)?, false)
            }
            None => (parser.finish()?, true),
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

/// The shared HTTP client. Read timeouts are enforced per request from
/// [`HttpLimits`] so they can be configured.
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("aispice/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

pub(crate) use redact::display_url;

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
            ..RetryPolicy::default()
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
            max_retry_after: Duration::from_millis(5),
            ..RetryPolicy::default()
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

    #[tokio::test]
    async fn retrier_caps_total_wait_and_retry_count() {
        let cancel = CancellationToken::new();
        let busy = ProviderError::Overloaded {
            status: 529,
            message: String::new(),
        };
        // Each step is at least 50 ms; a 120 ms budget allows two.
        let policy = RetryPolicy {
            max_retries: 10,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(100),
            max_total_delay: Duration::from_millis(120),
            ..RetryPolicy::default()
        };
        let mut r = Retrier::new(&policy, &cancel);
        let mut retries = 0;
        while r.backoff("t", &busy, false).await.unwrap() {
            retries += 1;
        }
        assert!((1..=2).contains(&retries), "{retries}");

        let greedy = RetryPolicy {
            max_retries: 1000,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            ..RetryPolicy::default()
        };
        let mut r = Retrier::new(&greedy, &cancel);
        let mut retries = 0;
        while r.backoff("t", &busy, false).await.unwrap() {
            retries += 1;
        }
        assert_eq!(retries, MAX_RETRIES);
    }

    #[test]
    fn hostile_retry_after_values_do_not_panic() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let parse = |name: &'static str, value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_str(value).unwrap());
            retry_after(&headers)
        };
        assert_eq!(parse("retry-after", "inf"), Some(Duration::MAX));
        assert_eq!(parse("retry-after", "1e300"), Some(Duration::MAX));
        assert_eq!(
            parse("retry-after", "99999999999999999999"),
            Some(Duration::MAX)
        );
        assert_eq!(parse("retry-after", "-5"), Some(Duration::ZERO));
        assert_eq!(parse("retry-after", "NaN"), None);
        assert_eq!(parse("retry-after", "soon"), None);
        assert_eq!(parse("retry-after", "2"), Some(Duration::from_secs(2)));
        assert_eq!(
            parse("retry-after-ms", "1500"),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(parse("retry-after-ms", "-inf"), Some(Duration::ZERO));
        // A huge value is longer than any cap, so the retrier gives up.
        let policy = RetryPolicy::default();
        let err = ProviderError::RateLimited {
            message: String::new(),
            retry_after: Some(Duration::MAX),
        };
        assert_eq!(policy.delay(0, &err), policy.max_retry_after);
    }

    #[test]
    fn redacted_errors_hide_secrets_everywhere() {
        let key = "sk-ant-api03-FAKEKEY1234567890";
        let errors = [
            ProviderError::Auth {
                status: 401,
                message: format!("invalid key {key}"),
            },
            ProviderError::RateLimited {
                message: format!("slow down {key}"),
                retry_after: None,
            },
            ProviderError::Overloaded {
                status: 500,
                message: format!("Bearer {key}"),
            },
            ProviderError::BadRequest {
                status: 400,
                message: key.to_string(),
            },
            ProviderError::Network(format!("http://u:{key}@host/")),
            ProviderError::Parse(format!("bad event {{\"k\":\"{key}\"}}")),
            ProviderError::TooLarge(key.to_string()),
        ];
        for err in errors {
            let err = err.redacted(&[key]);
            assert!(!format!("{err}").contains("FAKEKEY"), "{err}");
            assert!(!format!("{err:?}").contains("FAKEKEY"), "{err:?}");
        }
    }

    #[test]
    fn byte_sizes_read_naturally() {
        assert_eq!(byte_size(64 * 1024), "64 KiB");
        assert_eq!(byte_size(32 * 1024 * 1024), "32 MiB");
        assert_eq!(byte_size(1000), "1000 bytes");
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
