/*
 * The mock project's circuits. Each one is a small parametric model: the
 * state (part values and a few switches) builds the drawing, the summary, the
 * .asc text and the netlist, and the simulate function computes real numbers
 * from the same values, so an edit visibly changes every tab.
 */

import { eng } from "../../lib/format";
import type {
  ComponentInfo,
  DatasetMeta,
  Finding,
  Measurement,
  SchematicSummary,
  SimulatorId,
  SpecReport,
  SpecRow,
  VectorMeta,
  WaveData,
} from "../types";
import * as d from "./draw";
import { db, decimate, deg, linspace, logspace, parallel, parseSpice } from "./math";

export interface CircuitState {
  values: Record<string, string>;
  flags: Record<string, boolean>;
}

export interface Built {
  svg: string;
  summary: SchematicSummary;
  asc: string;
  netlist: string;
}

export interface SimOutput {
  datasets: DatasetMeta[];
  measurements: Measurement[];
  op: Record<string, number>;
  errors: string[];
  warnings: string[];
  log: string;
  duration_ms: number;
  wave(dataset: number, signals: string[], maxPoints?: number): WaveData;
}

export interface CircuitDef {
  build(s: CircuitState): Built;
  simulate(s: CircuitState, simulator: SimulatorId, path: string): SimOutput;
  specs(s: CircuitState): SpecReport | null;
}

/* Drawing and .asc text together --------------------------------------- */

type Pt = readonly [number, number];

class Sheet {
  private svg: string[] = [];
  private wires: string[] = [];
  private flags: string[] = [];
  private symbols: string[] = [];
  private texts: string[] = [];

  constructor(
    private width: number,
    private height: number,
  ) {}

  wire(...pts: Pt[]): this {
    this.svg.push(d.wire(...pts));
    for (let i = 1; i < pts.length; i++) {
      this.wires.push(`WIRE ${pts[i - 1][0]} ${pts[i - 1][1]} ${pts[i][0]} ${pts[i][1]}`);
    }
    return this;
  }

  junction(x: number, y: number): this {
    this.svg.push(d.junction(x, y));
    return this;
  }

  ground(x: number, y: number): this {
    this.svg.push(d.ground(x, y));
    this.flags.push(`FLAG ${x} ${y} 0`);
    return this;
  }

  label(x: number, y: number, name: string, tx: number, ty: number, anchor: "start" | "middle" | "end" = "middle"): this {
    this.svg.push(d.label(tx, ty, name, anchor));
    this.flags.push(`FLAG ${x} ${y} ${name}`);
    return this;
  }

  part(markup: string, symbol: string, x: number, y: number, orient: string, name: string, value: string): this {
    this.svg.push(markup);
    this.symbols.push(`SYMBOL ${symbol} ${x} ${y} ${orient}\nSYMATTR InstName ${name}\nSYMATTR Value ${value}`);
    return this;
  }

  directive(x: number, y: number, text: string): this {
    this.svg.push(d.directive(x, y, text));
    this.texts.push(`TEXT ${x} ${y} Left 2 !${text}`);
    return this;
  }

  comment(x: number, y: number, text: string): this {
    this.svg.push(d.comment(x, y, text));
    this.texts.push(`TEXT ${x} ${y} Left 2 ;${text}`);
    return this;
  }

  toSvg(): string {
    return d.sheet(this.width, this.height, this.svg);
  }

  toAsc(): string {
    return (
      ["Version 4", `SHEET 1 ${this.width} ${this.height}`, ...this.wires, ...this.flags, ...this.symbols, ...this.texts].join(
        "\n",
      ) + "\n"
    );
  }
}

/* Summary helpers ------------------------------------------------------- */

const DESCRIPTIONS: Record<string, string> = {
  res: "Resistor",
  cap: "Capacitor",
  voltage: "Voltage source",
  npn: "NPN bipolar transistor",
  opamp2: "Operational amplifier with supply pins",
};

