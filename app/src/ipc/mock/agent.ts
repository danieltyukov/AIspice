/*
 * The scripted agent behind the mock `send`. It follows the shape of a real
 * turn: think, say what it will do, read the schematic, edit it, simulate,
 * check the specs, answer. Tools act on the mock project for real, so the
 * schematic, waveforms and specs all change while the chat streams.
 */

import { eng } from "../../lib/format";
import type { Highlight, RunView, SchematicSummary, SpecReport, ToolOutput } from "../types";
import type { CircuitState } from "./circuits";

export interface EditPlan {
  next: CircuitState;
  summary: string;
  applied: string[];
  warnings: string[];
  highlights: Highlight[];
  ops: unknown[];
}

export interface EditResult {
  diff: string;
  snapshotBefore: string;
}

export interface AgentHost {
  circuit: string | null;
  state(): CircuitState | null;
  summary(): SchematicSummary | null;
  /** Diff the plan would produce, without applying it. */
  preview(plan: EditPlan): string;
  applyEdit(plan: EditPlan): EditResult;
  simulate(): RunView;
  specs(): SpecReport | null;
  approve(summary: string, diff: string): Promise<boolean>;
  editMode: "apply" | "ask";
}

export type Step =
  | { kind: "thinking"; text: string }
  | { kind: "text"; text: string }
  | { kind: "tool"; name: string; input: unknown; ms: number; run: () => Promise<ToolOutput> }
  | { kind: "step_end"; input: number; output: number }
  | { kind: "error"; message: string };

const text = (t: string): ToolOutput["content"] => [{ type: "text", text: t }];

function summaryText(s: SchematicSummary): string {
  const parts = s.components.map(
    (c) => `  ${c.name.padEnd(4)} ${c.symbol.padEnd(8)} ${(c.value ?? "").padEnd(16)} (${c.pins.map((p) => `${p.pin}:${p.net}`).join(" ")})`,
  );
  const nets = s.nets.map((n) => `  ${n.name}${n.labelled ? " [label]" : ""}: ${n.members.join(", ")}`);
  const checks = s.findings.length
    ? ["Checks:", ...s.findings.map((f) => `  ${f.severity} [${f.rule}] ${f.message}`)]
    : ["Checks: no problems found."];
  return [`Components (${s.components.length}):`, ...parts, `Nets (${s.nets.length}):`, ...nets, "Directives:", ...s.directives.map((d) => `  ${d}`), ...checks].join(
    "\n",
  );
}

function plan(circuit: string, s: CircuitState): EditPlan | null {
  if (circuit === "rc_lowpass.asc") {
    if (s.values.C1 === "160n" && s.flags.R2 && s.values.R1 === "1k") return null;
    const applied: string[] = [];
    const highlights: Highlight[] = [];
    const ops: unknown[] = [];
    if (s.values.R1 !== "1k") {
      applied.push(`Set R1 value ${s.values.R1} -> 1k`);
      highlights.push({ inst: "R1", kind: "changed" });
      ops.push({ op: "set_value", part: "R1", value: "1k" });
    }
    if (s.values.C1 !== "160n") {
      applied.push(`Set C1 value ${s.values.C1} -> 160n`);
      highlights.push({ inst: "C1", kind: "changed" });
      ops.push({ op: "set_value", part: "C1", value: "160n" });
    }
    if (!s.flags.R2) {
      applied.push("Added R2 (res, 100k) at (448, 112)", "Connected R2.A to out", "Connected R2.B to 0");
      highlights.push({ inst: "R2", kind: "added" });
      ops.push(
        { op: "add", symbol: "res", name: "R2", value: "100k", near: "C1" },
        { op: "connect", from: "R2.A", to: "out" },
        { op: "connect", from: "R2.B", to: "0" },
      );
    }
    return {
      next: { values: { ...s.values, R1: "1k", C1: "160n" }, flags: { ...s.flags, R2: true } },
      summary: s.flags.R2 ? "Set C1 to 160n" : "Set C1 to 160n; added R2 (100k) from out to 0",
      applied,
      warnings: s.flags.R2 ? [] : ["R2 loads the output, so the DC gain drops to -0.09 dB."],
      highlights,
      ops,
    };
  }
  if (circuit === "ce_amp.asc") {
    if (s.values.RC === "4.7k") return null;
    return {
      next: { values: { ...s.values, RC: "4.7k" }, flags: s.flags },
      summary: "Set RC to 4.7k",
      applied: [`Set RC value ${s.values.RC} -> 4.7k`],
      warnings: [],
      highlights: [{ inst: "RC", kind: "changed" }],
      ops: [{ op: "set_value", part: "RC", value: "4.7k" }],
    };
  }
  if (circuit === "opamp_inverting.asc") {
    if (!s.flags.RL || s.flags.RLgnd) return null;
    return {
      next: { values: s.values, flags: { ...s.flags, RLgnd: true } },
      summary: "Connected RL.B to ground",
      applied: ["Added ground flag at (480, 288)", "Connected RL.B to 0 (was N004)"],
      warnings: [],
      highlights: [{ inst: "RL", kind: "changed" }],
      ops: [{ op: "connect", from: "RL.B", to: "0" }],
    };
  }
  return null;
}

