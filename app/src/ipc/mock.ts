/*
 * The in-browser backend: used when the Tauri shell is absent (a browser
 * during development, Playwright, vitest). It implements the whole contract
 * in memory with a fake project of three circuits, real numbers for the
 * waveforms, undo history, sessions, and a scripted agent that streams.
 *
 * Delays imitate a real machine; pass `fast: true` (or open the app with
 * `?fast`) to run every delay as a zero timeout, keeping the event order.
 */

import { applyEvent } from "../chat/reduce";
import { readJson, writeJson } from "../lib/storage";
import { emptyCircuit, seedCircuits, type CircuitDef, type CircuitState } from "./mock/circuits";
import { script, type AgentHost, type EditPlan } from "./mock/agent";
import { unifiedDiff } from "./mock/diff";
import type {
  AgentEvent,
  Backend,
  BackendEvent,
  ChatMessage,
  CircuitEntry,
  CircuitView,
  Doctor,
  Highlight,
  ModelInfo,
  ProviderId,
  ProviderStatus,
  RunView,
  Session,
  SessionMeta,
  Settings,
  SimulatorId,
  Snapshot,
} from "./types";

export interface MockOptions {
  fast?: boolean;
  /** Fixed clock for tests. */
  now?: () => number;
}

interface Version {
  id: string;
  time: number;
  summary: string;
  state: CircuitState;
}

interface MockCircuit {
  path: string;
  def: CircuitDef;
  versions: Version[];
  cursor: number;
  modified: number;
  lastRun: string | null;
}

const MODELS: Record<ProviderId, ModelInfo[]> = {
  anthropic: [
    { id: "claude-opus-5-5", display_name: "Claude Opus 5.5", context_window: 200000, supports_tools: true, supports_vision: true },
    { id: "claude-sonnet-5-5", display_name: "Claude Sonnet 5.5", context_window: 200000, supports_tools: true, supports_vision: true },
    { id: "claude-haiku-5-5", display_name: "Claude Haiku 5.5", context_window: 200000, supports_tools: true, supports_vision: true },
  ],
  openai: [
    { id: "gpt-5", display_name: "GPT-5", context_window: 400000, supports_tools: true, supports_vision: true },
    { id: "gpt-5-mini", display_name: "GPT-5 mini", context_window: 400000, supports_tools: true, supports_vision: true },
  ],
  google: [
    { id: "gemini-2.5-pro", display_name: "Gemini 2.5 Pro", context_window: 1000000, supports_tools: true, supports_vision: true },
    { id: "gemini-3.1-flash-lite", display_name: "Gemini 3.1 Flash-Lite", context_window: 1000000, supports_tools: true, supports_vision: true },
  ],
  openrouter: [
    { id: "anthropic/claude-sonnet-5.5", display_name: "Anthropic: Claude Sonnet 5.5", supports_tools: true },
    { id: "qwen/qwen3-coder", display_name: "Qwen: Qwen3 Coder", supports_tools: true },
    { id: "deepseek/deepseek-chat-v3.1", display_name: "DeepSeek: V3.1", supports_tools: true },
  ],
  ollama: [
    { id: "qwen3:14b", display_name: "qwen3:14b", context_window: 40960, supports_tools: true, supports_vision: false },
    { id: "llama3.1:8b", display_name: "llama3.1:8b", context_window: 131072, supports_tools: true, supports_vision: false },
  ],
  custom: [],
};

const SETTINGS_KEY = "mock.settings";

const PROVIDERS: ProviderId[] = ["anthropic", "openai", "google", "openrouter", "ollama", "custom"];

export function isFastMode(): boolean {
  try {
    if (typeof location !== "undefined" && new URLSearchParams(location.search).has("fast")) return true;
  } catch {
    // No location (tests): fall through.
  }
  return import.meta.env?.MODE === "test" || import.meta.env?.VITE_MOCK_FAST === "1";
}

