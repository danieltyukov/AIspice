import { describe, expect, it } from "vitest";
import { createMockBackend } from "./mock";
import type { AgentEvent } from "./types";

async function opened() {
  const b = createMockBackend({ fast: true });
  await b.openProject("/home/you/circuits/filters");
  return b;
}

describe("mock backend", () => {
  it("refuses circuit work before a project is open", async () => {
    const b = createMockBackend({ fast: true });
    await expect(b.readCircuit("rc_lowpass.asc")).rejects.toThrow(/No project/);
  });

  it("opens a project with three circuits and real summaries", async () => {
    const b = createMockBackend({ fast: true });
    const p = await b.openProject("/home/you/circuits/filters/");
    expect(p.name).toBe("filters");
    expect(p.circuits.map((c) => c.path)).toEqual(["ce_amp.asc", "opamp_inverting.asc", "rc_lowpass.asc"]);
    const view = await b.readCircuit("rc_lowpass.asc");
    expect(view.summary.components.map((c) => c.name)).toEqual(["V1", "R1", "C1"]);
    expect(view.summary.nets.find((n) => n.name === "out")?.members).toEqual(["R1.B", "C1.A"]);
    expect(view.svg).toContain('data-inst="C1"');
    expect(view.netlist).toContain("C1 out 0 100n");
  });

  it("computes an RC response with -3 dB at the corner", async () => {
    const b = await opened();
    const run = await b.simulate("rc_lowpass.asc");
    expect(run.errors).toEqual([]);
    const fc = run.measurements.find((m) => m.name === "fc")!.value!;
    expect(fc).toBeCloseTo(1 / (2 * Math.PI * 1000 * 100e-9), 0);
    const wave = await b.waveform(run.run_id, 0, ["V(out)"]);
    expect(wave.log_x).toBe(true);
    const i = wave.x.findIndex((f) => f >= fc);
    expect(wave.series[0].y[i]).toBeCloseTo(-3, 0);
    // The first grid point at or above fc: within a degree or so of -45.
    expect(Math.abs(wave.series[0].phase![i] + 45)).toBeLessThan(1.5);
  });

  it("returns one series per step for a stepped run", async () => {
    const b = await opened();
    const run = await b.simulate("ce_amp.asc");
    expect(run.datasets[0].steps).toEqual(["RG=100", "RG=220", "RG=470"]);
    const wave = await b.waveform(run.run_id, 0, ["V(out)", "V(in)"], 200);
    expect(wave.series).toHaveLength(6);
    expect(wave.x).toHaveLength(200);
  });

  it("swaps views on undo, redo and restore", async () => {
    const b = await opened();
    const now = await b.readCircuit("rc_lowpass.asc");
    expect(now.can_undo).toBe(true);
    const undone = await b.undo("rc_lowpass.asc");
    expect(undone.summary.components.find((c) => c.name === "R1")?.value).toBe("2.2k");
    expect(undone.can_redo).toBe(true);
    const redone = await b.redo("rc_lowpass.asc");
    expect(redone.summary.components.find((c) => c.name === "R1")?.value).toBe("1k");
    const history = await b.history("rc_lowpass.asc");
    expect(history[0].current).toBe(true);
    const restored = await b.restore("rc_lowpass.asc", history[1].id);
    expect(restored.summary.components.find((c) => c.name === "R1")?.value).toBe("2.2k");
  });

  it("streams a scripted turn that edits, simulates and checks specs", async () => {
    const b = await opened();
    const changed: string[] = [];
    b.onEvent((e) => e.type === "circuit_changed" && changed.push(e.path));
    const session = await b.newSession("rc_lowpass.asc");
    const events: AgentEvent[] = [];
    await b.send(session.id, "Move the corner to 1 kHz", [], "rc_lowpass.asc", (e) => events.push(e));
    const types = events.map((e) => e.type);
    expect(types[0]).toBe("thinking_delta");
    expect(types).toContain("text_delta");
    expect(types.at(-1)).toBe("done");
    const tools = events.filter((e) => e.type === "tool_end").map((e) => (e.type === "tool_end" ? e.output.data?.kind : null));
    expect(tools).toEqual(["schematic", "edit", "run", "specs"]);
    const edit = events.find((e) => e.type === "tool_end" && e.output.data?.kind === "edit");
    if (edit?.type !== "tool_end" || edit.output.data?.kind !== "edit") throw new Error("no edit");
    expect(edit.output.data.highlights).toEqual([
      { inst: "C1", kind: "changed" },
      { inst: "R2", kind: "added" },
    ]);
    expect(edit.output.data.diff).toContain("+SYMATTR Value 160n");
    expect(changed).toEqual(["rc_lowpass.asc"]);
    const view = await b.readCircuit("rc_lowpass.asc");
    expect(view.summary.components.map((c) => c.name)).toContain("R2");
    const saved = await b.loadSession(session.id);
    expect(saved.messages).toHaveLength(2);
    expect(saved.meta.title).toBe("Move the corner to 1 kHz");
  });

  it("waits for approval in ask mode and leaves the circuit alone when declined", async () => {
    const b = await opened();
    await b.saveSettings({ ...(await b.settings()), edit_mode: "ask" });
    const session = await b.newSession("ce_amp.asc");
    const events: AgentEvent[] = [];
    await b.send(session.id, "Raise the gain", [], "ce_amp.asc", (e) => {
      events.push(e);
      if (e.type === "approval_request") void b.approve(e.request_id, false);
    });
    expect(events.some((e) => e.type === "approval_request")).toBe(true);
    const view = await b.readCircuit("ce_amp.asc");
    expect(view.summary.components.find((c) => c.name === "RC")?.value).toBe("3.9k");
    expect(events.at(-1)).toMatchObject({ type: "done" });
  });

  it("stops a turn on cancel", async () => {
    const b = await opened();
    const session = await b.newSession("rc_lowpass.asc");
    const events: AgentEvent[] = [];
    await b.send(session.id, "Move the corner", [], "rc_lowpass.asc", (e) => {
      events.push(e);
      if (e.type === "tool_start") void b.cancel(session.id);
    });
    expect(events.at(-1)).toEqual(expect.objectContaining({ type: "done", stop_reason: "cancelled" }));
    expect(events.filter((e) => e.type === "tool_start")).toHaveLength(1);
  });

  it("emits an error event when asked to fail", async () => {
    const b = await opened();
    const session = await b.newSession("rc_lowpass.asc");
    const events: AgentEvent[] = [];
    await b.send(session.id, "Please fail with an error", [], "rc_lowpass.asc", (e) => events.push(e));
    expect(events.at(-1)).toMatchObject({ type: "error" });
  });

  it("manages keys without ever returning them", async () => {
    const b = createMockBackend({ fast: true });
    await expect(b.models("openai")).rejects.toThrow(/API key/);
    await b.setKey("openai", "sk-test-1234567890");
    const status = (await b.keyStatus()).find((k) => k.id === "openai");
    expect(status).toEqual({ id: "openai", configured: true, source: "keychain" });
    expect(JSON.stringify(await b.keyStatus())).not.toContain("sk-test");
    expect((await b.models("openai")).map((m) => m.id)).toContain("gpt-5");
    await b.removeKey("openai");
    expect((await b.keyStatus()).find((k) => k.id === "openai")?.configured).toBe(false);
    await expect(b.removeKey("openrouter")).rejects.toThrow(/environment/);
  });
});
