/*
 * Number and time formatting.
 *
 * Plot ticks use SPICE suffixes (1k, 10k, 1Meg, 100u) because that is how
 * values are written in the netlist next to them. Readouts use engineering
 * notation with SI prefixes and a unit, which reads better in a sentence.
 */

const SPICE: ReadonlyArray<readonly [number, string]> = [
  [1e12, "T"],
  [1e9, "G"],
  [1e6, "Meg"],
  [1e3, "k"],
  [1, ""],
  [1e-3, "m"],
  [1e-6, "u"],
  [1e-9, "n"],
  [1e-12, "p"],
  [1e-15, "f"],
];

const SI: ReadonlyArray<readonly [number, string]> = [
  [1e12, "T"],
  [1e9, "G"],
  [1e6, "M"],
  [1e3, "k"],
  [1, ""],
  [1e-3, "m"],
  [1e-6, "µ"],
  [1e-9, "n"],
  [1e-12, "p"],
  [1e-15, "f"],
];

function pickScale(abs: number, table: ReadonlyArray<readonly [number, string]>): readonly [number, string] {
  for (const entry of table) {
    // A small tolerance so 999.9999 from float noise still lands on "1k".
    if (abs >= entry[0] * 0.9999995) return entry;
  }
  return table[table.length - 1];
}

function trimZeros(text: string): string {
  if (!text.includes(".")) return text;
  return text.replace(/\.?0+$/, "");
}

/** One value with a SPICE suffix, up to `digits` significant digits: 2200 -> "2.2k". */
export function spice(value: number, digits = 3): string {
  if (!Number.isFinite(value)) return String(value);
  if (value === 0) return "0";
  const abs = Math.abs(value);
  const [scale, suffix] = pickScale(abs, SPICE);
  const scaled = value / scale;
  const intDigits = Math.max(1, Math.floor(Math.log10(Math.abs(scaled) * 1.0000001)) + 1);
  const decimals = Math.max(0, digits - intDigits);
  let text = trimZeros(scaled.toFixed(decimals));
  if (text === "-0") text = "0";
  return text + suffix;
}

/**
 * Tick labels for one axis. Precision grows until neighbouring labels differ,
 * so a zoomed-in axis reads 1.001k, 1.002k rather than 1k, 1k.
 */
export function spiceTicks(values: number[]): string[] {
  for (let digits = 3; digits <= 7; digits++) {
    const labels = values.map((v) => spice(snapZero(v, values), digits));
    if (new Set(labels).size === labels.length) return labels;
  }
  return values.map((v) => spice(v, 8));
}

/** Ticks on a linear axis land on values like 1e-17 instead of 0. */
function snapZero(v: number, all: number[]): number {
  if (all.length < 2) return v;
  const step = Math.abs(all[1] - all[0]);
  return Math.abs(v) < step * 1e-6 ? 0 : v;
}

/** A readout value: 1005.3, "Hz" -> "1.005 kHz". */
export function eng(value: number | null | undefined, unit = "", digits = 4): string {
  if (value === null || value === undefined || Number.isNaN(value)) return "n/a";
  if (!Number.isFinite(value)) return value > 0 ? "inf" : "-inf";
  if (unit === "dB" || unit === "°" || unit === "deg") {
    const text = value.toFixed(Math.abs(value) >= 100 ? 1 : 2);
    return `${text === "-0.00" ? "0.00" : text} ${unit === "deg" ? "°" : unit}`.trim();
  }
  if (value === 0) return `0 ${unit}`.trim();
  const [scale, prefix] = pickScale(Math.abs(value), SI);
  const scaled = value / scale;
  const intDigits = Math.max(1, Math.floor(Math.log10(Math.abs(scaled) * 1.0000001)) + 1);
  const decimals = Math.max(0, digits - intDigits);
  return `${scaled.toFixed(decimals)} ${prefix}${unit}`.trim();
}

/** Simulation and tool durations: 85 ms, 0.4 s, 12 s, 2 min 5 s. */
export function duration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "";
  if (ms < 100) return `${Math.round(ms)} ms`;
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)} s`;
  if (ms < 60_000) return `${Math.round(ms / 1000)} s`;
  const min = Math.floor(ms / 60_000);
  const sec = Math.round((ms % 60_000) / 1000);
  return sec === 0 ? `${min} min` : `${min} min ${sec} s`;
}

/** "just now", "5 min ago", "3 h ago", "2 days ago", then a date. */
export function relative(time: number, now = Date.now()): string {
  const diff = Math.max(0, now - time);
  const min = Math.floor(diff / 60_000);
  if (min < 1) return "just now";
  if (min < 60) return `${min} min ago`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.floor(h / 24);
  if (d === 1) return "yesterday";
  if (d < 7) return `${d} days ago`;
  return new Date(time).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

/** Clock time for history rows: 14:05. */
export function clock(time: number): string {
  return new Date(time).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

export function plural(n: number, word: string, many = `${word}s`): string {
  return `${n} ${n === 1 ? word : many}`;
}

/** The file name from a project path: "sub/rc.asc" -> "rc.asc". */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** The unit a SPICE vector name implies: V(out) -> "V", I(R1) -> "A". */
export function vectorUnit(name: string): string {
  const n = name.trim().toLowerCase();
  if (n.startsWith("v(") || n.startsWith("v-") || n === "v") return "V";
  if (n.startsWith("i(") || n.startsWith("ix(") || n.startsWith("ib(") || n.startsWith("ic(") || n.startsWith("ie(")) return "A";
  if (n === "time") return "s";
  if (n === "frequency") return "Hz";
  return "";
}
