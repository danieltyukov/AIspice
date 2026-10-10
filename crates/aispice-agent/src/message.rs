//! Conversation types shared by every provider.
//!
//! The shape follows the Anthropic Messages API because it is the richest of
//! the wire formats aispice speaks (thinking blocks with signatures, images
//! inside tool results). Each provider converts to and from its own wire
//! format at the edge. Everything here serializes to JSON so a session can be
//! saved and resumed.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(content: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::User,
            content,
        }
    }

    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content,
        }
    }

    pub fn user_text(text: impl Into<String>) -> Self {
        Self::user(vec![ContentBlock::text(text)])
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self::assistant(vec![ContentBlock::text(text)])
    }

    /// All text blocks joined, ignoring thinking and tool blocks.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The tool calls in this message, in order.
    pub fn tool_uses(&self) -> Vec<ToolUse<'_>> {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => Some(ToolUse { id, name, input }),
                _ => None,
            })
            .collect()
    }
}

/// A borrowed view of one `ToolUse` block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolUse<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub input: &'a Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data_base64: String,
    },
    /// A tool call. `input` is the parsed JSON object. When the model produced
    /// arguments that do not parse, providers keep the raw text as a JSON
    /// string so the agent can hand it back as an error instead of guessing.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// The answer to a `ToolUse`. `content` holds only `Text` and `Image`.
    ToolResult {
        tool_use_id: String,
        content: Vec<ContentBlock>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
    /// Model reasoning. Anthropic signs these blocks and requires them back
    /// unchanged in later turns, so the signature is kept verbatim. Providers
    /// that stream plain reasoning text leave it `None`.
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    RedactedThinking {
        data: String,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(media_type: impl Into<String>, data_base64: impl Into<String>) -> Self {
        Self::Image {
            media_type: media_type.into(),
            data_base64: data_base64.into(),
        }
    }

    /// An image from raw bytes, base64 encoded here.
    pub fn image_bytes(media_type: impl Into<String>, bytes: &[u8]) -> Self {
        Self::image(
            media_type,
            base64::engine::general_purpose::STANDARD.encode(bytes),
        )
    }

    pub fn tool_result(
        tool_use_id: impl Into<String>,
        content: Vec<ContentBlock>,
        is_error: bool,
    ) -> Self {
        Self::ToolResult {
            tool_use_id: tool_use_id.into(),
            content,
            is_error,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blocks_serialize_with_a_type_tag() {
        let msg = Message::assistant(vec![
            ContentBlock::Thinking {
                thinking: "check the divider".into(),
                signature: Some("sig".into()),
            },
            ContentBlock::text("Running it."),
            ContentBlock::ToolUse {
                id: "toolu_1".into(),
                name: "simulate".into(),
                input: json!({"circuit": "rc.asc"}),
            },
        ]);
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            value,
            json!({
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "check the divider", "signature": "sig"},
                    {"type": "text", "text": "Running it."},
                    {"type": "tool_use", "id": "toolu_1", "name": "simulate", "input": {"circuit": "rc.asc"}}
                ]
            })
        );
    }

    #[test]
    fn session_round_trips_through_json() {
        let history = vec![
            Message::user(vec![
                ContentBlock::text("What does this do?"),
                ContentBlock::image_bytes("image/png", &[0x89, b'P', b'N', b'G']),
            ]),
            Message::assistant(vec![
                ContentBlock::RedactedThinking {
                    data: "opaque".into(),
                },
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "render".into(),
                    input: json!({}),
                },
            ]),
            Message::user(vec![ContentBlock::tool_result(
                "t1",
                vec![
                    ContentBlock::text("rendered"),
                    ContentBlock::image("image/png", "AAAA"),
                ],
                false,
            )]),
            Message::assistant(vec![ContentBlock::Thinking {
                thinking: "plain".into(),
                signature: None,
            }]),
        ];
        let text = serde_json::to_string(&history).unwrap();
        let back: Vec<Message> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, history);
    }

    #[test]
    fn is_error_defaults_to_false_and_is_omitted() {
        let block = ContentBlock::tool_result("t1", vec![ContentBlock::text("ok")], false);
        let value = serde_json::to_value(&block).unwrap();
        assert!(value.get("is_error").is_none());
        let parsed: ContentBlock = serde_json::from_value(
            json!({"type": "tool_result", "tool_use_id": "t1", "content": []}),
        )
        .unwrap();
        assert_eq!(parsed, ContentBlock::tool_result("t1", vec![], false));
    }

    #[test]
    fn helpers_pick_out_text_and_tool_uses() {
        let msg = Message::assistant(vec![
            ContentBlock::text("a"),
            ContentBlock::ToolUse {
                id: "1".into(),
                name: "x".into(),
                input: json!({"k": 1}),
            },
            ContentBlock::text("b"),
        ]);
        assert_eq!(msg.text(), "ab");
        let uses = msg.tool_uses();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].name, "x");
        assert_eq!(uses[0].input, &json!({"k": 1}));
    }

    #[test]
    fn image_bytes_are_base64_encoded() {
        let block = ContentBlock::image_bytes("image/png", b"hi");
        assert_eq!(block, ContentBlock::image("image/png", "aGk="));
    }
}
