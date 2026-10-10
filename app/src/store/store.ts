import { chatReducer, initialChat, type ChatAction, type ChatState } from "../chat/reduce";
import { baseName } from "../lib/format";
import { readJson, writeJson } from "../lib/storage";
import { applyTheme } from "../lib/theme";
import type {
  AgentEvent,
  Attachment,
  Backend,
  BackendEvent,
  ChatMessage,
  CircuitEntry,
  CircuitView,
  Doctor,
  Highlight,
  ProjectInfo,
  ProviderId,
  ProviderStatus,
  RunView,
  SessionMeta,
  Settings,
  SimulatorId,
  Snapshot,
  SpecReport,
  ToolData,
} from "../ipc/types";

/*
 * One store for the whole window: plain state, a subscribe function for
 * useSyncExternalStore, and async actions that talk to the backend. No state
 * library; the chat transcript goes through the pure reducer in chat/reduce.
 */

export type Tab = "schematic" | "waveforms" | "netlist" | "log" | "specs" | "history";

export const TABS: ReadonlyArray<readonly [Tab, string]> = [
  ["schematic", "Schematic"],
  ["waveforms", "Waveforms"],
  ["netlist", "Netlist"],
  ["log", "Log"],
  ["specs", "Specs"],
  ["history", "History"],
];

export interface Toast {
  id: number;
  tone: "info" | "error";
  text: string;
}

export interface AppState {
  booted: boolean;
  bootError: string | null;
  doctor: Doctor | null;
  doctorError: string | null;
  recent: string[];
  settings: Settings | null;
  keys: ProviderStatus[];

  project: ProjectInfo | null;
  opening: boolean;
  circuits: CircuitEntry[];
  active: string | null;
  view: CircuitView | null;
  viewLoading: boolean;
  viewError: string | null;
  highlights: Highlight[];
  runs: Record<string, RunView>;
  simulating: boolean;
  /** The toolbar's simulator choice; null follows the settings default. */
  simulator: SimulatorId | "auto" | null;
  specs: Record<string, SpecReport | null>;
  specsLoading: boolean;
  specsError: string | null;
  history: Snapshot[];
  tab: Tab;
  split: boolean;
  /** A request to center the schematic on a part (from a finding or a card). */
  locate: { inst: string; nonce: number } | null;

  chat: ChatState;
  sessions: SessionMeta[];
  sessionId: string | null;
  /** Edit snapshots restored from a chat card, so the card can say so. */
  undone: string[];
  /** Text to put in the composer (suggestions, retry). */
  draft: { text: string; nonce: number } | null;

  toasts: Toast[];
  settingsOpen: boolean;
  shortcutsOpen: boolean;
}

const initialState = (): AppState => ({
  booted: false,
  bootError: null,
  doctor: null,
  doctorError: null,
  recent: [],
  settings: null,
  keys: [],
  project: null,
  opening: false,
  circuits: [],
  active: null,
  view: null,
  viewLoading: false,
  viewError: null,
  highlights: [],
  runs: {},
  simulating: false,
  simulator: null,
  specs: {},
  specsLoading: false,
  specsError: null,
  history: [],
  tab: "schematic",
  split: readJson<boolean>("split", false),
  locate: null,
  chat: initialChat,
  sessions: [],
  sessionId: null,
  undone: [],
  draft: null,
  toasts: [],
  settingsOpen: false,
  shortcutsOpen: false,
});

