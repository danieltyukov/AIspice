//! Test doubles for driving the agent loop without a network.
//!
//! Compiled for this crate's own tests and, with the `testing` feature, for
//! other crates: a tool crate can script a model that calls its tools and
//! check what the loop sends back.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::message::{ContentBlock, Message};
use crate::provider::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StopReason, StreamEvent, Usage,
};
use crate::tool::{Tool, ToolContext, ToolOutput, ToolSpec};

/// One scripted model call.
#[derive(Debug)]
pub enum Step {
    /// Stream this reply (its text and thinking become deltas) and return it.
    Reply(ChatResponse),
    Fail(ProviderError),
    /// Never answer; return `Cancelled` once the run is cancelled.
    Hang,
}

/// A provider that answers each call with the next scripted step and
/// records every request it receives.
#[derive(Debug)]
pub struct ScriptedProvider {
    id: String,
    script: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<ChatRequest>>,
    models: Vec<ModelInfo>,
}

impl Default for ScriptedProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedProvider {
    pub fn new() -> Self {
        Self {
            id: "scripted".into(),
            script: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
            models: Vec::new(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_models(mut self, models: Vec<ModelInfo>) -> Self {
        self.models = models;
        self
    }

    pub fn then(self, step: Step) -> Self {
        self.push(step);
        self
    }

    pub fn then_reply(self, reply: ChatResponse) -> Self {
        self.then(Step::Reply(reply))
    }

    pub fn then_text(self, text: &str) -> Self {
        self.then_reply(text_reply(text))
    }

    /// Append a step to a provider already shared with an agent.
    pub fn push(&self, step: Step) {
        self.script.lock().unwrap().push_back(step);
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Steps not yet used.
    pub fn remaining(&self) -> usize {
        self.script.lock().unwrap().len()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn stream(
        &self,
        req: &ChatRequest,
        sink: &mut (dyn FnMut(StreamEvent) + Send),
        cancel: &CancellationToken,
    ) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(req.clone());
        let step = self.script.lock().unwrap().pop_front();
        match step {
            None => Err(ProviderError::BadRequest {
                status: 400,
                message: "ScriptedProvider: the script ran out of steps".into(),
            }),
            Some(Step::Fail(err)) => Err(err),
            Some(Step::Hang) => {
                cancel.cancelled().await;
                Err(ProviderError::Cancelled)
            }
            Some(Step::Reply(reply)) => {
                if cancel.is_cancelled() {
                    return Err(ProviderError::Cancelled);
                }
                for block in &reply.message.content {
                    match block {
                        ContentBlock::Text { text } => sink(StreamEvent::TextDelta(text.clone())),
                        ContentBlock::Thinking { thinking, .. } if !thinking.is_empty() => {
                            sink(StreamEvent::ThinkingDelta(thinking.clone()))
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            sink(StreamEvent::ToolUseStart {
                                id: id.clone(),
                                name: name.clone(),
                            });
                            sink(StreamEvent::ToolUseInputDelta {
                                id: id.clone(),
                                partial_json: input.to_string(),
                            });
                        }
                        _ => {}
                    }
                }
                Ok(reply)
            }
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(self.models.clone())
    }
}

fn scripted_usage() -> Usage {
    Usage {
        input_tokens: 10,
        output_tokens: 5,
        ..Usage::default()
    }
}

/// A final text answer.
pub fn text_reply(text: &str) -> ChatResponse {
    ChatResponse {
        message: Message::assistant_text(text),
        stop_reason: StopReason::EndTurn,
        usage: scripted_usage(),
    }
}

/// A turn that calls tools: `(id, name, input)` per call.
pub fn tool_reply<'a>(calls: impl IntoIterator<Item = (&'a str, &'a str, Value)>) -> ChatResponse {
    ChatResponse {
        message: Message::assistant(
            calls
                .into_iter()
                .map(|(id, name, input)| ContentBlock::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    input,
                })
                .collect(),
        ),
        stop_reason: StopReason::ToolUse,
        usage: scripted_usage(),
    }
}

type ToolFn = dyn Fn(ToolContext, Value) -> BoxFuture<'static, ToolOutput> + Send + Sync;

/// A tool made from a closure.
#[derive(Clone)]
pub struct FnTool {
    spec: ToolSpec,
    f: Arc<ToolFn>,
}

impl FnTool {
    /// The schema defaults to an object with no declared properties; set a
    /// real one with [`FnTool::with_schema`] when the test cares.
    pub fn new<F, Fut>(name: &str, description: &str, f: F) -> Self
    where
        F: Fn(ToolContext, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ToolOutput> + Send + 'static,
    {
        Self {
            spec: ToolSpec {
                name: name.into(),
                description: description.into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            f: Arc::new(move |ctx, input| f(ctx, input).boxed()),
        }
    }

    pub fn with_schema(mut self, schema: Value) -> Self {
        self.spec.input_schema = schema;
        self
    }
}

#[async_trait]
impl Tool for FnTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        (self.f)(ctx.clone(), input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replays_steps_in_order_and_records_requests() {
        let provider = ScriptedProvider::new()
            .then_text("one")
            .then(Step::Fail(ProviderError::Network("down".into())));
        let req = ChatRequest {
            model: "m".into(),
            system: String::new(),
            messages: vec![Message::user_text("hi")],
            tools: vec![],
            max_tokens: 10,
            temperature: None,
            thinking: None,
        };
        let cancel = CancellationToken::new();
        let mut deltas = Vec::new();
        let first = provider
            .stream(&req, &mut |e| deltas.push(e), &cancel)
            .await
            .unwrap();
        assert_eq!(first.message.text(), "one");
        assert_eq!(deltas, vec![StreamEvent::TextDelta("one".into())]);
        assert!(matches!(
            provider.stream(&req, &mut |_| {}, &cancel).await,
            Err(ProviderError::Network(_))
        ));
        assert!(matches!(
            provider.stream(&req, &mut |_| {}, &cancel).await,
            Err(ProviderError::BadRequest { .. })
        ));
        assert_eq!(provider.requests().len(), 3);
        assert_eq!(provider.remaining(), 0);
    }

    #[tokio::test]
    async fn fn_tool_calls_the_closure() {
        let tool = FnTool::new("echo", "echo", |_ctx, input| async move {
            ToolOutput::text(input["x"].to_string())
        })
        .with_schema(json!({"type": "object", "properties": {"x": {"type": "integer"}}}));
        assert_eq!(
            tool.spec().input_schema["properties"]["x"]["type"],
            "integer"
        );
        let out = tool.call(&ToolContext::default(), json!({"x": 7})).await;
        assert_eq!(out.text_content(), "7");
    }
}