function comp(
  name: string,
  symbol: string,
  value: string | null,
  at: Pt,
  orient: string,
  pins: Array<readonly [string, string]>,
  attrs?: Record<string, string>,
): ComponentInfo {
  return {
    name,
    symbol,
    value,
    ...(attrs ? { attrs } : {}),
    at: { x: at[0], y: at[1] },
    orient,
    pins: pins.map(([pin, net]) => ({ pin, net })),
    description: DESCRIPTIONS[symbol],
  };
}

function summarize(
  components: ComponentInfo[],
  labelled: string[],
  directives: string[],
  comments: string[],
  findings: Finding[],
): SchematicSummary {
  const nets = new Map<string, string[]>();
  for (const c of components) {
    for (const p of c.pins) {
      const list = nets.get(p.net) ?? [];
      list.push(`${c.name}.${p.pin}`);
      nets.set(p.net, list);
    }
  }
  const order = (n: string) => (n === "0" ? "0" : /^N\d+$/.test(n) ? `2${n}` : `1${n}`);
  return {
    components,
    nets: [...nets.entries()]
      .sort((a, b) => order(a[0]).localeCompare(order(b[0])))
      .map(([name, members]) => ({ name, labelled: name === "0" || labelled.includes(name), members })),
    directives,
    comments,
    analysis: directives.find((x) => /^\.(tran|ac|dc|op|noise|tf)\b/i.test(x)) ?? null,
    findings,
  };
}

function netlistOf(title: string, components: ComponentInfo[], directives: string[], extra: string[] = []): string {
  const lines = [`* ${title}`];
  for (const c of components) {
    const nets = c.pins.map((p) => p.net).join(" ");
    const prefix = c.symbol === "opamp2" ? "X" : "";
    lines.push(`${prefix}${c.name} ${nets} ${c.value ?? ""}`.trimEnd());
  }
  lines.push(...extra, ...directives, ".backanno", ".end");
  return lines.join("\n") + "\n";
}

/* Spec helpers ---------------------------------------------------------- */

function spec(name: string, value: number, min: number | null, max: number | null, unit: string): SpecRow {
  const margins: number[] = [];
  if (min !== null) margins.push(value - min);
  if (max !== null) margins.push(max - value);
  const margin = margins.length ? Math.min(...margins) : null;
  return {
    name,
    value,
    min,
    max,
    pass: margin === null || margin >= 0,
    margin,
    display: eng(value, unit, 3),
  };
}

function report(rows: SpecRow[]): SpecReport {
  const passed = rows.filter((r) => r.pass).length;
  return {
    rows,
    all_pass: passed === rows.length,
    summary: passed === rows.length ? `All ${rows.length} specs pass` : `${passed} of ${rows.length} specs pass`,
  };
}

function measurement(name: string, value: number, unit: string, note?: string): Measurement {
  return { name, value, unit, display: eng(value, unit, 4), note: note ?? null };
}

function vec(name: string, quantity: VectorMeta["quantity"]): VectorMeta {
  return { name, quantity };
}

function pick(x: number[], series: WaveData["series"], maxPoints?: number): { x: number[]; series: WaveData["series"] } {
  const idx = decimate(x, maxPoints);
  if (idx.length === x.length) return { x, series };
  return {
    x: idx.map((i) => x[i]),
    series: series.map((s) => ({
      ...s,
      y: idx.map((i) => s.y[i]),
      ...(s.phase ? { phase: idx.map((i) => s.phase![i]) } : {}),
    })),
  };
}

function ngspiceLog(path: string, simulator: SimulatorId, analysis: string, rows: number, extra: string[] = []): string {
  const head =
    simulator === "ltspice"
      ? ["LTspice 26.1 for Windows (Wine)", `Circuit: ${path}`, ""]
      : ["ngspice-44.2 batch mode", `Circuit: * ${path}`, "", "Reading netlist ... done"];
  return [
    ...head,
    "Doing analysis at TEMP = 27.000000 and TNOM = 27.000000",
    "",
    `${analysis}`,
    `No. of Data Rows : ${rows}`,
    ...extra,
    "",
    "Total analysis time (seconds) = 0.014",
    "Total elapsed time (seconds) = 0.051",
    "",
  ].join("\n");
}