export function createMockBackend(options: MockOptions = {}): Backend {
  const fast = options.fast ?? isFastMode();
  const now = options.now ?? (() => Date.now());
  const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, fast ? 0 : ms));

  let idCounter = 0;
  const nextId = (prefix: string) => `${prefix}-${(++idCounter).toString(36)}`;

  const listeners = new Set<(e: BackendEvent) => void>();
  const emit = (e: BackendEvent) => {
    for (const l of listeners) l(e);
  };

  let projectRoot: string | null = null;
  const circuits = new Map<string, MockCircuit>();
  const t0 = now();
  for (const seed of seedCircuits()) {
    const versions = seed.versions.map((v) => ({ id: nextId("snap"), time: t0 - v.ageMs, summary: v.summary, state: v.state }));
    circuits.set(seed.path, {
      path: seed.path,
      def: seed.def,
      versions,
      cursor: versions.length - 1,
      modified: versions[versions.length - 1].time,
      lastRun: null,
    });
  }

  const runs = new Map<string, { run: RunView; wave: ReturnType<CircuitDef["simulate"]>["wave"] }>();

  // Settings outlive a reload, as they do with the real backend's config file.
  let settings: Settings = {
    provider: "anthropic",
    model: "claude-opus-5-5",
    base_urls: {},
    simulator: "auto",
    ltspice_path: null,
    reload_ltspice: true,
    edit_mode: "apply",
    thinking: true,
    max_steps: 24,
    theme: "system",
    ...readJson<Partial<Settings>>(SETTINGS_KEY, {}),
  };

  const keys = new Map<ProviderId, ProviderStatus["source"]>([
    ["anthropic", "keychain"],
    ["openrouter", "env"],
  ]);
  const keyStatus = (): ProviderStatus[] =>
    PROVIDERS.map((id) => {
      if (id === "ollama") return { id, configured: true, source: null };
      const source = keys.get(id) ?? null;
      return { id, configured: source !== null, source };
    });

  const sessions = new Map<string, Session>();
  {
    const t = t0 - 22 * 3_600_000;
    const id = nextId("sess");
    const messages: ChatMessage[] = [
      { id: nextId("msg"), role: "user", parts: [{ kind: "text", text: "What corner frequency does this filter have?" }], time: t },
      {
        id: nextId("msg"),
        role: "assistant",
        parts: [{ kind: "text", text: "With R1 = 2.2k and C1 = 100n the corner is **723 Hz**: f = 1/(2 pi R C)." }],
        time: t + 4000,
      },
    ];
    sessions.set(id, {
      meta: { id, title: "What corner frequency does this filter have?", circuit: "rc_lowpass.asc", updated: t + 4000, message_count: 2 },
      messages,
    });
  }
  const running = new Map<string, { cancelled: boolean }>();
  const approvals = new Map<string, (ok: boolean) => void>();

  const requireProject = () => {
    if (projectRoot === null) throw new Error("No project is open.");
  };

  const getCircuit = (path: string): MockCircuit => {
    requireProject();
    const c = circuits.get(path);
    if (!c) throw new Error(`No circuit named ${path} in this project.`);
    return c;
  };

  const entry = (c: MockCircuit): CircuitEntry => ({
    path: c.path,
    name: c.path.split("/").pop() ?? c.path,
    modified: c.modified,
    last_run: c.lastRun,
  });

  const entries = () => [...circuits.values()].map(entry).sort((a, b) => a.name.localeCompare(b.name));

  const current = (c: MockCircuit) => c.versions[c.cursor];

  const view = (c: MockCircuit, highlights: Highlight[] = []): CircuitView => {
    const built = c.def.build(current(c).state);
    let svg = built.svg;
    // The real renderer may mark highlights in the SVG; the mock does too, so
    // the UI handles both an annotated SVG and a plain one.
    for (const h of highlights) {
      svg = svg.replace(`data-inst="${h.inst}"`, `data-inst="${h.inst}" data-hl="${h.kind}"`);
    }
    return {
      path: c.path,
      summary: built.summary,
      svg,
      asc: built.asc,
      netlist: built.netlist,
      can_undo: c.cursor > 0,
      can_redo: c.cursor < c.versions.length - 1,
    };
  };

  const resolveSim = (choice: SimulatorId | "auto" | undefined): SimulatorId => {
    const pick = choice ?? settings.simulator;
    return pick === "auto" ? "ngspice" : pick;
  };

  const doSimulate = (c: MockCircuit, simulator: SimulatorId): RunView => {
    if (simulator === "xyce" || simulator === "spectre") {
      const run: RunView = {
        run_id: nextId("run"),
        circuit: c.path,
        simulator,
        datasets: [],
        measurements: [],
        op: {},
        errors: [
          simulator === "xyce"
            ? "Xyce was not found. Install Xyce 7.9 or newer, or pick another simulator."
            : "Spectre needs an SSH host. Set one in the config file, or pick another simulator.",
        ],
        warnings: [],
        duration_ms: 12,
        log: `${simulator}: not available on this machine\n`,
      };
      runs.set(run.run_id, { run, wave: () => ({ kind: "other", x_name: "", x_unit: "", log_x: false, x: [], series: [] }) });
      return run;
    }
    const out = c.def.simulate(current(c).state, simulator, `${projectRoot}/${c.path}`);
    const run: RunView = {
      run_id: nextId("run"),
      circuit: c.path,
      simulator,
      datasets: out.datasets,
      measurements: out.measurements,
      op: out.op,
      errors: out.errors,
      warnings: out.warnings,
      duration_ms: simulator === "ltspice" ? out.duration_ms * 2.4 : out.duration_ms,
      log: out.log,
    };
    runs.set(run.run_id, { run, wave: out.wave });
    c.lastRun = run.run_id;
    return run;
  };

  const pushVersion = (c: MockCircuit, summary: string, state: CircuitState): string => {
    const before = current(c).id;
    c.versions = c.versions.slice(0, c.cursor + 1);
    c.versions.push({ id: nextId("snap"), time: now(), summary, state });
    c.cursor = c.versions.length - 1;
    c.modified = now();
    return before;
  };

  const touchSession = (s: Session) => {
    s.meta = { ...s.meta, updated: now(), message_count: s.messages.length };
  };

  const backend: Backend = {
    async doctor(): Promise<Doctor> {
      await wait(120);
      return {
        version: "0.2.0",
        platform: "linux-x86_64",
        simulators: [
          { id: "ngspice", found: true, version: "44.2", path: "/usr/bin/ngspice", notes: [] },
          {
            id: "ltspice",
            found: true,
            version: "26.1",
            path: "/home/you/.wine/drive_c/Program Files/ADI/LTspice/LTspice.exe",
            notes: ["Runs under Wine 9.0."],
          },
          { id: "xyce", found: false, version: null, path: null, notes: ["Optional. Install Xyce 7.9 or newer for large transient runs."] },
          { id: "spectre", found: false, version: null, path: null, notes: ["Optional. Needs an SSH host in the config file."] },
        ],
        providers: keyStatus(),
        ltspice_lib: "/home/you/.wine/drive_c/users/you/AppData/Local/LTspice/lib",
      };
    },

    async openProject(root) {
      await wait(150);
      projectRoot = root.replace(/[\\/]+$/, "");
      const name = projectRoot.split(/[\\/]/).pop() || projectRoot;
      return { root: projectRoot, name, circuits: entries() };
    },

    async pickFolder() {
      await wait(60);
      return "/home/you/circuits/filters";
    },

    async recentProjects() {
      await wait(40);
      return ["/home/you/circuits/filters", "/home/you/work/sensor-frontend", "/home/you/teaching/ee2-labs"];
    },

    async listCircuits() {
      requireProject();
      await wait(40);
      return entries();
    },

    async readCircuit(path, highlights) {
      const c = getCircuit(path);
      await wait(80);
      return view(c, highlights ?? []);
    },

    async newCircuit(name) {
      requireProject();
      await wait(80);
      const clean = name.trim().replace(/\.asc$/i, "");
      if (!/^[A-Za-z0-9_][A-Za-z0-9_.-]*$/.test(clean)) {
        throw new Error("Use letters, digits, dots, dashes and underscores only.");
      }
      const path = `${clean}.asc`;
      if (circuits.has(path)) throw new Error(`${path} already exists.`);
      const c: MockCircuit = {
        path,
        def: emptyCircuit(path),
        versions: [{ id: nextId("snap"), time: now(), summary: `Created ${path}`, state: { values: {}, flags: {} } }],
        cursor: 0,
        modified: now(),
        lastRun: null,
      };
      circuits.set(path, c);
      emit({ type: "circuits_listed", circuits: entries() });
      return entry(c);
    },

    async simulate(path, simulator) {
      const c = getCircuit(path);
      const sim = resolveSim(simulator);
      const run = doSimulate(c, sim);
      await wait(run.duration_ms);
      return run;
    },

    async waveform(runId, dataset, signals, maxPoints) {
      const r = runs.get(runId);
      if (!r) throw new Error(`Run ${runId} is no longer available. Simulate again.`);
      await wait(30);
      return r.wave(dataset, signals, maxPoints);
    },

    async checkSpecs(path) {
      const c = getCircuit(path);
      await wait(200);
      return c.def.specs(current(c).state);
    },

    async history(path) {
      const c = getCircuit(path);
      await wait(30);
      const list: Snapshot[] = c.versions.map((v, i) => ({ id: v.id, time: v.time, summary: v.summary, current: i === c.cursor }));
      return list.reverse();
    },

    async undo(path) {
      const c = getCircuit(path);
      if (c.cursor === 0) throw new Error("Nothing to undo.");
      c.cursor -= 1;
      c.modified = now();
      await wait(60);
      return view(c);
    },

    async redo(path) {
      const c = getCircuit(path);
      if (c.cursor >= c.versions.length - 1) throw new Error("Nothing to redo.");
      c.cursor += 1;
      c.modified = now();
      await wait(60);
      return view(c);
    },

    async restore(path, snapshotId) {
      const c = getCircuit(path);
      const index = c.versions.findIndex((v) => v.id === snapshotId);
      if (index < 0) throw new Error("That snapshot no longer exists.");
      c.cursor = index;
      c.modified = now();
      await wait(60);
      return view(c);
    },

    async openInLtspice(path) {
      getCircuit(path);
      await wait(100);
    },

    async sessions(circuit) {
      await wait(30);
      return [...sessions.values()]
        .map((s) => s.meta)
        .filter((m) => m.circuit === circuit)
        .sort((a, b) => b.updated - a.updated);
    },

    async loadSession(id) {
      const s = sessions.get(id);
      if (!s) throw new Error("That session no longer exists.");
      await wait(40);
      return structuredClone(s);
    },

    async newSession(circuit) {
      await wait(30);
      const id = nextId("sess");
      const meta: SessionMeta = { id, title: "New session", circuit, updated: now(), message_count: 0 };
      sessions.set(id, { meta, messages: [] });
      return meta;
    },

    async deleteSession(id) {
      await wait(30);
      sessions.delete(id);
    },

    async send(sessionId, text, attachments, circuit, onEvent) {
      const session = sessions.get(sessionId);
      if (!session) throw new Error("That session no longer exists.");
      const provider = keyStatus().find((p) => p.id === settings.provider);
      if (!provider?.configured) {
        throw new Error(`No API key for ${settings.provider}. Add one in Settings.`);
      }

      const user: ChatMessage = {
        id: nextId("msg"),
        role: "user",
        parts: [
          ...(text ? [{ kind: "text" as const, text }] : []),
          ...attachments.map((a) => ({ kind: "image" as const, media_type: a.media_type, data_base64: a.data_base64 })),
        ],
        time: now(),
      };
      session.messages.push(user);
      if (session.meta.title === "New session" && text) {
        session.meta = { ...session.meta, title: text.length > 60 ? `${text.slice(0, 57)}...` : text };
      }

      let assistant: ChatMessage = { id: nextId("msg"), role: "assistant", parts: [], time: now() };
      const token = { cancelled: false };
      running.set(sessionId, token);
      let steps = 0;
      const send = (e: AgentEvent) => {
        assistant = applyEvent(assistant, e);
        onEvent(e);
      };

      const c = circuit ? circuits.get(circuit) : undefined;
      const host: AgentHost = {
        circuit: c ? c.path : null,
        editMode: settings.edit_mode,
        state: () => (c ? current(c).state : null),
        summary: () => (c ? c.def.build(current(c).state).summary : null),
        preview: (plan: EditPlan) => (c ? unifiedDiff(c.def.build(current(c).state).asc, c.def.build(plan.next).asc, c.path) : ""),
        applyEdit: (plan: EditPlan) => {
          const before = c!.def.build(current(c!).state).asc;
          const snapshotBefore = pushVersion(c!, plan.summary, plan.next);
          const after = c!.def.build(plan.next).asc;
          emit({ type: "circuit_changed", path: c!.path, external: false });
          return { diff: unifiedDiff(before, after, c!.path), snapshotBefore };
        },
        simulate: () => doSimulate(c!, resolveSim(undefined)),
        specs: () => c!.def.specs(current(c!).state),
        approve: (summary, diff) =>
          new Promise<boolean>((resolve) => {
            const request_id = nextId("approval");
            approvals.set(request_id, resolve);
            send({ type: "approval_request", request_id, summary, diff });
          }),
      };

      const stream = async (kind: "text_delta" | "thinking_delta", body: string) => {
        const chunks = body.match(/\S+\s*|\s+/g) ?? [body];
        for (let i = 0; i < chunks.length; i += 2) {
          if (token.cancelled) return;
          send({ type: kind, text: chunks.slice(i, i + 2).join("") });
          await wait(kind === "thinking_delta" ? 14 : 22);
        }
      };

      try {
        await wait(250);
        for (const step of script(host, text)) {
          if (token.cancelled) break;
          if (step.kind === "thinking") {
            if (settings.thinking) await stream("thinking_delta", step.text);
          } else if (step.kind === "text") {
            await stream("text_delta", step.text);
          } else if (step.kind === "step_end") {
            steps += 1;
            send({ type: "step_end", input_tokens: step.input, output_tokens: step.output });
          } else if (step.kind === "error") {
            send({ type: "error", message: step.message });
            return;
          } else {
            const id = nextId("tool");
            const started = now();
            send({ type: "tool_start", id, name: step.name, input: step.input });
            await wait(step.ms);
            if (token.cancelled) break;
            const output = await step.run();
            if (token.cancelled && output.data?.kind !== "edit") break;
            const ms = step.name === "simulate" && output.data?.kind === "run" ? output.data.run.duration_ms : Math.max(step.ms, now() - started);
            send({ type: "tool_end", id, name: step.name, output, duration_ms: ms });
            await wait(120);
          }
        }
        send({ type: "done", steps, stop_reason: token.cancelled ? "cancelled" : "end_turn" });
      } finally {
        running.delete(sessionId);
        session.messages.push(assistant);
        touchSession(session);
      }
    },

    async cancel(sessionId) {
      const token = running.get(sessionId);
      if (token) token.cancelled = true;
      for (const [id, resolve] of approvals) {
        approvals.delete(id);
        resolve(false);
      }
    },

    async approve(requestId, approved) {
      const resolve = approvals.get(requestId);
      if (!resolve) throw new Error("That request has already been answered.");
      approvals.delete(requestId);
      resolve(approved);
    },

    async settings() {
      await wait(30);
      return structuredClone(settings);
    },

    async saveSettings(s) {
      await wait(60);
      if (!Number.isInteger(s.max_steps) || s.max_steps < 1 || s.max_steps > 200) {
        throw new Error("Max steps must be a whole number from 1 to 200.");
      }
      settings = structuredClone(s);
      writeJson(SETTINGS_KEY, settings);
    },

    async keyStatus() {
      await wait(30);
      return keyStatus();
    },

    async setKey(provider, key) {
      await wait(80);
      if (key.trim().length < 8) throw new Error("That key looks too short.");
      keys.set(provider, "keychain");
    },

    async removeKey(provider) {
      await wait(60);
      if (keys.get(provider) === "env") throw new Error("This key comes from an environment variable. Unset it there.");
      keys.delete(provider);
    },

    async models(provider) {
      await wait(fast ? 0 : 300);
      if (!keyStatus().find((p) => p.id === provider)?.configured) {
        throw new Error("Add an API key to list this provider's models.");
      }
      if (provider === "custom" && !settings.base_urls.custom) {
        throw new Error("Set a base URL for the custom endpoint first.");
      }
      return MODELS[provider];
    },

    onEvent(handler) {
      listeners.add(handler);
      return () => listeners.delete(handler);
    },
  };

  return backend;
}
