import { useState, useCallback, useRef, useEffect } from "react";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  thinking?: string;
  isStreaming?: boolean;
  changes?: ChangeInfo[];
}

export interface ChangeInfo {
  component?: string;
  description: string;
  filename: string;
}

export interface SessionMeta {
  id: string;
  title: string;
  createdAt: string;
  messageCount: number;
}

interface ChatStreamEvent {
  event_type: "thinking" | "text" | "done" | "error";
  data: string;
}

function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

export function useChat() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [sessions, setSessions] = useState<SessionMeta[]>([]);
  const [selectedModel, setSelectedModel] = useState("gemini-2.5-flash");
  const [selectedProvider, setSelectedProvider] = useState("google");

  const prevIsLoadingRef = useRef(false);
  const currentFileRef = useRef<string | null>(null);

  // Auto-persist when streaming finishes (isLoading goes from true to false)
  useEffect(() => {
    if (prevIsLoadingRef.current && !isLoading && currentFileRef.current) {
      const filename = currentFileRef.current;
      (async () => {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          await invoke("save_chat_session", {
            filename,
            messages,
          });
        } catch {
          // Outside Tauri or save failed — ignore
        }
      })();
    }
    prevIsLoadingRef.current = isLoading;
  }, [isLoading, messages]);

  const sendMessage = useCallback(
    async (text: string, activeFile: string) => {
      if (!text.trim() || isLoading) return;

      currentFileRef.current = activeFile;

      const userMsg: ChatMessage = {
        id: generateId(),
        role: "user",
        content: text.trim(),
      };

      const assistantId = generateId();
      const assistantMsg: ChatMessage = {
        id: assistantId,
        role: "assistant",
        content: "",
        thinking: "",
        isStreaming: true,
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setIsLoading(true);

      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const { Channel } = await import("@tauri-apps/api/core");

        const channel = new Channel<ChatStreamEvent>();

        channel.onmessage = (event: ChatStreamEvent) => {
          switch (event.event_type) {
            case "thinking":
              setMessages((prev) =>
                prev.map((m) =>
                  m.id === assistantId
                    ? { ...m, thinking: (m.thinking || "") + event.data }
                    : m,
                ),
              );
              break;

            case "text":
              setMessages((prev) =>
                prev.map((m) =>
                  m.id === assistantId
                    ? { ...m, content: m.content + event.data }
                    : m,
                ),
              );
              break;

            case "done": {
              let changes: ChangeInfo[] | undefined;
              try {
                const parsed = JSON.parse(event.data);
                if (parsed.changes && Array.isArray(parsed.changes)) {
                  changes = parsed.changes;
                }
                // Use full_text if present and message content is empty
                if (parsed.full_text) {
                  setMessages((prev) =>
                    prev.map((m) =>
                      m.id === assistantId
                        ? {
                            ...m,
                            content: m.content || parsed.full_text,
                            isStreaming: false,
                            changes,
                          }
                        : m,
                    ),
                  );
                  setIsLoading(false);
                  return;
                }
              } catch {
                // data might not be JSON
              }
              setMessages((prev) =>
                prev.map((m) =>
                  m.id === assistantId
                    ? { ...m, isStreaming: false, changes }
                    : m,
                ),
              );
              setIsLoading(false);
              break;
            }

            case "error":
              setMessages((prev) =>
                prev.map((m) =>
                  m.id === assistantId
                    ? {
                        ...m,
                        content: m.content || `Error: ${event.data}`,
                        isStreaming: false,
                      }
                    : m,
                ),
              );
              setIsLoading(false);
              break;
          }
        };

        // Build history from previous messages (excluding the ones we just added)
        const history = messages.map((m) => ({
          role: m.role,
          content: m.content,
        }));

        await invoke("send_chat_message_stream", {
          message: text.trim(),
          activeFile: activeFile,
          history,
          model: selectedModel,
          provider: selectedProvider,
          onEvent: channel,
        });
      } catch {
        // Outside Tauri — provide a mock response
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantId
              ? {
                  ...m,
                  content:
                    "Chat backend is not available. Running outside Tauri.",
                  isStreaming: false,
                }
              : m,
          ),
        );
        setIsLoading(false);
      }
    },
    [isLoading, selectedModel, selectedProvider],
  );

  const loadSessions = useCallback(async (filename: string) => {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<SessionMeta[]>("list_chat_sessions", {
        filename,
      });
      setSessions(result);
    } catch {
      setSessions([]);
    }
  }, []);

  const loadSession = useCallback(
    async (filename: string, sessionId: string) => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const result = await invoke<ChatMessage[]>("load_chat_session", {
          filename,
          sessionId,
        });
        setMessages(result);
      } catch {
        // ignore
      }
    },
    [],
  );

  const createNewSession = useCallback(() => {
    setMessages([]);
  }, []);

  const setModel = useCallback((model: string) => {
    setSelectedModel(model);
  }, []);

  const setProvider = useCallback((provider: string) => {
    setSelectedProvider(provider);
  }, []);

  return {
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
  };
}
