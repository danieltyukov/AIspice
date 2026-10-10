import { describe, expect, it } from "vitest";
import type { AgentEvent, ChatMessage, ToolOutput } from "../ipc/types";
import { applyEvent, chatReducer, initialChat, isUnfinished, type ChatState } from "./reduce";

const user: ChatMessage = { id: "u1", role: "user", parts: [{ kind: "text", text: "hi" }], time: 1 };

function run(events: AgentEvent[]): ChatState {
  let s = chatReducer(initialChat, { type: "send", user, assistantId: "a1", time: 2 });
  for (const event of events) s = chatReducer(s, { type: "event", event });
  return s;
}

const output: ToolOutput = { content: [{ type: "text", text: "ok" }], data: null, is_error: false };

describe("chatReducer", () => {
  it("adds the user message and an empty assistant message, and starts streaming", () => {
    const s = chatReducer(initialChat, { type: "send", user, assistantId: "a1", time: 2 });
    expect(s.streaming).toBe(true);
    expect(s.messages.map((m) => m.role)).toEqual(["user", "assistant"]);
    expect(s.messages[1].parts).toEqual([]);
  });

  it("joins consecutive text deltas and keeps thinking separate", () => {
    const s = run([
      { type: "thinking_delta", text: "Let me " },
      { type: "thinking_delta", text: "think." },
      { type: "text_delta", text: "Hello " },
      { type: "text_delta", text: "there." },
    ]);
    expect(s.messages[1].parts).toEqual([
      { kind: "thinking", text: "Let me think." },
      { kind: "text", text: "Hello there." },
    ]);
  });

  it("starts a new text part after a tool call", () => {
    const s = run([
      { type: "text_delta", text: "Reading." },
      { type: "tool_start", id: "t1", name: "read_schematic", input: { circuit: "rc.asc" } },
      { type: "tool_end", id: "t1", name: "read_schematic", output, duration_ms: 40 },
      { type: "text_delta", text: "Done." },
    ]);
    const parts = s.messages[1].parts;
    expect(parts.map((p) => p.kind)).toEqual(["text", "tool", "text"]);
    const tool = parts[1];
    expect(tool.kind === "tool" && tool.call.output).toEqual(output);
    expect(tool.kind === "tool" && tool.call.duration_ms).toBe(40);
  });

  it("records a tool end that had no start", () => {
    const s = run([{ type: "tool_end", id: "t9", name: "lint", output, duration_ms: 5 }]);
    const p = s.messages[1].parts[0];
    expect(p.kind === "tool" && p.call.id).toBe("t9");
  });

  it("counts steps and tokens", () => {
    const s = run([
      { type: "step_end", input_tokens: 100, output_tokens: 10 },
      { type: "step_end", input_tokens: 200, output_tokens: 20 },
    ]);
    expect(s.steps).toBe(2);
    expect(s.usage).toEqual({ input: 300, output: 30 });
  });

  it("stops streaming on done and keeps the stop reason", () => {
    const s = run([
      { type: "text_delta", text: "x" },
      { type: "done", steps: 1, stop_reason: "cancelled" },
    ]);
    expect(s.streaming).toBe(false);
    expect(s.stopReason).toBe("cancelled");
  });

  it("puts an error on the assistant message and stops", () => {
    const s = run([
      { type: "text_delta", text: "Let me check." },
      { type: "error", message: "529 Overloaded" },
    ]);
    expect(s.streaming).toBe(false);
    expect(s.messages[1].error).toBe("529 Overloaded");
    expect(s.messages[1].parts).toEqual([{ kind: "text", text: "Let me check." }]);
  });

  it("holds an approval request until it is resolved, and clears it on done", () => {
    let s = run([{ type: "approval_request", request_id: "r1", summary: "Set C1 to 160n", diff: "-a\n+b" }]);
    expect(s.approval).toEqual({ request_id: "r1", summary: "Set C1 to 160n", diff: "-a\n+b" });
    expect(s.streaming).toBe(true);
    s = chatReducer(s, { type: "approval_resolved" });
    expect(s.approval).toBeNull();
    s = chatReducer(s, { type: "event", event: { type: "approval_request", request_id: "r2", summary: "x", diff: "" } });
    s = chatReducer(s, { type: "event", event: { type: "done", steps: 2, stop_reason: "end_turn" } });
    expect(s.approval).toBeNull();
  });

  it("marks a failed send on the assistant message", () => {
    let s = chatReducer(initialChat, { type: "send", user, assistantId: "a1", time: 2 });
    s = chatReducer(s, { type: "failed", message: "No API key" });
    expect(s.streaming).toBe(false);
    expect(s.messages[1].error).toBe("No API key");
  });

  it("resets to a loaded session", () => {
    const s = chatReducer(run([{ type: "text_delta", text: "x" }]), { type: "reset", messages: [user] });
    expect(s).toEqual({ ...initialChat, messages: [user] });
  });

  it("never mutates the previous state", () => {
    const before = run([{ type: "text_delta", text: "a" }]);
    const snapshot = JSON.stringify(before);
    chatReducer(before, { type: "event", event: { type: "text_delta", text: "b" } });
    expect(JSON.stringify(before)).toBe(snapshot);
  });
});

describe("applyEvent and isUnfinished", () => {
  it("ignores empty deltas", () => {
    const m: ChatMessage = { id: "a", role: "assistant", parts: [], time: 0 };
    expect(applyEvent(m, { type: "text_delta", text: "" }).parts).toEqual([]);
  });

  it("treats a tool without output as unfinished only after streaming ends", () => {
    expect(isUnfinished(undefined, true)).toBe(false);
    expect(isUnfinished(undefined, false)).toBe(true);
    expect(isUnfinished(output, false)).toBe(false);
  });
});
