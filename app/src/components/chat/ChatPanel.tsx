import { useCallback, useEffect, useLayoutEffect, useRef, useState, type DragEvent } from "react";
import type { ChatMessage } from "../../ipc/types";
import { baseName, relative } from "../../lib/format";
import { useStore, useStoreApi } from "../../store/context";
import { DiffBlock } from "./DiffBlock";
import { Markdown } from "./Markdown";
import { ToolCard } from "./ToolCard";
import { Composer, type ComposerHandle } from "./Composer";
import "./Chat.css";

const SUGGESTIONS = ["Explain how this circuit works", "Check the specs and fix what fails", "Find problems in this schematic"];

export function ChatPanel({ width }: { width: number }) {
  const store = useStoreApi();
  const active = useStore((s) => s.active);
  const sessions = useStore((s) => s.sessions);
  const sessionId = useStore((s) => s.sessionId);
  const chat = useStore((s) => s.chat);
  const composer = useRef<ComposerHandle | null>(null);
  const [dragging, setDragging] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => setConfirmDelete(false), [sessionId]);

  const onReady = useCallback((h: ComposerHandle) => {
    composer.current = h;
  }, []);

  const onDrop = (e: DragEvent<HTMLElement>) => {
    e.preventDefault();
    setDragging(false);
    const files = [...e.dataTransfer.files].filter((f) => f.type.startsWith("image/"));
    if (files.length) composer.current?.addFiles(files);
  };

  return (
    <aside
      className="chat"
      aria-label="Chat"
      style={{ width }}
      data-dragging={dragging ? "" : undefined}
      onDragOver={(e) => {
        if ([...e.dataTransfer.items].some((i) => i.kind === "file")) {
          e.preventDefault();
          setDragging(true);
        }
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setDragging(false);
      }}
      onDrop={onDrop}
    >
      <header className="chat-head">
        <select
          className="select chat-session"
          aria-label="Session"
          value={sessionId ?? ""}
          onChange={(e) => void store.selectSession(e.target.value || null)}
          disabled={chat.streaming}
        >
          <option value="">New session</option>
          {sessions.map((s) => (
            <option key={s.id} value={s.id}>
              {s.title} ({relative(s.updated)})
            </option>
          ))}
        </select>
        {sessionId ? (
          confirmDelete ? (
            <button type="button" className="button button-danger" onClick={() => void store.deleteSession(sessionId)}>
              Delete?
            </button>
          ) : (
            <button type="button" className="button button-quiet" onClick={() => setConfirmDelete(true)} disabled={chat.streaming} title="Delete this session">
              Delete
            </button>
          )
        ) : null}
        <button type="button" className="button" onClick={() => void store.newSession()} disabled={chat.streaming || sessionId === null}>
          New
        </button>
      </header>

      <MessageList messages={chat.messages} streaming={chat.streaming} circuit={active} />

      {chat.approval ? <ApprovalPrompt summary={chat.approval.summary} diff={chat.approval.diff} /> : null}

      <Composer onReady={onReady} />
      {dragging ? <div className="chat-drop">Drop images to attach them</div> : null}
    </aside>
  );
}

function MessageList({ messages, streaming, circuit }: { messages: ChatMessage[]; streaming: boolean; circuit: string | null }) {
  const store = useStoreApi();
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [messages]);

  if (messages.length === 0) {
    return (
      <div className="chat-scroll">
        <div className="chat-empty">
          <p className="empty-title">{circuit ? `Ask about ${baseName(circuit)}` : "No circuit open"}</p>
          <p className="empty-text">
            The agent can read the schematic, edit it, simulate and check specs. Every edit can be undone.
          </p>
          {circuit ? (
            <ul className="suggestions">
              {SUGGESTIONS.map((s) => (
                <li key={s}>
                  <button type="button" className="suggestion" onClick={() => store.setDraft(s)}>
                    {s}
                  </button>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </div>
    );
  }

  const lastAssistant = messages[messages.length - 1]?.role === "assistant" ? messages[messages.length - 1].id : null;
  const lastUserText = [...messages].reverse().find((m) => m.role === "user")?.parts.find((p) => p.kind === "text");

  return (
    <div
      className="chat-scroll"
      ref={scroller}
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
      }}
      role="log"
      aria-label="Messages"
      aria-busy={streaming}
    >
      <ol className="messages">
        {messages.map((m) => (
          <li key={m.id} className="message" data-role={m.role}>
            {m.role === "user" ? (
              <UserMessage message={m} />
            ) : (
              <AssistantMessage
                message={m}
                streaming={streaming && m.id === lastAssistant}
                onRetry={lastUserText && lastUserText.kind === "text" && m.id === lastAssistant ? () => void store.send(lastUserText.text) : undefined}
              />
            )}
          </li>
        ))}
      </ol>
    </div>
  );
}

function UserMessage({ message }: { message: ChatMessage }) {
  return (
    <div className="user-bubble">
      {message.parts.map((p, i) =>
        p.kind === "text" ? (
          <p key={i} className="user-text">
            {p.text}
          </p>
        ) : p.kind === "image" ? (
          <img key={i} className="user-image" alt="Attached" src={`data:${p.media_type};base64,${p.data_base64}`} />
        ) : null,
      )}
    </div>
  );
}

function AssistantMessage({ message, streaming, onRetry }: { message: ChatMessage; streaming: boolean; onRetry?: () => void }) {
  const last = message.parts[message.parts.length - 1];
  return (
    <div className="assistant">
      {message.parts.map((p, i) => {
        if (p.kind === "text") return <Markdown key={i} text={p.text} />;
        if (p.kind === "thinking") return <Thinking key={i} text={p.text} live={streaming && p === last} />;
        if (p.kind === "tool") return <ToolCard key={p.call.id} call={p.call} streaming={streaming} />;
        if (p.kind === "image") return <img key={i} className="user-image" alt="Model output" src={`data:${p.media_type};base64,${p.data_base64}`} />;
        return null;
      })}
      {streaming && message.parts.length === 0 ? <p className="assistant-wait">Working...</p> : null}
      {message.error ? (
        <div className="assistant-error" role="alert">
          <p>{message.error}</p>
          {onRetry && !streaming ? (
            <button type="button" className="button" onClick={onRetry}>
              Try again
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function Thinking({ text, live }: { text: string; live: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="thinking" data-open={open ? "" : undefined}>
      <button type="button" className="thinking-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className="thinking-label">{live ? "Thinking..." : "Thinking"}</span>
        {!open ? <span className="thinking-preview">{text}</span> : null}
        <span className="thinking-toggle">{open ? "Hide" : "Show"}</span>
      </button>
      {open ? <p className="thinking-body">{text}</p> : null}
    </div>
  );
}

function ApprovalPrompt({ summary, diff }: { summary: string; diff: string }) {
  const store = useStoreApi();
  const applyRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    applyRef.current?.focus();
  }, []);
  return (
    <section className="approval" aria-label="Edit waiting for approval" role="alertdialog" aria-describedby="approval-summary">
      <p className="approval-title">Apply this edit?</p>
      <p className="approval-summary" id="approval-summary">
        {summary}
      </p>
      <DiffBlock diff={diff} label="Proposed changes" />
      <div className="approval-actions">
        <button type="button" className="button" onClick={() => void store.answerApproval(false)}>
          Discard
        </button>
        <button type="button" className="button button-primary" ref={applyRef} onClick={() => void store.answerApproval(true)}>
          Apply
        </button>
      </div>
    </section>
  );
}
