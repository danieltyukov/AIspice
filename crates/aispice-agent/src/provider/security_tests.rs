//! Credential and resource-limit tests across both providers.
//!
//! Every key here contains `LEAKCANARY`, and every error path is checked for
//! it in both `Display` and `Debug`. The resource tests use hostile servers:
//! oversized lines, streams that never end, bodies that never end, and
//! nonsense `retry-after` values.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

use super::*;
use crate::message::Message;

const CANARY: &str = "LEAKCANARY";
const ANTHROPIC_KEY: &str = "sk-ant-api03-LEAKCANARY-Zm9vYmFyYmF6cXV4";
/// Not shaped like any known key, so only the configured-secret pass can
/// catch it.
const GATEWAY_KEY: &str = "gateway-LEAKCANARY-secret-77";
const GATEWAY_HEADER: &str = "hdr-LEAKCANARY-9999";

fn assert_clean(err: &ProviderError) {
    let shown = format!("{err} | {err:?}");
    assert!(!shown.contains(CANARY), "key leaked: {shown}");
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "test-model".into(),
        system: String::new(),
        messages: vec![Message::user_text("hi")],
        tools: vec![],
        max_tokens: 1024,
        temperature: None,
        thinking: None,
    }
}

async fn stream(p: &dyn Provider) -> Result<ChatResponse, ProviderError> {
    p.stream(&request(), &mut |_| {}, &CancellationToken::new())
        .await
}

fn anthropic(base: &str) -> AnthropicProvider {
    AnthropicProvider::new(ANTHROPIC_KEY)
        .with_base_url(base)
        .with_retry(RetryPolicy::none())
}

fn gateway(base: &str) -> OpenAiProvider {
    OpenAiProvider::custom(base, Some(GATEWAY_KEY.into()))
        .with_header("X-Gateway-Token", GATEWAY_HEADER)
        .with_retry(RetryPolicy::none())
}

/// Responses that quote the credentials back, as some servers do.
fn echoing_responses(secrets: &str) -> Vec<ResponseTemplate> {
    vec![
        ResponseTemplate::new(401).set_body_json(json!({
            "error": {"type": "authentication_error", "message": format!("invalid key: {secrets}")}
        })),
        ResponseTemplate::new(400).set_body_string(format!(
            "rejected Authorization: Bearer {secrets} for {secrets}"
        )),
        ResponseTemplate::new(429)
            .insert_header("retry-after", "0")
            .set_body_json(json!({"error": {"message": format!("slow down {secrets}")}})),
        ResponseTemplate::new(503).set_body_string(format!("upstream saw {secrets}")),
        ResponseTemplate::new(200).set_body_raw(
            format!(
                "event: error\ndata: {}\n\n",
                json!({"type": "error", "error": {"type": "overloaded_error", "message": secrets}})
            ),
            "text/event-stream",
        ),
        ResponseTemplate::new(200).set_body_raw(
            format!("data: {{\"error\": {{\"code\": 502, \"message\": \"{secrets}\"}}}}\n\n"),
            "text/event-stream",
        ),
        ResponseTemplate::new(200).set_body_raw(
            format!("data: {{\"type\": \"broken {secrets}\n\n"),
            "text/event-stream",
        ),
    ]
}

#[tokio::test]
async fn anthropic_errors_never_show_the_key() {
    for response in echoing_responses(ANTHROPIC_KEY) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(response.clone())
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(response)
            .mount(&server)
            .await;
        let p = anthropic(&server.uri());
        let err = stream(&p).await.unwrap_err();
        assert_clean(&err);
        if let Err(err) = p.list_models().await {
            assert_clean(&err);
        }
    }
}

#[tokio::test]
async fn openai_compatible_errors_never_show_the_key_or_credential_headers() {
    let secrets = format!("{GATEWAY_KEY} {GATEWAY_HEADER}");
    for response in echoing_responses(&secrets) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(response.clone())
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(response)
            .mount(&server)
            .await;
        let p = gateway(&format!("{}/v1", server.uri()));
        let err = stream(&p).await.unwrap_err();
        assert_clean(&err);
        let err = p.list_models().await.unwrap_err();
        assert_clean(&err);
    }
}

