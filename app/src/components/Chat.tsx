import { useState, useRef, useEffect, useCallback } from "react";
import { useChat } from "../hooks/useChat";
import { Message } from "./Message";
import { Settings } from "./Settings";
import "./Chat.css";

interface ModelDef {
  id: string;
  label: string;
  provider: string;
}

const MODEL_GROUPS: { group: string; models: ModelDef[] }[] = [
  {
    group: "OpenAI",
    models: [
      { id: "gpt-4o", label: "GPT-4o", provider: "openai" },
      { id: "gpt-4o-mini", label: "GPT-4o Mini", provider: "openai" },
    ],
  },
  {
    group: "Anthropic",
    models: [
      {
        id: "claude-sonnet-4-20250514",
        label: "Claude Sonnet 4",
        provider: "anthropic",
      },
      {
        id: "claude-haiku-4-20250514",
        label: "Claude Haiku 4",
        provider: "anthropic",
      },
    ],
  },
  {
    group: "Google",
    models: [
      { id: "gemini-2.5-flash", label: "Gemini 2.5 Flash", provider: "google" },
      { id: "gemini-2.5-pro", label: "Gemini 2.5 Pro", provider: "google" },
    ],
  },
  {
    group: "OpenRouter",
    models: [
      {
        id: "anthropic/claude-sonnet-4-20250514",
        label: "Claude Sonnet 4 (OR)",
        provider: "openrouter",
      },
      {
        id: "google/gemini-2.5-flash-preview",
        label: "Gemini 2.5 Flash (OR)",
        provider: "openrouter",
      },
      {
        id: "openai/gpt-4o",
        label: "GPT-4o (OR)",
        provider: "openrouter",
      },
    ],
  },
  {
    group: "Ollama (Local)",
    models: [
      { id: "llama3", label: "Llama 3", provider: "ollama" },
      { id: "mistral", label: "Mistral", provider: "ollama" },
    ],
  },
];

// Build a flat lookup map from model id to provider
const MODEL_TO_PROVIDER: Record<string, string> = {};
for (const group of MODEL_GROUPS) {
  for (const m of group.models) {
    MODEL_TO_PROVIDER[m.id] = m.provider;
  }
}

interface Props {
  activeFile: string | null;
}

export function Chat({ activeFile }: Props) {
  const {
    messages,
    isLoading,
    sessions,
    selectedModel,
    selectedProvider,
    sendMessage,
    loadSessions,
    loadSession,
    createNewSession,
    setModel,
    setProvider,
  } = useChat();

  const [input, setInput] = useState("");
  const [selectedSessionId, setSelectedSessionId] = useState<string>("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Load sessions when activeFile changes
  useEffect(() => {
    if (activeFile) {
      loadSessions(activeFile);
    }
  }, [activeFile, loadSessions]);

  // Auto-scroll on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // Auto-resize textarea
  useEffect(() => {
    const textarea = textareaRef.current;
    if (textarea) {
      textarea.style.height = "auto";
      textarea.style.height = Math.min(textarea.scrollHeight, 160) + "px";
    }
  }, [input]);

  const handleModelChange = useCallback(
    (modelId: string) => {
      setModel(modelId);
      // Auto-set provider based on model
      const provider = MODEL_TO_PROVIDER[modelId];
      if (provider) {
        setProvider(provider);
      }
    },
    [setModel, setProvider],
  );

  const handleSend = useCallback(() => {
    if (!input.trim() || isLoading) return;
    const file = activeFile || "untitled.asc";
    sendMessage(input.trim(), file);
    setInput("");
  }, [input, isLoading, activeFile, sendMessage]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const handleSessionChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      const sessionId = e.target.value;
      setSelectedSessionId(sessionId);
      if (sessionId && activeFile) {
        loadSession(activeFile, sessionId);
      }
    },
    [activeFile, loadSession],
  );

  const handleNewSession = useCallback(() => {
    setSelectedSessionId("");
    createNewSession();
  }, [createNewSession]);

  return (
    <div className="chat">
      {/* Session selector */}
      <div className="chat__session-bar">
        <div className="chat__session-bar-inner">
          <select
            className="chat__session-select"
            value={selectedSessionId}
            onChange={handleSessionChange}
          >
            <option value="">
              New chat{sessions.length > 0 ? ` (${sessions.length} saved)` : ""}
            </option>
            {sessions.map((s) => (
              <option key={s.id} value={s.id}>
                {s.title} ({s.messageCount})
              </option>
            ))}
          </select>
          <button className="chat__session-new" onClick={handleNewSession}>
            + New
          </button>
        </div>
      </div>

      {/* Messages */}
      <div className="chat__messages">
        <div className="chat__messages-inner">
          {messages.length === 0 ? (
            <div className="chat__empty">
              <div className="chat__empty-icon">
                <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
                </svg>
              </div>
              <p className="chat__empty-title">
                {activeFile ? `Working on ${activeFile}` : "No file selected"}
              </p>
              <p className="chat__empty-hint">
                Ask about your circuit, request changes, or run simulations.
              </p>
            </div>
          ) : (
            messages.map((msg) => <Message key={msg.id} message={msg} />)
          )}
          <div ref={messagesEndRef} />
        </div>
      </div>

      {/* Input area */}
      <div className="chat__input-area">
        <div className="chat__input-area-inner">
          <div className="chat__selectors">
            <select
              className="chat__model-select"
              value={selectedModel}
              onChange={(e) => handleModelChange(e.target.value)}
            >
              {MODEL_GROUPS.map((g) => (
                <optgroup key={g.group} label={g.group}>
                  {g.models.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.label}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
            <span
              className="chat__provider-badge"
              title={`Provider: ${selectedProvider}`}
            >
              {selectedProvider}
            </span>
            <button
              className="settings-gear-btn"
              onClick={() => setSettingsOpen(true)}
              title="Settings"
            >
              &#9881;
            </button>
          </div>
          <div className="chat__input-row">
            <textarea
              ref={textareaRef}
              className="chat__input"
              placeholder="Ask about your circuit..."
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              rows={1}
            />
            <button
              className="chat__send"
              onClick={handleSend}
              disabled={isLoading || !input.trim()}
            >
              Send
            </button>
          </div>
        </div>
      </div>

      {/* Settings modal */}
      <Settings open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}
