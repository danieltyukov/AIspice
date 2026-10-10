/*
 * Pan and zoom as plain math, so it can be tested without a browser.
 *
 * A view is { x, y, k }: a point u in schematic units appears on screen at
 * u * k + (x, y). The drawing is shown by setting the SVG viewBox to the
 * window this describes, which keeps text and strokes crisp at any zoom.
 */

export interface View {
  x: number;
  y: number;
  k: number;
}

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export const MIN_ZOOM = 0.15;
export const MAX_ZOOM = 12;

const clampK = (k: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, k));

/** Fits the whole drawing in the pane with a margin, centered. */
export function fit(content: Box, width: number, height: number, pad = 28): View {
  if (content.width <= 0 || content.height <= 0 || width <= 0 || height <= 0) return { x: 0, y: 0, k: 1 };
  const k = clampK(Math.min((width - pad * 2) / content.width, (height - pad * 2) / content.height, 1.6));
  return {
    k,
    x: (width - content.width * k) / 2 - content.x * k,
    y: (height - content.height * k) / 2 - content.y * k,
  };
}

/** Zooms by `factor` keeping the screen point (sx, sy) fixed. */
export function zoomAt(v: View, factor: number, sx: number, sy: number): View {
  const k = clampK(v.k * factor);
  const f = k / v.k;
  return { k, x: sx - (sx - v.x) * f, y: sy - (sy - v.y) * f };
}

export function pan(v: View, dx: number, dy: number): View {
  return { ...v, x: v.x + dx, y: v.y + dy };
}

/** Centers a box in the pane, zooming in if the part would be tiny. */
export function centerOn(v: View, box: Box, width: number, height: number, minK = 1.4): View {
  const k = Math.max(v.k, minK);
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  return { k: clampK(k), x: width / 2 - cx * k, y: height / 2 - cy * k };
}

/** The viewBox string for a pane of the given size. */
export function viewBox(v: View, width: number, height: number): string {
  const f = (n: number) => Number(n.toFixed(3));
  return `${f(-v.x / v.k)} ${f(-v.y / v.k)} ${f(width / v.k)} ${f(height / v.k)}`;
}

export function screenToUser(v: View, sx: number, sy: number): { x: number; y: number } {
  return { x: (sx - v.x) / v.k, y: (sy - v.y) / v.k };
}

export function parseViewBox(text: string | null): Box | null {
  if (!text) return null;
  const n = text
    .trim()
    .split(/[\s,]+/)
    .map(Number);
  if (n.length !== 4 || n.some((v) => !Number.isFinite(v))) return null;
  return { x: n[0], y: n[1], width: n[2], height: n[3] };
}
