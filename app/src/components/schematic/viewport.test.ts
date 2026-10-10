import { describe, expect, it } from "vitest";
import { centerOn, fit, MAX_ZOOM, MIN_ZOOM, pan, parseViewBox, screenToUser, viewBox, zoomAt } from "./viewport";

describe("viewport math", () => {
  const content = { x: 0, y: 0, width: 560, height: 296 };

  it("fits a drawing centred with a margin, never past the zoom cap", () => {
    const v = fit(content, 800, 600, 28);
    expect(v.k).toBeCloseTo(Math.min((800 - 56) / 560, (600 - 56) / 296, 1.6));
    // Centred: the middle of the drawing lands in the middle of the pane.
    expect(content.width / 2 * v.k + v.x).toBeCloseTo(400);
    expect(content.height / 2 * v.k + v.y).toBeCloseTo(300);
    expect(fit(content, 4000, 4000).k).toBe(1.6);
  });

  it("zooms about a fixed screen point", () => {
    const v = { x: 10, y: 20, k: 1 };
    const before = screenToUser(v, 300, 200);
    const z = zoomAt(v, 2, 300, 200);
    const after = screenToUser(z, 300, 200);
    expect(z.k).toBe(2);
    expect(after.x).toBeCloseTo(before.x);
    expect(after.y).toBeCloseTo(before.y);
  });

  it("clamps zoom", () => {
    expect(zoomAt({ x: 0, y: 0, k: 1 }, 1000, 0, 0).k).toBe(MAX_ZOOM);
    expect(zoomAt({ x: 0, y: 0, k: 1 }, 0.0001, 0, 0).k).toBe(MIN_ZOOM);
  });

  it("pans and centres on a box", () => {
    expect(pan({ x: 1, y: 2, k: 3 }, 10, -5)).toEqual({ x: 11, y: -3, k: 3 });
    const c = centerOn({ x: 0, y: 0, k: 1 }, { x: 100, y: 100, width: 20, height: 20 }, 400, 300);
    expect(110 * c.k + c.x).toBeCloseTo(200);
    expect(110 * c.k + c.y).toBeCloseTo(150);
    expect(c.k).toBeGreaterThanOrEqual(1.4);
  });

  it("turns a view into a viewBox and parses one back", () => {
    expect(viewBox({ x: -50, y: -20, k: 2 }, 400, 300)).toBe("25 10 200 150");
    expect(parseViewBox("0 0 560 296")).toEqual(content);
    expect(parseViewBox("0,0,10")).toBeNull();
    expect(parseViewBox(null)).toBeNull();
  });
});
