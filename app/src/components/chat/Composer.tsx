import { useEffect, useLayoutEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent } from "react";
import type { Attachment } from "../../ipc/types";
import { baseName } from "../../lib/format";
import { hasMod } from "../../lib/keys";
import { useStore, useStoreApi } from "../../store/context";
import { ModelPicker } from "./ModelPicker";

const MAX_BYTES = 5 * 1024 * 1024;

export function readImage(file: File): Promise<Attachment> {
  return new Promise((resolve, reject) => {
    if (!file.type.startsWith("image/")) {
      reject(new Error(`${file.name || "That file"} is not an image.`));
      return;
    }
    if (file.size > MAX_BYTES) {
      reject(new Error(`${file.name || "That image"} is larger than 5 MB.`));
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      const comma = url.indexOf(",");
      resolve({ media_type: file.type, data_base64: url.slice(comma + 1), name: file.name || "pasted image" });
    };
    reader.onerror = () => reject(reader.error ?? new Error("The image could not be read."));
    reader.readAsDataURL(file);
  });
}

export interface ComposerHandle {
  addFiles: (files: File[]) => void;
}

export function Composer({ onReady }: { onReady?: (h: ComposerHandle) => void }) {
  const store = useStoreApi();
  const streaming = useStore((s) => s.chat.streaming);
  const active = useStore((s) => s.active);
  const draft = useStore((s) => s.draft);
  const [text, setText] = useState("");
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const ref = useRef<HTMLTextAreaElement>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [text]);

  useEffect(() => {
    if (!draft) return;
    setText(draft.text);
    ref.current?.focus();
  }, [draft]);

  const addFiles = (files: File[]) => {
    for (const f of files) {
      readImage(f)
        .then((a) => setAttachments((list) => [...list, a]))
        .catch((err: Error) => store.toast(err.message, "error"));
    }
  };

  useEffect(() => {
    onReady?.({ addFiles });
    // addFiles only closes over stable setters and the store.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onReady]);

  const submit = () => {
    if (streaming) return;
    if (!text.trim() && attachments.length === 0) return;
    void store.send(text, attachments);
    setText("");
    setAttachments([]);
  };

  // Ctrl/Cmd+Enter from anywhere in the window sends the draft.
  useEffect(() => {
    const onSend = () => {
      if (text.trim() || attachments.length) submit();
      else ref.current?.focus();
    };
    window.addEventListener("aispice:send", onSend);
    return () => window.removeEventListener("aispice:send", onSend);
  });

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Enter" || e.nativeEvent.isComposing) return;
    if (e.shiftKey) return;
    if (hasMod(e) || (!e.altKey && !e.ctrlKey && !e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      submit();
    }
  };

  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = [...e.clipboardData.items].filter((i) => i.kind === "file" && i.type.startsWith("image/")).map((i) => i.getAsFile()).filter((f): f is File => f !== null);
    if (files.length) {
      e.preventDefault();
      addFiles(files);
    }
  };

  const placeholder = active ? `Ask about ${baseName(active)}, or describe a change` : "Ask a question";

  return (
    <div className="composer">
      {attachments.length > 0 ? (
        <ul className="attachments" aria-label="Attached images">
          {attachments.map((a, i) => (
            <li key={i} className="attachment">
              <img src={`data:${a.media_type};base64,${a.data_base64}`} alt={a.name} />
              <button
                type="button"
                className="attachment-remove"
                aria-label={`Remove ${a.name}`}
                onClick={() => setAttachments((list) => list.filter((_, j) => j !== i))}
              >
                {"×"}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <textarea
        ref={ref}
        className="composer-input"
        rows={1}
        value={text}
        placeholder={placeholder}
        aria-label="Message"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
      />
      <div className="composer-bar">
        <ModelPicker />
        {streaming ? (
          <button type="button" className="button composer-send" onClick={() => void store.stop()}>
            Stop
          </button>
        ) : (
          <button type="button" className="button button-primary composer-send" onClick={submit} disabled={!text.trim() && attachments.length === 0}>
            Send
          </button>
        )}
      </div>
    </div>
  );
}