#[tokio::test]
async fn network_errors_drop_credentials_in_the_url() {
    // A port that was just free, so the connection is refused.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let base =
        format!("http://user:{GATEWAY_KEY}@127.0.0.1:{port}/v1?key={CANARY}0123456789abcdef");
    let p = gateway(&base);
    let err = stream(&p).await.unwrap_err();
    assert!(matches!(err, ProviderError::Network(_)), "{err:?}");
    assert_clean(&err);
    assert_clean(&p.list_models().await.unwrap_err());
    let a = anthropic(&base);
    assert_clean(&stream(&a).await.unwrap_err());
}

#[test]
fn debug_output_hides_keys_headers_and_url_credentials() {
    let base = format!("https://user:{GATEWAY_KEY}@proxy.example/v1?key={CANARY}");
    let shown = format!(
        "{:?} {:?} {:?}",
        gateway(&base),
        anthropic(&base),
        OpenAiProvider::openrouter(GATEWAY_KEY)
    );
    assert!(!shown.contains(CANARY), "{shown}");
    assert!(shown.contains("proxy.example"));
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One process-wide subscriber capturing every log line from every test. A
/// scoped subscriber would race with other test threads over tracing's
/// per-callsite interest cache, and a global one makes the check stronger:
/// no test may log a canary.
fn captured_logs() -> &'static Captured {
    static LOGS: std::sync::OnceLock<Captured> = std::sync::OnceLock::new();
    LOGS.get_or_init(|| {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other test installs a global subscriber");
        captured
    })
}

#[tokio::test]
async fn retry_logs_never_show_the_key() {
    let captured = captured_logs();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(format!("boom {GATEWAY_KEY}")))
        .mount(&server)
        .await;
    let p = gateway(&format!("{}/v1", server.uri())).with_retry(RetryPolicy {
        max_retries: 2,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
        ..RetryPolicy::default()
    });
    assert_clean(&stream(&p).await.unwrap_err());
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("retrying model request"), "{logs}");
    assert!(!logs.contains(CANARY), "{logs}");
}

// ---- resource limits ----

fn small_limits() -> HttpLimits {
    HttpLimits {
        max_line: 16 * 1024,
        max_stream_bytes: 64 * 1024,
        max_tool_input: 1024,
        max_blocks: 4,
        max_json_body: 4 * 1024,
        ..HttpLimits::default()
    }
}

async fn sse_server(body: String) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    server
}

fn assert_too_large(result: Result<ChatResponse, ProviderError>) {
    match result {
        Err(ProviderError::TooLarge(message)) => {
            assert!(message.contains("exceeded") || message.contains("more than"))
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn an_oversized_line_is_rejected() {
    // 32 KiB: over the 16 KiB line limit, under the 64 KiB stream limit.
    let line = "x".repeat(32 * 1024);
    let server = sse_server(format!("event: content_block_delta\ndata: {line}\n\n")).await;
    let p = anthropic(&server.uri()).with_limits(small_limits());
    match stream(&p).await {
        Err(ProviderError::TooLarge(message)) => assert!(message.contains("line"), "{message}"),
        other => panic!("{other:?}"),
    }

    let server = sse_server(format!("data: {line}\n\n")).await;
    let p = gateway(&format!("{}/v1", server.uri())).with_limits(small_limits());
    assert_too_large(stream(&p).await);
}

#[tokio::test]
async fn a_stream_that_never_finishes_hits_the_byte_cap() {
    let mut body = String::from(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
    );
    while body.len() < 256 * 1024 {
        body.push_str("event: ping\ndata: {\"type\": \"ping\"}\n\n");
    }
    let server = sse_server(body).await;
    let p = anthropic(&server.uri()).with_limits(small_limits());
    assert_too_large(stream(&p).await);

    let mut body = String::new();
    while body.len() < 256 * 1024 {
        body.push_str("data: {\"choices\":[]}\n\n");
    }
    let server = sse_server(body).await;
    let p = gateway(&format!("{}/v1", server.uri())).with_limits(small_limits());
    assert_too_large(stream(&p).await);
}

#[tokio::test]
async fn oversized_tool_input_is_rejected() {
    let big = "a".repeat(2048);
    let body = format!(
        "event: content_block_start\ndata: {}\n\nevent: content_block_delta\ndata: {}\n\n",
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "t", "name": "x", "input": {}}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": big}}),
    );
    let server = sse_server(body).await;
    let p = anthropic(&server.uri()).with_limits(small_limits());
    assert_too_large(stream(&p).await);

    let body = format!(
        "data: {}\n\n",
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "c", "function": {"name": "x", "arguments": big}}]}}]})
    );
    let server = sse_server(body).await;
    let p = gateway(&format!("{}/v1", server.uri())).with_limits(small_limits());
    assert_too_large(stream(&p).await);
}