/* rc_lowpass.asc: first-order RC, AC analysis ---------------------------- */

const rcLowpass: CircuitDef = {
  build(s) {
    const r1 = s.values.R1;
    const c1 = s.values.C1;
    const load = s.flags.R2;
    const k = new Sheet(560, 296);
    k.comment(48, 36, "1 kHz anti-alias filter in front of the ADC");
    k.part(d.voltage("V1", "AC 1", 96, 112, "ac"), "voltage", 96, 96, "R0", "V1", "AC 1");
    k.ground(96, 192);
    k.wire([96, 112], [96, 80], [176, 80]);
    k.label(96, 80, "in", 136, 72);
    k.part(d.resistorH("R1", r1, 176, 80), "res", 272, 64, "R90", "R1", r1);
    k.wire([256, 80], [336, 80]);
    k.part(d.capV("C1", c1, 336, 112), "cap", 320, 112, "R0", "C1", c1);
    k.wire([336, 80], [336, 112]);
    k.ground(336, 176);
    k.junction(336, 80);
    if (load) {
      k.wire([336, 80], [496, 80]);
      k.wire([448, 80], [448, 112]);
      k.part(d.resistorV("R2", "100k", 448, 112), "res", 432, 96, "R0", "R2", "100k");
      k.ground(448, 192);
      k.junction(448, 80);
      k.label(496, 80, "out", 504, 84, "start");
    } else {
      k.wire([336, 80], [400, 80]);
      k.label(400, 80, "out", 408, 84, "start");
    }
    k.directive(48, 268, ".ac dec 100 10 1Meg");

    const components = [
      comp("V1", "voltage", "AC 1", [96, 112], "R0", [
        ["+", "in"],
        ["-", "0"],
      ]),
      comp("R1", "res", r1, [176, 80], "R90", [
        ["A", "in"],
        ["B", "out"],
      ]),
      comp("C1", "cap", c1, [336, 112], "R0", [
        ["A", "out"],
        ["B", "0"],
      ]),
    ];
    if (load) {
      components.push(
        comp("R2", "res", "100k", [448, 112], "R0", [
          ["A", "out"],
          ["B", "0"],
        ]),
      );
    }
    const directives = [".ac dec 100 10 1Meg"];
    return {
      svg: k.toSvg(),
      asc: k.toAsc(),
      summary: summarize(components, ["in", "out"], directives, ["1 kHz anti-alias filter in front of the ADC"], []),
      netlist: netlistOf("rc_lowpass.asc", components, directives),
    };
  },

  simulate(s, simulator, path) {
    const p = rcParams(s);
    const f = logspace(1, 6, 100);
    const resp = f.map((hz) => rcResponse(p, hz));
    const vectors = [vec("frequency", "frequency"), vec("V(in)", "voltage"), vec("V(out)", "voltage"), vec("I(C1)", "current"), vec("I(R1)", "current")];
    const fc = 1 / (2 * Math.PI * p.rth * p.c);
    const at5k = rcResponse(p, 5000);
    return {
      datasets: [{ index: 0, plotname: "AC Analysis", kind: "ac", axis: "frequency", vectors, points: f.length, steps: [] }],
      measurements: [
        measurement("fc", fc, "Hz", "-3 dB below the DC gain"),
        measurement("gain_dc", db(p.g), "dB"),
        measurement("atten_5k", -db(Math.hypot(at5k.out[0], at5k.out[1])), "dB", "Attenuation at 5 kHz"),
        measurement("phase_1k", deg(Math.atan2(rcResponse(p, 1000).out[1], rcResponse(p, 1000).out[0])), "deg"),
      ],
      op: {},
      errors: [],
      warnings: [],
      log: ngspiceLog(path, simulator, "AC analysis: 10 Hz to 1 MHz, 100 points per decade", f.length),
      duration_ms: 412,
      wave(_dataset, signals, maxPoints) {
        const series = signals.map((name) => {
          const choose = (r: RcPoint): [number, number] =>
            name === "V(in)" ? [1, 0] : name === "V(out)" ? r.out : name === "I(C1)" ? r.ic : r.ir;
          return {
            name,
            step: "",
            y: resp.map((r) => db(Math.hypot(...choose(r)))),
            phase: resp.map((r) => {
              const [re, im] = choose(r);
              return deg(Math.atan2(im, re));
            }),
          };
        });
        const out = pick(f, series, maxPoints);
        return { kind: "ac", x_name: "frequency", x_unit: "Hz", log_x: true, x: out.x, series: out.series };
      },
    };
  },

  specs(s) {
    const p = rcParams(s);
    const fc = 1 / (2 * Math.PI * p.rth * p.c);
    const at5k = rcResponse(p, 5000);
    return report([
      spec("Corner frequency", fc, 900, 1100, "Hz"),
      spec("DC gain", db(p.g), -0.5, null, "dB"),
      spec("Attenuation at 5 kHz", -db(Math.hypot(at5k.out[0], at5k.out[1])), 20, null, "dB"),
    ]);
  },
};

