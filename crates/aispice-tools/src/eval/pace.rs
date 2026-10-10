//! Patience with rate limits during a benchmark.
//!
//! The providers already retry a 429 a few times within about two minutes,
//! which suits an interactive chat. A suite run on a free tier (a handful of
//! requests per minute) needs more: requests spaced out in advance, and a
//! rate-limited request retried for several minutes before the run counts it
//! as an error.

use aispice_agent::provider::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamEvent,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// A provider wrapper that spaces requests and waits out rate limits.
pub struct Patient {
    inner: Arc<dyn Provider>,
    /// Least time between the starts of two requests (60 s / requests per
    /// minute). Zero sends at once.
    min_interval: Duration,
    /// Shortest and longest wait after a 429, when the provider gives no
    /// usable hint.
    pub min_backoff: Duration,
    pub max_backoff: Duration,
    /// Total waiting allowed for one request before giving up.
    pub max_wait: Duration,
    last: Mutex<Option<Instant>>,
}

impl Patient {
    pub fn new(inner: Arc<dyn Provider>) -> Self {
        Self {
            inner,
            min_interval: Duration::ZERO,
            min_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(90),
            max_wait: Duration::from_secs(600),
            last: Mutex::new(None),
        }
    }

    /// Send at most `rpm` requests per minute.
    pub fn with_rpm(mut self, rpm: Option<f64>) -> Self {
        self.min_interval = match rpm {
            Some(r) if r > 0.0 => Duration::from_secs_f64(60.0 / r),
            _ => Duration::ZERO,
        };
        self
    }

    async fn pace(&self, cancel: &CancellationToken) -> Result<(), ProviderError> {
        if self.min_interval.is_zero() {
            return Ok(());
        }
        let mut last = self.last.lock().await;
        if let Some(t) = *last {
            let ready = t + self.min_interval;
            tokio::select! {
                _ = tokio::time::sleep_until(ready) => {}
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
            }
        }
        *last = Some(Instant::now());
        Ok(())
    }
}

/// A wait the provider suggests in its message, such as Google's
/// `Please retry in 38.2s` or `"retryDelay": "38s"`.
pub fn hinted_delay(message: &str) -> Option<Duration> {
    let lower = message.to_ascii_lowercase();
    for key in [
        "retry in ",
        "retrydelay\": \"",
        "retrydelay\":\"",
        "try again in ",
    ] {
        if let Some(at) = lower.find(key) {
            let rest = &lower[at + key.len()..];
            let digits: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let unit = rest[digits.len()..].trim_start();
            if let Ok(v) = digits.parse::<f64>()
                && v.is_finite()
                && v >= 0.0
            {
                let secs = if unit.starts_with("ms") {
                    v / 1000.0
                } else {
                    v
                };
                return Some(Duration::from_secs_f64(secs.min(3600.0)));
            }
        }
    }
    None
}

#[async_trait]
impl Provider for Patient {
    fn id(&self) -> &str {
        self.inner.id()
    }

    async fn stream(
        &self,
        req: &ChatRequest,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, ProviderError> {
        let mut waited = Duration::ZERO;
        let mut attempt: u32 = 0;
        loop {
            self.pace(cancel).await?;
            let mut streamed = false;
            let result = {
                let mut tracked = |e: StreamEvent| {
                    streamed = true;
                    sink(e);
                };
                self.inner.stream(req, &mut tracked, cancel).await
            };
            let err = match result {
                Err(err @ ProviderError::RateLimited { .. }) if !streamed => err,
                other => return other,
            };
            let ProviderError::RateLimited {
                message,
                retry_after,
            } = &err
            else {
                unreachable!("matched above")
            };
            let backoff = self
                .min_backoff
                .saturating_mul(1u32 << attempt.min(8))
                .min(self.max_backoff);
            let delay = retry_after
                .or_else(|| hinted_delay(message))
                .map(|d| d.max(self.min_backoff))
                .unwrap_or(backoff)
                .min(self.max_backoff);
            if waited + delay > self.max_wait {
                return Err(err);
            }
            tracing::warn!(
                provider = self.inner.id(),
                attempt = attempt + 1,
                delay_s = delay.as_secs_f64(),
                "rate limited during an eval run; waiting"
            );
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
            }
            waited += delay;
            attempt += 1;
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.inner.list_models().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hinted_delays() {
        assert_eq!(
            hinted_delay("Quota exceeded. Please retry in 38.5s."),
            Some(Duration::from_secs_f64(38.5))
        );
        assert_eq!(
            hinted_delay(r#"{"@type": "RetryInfo", "retryDelay": "21s"}"#),
            Some(Duration::from_secs(21))
        );
        assert_eq!(
            hinted_delay("Rate limit reached. Please try again in 250ms."),
            Some(Duration::from_millis(250))
        );
        assert_eq!(hinted_delay("slow down"), None);
    }
}