#[tokio::test]
async fn too_many_blocks_or_tool_calls_are_rejected() {
    let mut body = String::new();
    for i in 0..10 {
        body.push_str(&format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({"type": "content_block_start", "index": i, "content_block": {"type": "text", "text": ""}})
        ));
    }
    let server = sse_server(body).await;
    let p = anthropic(&server.uri()).with_limits(small_limits());
    assert_too_large(stream(&p).await);

    let mut body = String::new();
    for i in 0..10 {
        body.push_str(&format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": i, "id": format!("c{i}"), "function": {"name": "x", "arguments": "{}"}}]}}]})
        ));
    }
    let server = sse_server(body).await;
    let p = gateway(&format!("{}/v1", server.uri())).with_limits(small_limits());
    assert_too_large(stream(&p).await);
}

#[tokio::test]
async fn model_lists_are_size_capped() {
    let server = MockServer::start().await;
    let models: Vec<_> = (0..500)
        .map(|i| json!({"id": format!("model-{i}")}))
        .collect();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": models})))
        .mount(&server)
        .await;
    let p = gateway(&format!("{}/v1", server.uri())).with_limits(small_limits());
    assert!(matches!(
        p.list_models().await,
        Err(ProviderError::TooLarge(_))
    ));
    let a = anthropic(&server.uri()).with_limits(small_limits());
    assert!(matches!(
        a.list_models().await,
        Err(ProviderError::TooLarge(_))
    ));
}

/// A model list that always claims another page.
struct EndlessPages(AtomicUsize);

impl Respond for EndlessPages {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let n = self.0.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": format!("m{n}")}], "has_more": true, "last_id": format!("m{n}")
        }))
    }
}

#[tokio::test]
async fn endless_model_pagination_stops() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(EndlessPages(AtomicUsize::new(0)))
        .mount(&server)
        .await;
    let models = anthropic(&server.uri()).list_models().await.unwrap();
    assert_eq!(models.len(), 20);
    assert_eq!(server.received_requests().await.unwrap().len(), 20);
}

// ---- hostile raw servers ----

/// Serve each connection with `handler` on its own thread.
fn raw_server(handler: fn(TcpStream)) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || handler(stream));
        }
    });
    format!("http://{addr}")
}

/// Consume the request headers and body so the client is not blocked.
fn read_request(stream: &mut TcpStream) {
    let mut seen = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        seen.extend_from_slice(&buf[..n]);
        if let Some(end) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&seen[..end]).to_ascii_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if seen.len() >= end + 4 + len {
                return;
            }
        }
    }
}

