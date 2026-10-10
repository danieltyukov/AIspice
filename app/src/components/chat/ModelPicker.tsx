import { useEffect, useRef, useState } from "react";
import type { ModelInfo, ProviderId } from "../../ipc/types";
import { PROVIDER_NAMES, PROVIDER_ORDER } from "../../lib/names";
import { message } from "../../store/store";
import { useStore, useStoreApi } from "../../store/context";

/** The provider and model for the next turn, with the provider's live model list. */
export function ModelPicker() {
  const store = useStoreApi();
  const settings = useStore((s) => s.settings);
  const keys = useStore((s) => s.keys);
  const [open, setOpen] = useState(false);
  const [provider, setProvider] = useState<ProviderId>(settings?.provider ?? "anthropic");
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    if (open && settings) setProvider(settings.provider);
  }, [open, settings]);

  useEffect(() => {
    if (!open) return;
    let live = true;
    setModels(null);
    setError(null);
    store.backend
      .models(provider)
      .then((m) => {
        if (live) setModels(m);
      })
      .catch((err) => {
        if (live) setError(message(err));
      });
    return () => {
      live = false;
    };
  }, [open, provider, store]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  if (!settings) return null;

  const choose = async (model: string) => {
    const ok = await store.saveSettings({ ...settings, provider, model });
    if (ok) setOpen(false);
  };

  return (
    <div
      className="model-picker"
      ref={rootRef}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.stopPropagation();
          setOpen(false);
        }
      }}
    >
      <button
        type="button"
        className="model-button"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        title="Change the model"
      >
        <span className="model-provider">{PROVIDER_NAMES[settings.provider]}</span>
        <span className="model-id mono">{settings.model}</span>
      </button>
      {open ? (
        <div className="model-menu" role="dialog" aria-label="Choose a model">
          <label className="model-row">
            <span className="section-title">Provider</span>
            <select className="select" value={provider} onChange={(e) => setProvider(e.target.value as ProviderId)} aria-label="Provider">
              {PROVIDER_ORDER.map((id) => {
                const status = keys.find((k) => k.id === id);
                return (
                  <option key={id} value={id}>
                    {PROVIDER_NAMES[id]}
                    {status && !status.configured ? " (no key)" : ""}
                  </option>
                );
              })}
            </select>
          </label>
          {error ? (
            <p className="model-note error-text">{error}</p>
          ) : models === null ? (
            <p className="model-note">Loading models...</p>
          ) : models.length === 0 ? (
            <p className="model-note">This provider listed no models.</p>
          ) : (
            <ul className="model-list" role="listbox" aria-label="Models" ref={listRef}>
              {models.map((m) => {
                const selected = provider === settings.provider && m.id === settings.model;
                return (
                  <li key={m.id} role="option" aria-selected={selected}>
                    <button type="button" className="model-option" onClick={() => void choose(m.id)} data-selected={selected ? "" : undefined}>
                      <span className="model-option-name">{m.display_name}</span>
                      <span className="model-option-id mono">{m.id}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
          <button
            type="button"
            className="link model-settings"
            onClick={() => {
              setOpen(false);
              store.openSettings();
            }}
          >
            Keys and endpoints
          </button>
        </div>
      ) : null}
    </div>
  );
}
