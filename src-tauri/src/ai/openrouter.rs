use futures::StreamExt;
use tauri::ipc::Channel;

use super::provider::{ChatMessage, StreamEvent};

/// Stream a chat completion from OpenRouter using SSE.
///
/// This is a convenience wrapper around `stream_openai_compatible` that uses
/// the OpenRouter endpoint and bearer auth.
#[allow(dead_code)]
pub async fn stream_openrouter(
    api_key: &str,
    model: &str,
    messages: Vec<ChatMessage>,
    on_event: &Channel<StreamEvent>,
) -> Result<String, String> {
    stream_openai_compatible(
        api_key,
        model,
        messages,
        on_event,
        "https://openrouter.ai/api/v1/chat/completions",
        "bearer",
    )
    .await
}

/// Stream a chat completion from any OpenAI-compatible endpoint.
///
/// `auth_style`:
/// - `"bearer"` — sends `Authorization: Bearer <key>` (OpenAI, Google, OpenRouter)
/// - `"anthropic"` — sends `x-api-key: <key>` + `anthropic-version` header
/// - `"none"` — no auth header
///
/// For Anthropic, the request body is adapted to the Messages API format and
/// the SSE response is parsed accordingly.
pub async fn stream_openai_compatible(
    api_key: &str,
    model: &str,
    messages: Vec<ChatMessage>,
    on_event: &Channel<StreamEvent>,
    base_url: &str,
    auth_style: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    // Build request differently for Anthropic vs OpenAI-compatible
    let is_anthropic = auth_style == "anthropic";

    let body = if is_anthropic {
        // Anthropic Messages API format: system is separate, messages are user/assistant only
        let system_text = messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let api_messages: Vec<serde_json::Value> = messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": model,
            "messages": api_messages,
            "max_tokens": 4096,
            "stream": true,
        });

        if !system_text.is_empty() {
            body["system"] = serde_json::Value::String(system_text);
        }

        body
    } else {
        serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": true,
        })
    };

    let mut request = client
        .post(base_url)
        .header("Content-Type", "application/json");

    match auth_style {
        "bearer" => {
            request = request.header("Authorization", format!("Bearer {}", api_key));
        }
        "anthropic" => {
            request = request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }
        _ => {}
    }

    // Extra headers for OpenRouter
    if base_url.contains("openrouter.ai") {
        request = request
            .header("HTTP-Referer", "https://aispice.app")
            .header("X-Title", "AIspice");
    }

    let response = request
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown error".to_string());
        let msg = format!("API returned {}: {}", status, body_text);
        let _ = on_event.send(StreamEvent {
            event_type: "error".to_string(),
            data: msg.clone(),
        });
        return Err(msg);
    }

    let mut stream = response.bytes_stream();
    let mut accumulated = String::new();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("Stream read error: {}", e);
                let _ = on_event.send(StreamEvent {
                    event_type: "error".to_string(),
                    data: msg.clone(),
                });
                return Err(msg);
            }
        };

        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete SSE lines
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim();

                if data == "[DONE]" {
                    let _ = on_event.send(StreamEvent {
                        event_type: "done".to_string(),
                        data: accumulated.clone(),
                    });
                    return Ok(accumulated);
                }

                // Parse the SSE JSON payload
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    if is_anthropic {
                        // Anthropic SSE format
                        parse_anthropic_sse(&json, &mut accumulated, on_event);
                    } else {
                        // OpenAI-compatible SSE format
                        parse_openai_sse(&json, &mut accumulated, on_event);
                    }
                }
            }

            // Anthropic also sends "event:" lines; handle the message_stop event
            if is_anthropic && line.starts_with("event: message_stop") {
                let _ = on_event.send(StreamEvent {
                    event_type: "done".to_string(),
                    data: accumulated.clone(),
                });
                return Ok(accumulated);
            }
        }
    }

    // Stream ended without [DONE] — return what we have
    let _ = on_event.send(StreamEvent {
        event_type: "done".to_string(),
        data: accumulated.clone(),
    });
    Ok(accumulated)
}

/// Parse an OpenAI-compatible SSE JSON chunk.
fn parse_openai_sse(
    json: &serde_json::Value,
    accumulated: &mut String,
    on_event: &Channel<StreamEvent>,
) {
    // Check for reasoning/thinking content
    if let Some(reasoning) = json
        .pointer("/choices/0/delta/reasoning")
        .and_then(|v| v.as_str())
    {
        if !reasoning.is_empty() {
            let _ = on_event.send(StreamEvent {
                event_type: "thinking".to_string(),
                data: reasoning.to_string(),
            });
        }
    }

    // Check for regular content
    if let Some(content) = json
        .pointer("/choices/0/delta/content")
        .and_then(|v| v.as_str())
    {
        if !content.is_empty() {
            accumulated.push_str(content);
            let _ = on_event.send(StreamEvent {
                event_type: "text".to_string(),
                data: content.to_string(),
            });
        }
    }

    // Check for errors in the stream payload
    if let Some(err) = json.get("error") {
        let _ = on_event.send(StreamEvent {
            event_type: "error".to_string(),
            data: format!("Stream error: {}", err),
        });
    }
}

/// Parse an Anthropic Messages API SSE JSON chunk.
fn parse_anthropic_sse(
    json: &serde_json::Value,
    accumulated: &mut String,
    on_event: &Channel<StreamEvent>,
) {
    // Anthropic sends different event types as JSON:
    // {"type":"content_block_delta","delta":{"type":"text_delta","text":"..."}}
    // {"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"..."}}
    let event_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "content_block_delta" => {
            if let Some(delta) = json.get("delta") {
                let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match delta_type {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                accumulated.push_str(text);
                                let _ = on_event.send(StreamEvent {
                                    event_type: "text".to_string(),
                                    data: text.to_string(),
                                });
                            }
                        }
                    }
                    "thinking_delta" => {
                        if let Some(thinking) = delta.get("thinking").and_then(|v| v.as_str()) {
                            if !thinking.is_empty() {
                                let _ = on_event.send(StreamEvent {
                                    event_type: "thinking".to_string(),
                                    data: thinking.to_string(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        "error" => {
            let msg = json
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown Anthropic error");
            let _ = on_event.send(StreamEvent {
                event_type: "error".to_string(),
                data: msg.to_string(),
            });
        }
        _ => {}
    }
}
