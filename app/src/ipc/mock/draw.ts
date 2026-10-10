/*
 * A few hand-drawn schematic symbols for the mock backend. Coordinates follow
 * LTspice's grid (16 units). Output uses the same conventions the real
 * renderer is expected to follow:
 *
 *   <svg class="sch" viewBox="...">            root
 *   <g class="sch-part" data-inst="R1">        one placed part, clickable
 *     <rect class="sch-hit"/>                  transparent hit area
 *     <path class="sch-symbol"/>               body strokes
 *     <text class="sch-name">, <text class="sch-value">
 *   <polyline class="sch-wire"/>, <circle class="sch-junction"/>
 *   <text class="sch-label">                   net labels
 *   <text class="sch-directive">, <text class="sch-comment">
 */

export function esc(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

type Pt = readonly [number, number];

export function wire(...pts: Pt[]): string {
  return `<polyline class="sch-wire" points="${pts.map(([x, y]) => `${x},${y}`).join(" ")}"/>`;
}

export function junction(x: number, y: number): string {
  return `<circle class="sch-junction" cx="${x}" cy="${y}" r="3.5"/>`;
}

export function ground(x: number, y: number): string {
  return `<path class="sch-symbol sch-flag" d="M${x} ${y}V${y + 8}M${x - 12} ${y + 8}H${x + 12}M${x - 7} ${y + 13}H${x + 7}M${x - 2.5} ${y + 18}H${x + 2.5}"/>`;
}

export function label(x: number, y: number, text: string, anchor: "start" | "middle" | "end" = "middle"): string {
  return `<text class="sch-label" x="${x}" y="${y}" text-anchor="${anchor}">${esc(text)}</text>`;
}

export function directive(x: number, y: number, text: string): string {
  return `<text class="sch-directive" x="${x}" y="${y}">${esc(text)}</text>`;
}

export function comment(x: number, y: number, text: string): string {
  return `<text class="sch-comment" x="${x}" y="${y}">${esc(text)}</text>`;
}

function part(name: string, hit: [number, number, number, number], body: string, texts: string): string {
  const [x, y, w, h] = hit;
  return (
    `<g class="sch-part" data-inst="${esc(name)}">` +
    `<rect class="sch-hit" x="${x}" y="${y}" width="${w}" height="${h}" rx="4"/>` +
    body +
    texts +
    `</g>`
  );
}

function names(name: string, value: string, x: number, y: number, anchor: "start" | "middle" = "start", gap = 16): string {
  return (
    `<text class="sch-name" x="${x}" y="${y}" text-anchor="${anchor}">${esc(name)}</text>` +
    `<text class="sch-value" x="${x}" y="${y + gap}" text-anchor="${anchor}">${esc(value)}</text>`
  );
}

/** Vertical resistor, pins at (x, y) and (x, y + 80). */
export function resistorV(name: string, value: string, x: number, y: number): string {
  const d =
    `M${x} ${y}V${y + 16}L${x + 8} ${y + 20}L${x - 8} ${y + 28}L${x + 8} ${y + 36}` +
    `L${x - 8} ${y + 44}L${x + 8} ${y + 52}L${x - 8} ${y + 60}L${x} ${y + 64}V${y + 80}`;
  return part(name, [x - 14, y + 10, 28, 60], `<path class="sch-symbol" d="${d}"/>`, names(name, value, x + 16, y + 36));
}

/** Horizontal resistor, pins at (x, y) and (x + 80, y). */
export function resistorH(name: string, value: string, x: number, y: number): string {
  const d =
    `M${x} ${y}H${x + 16}L${x + 20} ${y - 8}L${x + 28} ${y + 8}L${x + 36} ${y - 8}` +
    `L${x + 44} ${y + 8}L${x + 52} ${y - 8}L${x + 60} ${y + 8}L${x + 64} ${y}H${x + 80}`;
  return part(
    name,
    [x + 10, y - 14, 60, 28],
    `<path class="sch-symbol" d="${d}"/>`,
    `<text class="sch-name" x="${x + 40}" y="${y - 16}" text-anchor="middle">${esc(name)}</text>` +
      `<text class="sch-value" x="${x + 40}" y="${y + 26}" text-anchor="middle">${esc(value)}</text>`,
  );
}

/** Vertical capacitor, pins at (x, y) and (x, y + 64). */
export function capV(name: string, value: string, x: number, y: number): string {
  const d = `M${x} ${y}V${y + 28}M${x - 16} ${y + 28}H${x + 16}M${x - 16} ${y + 36}H${x + 16}M${x} ${y + 36}V${y + 64}`;
  return part(name, [x - 18, y + 18, 36, 28], `<path class="sch-symbol" d="${d}"/>`, names(name, value, x + 22, y + 28));
}

/** Horizontal capacitor, pins at (x, y) and (x + 64, y). */
export function capH(name: string, value: string, x: number, y: number): string {
  const d = `M${x} ${y}H${x + 28}M${x + 28} ${y - 16}V${y + 16}M${x + 36} ${y - 16}V${y + 16}M${x + 36} ${y}H${x + 64}`;
  return part(
    name,
    [x + 18, y - 18, 28, 36],
    `<path class="sch-symbol" d="${d}"/>`,
    `<text class="sch-name" x="${x + 32}" y="${y - 22}" text-anchor="middle">${esc(name)}</text>` +
      `<text class="sch-value" x="${x + 32}" y="${y + 32}" text-anchor="middle">${esc(value)}</text>`,
  );
}

/** Voltage source, + at (x, y), - at (x, y + 80). */
export function voltage(name: string, value: string, x: number, y: number, kind: "dc" | "sine" | "ac" = "dc"): string {
  const cy = y + 40;
  let d = `M${x} ${y}V${y + 18}M${x} ${y + 62}V${y + 80}`;
  if (kind === "sine") {
    d += `M${x - 10} ${cy}q5 -11 10 0t10 0`;
  } else {
    d += `M${x} ${y + 25}V${y + 33}M${x - 4} ${y + 29}H${x + 4}M${x - 4} ${y + 52}H${x + 4}`;
  }
  return part(
    name,
    [x - 24, y + 16, 48, 48],
    `<circle class="sch-symbol" cx="${x}" cy="${cy}" r="22"/><path class="sch-symbol" d="${d}"/>`,
    names(name, value, x + 30, y + 36),
  );
}

/** NPN transistor: base at (x, y), collector at (x + 48, y - 48), emitter at (x + 48, y + 48). */
export function npn(name: string, value: string, x: number, y: number): string {
  const d =
    `M${x} ${y}H${x + 28}M${x + 28} ${y - 18}V${y + 18}` +
    `M${x + 28} ${y - 8}L${x + 48} ${y - 26}V${y - 48}` +
    `M${x + 28} ${y + 8}L${x + 48} ${y + 26}V${y + 48}`;
  const arrow = `M${x + 47} ${y + 25}l-10 -2l5 -6z`;
  return part(
    name,
    [x + 14, y - 34, 42, 68],
    `<path class="sch-symbol" d="${d}"/><path class="sch-symbol sch-fill" d="${arrow}"/>`,
    names(name, value, x + 58, y - 4),
  );
}

/**
 * Op amp: inverting input (x, y), non-inverting (x, y + 64), output
 * (x + 128, y + 32), V+ (x + 64, y - 16), V- (x + 64, y + 80).
 */
export function opamp(name: string, value: string, x: number, y: number): string {
  const d =
    `M${x + 16} ${y - 24}L${x + 16} ${y + 88}L${x + 112} ${y + 32}Z` +
    `M${x} ${y}H${x + 16}M${x} ${y + 64}H${x + 16}M${x + 112} ${y + 32}H${x + 128}` +
    `M${x + 64} ${y - 16}V${y + 4}M${x + 64} ${y + 60}V${y + 80}` +
    `M${x + 24} ${y}H${x + 32}M${x + 24} ${y + 64}H${x + 32}M${x + 28} ${y + 60}V${y + 68}`;
  return part(
    name,
    [x + 12, y - 26, 104, 116],
    `<path class="sch-symbol" d="${d}"/>`,
    `<text class="sch-name" x="${x + 96}" y="${y - 20}">${esc(name)}</text>` +
      `<text class="sch-value" x="${x + 96}" y="${y - 4}">${esc(value)}</text>`,
  );
}

export function sheet(width: number, height: number, body: string[]): string {
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" class="sch" viewBox="0 0 ${width} ${height}" ` +
    `width="${width}" height="${height}">` +
    body.join("") +
    `</svg>`
  );
}