const SCRIPT: Record<string, { thinking: string; intro: string; why: string; done: string; explain: string }> = {
  "rc_lowpass.asc": {
    thinking:
      "The corner of a first-order RC is 1/(2 pi R C). With R1 = 1k and C1 = 100n that is 1.59 kHz. For 1 kHz with R1 kept at 1k, C1 should be 159n, and 160n is the nearest E24 value. The ADC input loads the output node, so a 100k resistor there makes the model honest. I'll read the schematic to confirm the values first.",
    intro: "I'll read the schematic first to confirm the current values.",
    why: "The corner sits at **1.59 kHz**, set by R1 = 1k and C1 = 100n. I'll change C1 to **160n**, the nearest E24 value for 1 kHz, and add R2 = 100k at the output to stand in for the ADC input.",
    done: "The attenuation spec fails, and no first-order filter can pass it: a single pole gives about 14 dB at five times the corner. A second-order Sallen-Key stage at 1 kHz gives about 28 dB at 5 kHz. Should I replace the RC with one?",
    explain:
      "This is a first-order RC low-pass filter. R1 and C1 form a divider whose impedance ratio changes with frequency: below the corner, C1 is nearly an open circuit and V(out) follows V(in); above it, C1 shunts the signal to ground and the output falls at 20 dB per decade.\n\nThe corner is f = 1/(2 pi R1 C1). The `.ac` directive sweeps 10 Hz to 1 MHz so the Bode plot shows the flat band, the -3 dB point and the roll-off.",
  },
  "ce_amp.asc": {
    thinking:
      "Gain is (RC || RL) / (re + RE || RG). Raising RC raises the gain but lowers the collector voltage, and the spec allows 5 V minimum. With Ie near 1.45 mA, 4.7k puts the collector near 5.2 V.",
    intro: "I'll read the schematic to check the bias network.",
    why: "Gain at RG=220 is set by RC || RL over the emitter impedance. Raising RC from 3.9k to **4.7k** lifts the gain by about 14% and keeps the collector above 5 V.",
    done: "The collector voltage now has about 0.2 V of margin. If the 12 V supply can sag, 4.3k is the safer value.",
    explain:
      "A common-emitter stage. R1 and R2 bias the base near 2.1 V, which puts about 1.45 mA through RE. RC sets the collector voltage and, together with RL, the AC load. CE bypasses RE through RG, so the AC gain is roughly (RC || RL) / (re + RE || RG) while the DC bias stays fixed.\n\nThe `.step` directive runs three values of RG, which is why the waveforms show three output amplitudes.",
  },
  "opamp_inverting.asc": {
    thinking:
      "The lint warning says RL.B is on a net with only one pin. The load never conducts, so the load current spec cannot pass. Grounding RL.B fixes it without touching the gain network.",
    intro: "I'll read the schematic and its rule checks.",
    why: "RL's lower pin is not connected (net N004 reaches only RL.B), so the load draws no current. I'll connect RL.B to ground.",
    done: "The off-grid note on U1 is cosmetic. Moving U1 by 8 units would clear it.",
    explain:
      "An inverting amplifier. U1 holds its inverting input at virtual ground, so the current through Rin equals the current through Rf and the gain is -Rf/Rin = -10. The 1 kHz, 100 mV input gives a 1 V output, inverted.",
  },
};

