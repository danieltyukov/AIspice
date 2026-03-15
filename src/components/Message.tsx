import { useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { ChatMessage } from "../hooks/useChat";
import { FileChange } from "./FileChange";
import "./Message.css";

interface MessageProps {
  message: ChatMessage;
}

export function Message({ message }: MessageProps) {
  const [thinkingOpen, setThinkingOpen] = useState(false);

  const isUser = message.role === "user";
  const isStreaming = message.isStreaming ?? false;
  const hasContent = message.content.length > 0;
  const hasThinking = !!message.thinking && message.thinking.length > 0;

  return (
    <div className={`message message--${message.role}`}>
      <div className="message__bubble">
        {/* Thinking block */}
        {!isUser && hasThinking && (
          <div className="message__thinking">
            <button
              className="message__thinking-toggle"
              onClick={() => setThinkingOpen((o) => !o)}
            >
              <span
                className={`message__thinking-chevron${thinkingOpen ? " message__thinking-chevron--open" : ""}`}
              >
                &#9654;
              </span>
              Thinking...
            </button>
            {thinkingOpen && (
              <div className="message__thinking-content">
                {message.thinking}
              </div>
            )}
          </div>
        )}

        {/* Content */}
        {isUser ? (
          <span>{message.content}</span>
        ) : isStreaming && !hasContent ? (
          <div className="message__loading">
            <span className="message__loading-dot">Thinking</span>
            <span className="message__loading-dot">.</span>
            <span className="message__loading-dot">.</span>
          </div>
        ) : (
          <>
            <ReactMarkdown remarkPlugins={[remarkGfm]}>
              {message.content}
            </ReactMarkdown>
            {isStreaming && <span className="message__cursor" />}
          </>
        )}

        {/* File changes */}
        {message.changes && message.changes.length > 0 && (
          <div className="message__changes">
            {message.changes.map((change, i) => (
              <FileChange key={i} change={change} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
