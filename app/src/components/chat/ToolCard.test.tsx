import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ChatToolCall, RunView, SpecReport } from "../../ipc/types";
import { renderWithStore } from "../../test/render";
import { ToolCard } from "./ToolCard";
import { toolTitle } from "./toolTitle";

const run: RunView = {
  run_id: "run-1",
  circuit: "rc_lowpass.asc",
  simulator: "ngspice",
  datasets: [],
  measurements: [{ name: "fc", value: 1005, unit: "Hz", display: "1.005 kHz" }],
  op: {},
  errors: [],
  warnings: [],
  duration_ms: 412,
  log: "",
};

const report: SpecReport = {
  rows: [
    { name: "Corner frequency", value: 1005, min: 900, max: 1100, pass: true, margin: 95, display: "1.005 kHz" },
    { name: "DC gain", value: -0.09, min: -0.5, max: null, pass: true, margin: 0.41, display: "-0.09 dB" },
    { name: "Attenuation at 5 kHz", value: 14.2, min: 20, max: null, pass: false, margin: -5.8, display: "14.20 dB" },
  ],
  all_pass: false,
  summary: "2 of 3 specs pass",
};

const edit: ChatToolCall = {
  id: "t2",
  name: "edit_schematic",
  input: { circuit: "rc_lowpass.asc", ops: [] },
  duration_ms: 180,
  output: {
    content: [{ type: "text", text: "Applied 2 changes." }],
    is_error: false,
    data: {
      kind: "edit",
      circuit: "rc_lowpass.asc",
      summary: "Set C1 to 160n; added R2",
      diff: "--- a/rc_lowpass.asc\n+++ b/rc_lowpass.asc\n@@ -1,2 +1,2 @@\n SYMATTR InstName C1\n-SYMATTR Value 100n\n+SYMATTR Value 160n",
      applied: ["Set C1 value 100n -> 160n", "Added R2 (res, 100k)"],
      warnings: ["R2 loads the output."],
      highlights: [{ inst: "C1", kind: "changed" }],
      snapshot: "snap-1",
    },
  },
};

describe("toolTitle", () => {
  it("says what happened in plain words", () => {
    expect(toolTitle(edit, false).title).toBe("Edited rc_lowpass.asc");
    expect(toolTitle({ id: "a", name: "simulate", input: {}, output: { content: [], is_error: false, data: { kind: "run", run } } }, false).title).toBe(
      "Simulated with ngspice in 0.4 s",
    );
    expect(toolTitle({ id: "b", name: "check_specs", input: {}, output: { content: [], is_error: false, data: { kind: "specs", report } } }, false).title).toBe(
      "Checked 3 specs: 2 pass, 1 fails",
    );
  });

  it("describes running, stopped and failed calls", () => {
    expect(toolTitle({ id: "c", name: "simulate", input: { circuit: "a/rc.asc" } }, true)).toEqual({ title: "Simulating rc.asc...", tone: "running" });
    expect(toolTitle({ id: "c", name: "simulate", input: {} }, false).tone).toBe("stopped");
    expect(toolTitle({ id: "d", name: "edit_schematic", input: {}, output: { content: [], is_error: true } }, false)).toEqual({
      title: "Edit schematic failed",
      tone: "error",
    });
  });
});

describe("ToolCard", () => {
  it("opens an edit card with the applied list, warnings, a coloured diff and Undo", async () => {
    const { store } = await renderWithStore(<ToolCard call={edit} streaming={false} />, { project: true });
    expect(screen.getByRole("button", { name: /Edited rc_lowpass.asc/ })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Set C1 value 100n -> 160n")).toBeInTheDocument();
    expect(screen.getByText("R2 loads the output.")).toBeInTheDocument();
    const added = screen.getByText("+SYMATTR Value 160n");
    expect(added).toHaveAttribute("data-kind", "add");
    expect(screen.getByText("-SYMATTR Value 100n")).toHaveAttribute("data-kind", "del");
    let restored: string | null = null;
    store.backend.restore = async (_p, id) => {
      restored = id;
      return store.get().view!;
    };
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await screen.findByRole("button", { name: "Undone" });
    expect(restored).toBe("snap-1");
  });

  it("keeps a run card collapsed until asked, then offers the waveforms", async () => {
    const call: ChatToolCall = { id: "t3", name: "simulate", input: {}, duration_ms: 412, output: { content: [], is_error: false, data: { kind: "run", run } } };
    const { store } = await renderWithStore(<ToolCard call={call} streaming={false} />, { project: true });
    const head = screen.getByRole("button", { name: /Simulated with ngspice/ });
    expect(head).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(head);
    expect(screen.getByText("1.005 kHz")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open waveforms" }));
    expect(store.get().tab).toBe("waveforms");
  });

  it("shows a running call without a details toggle", async () => {
    await renderWithStore(<ToolCard call={{ id: "t4", name: "check_specs", input: {} }} streaming />);
    expect(screen.getByRole("button", { name: /Checking specs/ })).toBeDisabled();
  });

  it("renders a specs table with pass and fail chips", async () => {
    const call: ChatToolCall = { id: "t5", name: "check_specs", input: {}, output: { content: [], is_error: false, data: { kind: "specs", report } } };
    await renderWithStore(<ToolCard call={call} streaming={false} />);
    fireEvent.click(screen.getByRole("button", { name: /Checked 3 specs/ }));
    expect(screen.getAllByText("Pass")).toHaveLength(2);
    expect(screen.getByText("Fail")).toBeInTheDocument();
  });
});