export function* script(host: AgentHost, prompt: string): Generator<Step> {
  const circuit = host.circuit;
  const lower = prompt.toLowerCase();

  if (!circuit || !host.state()) {
    yield { kind: "text", text: "Pick a circuit from the list on the left and I can read, edit and simulate it." };
    yield { kind: "step_end", input: 1200, output: 40 };
    return;
  }

  if (lower.includes("fail with an error")) {
    yield { kind: "text", text: "Let me check that." };
    yield { kind: "error", message: "The provider returned 529 Overloaded. Try again in a moment." };
    return;
  }

  const lines = SCRIPT[circuit];
  const readTool = (): Step => ({
    kind: "tool",
    name: "read_schematic",
    input: { circuit },
    ms: 40,
    run: async () => {
      const s = host.summary()!;
      return { content: text(summaryText(s)), data: { kind: "schematic", circuit, summary: s }, is_error: false };
    },
  });

  if (!lines) {
    yield { kind: "thinking", text: "The schematic is empty, so there is nothing to simulate yet." };
    yield readTool();
    yield {
      kind: "text",
      text: "This schematic is empty: no parts, no ground and no analysis directive. Tell me what the circuit should do (for example, a 1 kHz RC low-pass driven by a 1 V step) and I'll draw it.",
    };
    yield { kind: "step_end", input: 2100, output: 90 };
    return;
  }

  if (lower.includes("explain")) {
    yield { kind: "thinking", text: "The question is about how the circuit works, not a change. Read it, then explain." };
    yield readTool();
    yield { kind: "step_end", input: 2400, output: 60 };
    yield { kind: "text", text: lines.explain };
    yield { kind: "step_end", input: 3100, output: 210 };
    return;
  }

  if (lower.includes("optimi") || lower.includes("yield") || lower.includes("monte carlo")) {
    yield { kind: "thinking", text: "Size the parts against the specs with the optimizer, then check the design holds with real tolerances." };
    yield { kind: "text", text: "I'll size R1 and C1 against your specs, then run a Monte Carlo analysis with 1% resistors and 5% capacitors." };
    yield readTool();
    yield { kind: "step_end", input: 2480, output: 96 };
    yield {
      kind: "tool",
      name: "optimize",
      input: { circuit, params: [{ name: "R1", min: "100", max: "100k" }, { name: "C1", min: "1n", max: "1u" }], series: "E24" },
      ms: 2600,
      run: async () => ({
        content: text("All specs pass after 23 simulations. R1 = 1k, C1 = 160n (E24)."),
        data: { kind: "optimize", best: { R1: "1k", C1: "160n" }, evaluations: 23, report: sized(host.specs()) },
        is_error: false,
      }),
    };
    yield { kind: "step_end", input: 4210, output: 140 };
    yield {
      kind: "tool",
      name: "monte_carlo",
      input: { circuit, runs: 500, tolerances: [{ part: "R*", tol: "1%" }, { part: "C*", tol: "5%" }] },
      ms: 3100,
      run: async () => ({
        content: text("Yield 97.6% over 500 runs."),
        data: {
          kind: "montecarlo",
          runs: 500,
          yield_pct: 97.6,
          report: [
            "yield     97.6%  (96.0% to 98.6%)",
            "runs      500, R 1%, C 5%",
            "fc        1.003 kHz, sd 26.1 Hz",
            "          12 runs outside limits",
            "gain_dc   -0.09 dB, all pass",
          ].join("\n"),
        },
        is_error: false,
      }),
    };
    yield { kind: "step_end", input: 5020, output: 120 };
    yield {
      kind: "text",
      text: "R1 = 1k and C1 = 160n put the corner at **1.005 kHz**. With 1% resistors and 5% capacitors, **97.6%** of builds meet every spec; the misses are all on the corner, so a 2% capacitor would close the gap.",
    };
    yield { kind: "step_end", input: 5600, output: 160 };
    return;
  }

  yield { kind: "thinking", text: lines.thinking };
  yield { kind: "text", text: lines.intro };
  yield readTool();
  yield { kind: "step_end", input: 2480, output: 96 };

  const edit = plan(circuit, host.state()!);
  if (edit) {
    yield { kind: "text", text: lines.why };
    let declined = false;
    yield {
      kind: "tool",
      name: "edit_schematic",
      input: { circuit, ops: edit.ops },
      ms: 180,
      run: async () => {
        if (host.editMode === "ask") {
          const ok = await host.approve(edit.summary, host.preview(edit));
          if (!ok) {
            declined = true;
            return { content: text("The user declined this edit. Nothing was changed."), data: null, is_error: false };
          }
        }
        const result = host.applyEdit(edit);
        return {
          content: text(`Applied ${edit.applied.length} changes to ${circuit}.`),
          data: {
            kind: "edit",
            circuit,
            summary: edit.summary,
            diff: result.diff,
            applied: edit.applied,
            warnings: edit.warnings,
            highlights: edit.highlights,
            snapshot: result.snapshotBefore,
          },
          is_error: false,
        };
      },
    };
    yield { kind: "step_end", input: 3920, output: 188 };
    if (declined) {
      yield { kind: "text", text: "Understood. I left the schematic unchanged." };
      yield { kind: "step_end", input: 4300, output: 24 };
      return;
    }
    yield { kind: "text", text: "Now I'll simulate and check the specs." };
  } else {
    yield { kind: "text", text: "That change is already in place. I'll simulate and check the specs to confirm." };
  }

  let run: RunView | null = null;
  yield {
    kind: "tool",
    name: "simulate",
    input: { circuit, simulator: "auto" },
    ms: 420,
    run: async () => {
      run = host.simulate();
      const ok = run.errors.length === 0;
      return {
        content: text(ok ? `${run.simulator} finished in ${(run.duration_ms / 1000).toFixed(2)} s.` : run.errors.join("\n")),
        data: { kind: "run", run },
        is_error: !ok,
      };
    },
  };
  let specs: SpecReport | null = null;
  yield {
    kind: "tool",
    name: "check_specs",
    input: { circuit },
    ms: 60,
    run: async () => {
      specs = host.specs();
      if (!specs) return { content: text("No specs are defined for this circuit."), data: null, is_error: false };
      return { content: text(specs.summary), data: { kind: "specs", report: specs }, is_error: false };
    },
  };
  yield { kind: "step_end", input: 5210, output: 74 };

  const report = specs as SpecReport | null;
  const table = report
    ? [
        "| Spec | Result | Limit |",
        "|---|---|---|",
        ...report.rows.map((r) => `| ${r.name} | ${r.display} | ${limitText(r.min, r.max, r.display)} |`),
      ].join("\n")
    : "";
  const lead = (run as RunView | null)?.measurements[0];
  const head = lead ? `Simulated: ${lead.name} is **${lead.display}**.` : "Simulated.";
  const verdict = report ? (report.all_pass ? "All specs pass." : report.summary + ".") : "";
  yield { kind: "text", text: [head, verdict, table, lines.done].filter(Boolean).join("\n\n") };
  yield { kind: "step_end", input: 6020, output: 236 };
}

/** The specs the optimizer sized for: the rows it was asked to meet, all passing. */
function sized(report: SpecReport | null): SpecReport | null {
  if (!report) return null;
  const rows = report.rows.filter((r) => r.pass);
  return { ...report, rows, all_pass: true, summary: `all ${rows.length} specs pass` };
}

function limitText(min: number | null, max: number | null, display: string): string {
  const unit = display.replace(/^[-\d.\s]+[TGMkmµunpf]?/, "").trim();
  const fmt = (v: number) => eng(v, unit, 3);
  if (min !== null && max !== null) return `${fmt(min)} to ${fmt(max)}`;
  if (min !== null) return `at least ${fmt(min)}`;
  if (max !== null) return `at most ${fmt(max)}`;
  return "";
}
