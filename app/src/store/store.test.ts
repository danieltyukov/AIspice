import { describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../ipc/mock";
import type { Backend } from "../ipc/types";
import { Store } from "./store";

async function ready(backend: Backend = createMockBackend({ fast: true })) {
  const store = new Store(backend);
  await store.boot();
  return store;
}

async function until(fn: () => boolean, ms = 2000) {
  const start = Date.now();
  while (!fn()) {
    if (Date.now() - start > ms) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 5));
  }
}

describe("Store", () => {
  it("boots with settings, recent projects and keys, and notifies subscribers", async () => {
    const store = new Store(createMockBackend({ fast: true }));
    const seen = vi.fn();
    store.subscribe(seen);
    await store.boot();
    const s = store.get();
    expect(s.booted).toBe(true);
    expect(s.settings?.provider).toBe("anthropic");
    expect(s.recent).toHaveLength(3);
    expect(seen).toHaveBeenCalled();
    // Keys and the doctor report load after boot, without holding it.
    await until(() => store.get().keys.length > 0 && store.get().doctor !== null);
    expect(store.get().keys.find((k) => k.id === "anthropic")?.configured).toBe(true);
  });

  it("imports a netlist as a new circuit and selects it", async () => {
    const store = await ready();
    await store.openFolder();
    expect(await store.importNetlist()).toBe(true);
    expect(store.get().active).toBe("sallen_key.asc");
    expect(store.get().circuits.some((c) => c.path === "sallen_key.asc")).toBe(true);
    // A second import of the same file gets its own name.
    await store.importNetlist();
    expect(store.get().active).toBe("sallen_key-1.asc");
  });

  it("opens a folder and selects the most recently changed circuit", async () => {
    const store = await ready();
    await store.openFolder();
    const s = store.get();
    expect(s.project?.name).toBe("filters");
    expect(s.active).toBe("rc_lowpass.asc");
    expect(s.view?.path).toBe("rc_lowpass.asc");
    expect(s.recent[0]).toBe("/home/you/circuits/filters");
    await until(() => store.get().history.length === 2);
  });

  it("simulates, stores the run per circuit and moves to the waveforms", async () => {
    const store = await ready();
    await store.openProject("/p");
    await store.simulate();
    const s = store.get();
    expect(s.runs["rc_lowpass.asc"].simulator).toBe("ngspice");
    expect(s.tab).toBe("waveforms");
    expect(s.simulating).toBe(false);
  });

  it("reports a failed simulator as a toast and keeps the tab", async () => {
    const store = await ready();
    await store.openProject("/p");
    store.setSimulator("xyce");
    await store.simulate();
    expect(store.get().tab).toBe("schematic");
    expect(store.get().toasts.at(-1)?.tone).toBe("error");
  });

  it("undoes and redoes with the view following", async () => {
    const store = await ready();
    await store.openProject("/p");
    await store.undo();
    expect(store.get().view?.summary.components.find((c) => c.name === "R1")?.value).toBe("2.2k");
    await store.redo();
    expect(store.get().view?.summary.components.find((c) => c.name === "R1")?.value).toBe("1k");
  });

  it("runs a chat turn: edit highlights, run and specs flow into the panes", async () => {
    const store = await ready();
    await store.openProject("/p");
    await store.send("Move the corner to 1 kHz");
    const s = store.get();
    expect(s.chat.streaming).toBe(false);
    expect(s.sessionId).not.toBeNull();
    expect(s.sessions[0].title).toBe("Move the corner to 1 kHz");
    expect(s.highlights.map((h) => h.inst)).toEqual(["C1", "R2"]);
    expect(s.runs["rc_lowpass.asc"]).toBeDefined();
    expect(s.specs["rc_lowpass.asc"]?.rows).toHaveLength(3);
    await until(() => store.get().view?.summary.components.some((c) => c.name === "R2") ?? false);
    const tools = s.chat.messages[1].parts.filter((p) => p.kind === "tool");
    expect(tools).toHaveLength(4);
  });

  it("undoes an edit from its card and remembers that it did", async () => {
    const store = await ready();
    await store.openProject("/p");
    await store.send("Move the corner to 1 kHz");
    const part = store.get().chat.messages[1].parts.find((p) => p.kind === "tool" && p.call.output?.data?.kind === "edit");
    if (part?.kind !== "tool" || part.call.output?.data?.kind !== "edit") throw new Error("no edit card");
    await store.undoEdit(part.call.output.data);
    expect(store.get().undone).toContain(part.call.output.data.snapshot);
    expect(store.get().view?.summary.components.some((c) => c.name === "R2")).toBe(false);
    expect(store.get().highlights).toEqual([]);
  });

  it("asks before applying when edit mode is ask", async () => {
    const store = await ready();
    await store.openProject("/p");
    await store.saveSettings({ ...store.get().settings!, edit_mode: "ask" });
    const turn = store.send("Move the corner to 1 kHz");
    await until(() => store.get().chat.approval !== null);
    expect(store.get().chat.approval?.diff).toContain("160n");
    await store.answerApproval(true);
    await turn;
    expect(store.get().chat.approval).toBeNull();
    expect(store.get().highlights.length).toBeGreaterThan(0);
  });

  it("shows a send failure on the message, for example a missing key", async () => {
    const store = await ready();
    await store.openProject("/p");
    await store.saveSettings({ ...store.get().settings!, provider: "openai" });
    await store.send("hello");
    const last = store.get().chat.messages.at(-1);
    expect(last?.error).toMatch(/No API key for openai/);
    expect(store.get().chat.streaming).toBe(false);
  });

  it("switches circuits with a fresh chat and its own sessions", async () => {
    const store = await ready();
    await store.openProject("/p");
    await until(() => store.get().sessions.length === 1);
    await store.selectSession(store.get().sessions[0].id);
    expect(store.get().chat.messages).toHaveLength(2);
    await store.selectCircuit("ce_amp.asc");
    await until(() => store.get().view?.path === "ce_amp.asc");
    expect(store.get().chat.messages).toEqual([]);
    expect(store.get().sessionId).toBeNull();
  });

  it("saves settings and applies the theme", async () => {
    const store = await ready();
    await store.saveSettings({ ...store.get().settings!, theme: "dark" });
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    await store.saveSettings({ ...store.get().settings!, theme: "system" });
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  it("sets and removes keys and refreshes status", async () => {
    const store = await ready();
    expect(await store.setKey("google", "AIza-1234567890")).toBe(true);
    expect(store.get().keys.find((k) => k.id === "google")?.configured).toBe(true);
    await store.removeKey("google");
    expect(store.get().keys.find((k) => k.id === "google")?.configured).toBe(false);
    expect(await store.setKey("google", "short")).toBe(false);
    expect(store.get().toasts.at(-1)?.text).toMatch(/too short/);
  });
});