export function message(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

let uid = 0;
const newId = (prefix: string) => `${prefix}-${Date.now().toString(36)}-${(++uid).toString(36)}`;

export class Store {
  private state: AppState = initialState();
  private listeners = new Set<() => void>();
  private toastSeq = 0;
  private unlisten: (() => void) | null = null;
  private viewSeq = 0;

  constructor(readonly backend: Backend) {}

  get = (): AppState => this.state;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private set(patch: Partial<AppState> | ((s: AppState) => Partial<AppState>)): void {
    const next = typeof patch === "function" ? patch(this.state) : patch;
    this.state = { ...this.state, ...next };
    for (const l of this.listeners) l();
  }

  private chat(action: ChatAction): void {
    this.set((s) => ({ chat: chatReducer(s.chat, action) }));
  }

  /* Boot and project ---------------------------------------------------- */

  async boot(): Promise<void> {
    this.unlisten?.();
    this.unlisten = this.backend.onEvent((e) => this.onBackendEvent(e));
    const [settings, recent, keys] = await Promise.allSettled([
      this.backend.settings(),
      this.backend.recentProjects(),
      this.backend.keyStatus(),
    ]);
    if (settings.status === "fulfilled") applyTheme(settings.value.theme);
    this.set({
      booted: true,
      settings: settings.status === "fulfilled" ? settings.value : null,
      bootError: settings.status === "rejected" ? message(settings.reason) : null,
      recent: recent.status === "fulfilled" ? recent.value : [],
      keys: keys.status === "fulfilled" ? keys.value : [],
    });
    void this.refreshDoctor();
  }

  dispose(): void {
    this.unlisten?.();
    this.unlisten = null;
  }

  async refreshDoctor(): Promise<void> {
    try {
      const doctor = await this.backend.doctor();
      this.set({ doctor, doctorError: null });
    } catch (err) {
      this.set({ doctorError: message(err) });
    }
  }

  async openFolder(): Promise<void> {
    try {
      const root = await this.backend.pickFolder();
      if (root) await this.openProject(root);
    } catch (err) {
      this.toast(`Could not open the folder: ${message(err)}`, "error");
    }
  }

  async openProject(root: string): Promise<void> {
    this.set({ opening: true });
    try {
      const project = await this.backend.openProject(root);
      this.set((s) => ({
        project,
        circuits: project.circuits,
        opening: false,
        active: null,
        view: null,
        runs: {},
        specs: {},
        history: [],
        highlights: [],
        recent: [project.root, ...s.recent.filter((r) => r !== project.root)].slice(0, 8),
      }));
      const first = [...project.circuits].sort((a, b) => b.modified - a.modified)[0];
      if (first) await this.selectCircuit(first.path);
      else this.chat({ type: "reset", messages: [] });
    } catch (err) {
      this.set({ opening: false });
      this.toast(`Could not open ${root}: ${message(err)}`, "error");
    }
  }

  closeProject(): void {
    this.set({
      project: null,
      circuits: [],
      active: null,
      view: null,
      history: [],
      highlights: [],
      sessions: [],
      sessionId: null,
      chat: initialChat,
    });
  }

  private onBackendEvent(e: BackendEvent): void {
    if (e.type === "circuits_listed") {
      this.set({ circuits: e.circuits });
    } else if (e.type === "circuit_changed") {
      if (e.path === this.state.active) {
        void this.refreshView();
        void this.loadHistory();
      }
      void this.refreshCircuits();
    }
  }

  async refreshCircuits(): Promise<void> {
    if (!this.state.project) return;
    try {
      this.set({ circuits: await this.backend.listCircuits() });
    } catch {
      // The list stays as it was; the next change refreshes it.
    }
  }

  /* Circuit ------------------------------------------------------------- */

  async selectCircuit(path: string): Promise<void> {
    if (path === this.state.active && this.state.view) return;
    this.set({ active: path, viewLoading: true, viewError: null, highlights: [], history: [], specsError: null });
    void this.loadHistory();
    void this.loadSessions(path);
    await this.refreshView();
  }

  async refreshView(): Promise<void> {
    const path = this.state.active;
    if (!path) return;
    const seq = ++this.viewSeq;
    try {
      const view = await this.backend.readCircuit(path, this.state.highlights);
      if (seq !== this.viewSeq || path !== this.state.active) return;
      this.set({ view, viewLoading: false, viewError: null });
    } catch (err) {
      if (seq !== this.viewSeq) return;
      this.set({ viewLoading: false, viewError: message(err) });
    }
  }

  async createCircuit(name: string): Promise<boolean> {
    try {
      const entry = await this.backend.newCircuit(name);
      await this.refreshCircuits();
      await this.selectCircuit(entry.path);
      return true;
    } catch (err) {
      this.toast(message(err), "error");
      return false;
    }
  }

  clearHighlights(): void {
    if (this.state.highlights.length === 0) return;
    this.set({ highlights: [] });
    void this.refreshView();
  }

  locate(inst: string): void {
    this.set((s) => ({ tab: s.split ? s.tab : "schematic", locate: { inst, nonce: (s.locate?.nonce ?? 0) + 1 } }));
  }

  effectiveSimulator(): SimulatorId | "auto" {
    return this.state.simulator ?? this.state.settings?.simulator ?? "auto";
  }

  setSimulator(sim: SimulatorId | "auto"): void {
    this.set({ simulator: sim });
  }

  async simulate(sim?: SimulatorId | "auto"): Promise<void> {
    const path = this.state.active;
    if (!path || this.state.simulating) return;
    this.set({ simulating: true });
    try {
      const run = await this.backend.simulate(path, sim ?? this.effectiveSimulator());
      this.set((s) => ({
        simulating: false,
        runs: { ...s.runs, [run.circuit]: run },
        tab: s.tab === "schematic" && run.errors.length === 0 ? "waveforms" : s.tab,
      }));
      if (run.errors.length > 0) this.toast(`Simulation failed: ${run.errors[0]}`, "error");
      void this.refreshCircuits();
    } catch (err) {
      this.set({ simulating: false });
      this.toast(`Simulation failed: ${message(err)}`, "error");
    }
  }

  private async applyView(promise: Promise<CircuitView>, done?: string): Promise<boolean> {
    try {
      const view = await promise;
      if (view.path === this.state.active) this.set({ view, highlights: [] });
      void this.loadHistory();
      void this.refreshCircuits();
      if (done) this.toast(done);
      return true;
    } catch (err) {
      this.toast(message(err), "error");
      return false;
    }
  }

  async undo(): Promise<void> {
    const { active, view } = this.state;
    if (!active || !view?.can_undo) return;
    await this.applyView(this.backend.undo(active));
  }

  async redo(): Promise<void> {
    const { active, view } = this.state;
    if (!active || !view?.can_redo) return;
    await this.applyView(this.backend.redo(active));
  }

  async restore(snapshotId: string): Promise<void> {
    const { active } = this.state;
    if (!active) return;
    await this.applyView(this.backend.restore(active, snapshotId), "Snapshot restored.");
  }

  /** The Undo button on an edit card: back to the snapshot taken before that edit. */
  async undoEdit(data: Extract<ToolData, { kind: "edit" }>): Promise<void> {
    const ok = await this.applyView(this.backend.restore(data.circuit, data.snapshot), `Undid the edit to ${baseName(data.circuit)}.`);
    if (ok) this.set((s) => ({ undone: [...s.undone, data.snapshot] }));
  }

  async openInLtspice(): Promise<void> {
    const path = this.state.active;
    if (!path) return;
    try {
      await this.backend.openInLtspice(path);
      this.toast(`Opened ${baseName(path)} in LTspice.`);
    } catch (err) {
      this.toast(`Could not open LTspice: ${message(err)}`, "error");
    }
  }

  async checkSpecs(): Promise<void> {
    const path = this.state.active;
    if (!path) return;
    this.set({ specsLoading: true, specsError: null });
    try {
      const report = await this.backend.checkSpecs(path);
      this.set((s) => ({ specsLoading: false, specs: { ...s.specs, [path]: report } }));
    } catch (err) {
      this.set({ specsLoading: false, specsError: message(err) });
    }
  }

  async loadHistory(): Promise<void> {
    const path = this.state.active;
    if (!path) return;
    try {
      const history = await this.backend.history(path);
      if (path === this.state.active) this.set({ history });
    } catch {
      // History is informational; the tab shows what it last had.
    }
  }

  setTab(tab: Tab): void {
    this.set({ tab });
  }

  toggleSplit(): void {
    const split = !this.state.split;
    writeJson("split", split);
    this.set((s) => ({ split, tab: split && s.tab === "schematic" ? "waveforms" : s.tab }));
  }

  /* Chat ---------------------------------------------------------------- */

  private async loadSessions(circuit: string | null): Promise<void> {
    this.set({ sessionId: null, chat: initialChat, sessions: [] });
    try {
      const sessions = await this.backend.sessions(circuit);
      if (circuit === this.state.active) this.set({ sessions });
    } catch {
      // An empty picker is the honest fallback.
    }
  }

  async selectSession(id: string | null): Promise<void> {
    if (id === null) {
      this.set({ sessionId: null, chat: initialChat });
      return;
    }
    try {
      const session = await this.backend.loadSession(id);
      this.set({ sessionId: id, chat: chatReducer(initialChat, { type: "reset", messages: session.messages }) });
    } catch (err) {
      this.toast(message(err), "error");
    }
  }

  async newSession(): Promise<void> {
    this.set({ sessionId: null, chat: initialChat });
  }

  async deleteSession(id: string): Promise<void> {
    try {
      await this.backend.deleteSession(id);
      this.set((s) => ({
        sessions: s.sessions.filter((m) => m.id !== id),
        ...(s.sessionId === id ? { sessionId: null, chat: initialChat } : {}),
      }));
    } catch (err) {
      this.toast(message(err), "error");
    }
  }

  setDraft(text: string): void {
    this.set((s) => ({ draft: { text, nonce: (s.draft?.nonce ?? 0) + 1 } }));
  }

  async send(text: string, attachments: Attachment[] = []): Promise<void> {
    const trimmed = text.trim();
    if ((!trimmed && attachments.length === 0) || this.state.chat.streaming) return;
    const circuit = this.state.active;
    let sessionId = this.state.sessionId;

    const user: ChatMessage = {
      id: newId("user"),
      role: "user",
      parts: [
        ...(trimmed ? [{ kind: "text" as const, text: trimmed }] : []),
        ...attachments.map((a) => ({ kind: "image" as const, media_type: a.media_type, data_base64: a.data_base64 })),
      ],
      time: Date.now(),
    };
    this.chat({ type: "send", user, assistantId: newId("assistant"), time: Date.now() });

    try {
      if (!sessionId) {
        const meta = await this.backend.newSession(circuit);
        sessionId = meta.id;
        this.set((s) => ({ sessionId: meta.id, sessions: [meta, ...s.sessions] }));
      }
      const turnSession = sessionId;
      await this.backend.send(turnSession, trimmed, attachments, circuit, (e) => {
        this.onAgentEvent(e, turnSession, circuit);
      });
    } catch (err) {
      if (this.state.sessionId === sessionId || sessionId === null) this.chat({ type: "failed", message: message(err) });
    } finally {
      if (this.state.chat.streaming && this.state.sessionId === sessionId) {
        this.chat({ type: "event", event: { type: "done", steps: this.state.chat.steps, stop_reason: "end_turn" } });
      }
      if (circuit === this.state.active) {
        try {
          this.set({ sessions: await this.backend.sessions(circuit) });
        } catch {
          // Titles refresh on the next turn.
        }
      }
    }
  }

  private onAgentEvent(e: AgentEvent, sessionId: string, circuit: string | null): void {
    if (this.state.sessionId === sessionId) this.chat({ type: "event", event: e });
    if (e.type !== "tool_end" || !e.output.data) return;
    const data = e.output.data;
    if (data.kind === "edit") {
      if (data.circuit === this.state.active) {
        this.set({ highlights: data.highlights });
        void this.refreshView();
        void this.loadHistory();
      }
      void this.refreshCircuits();
    } else if (data.kind === "run") {
      this.set((s) => ({ runs: { ...s.runs, [data.run.circuit]: data.run } }));
    } else if (data.kind === "specs" && circuit) {
      this.set((s) => ({ specs: { ...s.specs, [circuit]: data.report } }));
    }
  }

  async stop(): Promise<void> {
    const id = this.state.sessionId;
    if (!id || !this.state.chat.streaming) return;
    try {
      await this.backend.cancel(id);
    } catch (err) {
      this.toast(`Could not stop: ${message(err)}`, "error");
    }
  }

  async answerApproval(approved: boolean): Promise<void> {
    const approval = this.state.chat.approval;
    if (!approval) return;
    this.chat({ type: "approval_resolved" });
    try {
      await this.backend.approve(approval.request_id, approved);
    } catch (err) {
      this.toast(message(err), "error");
    }
  }

  openRun(run: RunView): void {
    this.set((s) => ({ runs: { ...s.runs, [run.circuit]: run }, tab: "waveforms" }));
    if (run.circuit !== this.state.active) void this.selectCircuit(run.circuit);
  }

  /* Settings ------------------------------------------------------------ */

  openSettings(): void {
    this.set({ settingsOpen: true });
    void this.refreshKeys();
  }

  closeSettings(): void {
    this.set({ settingsOpen: false });
  }

  setShortcutsOpen(open: boolean): void {
    this.set({ shortcutsOpen: open });
  }

  async saveSettings(next: Settings): Promise<boolean> {
    try {
      await this.backend.saveSettings(next);
      applyTheme(next.theme);
      this.set({ settings: next });
      return true;
    } catch (err) {
      this.toast(`Settings were not saved: ${message(err)}`, "error");
      return false;
    }
  }

  async refreshKeys(): Promise<void> {
    try {
      const keys = await this.backend.keyStatus();
      this.set((s) => ({ keys, doctor: s.doctor ? { ...s.doctor, providers: keys } : s.doctor }));
    } catch {
      // Status stays as last known.
    }
  }

  async setKey(provider: ProviderId, key: string): Promise<boolean> {
    try {
      await this.backend.setKey(provider, key);
      await this.refreshKeys();
      return true;
    } catch (err) {
      this.toast(`Key not saved: ${message(err)}`, "error");
      return false;
    }
  }

  async removeKey(provider: ProviderId): Promise<void> {
    try {
      await this.backend.removeKey(provider);
      await this.refreshKeys();
    } catch (err) {
      this.toast(`Key not removed: ${message(err)}`, "error");
    }
  }

  /* Toasts -------------------------------------------------------------- */

  toast(text: string, tone: Toast["tone"] = "info"): void {
    const id = ++this.toastSeq;
    this.set((s) => ({ toasts: [...s.toasts.slice(-3), { id, tone, text }] }));
    setTimeout(() => this.dismissToast(id), tone === "error" ? 8000 : 4000);
  }

  dismissToast(id: number): void {
    if (!this.state.toasts.some((t) => t.id === id)) return;
    this.set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  }
}
