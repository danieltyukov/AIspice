import type { SimulatorId } from "../ipc/types";
import { MOD } from "../lib/keys";
import { SIMULATOR_NAMES, SIMULATOR_ORDER } from "../lib/names";
import { useStore, useStoreApi } from "../store/context";
import "./SimulateControl.css";

/** The Simulate button with its simulator picker. The picker starts at the settings default. */
export function SimulateControl({ compact = false }: { compact?: boolean }) {
  const store = useStoreApi();
  const simulating = useStore((s) => s.simulating);
  const active = useStore((s) => s.active);
  const override = useStore((s) => s.simulator);
  const preferred = useStore((s) => s.settings?.simulator ?? "auto");
  const doctor = useStore((s) => s.doctor);
  const choice = override ?? preferred;

  const autoName = (() => {
    const found = doctor?.simulators.find((s) => s.found);
    return found ? `Auto (${SIMULATOR_NAMES[found.id]})` : "Auto";
  })();

  return (
    <div className="sim-control" data-compact={compact ? "" : undefined}>
      <button
        type="button"
        className="button button-primary sim-run"
        onClick={() => void store.simulate()}
        disabled={!active || simulating}
        title={`Simulate (${MOD}+R)`}
      >
        {simulating ? "Simulating..." : "Simulate"}
      </button>
      <select
        className="select sim-pick"
        aria-label="Simulator"
        value={choice}
        onChange={(e) => store.setSimulator(e.target.value as SimulatorId | "auto")}
        disabled={simulating}
      >
        <option value="auto">{autoName}</option>
        {SIMULATOR_ORDER.map((id) => {
          const status = doctor?.simulators.find((s) => s.id === id);
          return (
            <option key={id} value={id}>
              {SIMULATOR_NAMES[id]}
              {status && !status.found ? " (not found)" : ""}
            </option>
          );
        })}
      </select>
    </div>
  );
}
