import { useEffect, useState } from "react";
import type { ModelInfo, ProviderId, ProviderStatus, Settings, SimulatorId } from "../ipc/types";
import { DEFAULT_BASE_URLS, keySourceText, PROVIDER_NAMES, PROVIDER_ORDER, SIMULATOR_NAMES, SIMULATOR_ORDER } from "../lib/names";
import { message } from "../store/store";
import { useStore, useStoreApi } from "../store/context";
import { Dialog } from "./Dialog";
import "./SettingsDialog.css";

const THEMES: Array<[Settings["theme"], string]> = [
  ["system", "System"],
  ["light", "Light"],
  ["dark", "Dark"],
];

export function SettingsDialog() {
  const store = useStoreApi();
  const saved = useStore((s) => s.settings);
  const keys = useStore((s) => s.keys);
  const [draft, setDraft] = useState<Settings | null>(saved);
  const [saving, setSaving] = useState(false);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [modelError, setModelError] = useState<string | null>(null);

  const provider = draft?.provider;
  const keyFor = keys.find((k) => k.id === provider);
  useEffect(() => {
    if (!provider) return;
    let live = true;
    setModels(null);
    setModelError(null);
    store.backend
      .models(provider)
      .then((m) => live && setModels(m))
      .catch((err) => live && setModelError(message(err)));
    return () => {
      live = false;
    };
  }, [provider, store, keyFor?.configured]);

  if (!draft) {
    return (
      <Dialog title="Settings" onClose={() => store.closeSettings()}>
        <p className="panel-note">Settings could not be loaded from the backend.</p>
      </Dialog>
    );
  }

  const patch = (p: Partial<Settings>) => setDraft({ ...draft, ...p });
  const save = async () => {
    setSaving(true);
    const ok = await store.saveSettings(draft);
    setSaving(false);
    if (ok) {
      store.closeSettings();
      store.toast("Settings saved.");
    }
  };
  const modelIds = models?.map((m) => m.id) ?? [];

  return (
    <Dialog
      title="Settings"
      description="Keys are stored in the system keychain as soon as you set them. Everything else applies when you save."
      onClose={() => store.closeSettings()}
      wide
      footer={
        <>
          <button type="button" className="button" onClick={() => store.closeSettings()}>
            Cancel
          </button>
          <button type="button" className="button button-primary" onClick={() => void save()} disabled={saving}>
            {saving ? "Saving..." : "Save"}
          </button>
        </>
      }
    >
      <div className="settings">
        <section className="settings-section" aria-labelledby="set-model">
          <h3 className="settings-title" id="set-model">
            Model
          </h3>
          <div className="settings-grid">
            <label className="field">
              <span className="field-label">Provider</span>
              <select
                className="select"
                value={draft.provider}
                onChange={(e) => patch({ provider: e.target.value as ProviderId })}
              >
                {PROVIDER_ORDER.map((id) => (
                  <option key={id} value={id}>
                    {PROVIDER_NAMES[id]}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="field-label">Model</span>
              {models && models.length > 0 ? (
                <select className="select" value={draft.model} onChange={(e) => patch({ model: e.target.value })}>
                  {modelIds.includes(draft.model) ? null : <option value={draft.model}>{draft.model || "Choose a model"}</option>}
                  {models.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.display_name === m.id ? m.id : `${m.display_name} (${m.id})`}
                    </option>
                  ))}
                </select>
              ) : (
                <input className="input" value={draft.model} onChange={(e) => patch({ model: e.target.value })} placeholder="model id" />
              )}
              <span className={modelError ? "field-note error-text" : "field-note"}>
                {modelError ? modelError : models === null ? "Loading the model list..." : `${models.length} models available`}
              </span>
            </label>
          </div>
        </section>

        <section className="settings-section" aria-labelledby="set-providers">
          <h3 className="settings-title" id="set-providers">
            Providers
          </h3>
          <ul className="providers">
            {PROVIDER_ORDER.map((id) => (
              <ProviderRow
                key={id}
                id={id}
                status={keys.find((k) => k.id === id) ?? { id, configured: false, source: null }}
                baseUrl={draft.base_urls[id] ?? ""}
                onBaseUrl={(url) => {
                  const base_urls = { ...draft.base_urls };
                  if (url.trim()) base_urls[id] = url.trim();
                  else delete base_urls[id];
                  patch({ base_urls });
                }}
              />
            ))}
          </ul>
        </section>

        <section className="settings-section" aria-labelledby="set-sim">
          <h3 className="settings-title" id="set-sim">
            Simulation
          </h3>
          <div className="field">
            <span className="field-label" id="set-sim-pref">
              Simulator
            </span>
            <div className="segmented" role="group" aria-labelledby="set-sim-pref">
              {(["auto", ...SIMULATOR_ORDER] as Array<SimulatorId | "auto">).map((id) => (
                <button key={id} type="button" className="segment" aria-pressed={draft.simulator === id} onClick={() => patch({ simulator: id })}>
                  {id === "auto" ? "Auto" : SIMULATOR_NAMES[id]}
                </button>
              ))}
            </div>
            <span className="field-note">Auto picks an installed simulator for each run.</span>
          </div>
          <label className="field">
            <span className="field-label">LTspice path</span>
            <input
              className="input mono"
              value={draft.ltspice_path ?? ""}
              placeholder="Found automatically"
              onChange={(e) => patch({ ltspice_path: e.target.value.trim() ? e.target.value : null })}
            />
          </label>
          <label className="check">
            <input type="checkbox" checked={draft.reload_ltspice} onChange={(e) => patch({ reload_ltspice: e.target.checked })} />
            Reload the schematic in LTspice after each edit
          </label>
        </section>

        <section className="settings-section" aria-labelledby="set-agent">
          <h3 className="settings-title" id="set-agent">
            Agent
          </h3>
          <div className="field">
            <span className="field-label" id="set-edit-mode">
              Edits
            </span>
            <div className="segmented" role="group" aria-labelledby="set-edit-mode">
              <button type="button" className="segment" aria-pressed={draft.edit_mode === "apply"} onClick={() => patch({ edit_mode: "apply" })}>
                Apply right away
              </button>
              <button type="button" className="segment" aria-pressed={draft.edit_mode === "ask"} onClick={() => patch({ edit_mode: "ask" })}>
                Ask before applying
              </button>
            </div>
            <span className="field-note">Either way, every edit is a snapshot you can undo.</span>
          </div>
          <label className="check">
            <input type="checkbox" checked={draft.thinking} onChange={(e) => patch({ thinking: e.target.checked })} />
            Let the model think before it answers (slower, uses more tokens)
          </label>
          <label className="field field-narrow">
            <span className="field-label">Max steps per turn</span>
            <input
              className="input mono"
              type="number"
              min={1}
              max={200}
              value={draft.max_steps}
              onChange={(e) => patch({ max_steps: Number(e.target.value) })}
              aria-invalid={!Number.isInteger(draft.max_steps) || draft.max_steps < 1 || draft.max_steps > 200 ? "true" : undefined}
            />
          </label>
        </section>

        <section className="settings-section" aria-labelledby="set-look">
          <h3 className="settings-title" id="set-look">
            Appearance
          </h3>
          <div className="segmented" role="group" aria-labelledby="set-look">
            {THEMES.map(([id, label]) => (
              <button key={id} type="button" className="segment" aria-pressed={draft.theme === id} onClick={() => patch({ theme: id })}>
                {label}
              </button>
            ))}
          </div>
        </section>
      </div>
    </Dialog>
  );
}

