import type { AgentEvent, ChatMessage, ChatPart, ToolOutput } from "../ipc/types";

/*
 * The chat transcript as a pure reducer over agent events.
 *
 * A turn is a user message followed by one assistant message that the event
 * stream fills in: text and thinking deltas append to the last part of the
 * same kind, tool calls become parts that later receive their output. The
 * mock backend uses the same function to keep its stored sessions in step.
 */

export interface Approval {
  request_id: string;
  summary: string;
  diff: string;
}

export interface ChatState {
  messages: ChatMessage[];
  streaming: boolean;
  approval: Approval | null;
  usage: { input: number; output: number };
  steps: number;
  stopReason: string | null;
}

export type ChatAction =
  | { type: "reset"; messages: ChatMessage[] }
  | { type: "send"; user: ChatMessage; assistantId: string; time: number }
  | { type: "event"; event: AgentEvent }
  | { type: "approval_resolved" }
  | { type: "failed"; message: string };

export const initialChat: ChatState = {
  messages: [],
  streaming: false,
  approval: null,
  usage: { input: 0, output: 0 },
  steps: 0,
  stopReason: null,
};

/** Applies one agent event to an assistant message, returning a new message. */
export function applyEvent(message: ChatMessage, event: AgentEvent): ChatMessage {
  switch (event.type) {
    case "text_delta":
      return { ...message, parts: appendText(message.parts, "text", event.text) };
    case "thinking_delta":
      return { ...message, parts: appendText(message.parts, "thinking", event.text) };
    case "tool_start":
      return {
        ...message,
        parts: [...message.parts, { kind: "tool", call: { id: event.id, name: event.name, input: event.input } }],
      };
    case "tool_end": {
      let found = false;
      const parts = message.parts.map((p): ChatPart => {
        if (p.kind === "tool" && p.call.id === event.id) {
          found = true;
          return { kind: "tool", call: { ...p.call, output: event.output, duration_ms: event.duration_ms } };
        }
        return p;
      });
      if (!found) {
        parts.push({
          kind: "tool",
          call: { id: event.id, name: event.name, input: null, output: event.output, duration_ms: event.duration_ms },
        });
      }
      return { ...message, parts };
    }
    case "error":
      return { ...message, error: event.message };
    default:
      return message;
  }
}

function appendText(parts: ChatPart[], kind: "text" | "thinking", text: string): ChatPart[] {
  if (text === "") return parts;
  const last = parts[parts.length - 1];
  if (last && last.kind === kind) {
    return [...parts.slice(0, -1), { kind, text: last.text + text }];
  }
  return [...parts, { kind, text }];
}

function updateLastAssistant(messages: ChatMessage[], fn: (m: ChatMessage) => ChatMessage): ChatMessage[] {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") {
      const next = messages.slice();
      next[i] = fn(messages[i]);
      return next;
    }
    if (messages[i].role === "user") break;
  }
  return messages;
}

export function chatReducer(state: ChatState, action: ChatAction): ChatState {
  switch (action.type) {
    case "reset":
      return { ...initialChat, messages: action.messages };
    case "send":
      return {
        ...state,
        streaming: true,
        approval: null,
        stopReason: null,
        messages: [
          ...state.messages,
          action.user,
          { id: action.assistantId, role: "assistant", parts: [], time: action.time },
        ],
      };
    case "event": {
      const e = action.event;
      switch (e.type) {
        case "step_end":
          return {
            ...state,
            steps: state.steps + 1,
            usage: { input: state.usage.input + e.input_tokens, output: state.usage.output + e.output_tokens },
          };
        case "done":
          return { ...state, streaming: false, approval: null, stopReason: e.stop_reason };
        case "approval_request":
          return { ...state, approval: { request_id: e.request_id, summary: e.summary, diff: e.diff } };
        case "error":
          return {
            ...state,
            streaming: false,
            approval: null,
            messages: updateLastAssistant(state.messages, (m) => applyEvent(m, e)),
          };
        default:
          return { ...state, messages: updateLastAssistant(state.messages, (m) => applyEvent(m, e)) };
      }
    }
    case "approval_resolved":
      return { ...state, approval: null };
    case "failed":
      return {
        ...state,
        streaming: false,
        approval: null,
        messages: updateLastAssistant(state.messages, (m) => ({ ...m, error: action.message })),
      };
  }
}

/** A tool call that ended without output because the turn stopped first. */
export function isUnfinished(output: ToolOutput | undefined, streaming: boolean): boolean {
  return output === undefined && !streaming;
}
