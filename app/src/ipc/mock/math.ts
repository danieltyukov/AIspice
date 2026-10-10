/* Numeric helpers for the mock simulator. */

const SUFFIX: Record<string, number> = {
  t: 1e12,
  g: 1e9,
  meg: 1e6,
  k: 1e3,
  m: 1e-3,
  u: 1e-6,
  n: 1e-9,
  p: 1e-12,
  f: 1e-15,
};

/** "4.7k" -> 4700, "1Meg" -> 1e6, "160n" -> 1.6e-7. */
export function parseSpice(text: string): number {
  const m = /^\s*([-+]?\d*\.?\d+(?:e[-+]?\d+)?)\s*(meg|[tgkmunpf])?/i.exec(text);
  if (!m) return NaN;
  const base = Number(m[1]);
  const suffix = m[2]?.toLowerCase();
  return suffix ? base * SUFFIX[suffix] : base;
}

export function linspace(a: number, b: number, n: number): number[] {
  const out = new Array<number>(n);
  for (let i = 0; i < n; i++) out[i] = a + ((b - a) * i) / (n - 1);
  return out;
}

export function logspace(decStart: number, decEnd: number, perDecade: number): number[] {
  const n = Math.round((decEnd - decStart) * perDecade) + 1;
  const out = new Array<number>(n);
  for (let i = 0; i < n; i++) out[i] = 10 ** (decStart + i / perDecade);
  return out;
}

export function db(mag: number): number {
  return 20 * Math.log10(Math.max(mag, 1e-30));
}

export function deg(rad: number): number {
  return (rad * 180) / Math.PI;
}

export function parallel(a: number, b: number): number {
  return (a * b) / (a + b);
}

/** Keeps at most `max` points by even striding; the last point always survives. */
export function decimate<T>(xs: T[], max: number | undefined): number[] {
  const n = xs.length;
  if (!max || n <= max) return Array.from({ length: n }, (_, i) => i);
  const idx: number[] = [];
  const step = (n - 1) / (max - 1);
  for (let i = 0; i < max; i++) idx.push(Math.round(i * step));
  return idx;
}
