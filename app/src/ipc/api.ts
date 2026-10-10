/*
 * The Tauri implementation of `Backend`.
 *
 * Commands (Rust name, then the argument keys this file sends). Argument keys
 * are camelCase because that is how Tauri maps snake_case Rust parameters by
 * default (run_id arrives from `runId`), so the Rust side needs no rename_all.
 * Results are the serde shapes in types.ts.
 *
 *   doctor            {}                                      -> Doctor
 *   open_project      { root }                                -> ProjectInfo
 *   recent_projects   {}                                      -> string[]
 *   list_circuits     {}                                      -> CircuitEntry[]
 *   read_circuit      { path, highlights }                    -> CircuitView
 *   new_circuit       { name }                                -> CircuitEntry
 *   simulate          { path, simulator }                     -> RunView      (simulator: SimulatorId | "auto" | null)
 *   waveform          { runId, dataset, signals, maxPoints }  -> WaveData     (maxPoints: number | null)
 *   check_specs       { path }                                -> SpecReport | null
 *   history           { path }                                -> Snapshot[]   (newest first)
 *   undo              { path }                                -> CircuitView
 *   redo              { path }                                -> CircuitView
 *   restore           { path, snapshotId }                    -> CircuitView
 *   open_in_ltspice   { path }                                -> null
 *   sessions          { circuit }                             -> SessionMeta[] (circuit: string | null)
 *   load_session      { id }                                  -> Session
 *   new_session       { circuit }                             -> SessionMeta
 *   delete_session    { id }                                  -> null
 *   send              { sessionId, text, attachments, circuit, onEvent }
 *                                                             -> null, returned when the turn ends;
 *                                                                onEvent is a Channel<AgentEvent>
 *   cancel            { sessionId }                           -> null
 *   approve           { requestId, approved }                 -> null
 *   settings          {}                                      -> Settings
 *   save_settings     { settings }                            -> null
 *   key_status        {}                                      -> ProviderStatus[]
 *   set_key           { provider, key }                       -> null
 *   remove_key        { provider }                            -> null
 *   models            { provider }                            -> ModelInfo[]
 *
 * Backend-initiated events arrive on the Tauri event "aispice://event" with a
 * BackendEvent payload.
 *
 * Choosing a folder does not go through a command: the front end calls the
 * dialog plugin (`dialog:allow-open`) and passes the result to open_project.
 */

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AgentEvent,
  Backend,
  BackendEvent,
  CircuitEntry,
  CircuitView,
  Doctor,
  ModelInfo,
  ProjectInfo,
  ProviderStatus,
  RunView,
  Session,
  SessionMeta,
  Settings,
  Snapshot,
  SpecReport,
  WaveData,
} from "./types";

export const BACKEND_EVENT = "aispice://event";

export function createTauriBackend(): Backend {
  return {
    doctor: () => invoke<Doctor>("doctor"),
    openProject: (root) => invoke<ProjectInfo>("open_project", { root }),
    pickFolder: async () => {
      const picked = await open({ directory: true, multiple: false, title: "Open a folder of circuits" });
      return typeof picked === "string" ? picked : null;
    },
    recentProjects: () => invoke<string[]>("recent_projects"),
    listCircuits: () => invoke<CircuitEntry[]>("list_circuits"),
    readCircuit: (path, highlights) => invoke<CircuitView>("read_circuit", { path, highlights: highlights ?? [] }),
    newCircuit: (name) => invoke<CircuitEntry>("new_circuit", { name }),
    pickNetlist: async () => {
      const picked = await open({
        multiple: false,
        directory: false,
        title: "Import a SPICE netlist",
        filters: [{ name: "SPICE netlist", extensions: ["cir", "net", "sp", "spi", "cki", "txt"] }],
      });
      return typeof picked === "string" ? picked : null;
    },
    importNetlist: (path) => invoke<CircuitEntry>("import_netlist", { path }),
    simulate: (path, simulator) => invoke<RunView>("simulate", { path, simulator: simulator ?? null }),
    waveform: (runId, dataset, signals, maxPoints) =>
      invoke<WaveData>("waveform", { runId, dataset, signals, maxPoints: maxPoints ?? null }),
    checkSpecs: (path) => invoke<SpecReport | null>("check_specs", { path }),
    history: (path) => invoke<Snapshot[]>("history", { path }),
    undo: (path) => invoke<CircuitView>("undo", { path }),
    redo: (path) => invoke<CircuitView>("redo", { path }),
    restore: (path, snapshotId) => invoke<CircuitView>("restore", { path, snapshotId }),
    openInLtspice: (path) => invoke<void>("open_in_ltspice", { path }),

    sessions: (circuit) => invoke<SessionMeta[]>("sessions", { circuit }),
    loadSession: (id) => invoke<Session>("load_session", { id }),
    newSession: (circuit) => invoke<SessionMeta>("new_session", { circuit }),
    deleteSession: (id) => invoke<void>("delete_session", { id }),
    send: async (sessionId, text, attachments, circuit, onEvent) => {
      const channel = new Channel<AgentEvent>();
      channel.onmessage = onEvent;
      await invoke<void>("send", { sessionId, text, attachments, circuit, onEvent: channel });
    },
    cancel: (sessionId) => invoke<void>("cancel", { sessionId }),
    approve: (requestId, approved) => invoke<void>("approve", { requestId, approved }),

    settings: () => invoke<Settings>("settings"),
    saveSettings: (settings) => invoke<void>("save_settings", { settings }),
    keyStatus: () => invoke<ProviderStatus[]>("key_status"),
    setKey: (provider, key) => invoke<void>("set_key", { provider, key }),
    removeKey: (provider) => invoke<void>("remove_key", { provider }),
    models: (provider) => invoke<ModelInfo[]>("models", { provider }),

    onEvent: (handler) => {
      let disposed = false;
      let unlisten: (() => void) | null = null;
      void listen<BackendEvent>(BACKEND_EVENT, (e) => handler(e.payload)).then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
  };
}