interface RcParams {
  r1: number;
  c: number;
  g: number;
  rth: number;
}

interface RcPoint {
  out: [number, number];
  ic: [number, number];
  ir: [number, number];
}

function rcParams(s: CircuitState): RcParams {
  const r1 = parseSpice(s.values.R1);
  const c = parseSpice(s.values.C1);
  const rl = s.flags.R2 ? 100e3 : Infinity;
  const g = Number.isFinite(rl) ? rl / (r1 + rl) : 1;
  const rth = Number.isFinite(rl) ? parallel(r1, rl) : r1;
  return { r1, c, g, rth };
}

function rcResponse(p: RcParams, hz: number): RcPoint {
  const w = 2 * Math.PI * hz;
  const x = w * p.rth * p.c;
  const re = p.g / (1 + x * x);
  const im = (-p.g * x) / (1 + x * x);
  return {
    out: [re, im],
    ic: [-w * p.c * im, w * p.c * re],
    ir: [(1 - re) / p.r1, -im / p.r1],
  };
}

/* ce_amp.asc: common-emitter stage, stepped transient ---------------------- */

const CE_STEPS = [100, 220, 470];

const ceAmp: CircuitDef = {
  build(s) {
    const rc = s.values.RC;
    const k = new Sheet(640, 456);
    k.comment(48, 28, "Common-emitter stage, gain set by RC and RE || RG");
    k.part(d.voltage("V2", "12", 64, 96), "voltage", 64, 80, "R0", "V2", "12");
    k.label(64, 96, "vcc", 64, 86);
    k.ground(64, 176);
    k.part(d.voltage("V1", "SINE(0 10m 1k)", 128, 256, "sine"), "voltage", 128, 240, "R0", "V1", "SINE(0 10m 1k)");
    k.wire([128, 256], [128, 224], [160, 224]);
    k.label(128, 224, "in", 128, 214);
    k.ground(128, 336);
    k.part(d.capH("C1", "10u", 160, 224), "cap", 192, 208, "R90", "C1", "10u");
    k.wire([224, 224], [336, 224]);
    k.junction(240, 224);
    k.part(d.resistorV("R1", "47k", 240, 72), "res", 224, 56, "R0", "R1", "47k");
    k.wire([240, 152], [240, 224]);
    k.part(d.resistorV("R2", "10k", 240, 256), "res", 224, 240, "R0", "R2", "10k");
    k.wire([240, 224], [240, 256]);
    k.ground(240, 336);
    k.wire([240, 72], [240, 56], [384, 56], [384, 72]);
    k.label(312, 56, "vcc", 312, 48);
    k.part(d.npn("Q1", "2N3904", 336, 224), "npn", 320, 176, "R0", "Q1", "2N3904");
    k.part(d.resistorV("RC", rc, 384, 72), "res", 368, 56, "R0", "RC", rc);
    k.wire([384, 152], [384, 176]);
    k.junction(384, 160);
    k.wire([384, 160], [432, 160]);
    k.part(d.capH("C2", "10u", 432, 160), "cap", 464, 144, "R90", "C2", "10u");
    k.wire([496, 160], [560, 160]);
    k.junction(528, 160);
    k.label(560, 160, "out", 568, 164, "start");
    k.part(d.resistorV("RL", "10k", 528, 192), "res", 512, 176, "R0", "RL", "10k");
    k.wire([528, 160], [528, 192]);
    k.ground(528, 272);
    k.wire([384, 272], [384, 288]);
    k.part(d.resistorV("RE", "1k", 384, 288), "res", 368, 272, "R0", "RE", "1k");
    k.ground(384, 368);
    k.junction(384, 280);
    k.wire([384, 280], [448, 280], [448, 288]);
    k.part(d.capV("CE", "100u", 448, 288), "cap", 432, 288, "R0", "CE", "100u");
    k.part(d.resistorV("RG", "{RG}", 448, 352), "res", 432, 336, "R0", "RG", "{RG}");
    k.ground(448, 432);
    k.directive(48, 420, ".tran 0 3m");
    k.directive(48, 440, ".step param RG list 100 220 470");

    const components = [
      comp("V1", "voltage", "SINE(0 10m 1k)", [128, 256], "R0", [
        ["+", "in"],
        ["-", "0"],
      ]),
      comp("V2", "voltage", "12", [64, 96], "R0", [
        ["+", "vcc"],
        ["-", "0"],
      ]),
      comp("C1", "cap", "10u", [160, 224], "R90", [
        ["A", "in"],
        ["B", "N001"],
      ]),
      comp("R1", "res", "47k", [240, 72], "R0", [
        ["A", "vcc"],
        ["B", "N001"],
      ]),
      comp("R2", "res", "10k", [240, 256], "R0", [
        ["A", "N001"],
        ["B", "0"],
      ]),
      comp("Q1", "npn", "2N3904", [336, 224], "R0", [
        ["C", "N002"],
        ["B", "N001"],
        ["E", "N003"],
      ]),
      comp("RC", "res", rc, [384, 72], "R0", [
        ["A", "vcc"],
        ["B", "N002"],
      ]),
      comp("C2", "cap", "10u", [432, 160], "R90", [
        ["A", "N002"],
        ["B", "out"],
      ]),
      comp("RL", "res", "10k", [528, 192], "R0", [
        ["A", "out"],
        ["B", "0"],
      ]),
      comp("RE", "res", "1k", [384, 288], "R0", [
        ["A", "N003"],
        ["B", "0"],
      ]),
      comp("CE", "cap", "100u", [448, 288], "R0", [
        ["A", "N003"],
        ["B", "N004"],
      ]),
      comp("RG", "res", "{RG}", [448, 352], "R0", [
        ["A", "N004"],
        ["B", "0"],
      ]),
    ];
    const directives = [".tran 0 3m", ".step param RG list 100 220 470"];
    return {
      svg: k.toSvg(),
      asc: k.toAsc(),
      summary: summarize(components, ["in", "out", "vcc"], directives, ["Common-emitter stage, gain set by RC and RE || RG"], []),
      netlist: netlistOf("ce_amp.asc", components, directives, [
        ".model 2N3904 NPN(IS=6.734f BF=416.4 VAF=74.03 RB=10 CJC=3.638p CJE=4.493p TF=301.2p)",
      ]),
    };
  },

  simulate(s, simulator, path) {
    const rc = parseSpice(s.values.RC);
    const p = ceParams(rc);
    const t = linspace(0, 3e-3, 601);
    const vectors = [
      vec("time", "time"),
      vec("V(in)", "voltage"),
      vec("V(out)", "voltage"),
      vec("V(n001)", "voltage"),
      vec("V(n002)", "voltage"),
      vec("V(n003)", "voltage"),
      vec("Ic(Q1)", "current"),
    ];
    const steps = CE_STEPS.map((rg) => `RG=${rg}`);
    return {
      datasets: [{ index: 0, plotname: "Transient Analysis", kind: "transient", axis: "time", vectors, points: t.length, steps }],
      measurements: [
        ...CE_STEPS.map((rg) => measurement(`gain RG=${rg}`, -p.gain(rg), "V/V", "Small-signal gain V(out)/V(in)")),
        measurement("vc", p.vc, "V", "Collector bias"),
        measurement("ie", p.ie, "A", "Emitter current"),
      ],
      op: { "V(n001)": 2.104, "V(n002)": p.vc, "V(n003)": 1.452, "Ic(Q1)": p.ie * 0.9976, "I(V2)": -(p.ie + 0.21e-3) },
      errors: [],
      warnings: [],
      log: ngspiceLog(path, simulator, "Transient analysis: 0 to 3 ms, 3 steps of RG", t.length * 3, [
        "Stepping RG: 100, 220, 470",
      ]),
      duration_ms: 687,
      wave(_dataset, signals, maxPoints) {
        const series = [];
        for (const name of signals) {
          for (const rg of CE_STEPS) {
            series.push({ name, step: `RG=${rg}`, y: t.map((tt) => ceSample(p, rg, name, tt)) });
          }
        }
        const out = pick(t, series, maxPoints);
        return { kind: "transient", x_name: "time", x_unit: "s", log_x: false, x: out.x, series: out.series };
      },
    };
  },

  specs(s) {
    const p = ceParams(parseSpice(s.values.RC));
    return report([
      spec("Gain at RG=220", p.gain(220), 12, null, "V/V"),
      spec("Collector voltage", p.vc, 5, 8, "V"),
      spec("Supply current", p.ie + 0.21e-3, null, 2e-3, "A"),
    ]);
  },
};

