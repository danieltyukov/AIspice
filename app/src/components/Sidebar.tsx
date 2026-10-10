import { useMemo, useState } from "react";
import { relative } from "../lib/format";
import { SIMULATOR_NAMES } from "../lib/names";
import { useStore, useStoreApi } from "../store/context";
import { Mark } from "./Mark";
import "./Sidebar.css";

export function Sidebar({ width }: { width: number }) {
  const store = useStoreApi();
  const project = useStore((s) => s.project);
  const circuits = useStore((s) => s.circuits);
  const active = useStore((s) => s.active);
  const doctor = useStore((s) => s.doctor);
  const [filter, setFilter] = useState("");
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return q ? circuits.filter((c) => c.name.toLowerCase().includes(q) || c.path.toLowerCase().includes(q)) : circuits;
  }, [circuits, filter]);

  const now = Date.now();

  const submit = async () => {
    const clean = name.trim();
    if (!clean) {
      setNameError("Give the circuit a name.");
      return;
    }
    if (!/^[A-Za-z0-9_][A-Za-z0-9_.-]*$/.test(clean)) {
      setNameError("Letters, digits, dots, dashes and underscores only.");
      return;
    }
    if (await store.createCircuit(clean)) {
      setCreating(false);
      setName("");
      setNameError(null);
    }
  };

  return (
    <nav className="sidebar" aria-label="Circuits" style={{ width }}>
      <div className="side-brand">
        <Mark size={20} />
        <span className="side-word">aispice</span>
        <button type="button" className="button button-quiet side-close" onClick={() => store.closeProject()} title="Back to the start screen">
          Projects
        </button>
      </div>

      {project ? (
        <div className="side-project">
          <p className="side-project-name">{project.name}</p>
          <p className="side-project-root mono" title={project.root}>
            <bdi>{project.root}</bdi>
          </p>
        </div>
      ) : null}

      <div className="side-search">
        <input
          className="input side-filter"
          type="search"
          placeholder="Filter circuits"
          aria-label="Filter circuits"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") setFilter("");
          }}
        />
      </div>

      <div className="side-list-wrap">
        {circuits.length === 0 ? (
          <p className="side-empty">No .asc files in this folder yet.</p>
        ) : shown.length === 0 ? (
          <p className="side-empty">No circuit matches "{filter}".</p>
        ) : (
          <ul className="side-list">
            {shown.map((c) => (
              <li key={c.path}>
                <button
                  type="button"
                  className="side-item"
                  aria-current={c.path === active ? "true" : undefined}
                  onClick={() => void store.selectCircuit(c.path)}
                  title={c.path}
                >
                  <span className="side-item-name">{c.name}</span>
                  <time className="side-item-time" dateTime={new Date(c.modified).toISOString()}>
                    {relative(c.modified, now)}
                  </time>
                </button>
              </li>
            ))}
          </ul>
        )}

        {creating ? (
          <form
            className="side-new"
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <input
              className="input"
              autoFocus
              aria-label="New circuit name"
              placeholder="name.asc"
              value={name}
              aria-invalid={nameError ? "true" : undefined}
              onChange={(e) => {
                setName(e.target.value);
                setNameError(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setCreating(false);
                  setName("");
                  setNameError(null);
                }
              }}
            />
            {nameError ? <p className="side-error">{nameError}</p> : null}
            <div className="side-new-actions">
              <button type="submit" className="button button-primary">
                Create
              </button>
              <button
                type="button"
                className="button"
                onClick={() => {
                  setCreating(false);
                  setName("");
                  setNameError(null);
                }}
              >
                Cancel
              </button>
            </div>
          </form>
        ) : (
          <button type="button" className="side-add" onClick={() => setCreating(true)}>
            New circuit
          </button>
        )}
      </div>

      <div className="side-foot">
        <p className="section-title">Simulators</p>
        {doctor ? (
          <ul className="side-sims">
            {doctor.simulators.map((s) => (
              <li key={s.id} className="side-sim" title={s.found ? (s.path ?? "") : (s.notes[0] ?? "Not found")}>
                <span className="dot" data-on={s.found ? "" : undefined} aria-hidden="true" />
                <span className="side-sim-name">{SIMULATOR_NAMES[s.id]}</span>
                <span className="side-sim-ver mono">{s.found ? (s.version ?? "") : "not found"}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="side-empty">Checking...</p>
        )}
        <button type="button" className="button side-settings" onClick={() => store.openSettings()}>
          Settings
        </button>
      </div>
    </nav>
  );
}