fn chunk(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

const SSE_HEAD: &[u8] =
    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
const PING: &[u8] = b"event: ping\ndata: {\"type\": \"ping\"}\n\n";

fn stall(mut stream: TcpStream) {
    read_request(&mut stream);
    let _ = stream.write_all(SSE_HEAD);
    let _ = stream.write_all(&chunk(PING));
    let _ = stream.flush();
    std::thread::sleep(Duration::from_secs(30));
}

fn trickle(mut stream: TcpStream) {
    read_request(&mut stream);
    let _ = stream.write_all(SSE_HEAD);
    while stream.write_all(&chunk(PING)).is_ok() {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn endless_error_body(mut stream: TcpStream) {
    read_request(&mut stream);
    let head = b"HTTP/1.1 500 Internal Server Error\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\n\r\n";
    if stream.write_all(head).is_err() {
        return;
    }
    let block = chunk(&[b'x'; 16 * 1024]);
    while stream.write_all(&block).is_ok() {}
}

#[tokio::test]
async fn a_stalled_stream_hits_the_idle_timeout() {
    let base = raw_server(stall);
    let limits = HttpLimits {
        idle_timeout: Duration::from_millis(200),
        ..HttpLimits::default()
    };
    let started = Instant::now();
    let err = stream(&anthropic(&base).with_limits(limits.clone()))
        .await
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    match err {
        ProviderError::Network(message) => assert!(message.contains("no data"), "{message}"),
        other => panic!("{other:?}"),
    }
    let err = stream(&gateway(&format!("{base}/v1")).with_limits(limits))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Network(_)), "{err:?}");
}

#[tokio::test]
async fn a_stream_that_trickles_forever_hits_the_request_timeout() {
    let base = raw_server(trickle);
    let limits = HttpLimits {
        request_timeout: Duration::from_millis(300),
        ..HttpLimits::default()
    };
    let started = Instant::now();
    let err = stream(&anthropic(&base).with_limits(limits))
        .await
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    match err {
        ProviderError::Network(message) => assert!(message.contains("timed out"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_endless_error_body_is_read_only_up_to_the_cap() {
    let base = raw_server(endless_error_body);
    let started = Instant::now();
    let err = stream(&gateway(&format!("{base}/v1"))).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    match &err {
        ProviderError::Overloaded {
            status: 500,
            message,
        } => {
            assert!(message.chars().count() < 1100, "{}", message.len());
            assert!(message.ends_with("(truncated)"));
        }
        other => panic!("{other:?}"),
    }
}

// ---- hostile retry-after ----

async fn rate_limited_then_ok(retry_after: &str, times: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", retry_after)
                .set_body_json(json!({"error": {"message": "rate limited"}})),
        )
        .up_to_n_times(times)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    server
}

fn patient_retry() -> RetryPolicy {
    RetryPolicy {
        max_retries: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
        ..RetryPolicy::default()
    }
}

#[tokio::test]
async fn absurd_retry_after_values_give_up_at_once() {
    for value in ["1e300", "inf", "99999999999999999999", "3600"] {
        let server = rate_limited_then_ok(value, u64::MAX).await;
        let p = gateway(&format!("{}/v1", server.uri())).with_retry(patient_retry());
        let started = Instant::now();
        let err = stream(&p).await.unwrap_err();
        assert!(
            matches!(err, ProviderError::RateLimited { .. }),
            "{value}: {err:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2), "{value}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "{value}"
        );
    }
}

#[tokio::test]
async fn negative_or_garbage_retry_after_still_retries_promptly() {
    for value in ["-1", "NaN", "soon"] {
        let server = rate_limited_then_ok(value, 1).await;
        let p = gateway(&format!("{}/v1", server.uri())).with_retry(patient_retry());
        let started = Instant::now();
        let response = stream(&p).await.unwrap();
        assert_eq!(response.message.text(), "ok", "{value}");
        assert!(started.elapsed() < Duration::from_secs(2), "{value}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "{value}"
        );
    }
}

#[tokio::test]
async fn total_retry_time_is_bounded() {
    let server = rate_limited_then_ok("30", u64::MAX).await;
    let p = gateway(&format!("{}/v1", server.uri())).with_retry(RetryPolicy {
        max_total_delay: Duration::from_secs(1),
        ..patient_retry()
    });
    let started = Instant::now();
    assert!(stream(&p).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