function ceParams(rc: number) {
  const ie = 1.452e-3;
  const re = 0.02585 / ie;
  const rcl = parallel(rc, 10e3);
  return {
    ie,
    rcl,
    vc: 12 - ie * 0.9976 * rc,
    gain: (rg: number) => rcl / (re + parallel(1000, rg)),
  };
}

function ceSample(p: ReturnType<typeof ceParams>, rg: number, name: string, t: number): number {
  const vin = 0.01 * Math.sin(2 * Math.PI * 1000 * t);
  const a = p.gain(rg);
  const env = 1 - Math.exp(-t / 0.35e-3);
  const ac = -a * vin * (1 + 0.04 * a * vin);
  switch (name) {
    case "V(in)":
      return vin;
    case "V(out)":
      return ac * env;
    case "V(n001)":
      return 2.104 + vin * (1 - Math.exp(-t / 0.1e-3)) * 0.97;
    case "V(n002)":
      return p.vc + ac;
    case "V(n003)":
      return 1.452 + vin * 0.9 * (parallel(1000, rg) / (17.8 + parallel(1000, rg)));
    case "Ic(Q1)":
      return p.ie * 0.9976 - ac / p.rcl;
    default:
      return 0;
  }
}

/* opamp_inverting.asc: inverting amplifier, transient ---------------------- */