function ProviderRow({
  id,
  status,
  baseUrl,
  onBaseUrl,
}: {
  id: ProviderId;
  status: ProviderStatus;
  baseUrl: string;
  onBaseUrl: (url: string) => void;
}) {
  const store = useStoreApi();
  const [editing, setEditing] = useState(false);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [showUrl, setShowUrl] = useState(Boolean(baseUrl) || id === "custom");
  const needsKey = id !== "ollama";
  const removable = status.configured && (status.source === "keychain" || status.source === "file");

  const saveKey = async () => {
    if (!key.trim()) return;
    setBusy(true);
    const ok = await store.setKey(id, key.trim());
    setBusy(false);
    // The key never stays in the field, saved or not.
    setKey("");
    if (ok) setEditing(false);
  };

  return (
    <li className="provider" aria-label={PROVIDER_NAMES[id]}>
      <div className="provider-row">
        <span className="dot" data-on={status.configured ? "" : undefined} aria-hidden="true" />
        <span className="provider-name">{PROVIDER_NAMES[id]}</span>
        <span className="provider-status" data-testid={`key-status-${id}`}>
          {keySourceText(status)}
        </span>
        <span className="provider-actions">
          {needsKey && !editing ? (
            <button type="button" className="button" onClick={() => setEditing(true)}>
              {status.configured ? "Replace key" : "Set key"}
            </button>
          ) : null}
          {removable && !editing ? (
            <button type="button" className="button button-danger" onClick={() => void store.removeKey(id)}>
              Remove
            </button>
          ) : null}
          {!showUrl ? (
            <button type="button" className="button button-quiet" onClick={() => setShowUrl(true)}>
              Base URL
            </button>
          ) : null}
        </span>
      </div>
      {editing ? (
        <form
          className="provider-key"
          onSubmit={(e) => {
            e.preventDefault();
            void saveKey();
          }}
        >
          <input
            className="input mono"
            type="password"
            autoComplete="off"
            spellCheck={false}
            autoFocus
            aria-label={`${PROVIDER_NAMES[id]} API key`}
            placeholder="Paste the API key"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.stopPropagation();
                setKey("");
                setEditing(false);
              }
            }}
          />
          <button type="submit" className="button button-primary" disabled={busy || !key.trim()}>
            {busy ? "Saving..." : "Save key"}
          </button>
          <button
            type="button"
            className="button"
            onClick={() => {
              setKey("");
              setEditing(false);
            }}
          >
            Cancel
          </button>
        </form>
      ) : null}
      {showUrl ? (
        <label className="provider-url">
          <span className="field-label">Base URL</span>
          <input
            className="input mono"
            value={baseUrl}
            placeholder={DEFAULT_BASE_URLS[id]}
            onChange={(e) => onBaseUrl(e.target.value)}
            aria-label={`${PROVIDER_NAMES[id]} base URL`}
          />
        </label>
      ) : null}
    </li>
  );
}
