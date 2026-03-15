use std::path::Path;
use tauri::ipc::Channel;
use tauri::State;

use crate::ai::ollama;
use crate::ai::openrouter;
use crate::ai::prompt::build_system_prompt;
use crate::ai::provider::{ChatMessage, StreamEvent};
use crate::formats::encoding;
use crate::state::AppState;

/// Read a file from the working directory, returning its text content with line
/// numbers prepended (e.g. "  1 | Version 4").
fn read_file_with_line_numbers(working_dir: &str, filename: &str) -> Result<String, String> {
    let path = Path::new(working_dir).join(filename);
    let bytes =
        std::fs::read(&path).map_err(|e| format!("Failed to read {}: {}", filename, e))?;
    let (text, _encoding) = encoding::detect_and_decode(&bytes);

    let numbered: String = text
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{:>4} | {}", i + 1, line))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(numbered)
}

/// Determine which provider to use and extract the appropriate API key.
/// Returns (base_url, api_key, auth_style) where auth_style is "bearer" or "anthropic".
fn resolve_provider(
    provider: &str,
    model: &str,
    api_keys: &std::collections::HashMap<String, String>,
) -> Result<(String, String, String), String> {
    match provider {
        "openai" => {
            let key = api_keys
                .get("openai")
                .filter(|k| !k.is_empty())
                .ok_or_else(|| "OpenAI API key not configured. Add it in Settings.".to_string())?;
            Ok((
                "https://api.openai.com/v1/chat/completions".to_string(),
                key.clone(),
                "bearer".to_string(),
            ))
        }
        "anthropic" => {
            let key = api_keys
                .get("anthropic")
                .filter(|k| !k.is_empty())
                .ok_or_else(|| {
                    "Anthropic API key not configured. Add it in Settings.".to_string()
                })?;
            Ok((
                "https://api.anthropic.com/v1/messages".to_string(),
                key.clone(),
                "anthropic".to_string(),
            ))
        }
        "google" => {
            let key = api_keys
                .get("google")
                .filter(|k| !k.is_empty())
                .ok_or_else(|| {
                    "Google API key not configured. Add it in Settings.".to_string()
                })?;
            Ok((
                "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
                    .to_string(),
                key.clone(),
                "bearer".to_string(),
            ))
        }
        "openrouter" => {
            let key = api_keys
                .get("openrouter")
                .filter(|k| !k.is_empty())
                .ok_or_else(|| {
                    "OpenRouter API key not configured. Add it in Settings.".to_string()
                })?;
            Ok((
                "https://openrouter.ai/api/v1/chat/completions".to_string(),
                key.clone(),
                "bearer".to_string(),
            ))
        }
        "ollama" => {
            // No key needed
            Ok((String::new(), String::new(), "none".to_string()))
        }
        _ => {
            // Auto-detect provider from model name
            if model.starts_with("gpt-") || model.starts_with("o1-") || model.starts_with("o3-") {
                return resolve_provider("openai", model, api_keys);
            }
            if model.starts_with("claude-") {
                return resolve_provider("anthropic", model, api_keys);
            }
            if model.starts_with("gemini-") {
                return resolve_provider("google", model, api_keys);
            }
            // Default to openrouter
            resolve_provider("openrouter", model, api_keys)
        }
    }
}

/// Stream a chat message to the AI provider and forward events to the frontend.
///
/// The `on_event` channel receives `StreamEvent` objects with:
/// - `event_type: "thinking"` -- reasoning tokens (OpenRouter only)
/// - `event_type: "text"` -- content tokens
/// - `event_type: "done"` -- streaming complete, data = full text
/// - `event_type: "error"` -- an error occurred
#[tauri::command]
pub async fn send_chat_message_stream(
    message: String,
    active_file: String,
    history: Vec<ChatMessage>,
    model: String,
    provider: String,
    on_event: Channel<StreamEvent>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Clone state values immediately so we don't hold locks across awaits
    let api_keys = {
        let guard = state
            .api_keys
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        guard.clone()
    };

    let working_dir = {
        let guard = state
            .working_directory
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        guard.clone().unwrap_or_default()
    };

    // Read the active file content (if a file is selected)
    let file_context = if !active_file.is_empty() && !working_dir.is_empty() {
        match read_file_with_line_numbers(&working_dir, &active_file) {
            Ok(content) => format!(
                "\n\n--- Active file: {} ---\n{}\n--- End of file ---",
                active_file, content
            ),
            Err(e) => format!("\n\n[Could not read {}: {}]", active_file, e),
        }
    } else {
        String::new()
    };

    // Build the full message with file context
    let user_content = if file_context.is_empty() {
        message
    } else {
        format!("{}{}", message, file_context)
    };

    // Build conversation messages
    let system_prompt = build_system_prompt();
    let mut messages: Vec<ChatMessage> = Vec::with_capacity(history.len() + 2);

    messages.push(ChatMessage {
        role: "system".to_string(),
        content: system_prompt,
    });

    // Append conversation history
    for msg in &history {
        messages.push(msg.clone());
    }

    // Append the new user message
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_content,
    });

    // Resolve provider configuration
    let (base_url, api_key, auth_style) = match resolve_provider(&provider, &model, &api_keys) {
        Ok(v) => v,
        Err(msg) => {
            let _ = on_event.send(StreamEvent {
                event_type: "error".to_string(),
                data: msg.clone(),
            });
            return Err(msg);
        }
    };

    // Route to the appropriate provider
    let result = match provider.as_str() {
        "ollama" => ollama::stream_ollama(&model, messages, &on_event, None).await,
        "anthropic" => {
            // Anthropic has a different API format - use dedicated streaming
            openrouter::stream_openai_compatible(
                &api_key,
                &model,
                messages,
                &on_event,
                &base_url,
                &auth_style,
            )
            .await
        }
        _ => {
            // OpenAI-compatible providers (openai, google, openrouter)
            openrouter::stream_openai_compatible(
                &api_key,
                &model,
                messages,
                &on_event,
                &base_url,
                &auth_style,
            )
            .await
        }
    };

    if let Err(e) = result {
        return Err(e);
    }

    Ok(())
}