const opampInverting: CircuitDef = {
  build(s) {
    const rl = s.flags.RL;
    const rlGnd = s.flags.RLgnd;
    const k = new Sheet(640, 356);
    k.comment(48, 36, "Inverting amplifier, gain = -Rf/Rin");
    k.part(d.voltage("V1", "SINE(0 100m 1k)", 80, 176, "sine"), "voltage", 80, 160, "R0", "V1", "SINE(0 100m 1k)");
    k.ground(80, 256);
    k.wire([80, 176], [80, 144], [128, 144]);
    k.part(d.resistorH("Rin", "1k", 128, 144), "res", 224, 128, "R90", "Rin", "1k");
    k.wire([208, 144], [272, 144]);
    k.junction(240, 144);
    k.wire([240, 144], [240, 80], [272, 80]);
    k.part(d.resistorH("Rf", "10k", 272, 80), "res", 368, 64, "R90", "Rf", "10k");
    k.wire([352, 80], [432, 80], [432, 176]);
    k.part(d.opamp("U1", "LT1001", 272, 144), "OpAmps\\opamp2", 304, 112, "R0", "U1", "LT1001");
    k.wire([272, 208], [248, 208], [248, 232]);
    k.ground(248, 232);
    k.label(336, 128, "vcc", 336, 122);
    k.label(336, 224, "vee", 336, 240);
    k.wire([400, 176], [480, 176]);
    k.junction(432, 176);
    if (rl) {
      k.wire([480, 176], [480, 208]);
      k.part(d.resistorV("RL", "10k", 480, 208), "res", 464, 192, "R0", "RL", "10k");
      if (rlGnd) k.ground(480, 288);
    }
    k.part(d.voltage("V2", "15", 560, 88), "voltage", 560, 72, "R0", "V2", "15");
    k.label(560, 88, "vcc", 560, 80);
    k.ground(560, 168);
    k.part(d.voltage("V3", "-15", 560, 216), "voltage", 560, 200, "R0", "V3", "-15");
    k.label(560, 216, "vee", 560, 208);
    k.ground(560, 296);
    k.directive(48, 336, ".tran 0 3m");

    const components = [
      comp("V1", "voltage", "SINE(0 100m 1k)", [80, 176], "R0", [
        ["+", "N001"],
        ["-", "0"],
      ]),
      comp("Rin", "res", "1k", [128, 144], "R90", [
        ["A", "N001"],
        ["B", "N002"],
      ]),
      comp("Rf", "res", "10k", [272, 80], "R90", [
        ["A", "N002"],
        ["B", "N003"],
      ]),
      comp("U1", "opamp2", "LT1001", [272, 144], "R0", [
        ["In+", "0"],
        ["In-", "N002"],
        ["V+", "vcc"],
        ["V-", "vee"],
        ["OUT", "N003"],
      ]),
      comp("V2", "voltage", "15", [560, 88], "R0", [
        ["+", "vcc"],
        ["-", "0"],
      ]),
      comp("V3", "voltage", "-15", [560, 216], "R0", [
        ["+", "vee"],
        ["-", "0"],
      ]),
    ];
    if (rl) {
      components.splice(
        4,
        0,
        comp("RL", "res", "10k", [480, 208], "R0", [
          ["A", "N003"],
          ["B", rlGnd ? "0" : "N004"],
        ]),
      );
    }
    const findings: Finding[] = [];
    if (rl && !rlGnd) {
      findings.push({
        severity: "warning",
        rule: "single-pin-net",
        message: "Net N004 reaches only one pin (RL.B), so RL carries no current.",
        parts: ["RL"],
        nets: ["N004"],
        at: { x: 480, y: 288 },
      });
    }
    findings.push({
      severity: "info",
      rule: "off-grid",
      message: "U1 sits 8 units off the 16-unit grid.",
      parts: ["U1"],
      at: { x: 272, y: 144 },
    });
    const directives = [".tran 0 3m"];
    return {
      svg: k.toSvg(),
      asc: k.toAsc(),
      summary: summarize(components, ["vcc", "vee"], directives, ["Inverting amplifier, gain = -Rf/Rin"], findings),
      netlist: netlistOf("opamp_inverting.asc", components, directives, [".lib LTC.lib"]),
    };
  },

  simulate(s, simulator, path) {
    const t = linspace(0, 3e-3, 601);
    const loaded = s.flags.RL && s.flags.RLgnd;
    const gain = -10 / (1 + 11 / 1000);
    const vectors = [
      vec("time", "time"),
      vec("V(n001)", "voltage"),
      vec("V(n002)", "voltage"),
      vec("V(n003)", "voltage"),
      vec("I(Rf)", "current"),
      ...(s.flags.RL ? [vec("I(RL)", "current")] : []),
    ];
    const sample = (name: string, tt: number): number => {
      const vin = 0.1 * Math.sin(2 * Math.PI * 1000 * tt);
      const vout = gain * vin;
      const vn = -vout / 1000;
      switch (name) {
        case "V(n001)":
          return vin;
        case "V(n002)":
          return vn;
        case "V(n003)":
          return vout;
        case "I(Rf)":
          return (vn - vout) / 10e3;
        case "I(RL)":
          return loaded ? vout / 10e3 : 0;
        default:
          return 0;
      }
    };
    return {
      datasets: [{ index: 0, plotname: "Transient Analysis", kind: "transient", axis: "time", vectors, points: t.length, steps: [] }],
      measurements: [
        measurement("gain", gain, "V/V", "V(n003)/V(n001)"),
        measurement("vout_pp", Math.abs(gain) * 0.2, "V"),
        ...(s.flags.RL ? [measurement("iload_pk", loaded ? (Math.abs(gain) * 0.1) / 10e3 : 0, "A", "Peak current in RL")] : []),
      ],
      op: {},
      errors: [],
      warnings: s.flags.RL && !loaded ? ["Net N004 has only one connection; RL is floating."] : [],
      log: ngspiceLog(path, simulator, "Transient analysis: 0 to 3 ms", t.length),
      duration_ms: 351,
      wave(_dataset, signals, maxPoints) {
        const series = signals.map((name) => ({ name, step: "", y: t.map((tt) => sample(name, tt)) }));
        const out = pick(t, series, maxPoints);
        return { kind: "transient", x_name: "time", x_unit: "s", log_x: false, x: out.x, series: out.series };
      },
    };
  },

  specs(s) {
    const gain = -10 / (1 + 11 / 1000);
    const loaded = s.flags.RL && s.flags.RLgnd;
    return report([
      spec("Closed-loop gain", gain, -10.2, -9.8, "V/V"),
      spec("Load current, peak", loaded ? (Math.abs(gain) * 0.1) / 10e3 : 0, 50e-6, null, "A"),
    ]);
  },
};

