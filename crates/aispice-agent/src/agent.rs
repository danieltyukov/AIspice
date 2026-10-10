//! The agent loop: stream a reply, run the tools it asks for, feed the
//! results back, repeat until the model is done.
//!
//! The history is append-only. Providers that sign thinking blocks bind them
//! to the exact conversation prefix, so editing earlier turns would
//! invalidate them (and the prompt cache). The only change ever made to an
//! existing message is appending to a trailing user message that the model
//! has not answered yet.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::message::{ContentBlock, Message, Role};
use crate::provider::{
    ChatRequest, Provider, ProviderError, StopReason, StreamEvent, ThinkingConfig, Usage,
};
use crate::tool::{Registry, Tool, ToolContext, ToolOutput};

pub const DEFAULT_MAX_STEPS: u32 = 40;

/// How long tools get to wind down after a cancel before they are dropped.
/// Long enough to kill a simulator and report what it had, short enough
/// that the stop button feels immediate.
const CANCEL_GRACE: Duration = Duration::from_secs(1);

pub struct Agent {
    pub provider: Arc<dyn Provider>,
    pub registry: Arc<Registry>,
    pub model: String,
    pub system: String,
    /// Model calls per run. Each call that asks for tools costs one step.
    pub max_steps: u32,
    pub max_tokens: u32,
    pub thinking: Option<ThinkingConfig>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        registry: Arc<Registry>,
        model: impl Into<String>,
    ) -> Self {
        let max_tokens = crate::providers::default_max_tokens(provider.id());
        Self {
            provider,
            registry,
            model: model.into(),
            system: String::new(),
            max_steps: DEFAULT_MAX_STEPS,
            max_tokens,
            thinking: Some(ThinkingConfig::adaptive()),
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = system.into();
        self
    }

    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_thinking(mut self, thinking: Option<ThinkingConfig>) -> Self {
        self.thinking = thinking;
        self
    }

    /// Run one user turn to completion.
    ///
    /// `history` is updated in place as the run goes, so after an error or a
    /// cancel it still holds everything that happened and stays valid to send
    /// again. Cancellation (through `ctx.cancel`) is a normal outcome, not an
    /// error. Tool failures, unknown tools and unparseable tool input become
    /// error results the model can read and recover from.
    pub async fn run(
        &self,
        history: &mut Vec<Message>,
        user: Vec<ContentBlock>,
        ctx: ToolContext,
        on_event: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Result<RunSummary, AgentError> {
        let cancel = ctx.cancel.clone();
        push_user(history, user);
        // Fixed for the whole run: the tool list is the start of the prompt,
        // so it must not change between steps or the cache misses.
        let tools = self.registry.specs();
        let mut summary = RunSummary {
            steps: 0,
            usage: Usage::default(),
            stop_reason: AgentStopReason::EndTurn,
        };

        for step in 1..=self.max_steps {
            if cancel.is_cancelled() {
                return Ok(finish(summary, AgentStopReason::Cancelled, on_event));
            }
            summary.steps = step;
            let request = ChatRequest {
                model: self.model.clone(),
                system: self.system.clone(),
                messages: history.clone(),
                tools: tools.clone(),
                max_tokens: self.max_tokens,
                temperature: None,
                thinking: self.thinking.clone(),
            };
            let result = {
                let mut sink = |event: StreamEvent| match event {
                    StreamEvent::TextDelta(text) => on_event(AgentEvent::TextDelta { text }),
                    StreamEvent::ThinkingDelta(text) => {
                        on_event(AgentEvent::ThinkingDelta { text })
                    }
                    StreamEvent::ToolUseStart { .. } | StreamEvent::ToolUseInputDelta { .. } => {}
                };
                self.provider.stream(&request, &mut sink, &cancel).await
            };
            let response = match result {
                Ok(response) => response,
                Err(ProviderError::Cancelled) => {
                    return Ok(finish(summary, AgentStopReason::Cancelled, on_event));
                }
                Err(err) => {
                    on_event(AgentEvent::Error {
                        message: err.to_string(),
                    });
                    return Err(AgentError::Provider(err));
                }
            };
            summary.usage += response.usage;
            on_event(AgentEvent::StepEnd {
                step,
                usage: response.usage,
            });

            if response.stop_reason == StopReason::Refusal {
                // A declined turn may hold partial output, possibly cut off
                // mid tool call. It is not an answer, so it stays out of the
                // history.
                return Ok(finish(summary, AgentStopReason::Refusal, on_event));
            }
            let calls: Vec<Call> = response
                .message
                .tool_uses()
                .into_iter()
                .map(|c| Call {
                    id: c.id.to_string(),
                    name: c.name.to_string(),
                    input: c.input.clone(),
                })
                .collect();
            if !response.message.content.is_empty() {
                history.push(response.message);
            }
            if calls.is_empty() {
                let reason = match response.stop_reason {
                    StopReason::MaxTokens => AgentStopReason::MaxTokens,
                    StopReason::Other(other) => AgentStopReason::Other(other),
                    _ => AgentStopReason::EndTurn,
                };
                return Ok(finish(summary, reason, on_event));
            }

            let (results, cancelled) = if response.stop_reason == StopReason::MaxTokens {
                // The last call's input was probably cut off, and a cut-off
                // object can still parse. Run nothing from this turn.
                (truncated_results(&calls, on_event), false)
            } else {
                self.execute(&calls, &ctx, on_event).await
            };
            history.push(Message::user(results));
            if cancelled {
                return Ok(finish(summary, AgentStopReason::Cancelled, on_event));
            }
        }

        let note = format!(
            "Stopped after {} steps, the limit for one run. Send a message to continue from here.",
            self.max_steps
        );
        on_event(AgentEvent::TextDelta { text: note.clone() });
        history.push(Message::assistant_text(note));
        Ok(finish(summary, AgentStopReason::MaxSteps, on_event))
    }

    /// Run every call of one turn concurrently. Results come back in call
    /// order, which is what providers expect; `ToolEnd` events go out as
    /// each call finishes so the UI can update early.
    async fn execute(
        &self,
        calls: &[Call],
        ctx: &ToolContext,
        on_event: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> (Vec<ContentBlock>, bool) {
        for call in calls {
            on_event(AgentEvent::ToolStart {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
            });
        }
        let available = self.registry.names();
        let started = Instant::now();
        let mut pending: FuturesUnordered<BoxFuture<'static, (usize, ToolOutput, Duration)>> =
            calls
                .iter()
                .enumerate()
                .map(|(index, call)| {
                    let tool = self.registry.get(&call.name);
                    let mut call_ctx = ctx.clone();
                    call_ctx.call_id = Some(call.id.clone());
                    let (name, input) = (call.name.clone(), call.input.clone());
                    let available = available.clone();
                    async move {
                        let begun = Instant::now();
                        let output = run_tool(tool, &name, input, &call_ctx, &available).await;
                        (index, output, begun.elapsed())
                    }
                    .boxed()
                })
                .collect();

        let mut outputs: Vec<Option<(ToolOutput, Duration)>> = vec![None; calls.len()];
        let mut deadline: Option<Instant> = None;
        loop {
            tokio::select! {
                biased;
                next = pending.next() => match next {
                    Some((index, output, elapsed)) => {
                        let call = &calls[index];
                        on_event(AgentEvent::ToolEnd {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            output: output.clone(),
                            duration_ms: elapsed.as_millis() as u64,
                        });
                        outputs[index] = Some((output, elapsed));
                    }
                    None => break,
                },
                _ = ctx.cancel.cancelled(), if deadline.is_none() => {
                    deadline = Some(Instant::now() + CANCEL_GRACE);
                }
                _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => break,
            }
        }
        drop(pending);

        let results = calls
            .iter()
            .zip(outputs)
            .map(|(call, done)| {
                let output = match done {
                    Some((output, _)) => output,
                    None => {
                        let output = ToolOutput::error("Cancelled before the tool finished.");
                        on_event(AgentEvent::ToolEnd {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            output: output.clone(),
                            duration_ms: started.elapsed().as_millis() as u64,
                        });
                        output
                    }
                };
                result_block(&call.id, output)
            })
            .collect();
        (results, ctx.cancel.is_cancelled())
    }
}

struct Call {
    id: String,
    name: String,
    input: Value,
}

async fn run_tool(
    tool: Option<Arc<dyn Tool>>,
    name: &str,
    input: Value,
    ctx: &ToolContext,
    available: &[String],
) -> ToolOutput {
    let Some(tool) = tool else {
        return ToolOutput::error(format!(
            "Unknown tool `{name}`. Available tools: {}.",
            available.join(", ")
        ));
    };
    if !input.is_object() {
        let raw = match &input {
            Value::String(raw) => raw.clone(),
            other => other.to_string(),
        };
        return ToolOutput::error(json!({"INVALID_JSON": raw}).to_string()).with_text(
            "The arguments were not a valid JSON object. Call the tool again with complete, valid JSON.",
        );
    }
    match AssertUnwindSafe(tool.call(ctx, input)).catch_unwind().await {
        Ok(output) => output,
        Err(panic) => {
            let detail = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no details".into());
            tracing::error!(tool = name, %detail, "tool panicked");
            ToolOutput::error(format!("The tool `{name}` crashed: {detail}"))
        }
    }
}

fn truncated_results(
    calls: &[Call],
    on_event: &mut (dyn FnMut(AgentEvent) + Send),
) -> Vec<ContentBlock> {
    calls
        .iter()
        .map(|call| {
            let output = ToolOutput::error(
                "Not run: the response reached the max_tokens limit before this tool call was complete. \
                 Try again with a shorter input or split the work into smaller calls.",
            );
            on_event(AgentEvent::ToolStart {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
            });
            on_event(AgentEvent::ToolEnd {
                id: call.id.clone(),
                name: call.name.clone(),
                output: output.clone(),
                duration_ms: 0,
            });
            result_block(&call.id, output)
        })
        .collect()
}

/// The part of a tool output the model sees: text and images only.
fn result_block(tool_use_id: &str, output: ToolOutput) -> ContentBlock {
    let content = output
        .content
        .into_iter()
        .filter(|c| matches!(c, ContentBlock::Text { .. } | ContentBlock::Image { .. }))
        .collect();
    ContentBlock::tool_result(tool_use_id, content, output.is_error)
}

/// Add the user's turn, keeping the history valid to send.
///
/// Tool calls left unanswered by an interrupted run get error results first,
/// since providers reject a tool call without a result. If the history ends
/// in a user message the model never answered (a cancel or an error before
/// the reply), the new content joins it rather than starting a second user
/// turn in a row.
fn push_user(history: &mut Vec<Message>, mut user: Vec<ContentBlock>) {
    if let Some(last) = history.last()
        && last.role == Role::Assistant
    {
        let open: Vec<ContentBlock> = last
            .tool_uses()
            .into_iter()
            .map(|call| {
                ContentBlock::tool_result(
                    call.id,
                    vec![ContentBlock::text(
                        "Not run: the previous run was interrupted.",
                    )],
                    true,
                )
            })
            .collect();
        user.splice(0..0, open);
    }
    if user.is_empty() {
        user.push(ContentBlock::text("Continue."));
    }
    match history.last_mut() {
        Some(last) if last.role == Role::User => last.content.extend(user),
        _ => history.push(Message::user(user)),
    }
}

fn finish(
    mut summary: RunSummary,
    stop_reason: AgentStopReason,
    on_event: &mut (dyn FnMut(AgentEvent) + Send),
) -> RunSummary {
    summary.stop_reason = stop_reason;
    on_event(AgentEvent::Done {
        summary: summary.clone(),
    });
    summary
}

/// What a run reports as it goes. Serialized with a `type` tag for the
/// desktop UI channel, e.g. `{"type": "text_delta", "text": "..."}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ToolStart {
        id: String,
        name: String,
        input: Value,
    },
    ToolEnd {
        id: String,
        name: String,
        output: ToolOutput,
        duration_ms: u64,
    },
    StepEnd {
        step: u32,
        usage: Usage,
    },
    Done {
        summary: RunSummary,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    /// Model calls made.
    pub steps: u32,
    pub usage: Usage,
    pub stop_reason: AgentStopReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStopReason {
    EndTurn,
    MaxSteps,
    MaxTokens,
    Refusal,
    Cancelled,
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatResponse, ThinkingConfig};
    use crate::testing::{FnTool, ScriptedProvider, Step, tool_reply};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Barrier;
    use tokio_util::sync::CancellationToken;

    fn add_tool() -> FnTool {
        FnTool::new("add", "Add two numbers", |_ctx, input| async move {
            let a = input["a"].as_f64().unwrap_or(0.0);
            let b = input["b"].as_f64().unwrap_or(0.0);
            ToolOutput::text(format!("{}", a + b)).with_data(json!({"sum": a + b}))
        })
    }

    fn agent(provider: Arc<ScriptedProvider>, registry: Registry) -> Agent {
        Agent::new(provider, Arc::new(registry), "test-model").with_system("Be brief.")
    }

    async fn run(
        agent: &Agent,
        history: &mut Vec<Message>,
        text: &str,
        ctx: ToolContext,
    ) -> (Result<RunSummary, AgentError>, Vec<AgentEvent>) {
        let mut events = Vec::new();
        let result = agent
            .run(history, vec![ContentBlock::text(text)], ctx, &mut |e| {
                events.push(e)
            })
            .await;
        (result, events)
    }

    fn kinds(events: &[AgentEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                AgentEvent::TextDelta { .. } => "text",
                AgentEvent::ThinkingDelta { .. } => "thinking",
                AgentEvent::ToolStart { .. } => "tool_start",
                AgentEvent::ToolEnd { .. } => "tool_end",
                AgentEvent::StepEnd { .. } => "step_end",
                AgentEvent::Done { .. } => "done",
                AgentEvent::Error { .. } => "error",
            })
            .collect()
    }

    fn result_of(message: &Message, index: usize) -> (&str, String, bool) {
        match &message.content[index] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => (
                tool_use_id.as_str(),
                content.iter().filter_map(ContentBlock::as_text).collect(),
                *is_error,
            ),
            other => panic!("not a tool result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn multi_step_tool_use() {
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(tool_reply([("t1", "add", json!({"a": 2, "b": 3}))]))
                .then_reply(tool_reply([("t2", "add", json!({"a": 5, "b": 10}))]))
                .then_text("The total is 15."),
        );
        let agent = agent(provider.clone(), Registry::new().with(add_tool()));
        let mut history = Vec::new();
        let (result, events) =
            run(&agent, &mut history, "add things", ToolContext::default()).await;
        let summary = result.unwrap();
        assert_eq!(summary.steps, 3);
        assert_eq!(summary.stop_reason, AgentStopReason::EndTurn);
        assert_eq!(summary.usage.input_tokens, 30);
        assert_eq!(summary.usage.output_tokens, 15);

        assert_eq!(history.len(), 6);
        assert_eq!(history[0], Message::user_text("add things"));
        assert_eq!(result_of(&history[2], 0), ("t1", "5".into(), false));
        assert_eq!(result_of(&history[4], 0), ("t2", "15".into(), false));
        assert_eq!(history[5], Message::assistant_text("The total is 15."));

        assert_eq!(
            kinds(&events),
            vec![
                "step_end",
                "tool_start",
                "tool_end",
                "step_end",
                "tool_start",
                "tool_end",
                "text",
                "step_end",
                "done"
            ]
        );
        let AgentEvent::ToolEnd { output, .. } = &events[2] else {
            panic!()
        };
        assert_eq!(output.data, Some(json!({"sum": 5.0})));

        let requests = provider.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].system, "Be brief.");
        assert_eq!(requests[0].tools[0].name, "add");
        assert_eq!(requests[0].thinking, Some(ThinkingConfig::adaptive()));
        assert_eq!(requests[2].messages, history[..5].to_vec());
    }

    #[tokio::test]
    async fn two_tools_in_one_turn_run_concurrently_and_keep_order() {
        // Each tool waits for the other at a barrier, so this only finishes
        // if both run at the same time. The first finishes last.
        let barrier = Arc::new(Barrier::new(2));
        let (b1, b2) = (barrier.clone(), barrier.clone());
        let slow = FnTool::new("slow", "slow", move |_ctx, _input| {
            let b = b1.clone();
            async move {
                b.wait().await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                ToolOutput::text("slow done")
            }
        });
        let fast = FnTool::new("fast", "fast", move |_ctx, _input| {
            let b = b2.clone();
            async move {
                b.wait().await;
                ToolOutput::text("fast done")
            }
        });
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(tool_reply([
                    ("a", "slow", json!({})),
                    ("b", "fast", json!({})),
                ]))
                .then_text("ok"),
        );
        let agent = agent(provider, Registry::new().with(slow).with(fast));
        let mut history = Vec::new();
        let (result, events) = tokio::time::timeout(
            Duration::from_secs(5),
            run(&agent, &mut history, "go", ToolContext::default()),
        )
        .await
        .expect("tools ran one after the other and deadlocked");
        result.unwrap();
        assert_eq!(result_of(&history[2], 0), ("a", "slow done".into(), false));
        assert_eq!(result_of(&history[2], 1), ("b", "fast done".into(), false));
        let ends: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ToolEnd { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ends, vec!["b", "a"]);
    }

    #[tokio::test]
    async fn unknown_tool_becomes_an_error_result() {
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(tool_reply([("t1", "frobnicate", json!({}))]))
                .then_reply(tool_reply([("t2", "add", json!({"a": 1, "b": 1}))]))
                .then_text("Recovered."),
        );
        let agent = agent(provider, Registry::new().with(add_tool()));
        let mut history = Vec::new();
        let (result, _) = run(&agent, &mut history, "go", ToolContext::default()).await;
        assert_eq!(result.unwrap().stop_reason, AgentStopReason::EndTurn);
        let (id, text, is_error) = result_of(&history[2], 0);
        assert_eq!(id, "t1");
        assert!(is_error);
        assert_eq!(text, "Unknown tool `frobnicate`. Available tools: add.");
        assert_eq!(result_of(&history[4], 0), ("t2", "2".into(), false));
    }

    #[tokio::test]
    async fn unparseable_input_is_returned_without_running_the_tool() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let tool = FnTool::new("simulate", "sim", move |_ctx, _input| {
            counter.fetch_add(1, Ordering::SeqCst);
            async { ToolOutput::text("ran") }
        });
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(tool_reply([(
                    "t1",
                    "simulate",
                    Value::String("{\"circuit\": \"rc".into()),
                )]))
                .then_text("Retrying later."),
        );
        let agent = agent(provider, Registry::new().with(tool));
        let mut history = Vec::new();
        run(&agent, &mut history, "go", ToolContext::default())
            .await
            .0
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let (_, text, is_error) = result_of(&history[2], 0);
        assert!(is_error);
        assert!(
            text.starts_with(r#"{"INVALID_JSON":"{\"circuit\": \"rc"}"#),
            "{text}"
        );
    }

    #[tokio::test]
    async fn stops_at_max_steps_with_a_note() {
        let mut script = ScriptedProvider::new();
        for i in 0..5 {
            script = script.then_reply(tool_reply([(
                format!("t{i}").as_str(),
                "add",
                json!({"a": i, "b": 1}),
            )]));
        }
        let provider = Arc::new(script);
        let agent = agent(provider.clone(), Registry::new().with(add_tool())).with_max_steps(3);
        let mut history = Vec::new();
        let (result, events) = run(&agent, &mut history, "loop", ToolContext::default()).await;
        let summary = result.unwrap();
        assert_eq!(summary.stop_reason, AgentStopReason::MaxSteps);
        assert_eq!(summary.steps, 3);
        assert_eq!(provider.requests().len(), 3);
        assert_eq!(provider.remaining(), 2);
        let last = history.last().unwrap();
        assert_eq!(last.role, Role::Assistant);
        assert!(last.text().contains("Stopped after 3 steps"));
        assert!(matches!(events.last(), Some(AgentEvent::Done { .. })));
        // The history is still valid to continue: a new message follows the note.
        let next = Arc::new(ScriptedProvider::new().then_text("continuing"));
        let agent2 = Agent::new(
            next.clone(),
            Arc::new(Registry::new().with(add_tool())),
            "m",
        );
        run(&agent2, &mut history, "go on", ToolContext::default())
            .await
            .0
            .unwrap();
        let sent = &next.requests()[0].messages;
        assert_eq!(sent.last().unwrap(), &Message::user_text("go on"));
    }

    #[tokio::test]
    async fn cancel_during_tools_records_results_and_stops() {
        let cooperative = FnTool::new("simulate", "sim", |ctx, _input| async move {
            ctx.cancel.cancelled().await;
            ToolOutput::error("Simulation killed.")
        });
        let stubborn = FnTool::new("sweep", "sweep", |_ctx, _input| async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            ToolOutput::text("never")
        });
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(tool_reply([
                    ("t1", "simulate", json!({})),
                    ("t2", "sweep", json!({})),
                ]))
                .then_text("unreachable"),
        );
        let agent = agent(
            provider.clone(),
            Registry::new().with(cooperative).with(stubborn),
        );
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            trigger.cancel();
        });
        let mut history = Vec::new();
        let started = std::time::Instant::now();
        let (result, events) = run(&agent, &mut history, "go", ToolContext::new(cancel)).await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(result.unwrap().stop_reason, AgentStopReason::Cancelled);
        assert_eq!(provider.remaining(), 1);
        assert_eq!(history.len(), 3);
        assert_eq!(
            result_of(&history[2], 0),
            ("t1", "Simulation killed.".into(), true)
        );
        assert_eq!(
            result_of(&history[2], 1),
            ("t2", "Cancelled before the tool finished.".into(), true)
        );
        let ends = kinds(&events).iter().filter(|k| **k == "tool_end").count();
        assert_eq!(ends, 2);
    }

    #[tokio::test]
    async fn cancel_while_streaming_then_continue() {
        let provider = Arc::new(ScriptedProvider::new().then(Step::Hang).then_text("Hello."));
        let agent = agent(provider.clone(), Registry::new());
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            trigger.cancel();
        });
        let mut history = Vec::new();
        let (result, _) = run(&agent, &mut history, "first", ToolContext::new(cancel)).await;
        assert_eq!(result.unwrap().stop_reason, AgentStopReason::Cancelled);
        assert_eq!(history, vec![Message::user_text("first")]);

        let (result, _) = run(&agent, &mut history, "second", ToolContext::default()).await;
        assert_eq!(result.unwrap().stop_reason, AgentStopReason::EndTurn);
        assert_eq!(
            provider.requests()[1].messages,
            vec![Message::user(vec![
                ContentBlock::text("first"),
                ContentBlock::text("second"),
            ])]
        );
    }

    #[tokio::test]
    async fn already_cancelled_run_makes_no_request() {
        let provider = Arc::new(ScriptedProvider::new().then_text("x"));
        let agent = agent(provider.clone(), Registry::new());
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut history = Vec::new();
        let (result, events) = run(&agent, &mut history, "hi", ToolContext::new(cancel)).await;
        assert_eq!(result.unwrap().stop_reason, AgentStopReason::Cancelled);
        assert!(provider.requests().is_empty());
        assert_eq!(kinds(&events), vec!["done"]);
    }

    #[tokio::test]
    async fn max_tokens_mid_tool_call_runs_nothing() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let tool = FnTool::new("write", "write", move |_ctx, _input| {
            counter.fetch_add(1, Ordering::SeqCst);
            async { ToolOutput::text("written") }
        });
        let mut truncated = tool_reply([("t1", "write", json!({"body": "partial"}))]);
        truncated.stop_reason = StopReason::MaxTokens;
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(truncated)
                .then_text("I will split it."),
        );
        let agent = agent(provider, Registry::new().with(tool));
        let mut history = Vec::new();
        let (result, _) = run(&agent, &mut history, "go", ToolContext::default()).await;
        assert_eq!(result.unwrap().stop_reason, AgentStopReason::EndTurn);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let (_, text, is_error) = result_of(&history[2], 0);
        assert!(is_error);
        assert!(text.contains("max_tokens"));
    }

    #[tokio::test]
    async fn refusal_keeps_partial_output_out_of_history() {
        let provider = Arc::new(ScriptedProvider::new().then_reply(ChatResponse {
            message: Message::assistant_text("Partial"),
            stop_reason: StopReason::Refusal,
            usage: Usage::default(),
        }));
        let agent = agent(provider, Registry::new());
        let mut history = Vec::new();
        let (result, _) = run(&agent, &mut history, "hi", ToolContext::default()).await;
        assert_eq!(result.unwrap().stop_reason, AgentStopReason::Refusal);
        assert_eq!(history, vec![Message::user_text("hi")]);
    }

    #[tokio::test]
    async fn provider_errors_surface_as_errors() {
        let provider = Arc::new(ScriptedProvider::new().then(Step::Fail(
            ProviderError::BadRequest {
                status: 400,
                message: "model not found".into(),
            },
        )));
        let agent = agent(provider, Registry::new());
        let mut history = Vec::new();
        let (result, events) = run(&agent, &mut history, "hi", ToolContext::default()).await;
        assert!(matches!(
            result,
            Err(AgentError::Provider(ProviderError::BadRequest { .. }))
        ));
        assert_eq!(
            events,
            vec![AgentEvent::Error {
                message: "request rejected (HTTP 400): model not found".into()
            }]
        );
    }

    #[tokio::test]
    async fn a_panicking_tool_is_an_error_not_a_crash() {
        let tool = FnTool::new("boom", "boom", |_ctx, _input| async move {
            panic!("divide by zero in solver");
        });
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(tool_reply([("t1", "boom", json!({}))]))
                .then_text("It crashed."),
        );
        let agent = agent(provider, Registry::new().with(tool));
        let mut history = Vec::new();
        run(&agent, &mut history, "go", ToolContext::default())
            .await
            .0
            .unwrap();
        let (_, text, is_error) = result_of(&history[2], 0);
        assert!(is_error);
        assert_eq!(text, "The tool `boom` crashed: divide by zero in solver");
    }

    #[tokio::test]
    async fn open_tool_calls_from_an_interrupted_run_are_closed() {
        let provider = Arc::new(ScriptedProvider::new().then_text("ok"));
        let agent = agent(provider.clone(), Registry::new());
        let mut history = vec![
            Message::user_text("start"),
            Message::assistant(vec![ContentBlock::ToolUse {
                id: "old".into(),
                name: "simulate".into(),
                input: json!({}),
            }]),
        ];
        run(&agent, &mut history, "next", ToolContext::default())
            .await
            .0
            .unwrap();
        let sent = provider.requests()[0].messages.clone();
        assert_eq!(sent.len(), 3);
        assert_eq!(
            sent[2],
            Message::user(vec![
                ContentBlock::tool_result(
                    "old",
                    vec![ContentBlock::text(
                        "Not run: the previous run was interrupted."
                    )],
                    true
                ),
                ContentBlock::text("next"),
            ])
        );
    }

    #[tokio::test]
    async fn thinking_blocks_go_back_unchanged() {
        let mut reply = tool_reply([("t1", "add", json!({"a": 1, "b": 2}))]);
        reply.message.content.insert(
            0,
            ContentBlock::Thinking {
                thinking: "add them".into(),
                signature: Some("sig-123".into()),
            },
        );
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(reply.clone())
                .then_text("3"),
        );
        let agent = agent(provider.clone(), Registry::new().with(add_tool()));
        let mut history = Vec::new();
        let (_, events) = run(&agent, &mut history, "1+2", ToolContext::default()).await;
        assert_eq!(provider.requests()[1].messages[1], reply.message);
        assert!(events.contains(&AgentEvent::ThinkingDelta {
            text: "add them".into()
        }));
    }

    #[tokio::test]
    async fn tools_get_the_call_id_state_and_progress_sink() {
        struct Project {
            name: &'static str,
        }
        let progress = Arc::new(Mutex::new(Vec::new()));
        let seen = progress.clone();
        let tool = FnTool::new("probe", "probe", |ctx, _input| async move {
            ctx.progress("halfway", Some(0.5));
            let name = ctx.state::<Project>().map(|p| p.name).unwrap_or("none");
            ToolOutput::text(format!("{} {}", ctx.call_id.unwrap_or_default(), name))
        });
        let provider = Arc::new(
            ScriptedProvider::new()
                .then_reply(tool_reply([("call-9", "probe", json!({}))]))
                .then_text("done"),
        );
        let agent = agent(provider, Registry::new().with(tool));
        let ctx = ToolContext::default()
            .with_state(Arc::new(Project { name: "filter" }))
            .with_events(crate::tool::EventSink::new(move |e| {
                seen.lock().unwrap().push(e)
            }));
        let mut history = Vec::new();
        run(&agent, &mut history, "go", ctx).await.0.unwrap();
        assert_eq!(result_of(&history[2], 0).1, "call-9 filter");
        let progress = progress.lock().unwrap();
        assert_eq!(progress[0].call_id.as_deref(), Some("call-9"));
        assert_eq!(progress[0].message, "halfway");
    }

    #[test]
    fn events_serialize_with_a_type_tag() {
        let event = AgentEvent::ToolEnd {
            id: "t1".into(),
            name: "simulate".into(),
            output: ToolOutput::text("ok").with_data(json!({"v": 1})),
            duration_ms: 12,
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "type": "tool_end",
                "id": "t1",
                "name": "simulate",
                "output": {"content": [{"type": "text", "text": "ok"}], "data": {"v": 1}, "is_error": false},
                "duration_ms": 12
            })
        );
        assert_eq!(
            serde_json::to_value(AgentEvent::TextDelta { text: "hi".into() }).unwrap(),
            json!({"type": "text_delta", "text": "hi"})
        );
        let done = AgentEvent::Done {
            summary: RunSummary {
                steps: 2,
                usage: Usage::default(),
                stop_reason: AgentStopReason::MaxSteps,
            },
        };
        let value = serde_json::to_value(&done).unwrap();
        assert_eq!(value["summary"]["stop_reason"], "max_steps");
        let back: AgentEvent = serde_json::from_value(value).unwrap();
        assert_eq!(back, done);
    }

    #[test]
    fn run_future_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        let provider = Arc::new(ScriptedProvider::new());
        let agent = agent(provider, Registry::new());
        let mut history = Vec::new();
        let mut on_event = |_e: AgentEvent| {};
        let future = agent.run(&mut history, vec![], ToolContext::default(), &mut on_event);
        assert_send(&future);
    }

    #[test]
    fn defaults() {
        let agent = agent(Arc::new(ScriptedProvider::new()), Registry::new());
        assert_eq!(agent.max_steps, DEFAULT_MAX_STEPS);
        assert_eq!(agent.max_tokens, 16_384);
        let anthropic = Agent::new(
            Arc::new(ScriptedProvider::new().with_id("anthropic")),
            Arc::new(Registry::new()),
            "claude-opus-5-5",
        );
        assert_eq!(anthropic.max_tokens, 64_000);
    }
}
