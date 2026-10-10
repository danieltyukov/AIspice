import { duration } from "../lib/format";
import { PROVIDER_NAMES, SIMULATOR_NAMES } from "../lib/names";
import { useStore, useStoreApi } from "../store/context";
import "./StatusBar.css";

export function StatusBar() {
  const store = useStoreApi();
  const active = useStore((s) => s.active);
  const settings = useStore((s) => s.settings);
  const doctor = useStore((s) => s.doctor);
  const simulating = useStore((s) => s.simulating);
  const streaming = useStore((s) => s.chat.streaming);
  const steps = useStore((s) => s.chat.steps);
  const runs = useStore((s) => s.runs);
  const override = useStore((s) => s.simulator);
  const run = active ? runs[active] : undefined;

  const choice = override ?? settings?.simulator ?? "auto";
  const resolved = choice === "auto" ? doctor?.simulators.find((s) => s.found) : doctor?.simulators.find((s) => s.id === choice);
  const version = resolved?.version ? ` ${resolved.version}` : "";
  const simText =
    choice === "auto"
      ? resolved
        ? `Auto (${SIMULATOR_NAMES[resolved.id]}${version})`
        : "Auto"
      : `${SIMULATOR_NAMES[choice]}${version}`;
  const ltspice = doctor?.simulators.find((s) => s.id === "ltspice");

  let state: { tone: "idle" | "work" | "ok" | "error"; text: string };
  if (simulating) state = { tone: "work", text: "Simulating" };
  else if (streaming) state = { tone: "work", text: steps > 0 ? `Agent working, step ${steps + 1}` : "Agent working" };
  else if (run && run.errors.length > 0) state = { tone: "error", text: "Last run failed" };
  else if (run) state = { tone: "ok", text: `Last run ${duration(run.duration_ms)} on ${SIMULATOR_NAMES[run.simulator]}` };
  else state = { tone: "idle", text: "Ready" };

  return (
    <footer className="statusbar">
      <span className="status-item status-state" role="status" data-tone={state.tone}>
        <span className="status-dot" aria-hidden="true" />
        {state.text}
      </span>
      {active ? <span className="status-item mono status-path">{active}</span> : null}
      <span className="status-item" title="Simulator for the next run">
        {simText}
      </span>
      {ltspice ? (
        <span className="status-item" title={ltspice.path ?? undefined}>
          {ltspice.found ? (settings?.reload_ltspice ? "LTspice linked" : "LTspice found") : "LTspice not found"}
        </span>
      ) : null}
      <span className="status-spacer" />
      {settings ? (
        <span className="status-item" title="Model for the next turn">
          {PROVIDER_NAMES[settings.provider]} <span className="mono">{settings.model}</span>
        </span>
      ) : null}
      <button type="button" className="status-help" onClick={() => store.setShortcutsOpen(true)} aria-label="Keyboard shortcuts">
        Shortcuts <kbd>?</kbd>
      </button>
    </footer>
  );
}