/* A new, empty schematic ------------------------------------------------- */

export const emptyCircuit = (name: string): CircuitDef => ({
  build() {
    const k = new Sheet(480, 200);
    k.comment(48, 96, "Empty schematic. Ask the agent to draw a circuit, or open it in LTspice.");
    return {
      svg: k.toSvg(),
      asc: k.toAsc(),
      summary: summarize(
        [],
        [],
        [],
        [],
        [
          { severity: "error", rule: "no-ground", message: "The circuit has no ground (a flag labelled 0)." },
          { severity: "warning", rule: "no-analysis", message: "There is no analysis directive (.tran, .ac, .dc, .op, .noise, .tf)." },
        ],
      ),
      netlist: `* ${name}\n.backanno\n.end\n`,
    };
  },
  simulate(_s, simulator, path) {
    return {
      datasets: [],
      measurements: [],
      op: {},
      errors: ["No analysis directive. Add .tran, .ac, .dc or .op to the schematic."],
      warnings: [],
      log: ngspiceLog(path, simulator, "Error: no analysis found", 0),
      duration_ms: 38,
      wave: () => ({ kind: "other", x_name: "", x_unit: "", log_x: false, x: [], series: [] }),
    };
  },
  specs: () => null,
});

export interface Seed {
  path: string;
  def: CircuitDef;
  /** Oldest first; the last one is current. */
  versions: Array<{ summary: string; ageMs: number; state: CircuitState }>;
}

const HOUR = 3_600_000;

export function seedCircuits(): Seed[] {
  return [
    {
      path: "rc_lowpass.asc",
      def: rcLowpass,
      versions: [
        { summary: "Opened rc_lowpass.asc", ageMs: 26 * HOUR, state: { values: { R1: "2.2k", C1: "100n" }, flags: {} } },
        { summary: "Set R1 to 1k", ageMs: 0.2 * HOUR, state: { values: { R1: "1k", C1: "100n" }, flags: {} } },
      ],
    },
    {
      path: "ce_amp.asc",
      def: ceAmp,
      versions: [{ summary: "Opened ce_amp.asc", ageMs: 3 * HOUR, state: { values: { RC: "3.9k" }, flags: {} } }],
    },
    {
      path: "opamp_inverting.asc",
      def: opampInverting,
      versions: [
        { summary: "Opened opamp_inverting.asc", ageMs: 50 * HOUR, state: { values: {}, flags: {} } },
        { summary: "Added load resistor RL", ageMs: 49 * HOUR, state: { values: {}, flags: { RL: true } } },
      ],
    },
  ];
}
