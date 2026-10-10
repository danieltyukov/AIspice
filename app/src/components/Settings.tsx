import { useState, useEffect, useCallback } from "react";
import "./Settings.css";

const PROVIDERS = [
  { id: "openai", label: "OpenAI", placeholder: "sk-..." },
  { id: "anthropic", label: "Anthropic / Claude", placeholder: "sk-ant-..." },
  { id: "google", label: "Google / Gemini", placeholder: "AIza..." },
  { id: "openrouter", label: "OpenRouter", placeholder: "sk-or-..." },
];

interface Props {
  open: boolean;
  onClose: () => void;
}

export function Settings({ open, onClose }: Props) {
  const [keys, setKeys] = useState<Record<string, string>>({});
  const [editedKeys, setEditedKeys] = useState<Record<string, string>>({});
  const [status, setStatus] = useState("");

  // Load keys when the modal opens
  useEffect(() => {
    if (!open) return;
    (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const masked = await invoke<Record<string, string>>("get_api_keys");
        setKeys(masked);
        setEditedKeys({});
        setStatus("");
      } catch {
        // Outside Tauri
        setKeys({});
      }
    })();
  }, [open]);

  const handleInputChange = useCallback(
    (provider: string, value: string) => {
      setEditedKeys((prev) => ({ ...prev, [provider]: value }));
    },
    [],
  );

  const handleDelete = useCallback(async (provider: string) => {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("remove_api_key", { provider });
      setKeys((prev) => {
        const next = { ...prev };
        delete next[provider];
        return next;
      });
      setEditedKeys((prev) => {
        const next = { ...prev };
        delete next[provider];
        return next;
      });
      setStatus(`Removed ${provider} key`);
    } catch {
      setStatus("Failed to remove key");
    }
  }, []);

  const handleSave = useCallback(async () => {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      let savedCount = 0;
      for (const [provider, key] of Object.entries(editedKeys)) {
        if (key.trim()) {
          await invoke("set_api_key", { provider, key: key.trim() });
          savedCount++;
        }
      }
      if (savedCount > 0) {
        // Reload masked keys
        const masked = await invoke<Record<string, string>>("get_api_keys");
        setKeys(masked);
        setEditedKeys({});
        setStatus(`Saved ${savedCount} key${savedCount > 1 ? "s" : ""}`);
      } else {
        setStatus("No changes to save");
      }
    } catch {
      setStatus("Failed to save keys");
    }
  }, [editedKeys]);

  if (!open) return null;

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div className="settings-card" onClick={(e) => e.stopPropagation()}>
        <div className="settings-header">
          <h2>Settings</h2>
          <button className="settings-close" onClick={onClose}>
            &#x2715;
          </button>
        </div>

        <div className="settings-body">
          <p className="settings-section-title">Provider API Keys</p>
          {PROVIDERS.map((p) => (
            <div key={p.id} className="settings-provider-row">
              <span className="settings-provider-name">{p.label}</span>
              <input
                type="password"
                className="settings-key-input"
                placeholder={
                  keys[p.id] ? `Current: ${keys[p.id]}` : p.placeholder
                }
                value={editedKeys[p.id] ?? ""}
                onChange={(e) => handleInputChange(p.id, e.target.value)}
              />
              {keys[p.id] && (
                <button
                  className="settings-delete-btn"
                  onClick={() => handleDelete(p.id)}
                  title={`Remove ${p.label} key`}
                >
                  Del
                </button>
              )}
            </div>
          ))}
        </div>

        {status && <p className="settings-status">{status}</p>}

        <div className="settings-footer">
          <button className="settings-cancel-btn" onClick={onClose}>
            Close
          </button>
          <button className="settings-save-btn" onClick={handleSave}>
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
