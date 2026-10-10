use futures::StreamExt;
use tauri::ipc::Channel;

use super::provider::{ChatMessage, StreamEvent};

/// Default base URL for the local Ollama server.
const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Stream a chat completion from a local Ollama instance using NDJSON.
///
/// Sends `StreamEvent` messages through `on_event`:
/// - `event_type: "text"` with content tokens
/// - `event_type: "done"` with the full accumulated text
/// - `event_type: "error"` if anything goes wrong
pub async fn stream_ollama(
    model: &str,
    messages: Vec<ChatMessage>,
    on_event: &Channel<StreamEvent>,
    base_url: Option<&str>,
) -> Result<String, String> {
    let base = base_url.unwrap_or(DEFAULT_OLLAMA_URL);
    let url = format!("{}/api/chat", base.trim_end_matches('/'));

    let client = reqwest::Client::new();

    // Build the messages array in Ollama's expected format
    let ollama_messages: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content,
            })
        })
        .collect();

    let body = serde_json::json!({
        "model": model,
        "messages": ollama_messages,
        "stream": true,
    });

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown error".to_string());
        let msg = format!("Ollama returned {}: {}", status, body_text);
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

        // Process complete NDJSON lines
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(json) => {
                    // Check for errors
                    if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
                        let msg = format!("Ollama error: {}", err);
                        let _ = on_event.send(StreamEvent {
                            event_type: "error".to_string(),
                            data: msg.clone(),
                        });
                        return Err(msg);
                    }

                    // Extract message content
                    if let Some(content) = json
                        .pointer("/message/content")
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

                    // Check if done
                    if json.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                        let _ = on_event.send(StreamEvent {
                            event_type: "done".to_string(),
                            data: accumulated.clone(),
                        });
                        return Ok(accumulated);
                    }
                }
                Err(e) => {
                    // Non-fatal: skip malformed lines
                    eprintln!("Ollama: failed to parse NDJSON line: {}", e);
                }
            }
        }
    }

    // Stream ended without done:true — return what we have
    let _ = on_event.send(StreamEvent {
        event_type: "done".to_string(),
        data: accumulated.clone(),
    });
    Ok(accumulated)
}
