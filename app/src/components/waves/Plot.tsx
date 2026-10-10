import { useEffect, useRef, useState } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { spiceTicks } from "../../lib/format";
import { token } from "../../lib/theme";

export interface PlotSeries {
  label: string;
  color: string;
  values: number[];
  dash?: number[];
}

export interface Cursors {
  a: number | null;
  b: number | null;
}

export interface PlotProps {
  x: number[];
  series: PlotSeries[];
  logX: boolean;
  xLabel: string;
  yLabel: string;
  /** Used instead of yLabel when the pane is short. */
  yShort?: string;
  cursors: Cursors;
  onPick: (index: number) => void;
  syncKey?: string;
  /** Shared x range for panes that zoom together; null means the full range. */
  xRange?: [number, number] | null;
  onXRange?: (range: [number, number] | null) => void;
  showXLabels?: boolean;
  palette: number;
  ariaLabel: string;
}

/** On a log axis, grid lines at 1, 2 and 5 of each decade; nine per decade is noise. */
function logLines(_u: uPlot, splits: number[]): Array<number | null> {
  return splits.map((v) => {
    const m = Math.round(v / 10 ** Math.floor(Math.log10(v) + 1e-9));
    return m === 1 || m === 2 || m === 5 ? v : null;
  });
}

/** uPlot in a resizing box, with two placeable cursors drawn on its canvas. */
export function Plot({
  x,
  series,
  logX,
  xLabel,
  yLabel,
  yShort,
  cursors,
  onPick,
  syncKey,
  xRange,
  onXRange,
  showXLabels = true,
  palette,
  ariaLabel,
}: PlotProps) {
  const boxRef = useRef<HTMLDivElement>(null);
  const plotRef = useRef<uPlot | null>(null);
  // A short pane gets a short axis label; the long one would overrun it.
  const [compact, setCompact] = useState(false);
  const compactRef = useRef(compact);
  compactRef.current = compact;
  const cursorsRef = useRef(cursors);
  const pickRef = useRef(onPick);
  const rangeRef = useRef(onXRange);
  cursorsRef.current = cursors;
  pickRef.current = onPick;
  rangeRef.current = onXRange;

  useEffect(() => {
    const box = boxRef.current;
    if (!box || x.length === 0) return;

    const text = token("--muted", "#666");
    const grid = token("--border", "#ddd");
    const ink = token("--text", "#111");
    const faint = token("--faint", "#888");
    const mono = token("--font-mono", "monospace");
    const font = `11px ${mono}`;
    const full: [number, number] = [x[0], x[x.length - 1]];

    const values = (_u: uPlot, splits: number[]) => {
      if (!logX) return spiceTicks(splits);
      const labels = spiceTicks(splits);
      return splits.map((v, i) => (Math.abs(Math.log10(v) - Math.round(Math.log10(v))) < 1e-6 ? labels[i] : ""));
    };

    const drawCursors = (u: uPlot) => {
      const { ctx } = u;
      const { left, top, width, height } = u.bbox;
      const dpr = devicePixelRatio || 1;
      const marks: Array<[keyof Cursors, string, number[]]> = [
        ["a", ink, []],
        ["b", ink, [4 * dpr, 3 * dpr]],
      ];
      for (const [key, color, dash] of marks) {
        const idx = cursorsRef.current[key];
        if (idx === null || idx >= u.data[0].length) continue;
        const px = Math.round(u.valToPos(u.data[0][idx], "x", true));
        if (px < left - 1 || px > left + width + 1) continue;
        ctx.save();
        ctx.strokeStyle = color;
        ctx.lineWidth = dpr;
        ctx.setLineDash(dash);
        ctx.beginPath();
        ctx.moveTo(px + 0.5, top);
        ctx.lineTo(px + 0.5, top + height);
        ctx.stroke();
        ctx.setLineDash([]);
        ctx.fillStyle = color;
        ctx.font = `600 ${11 * dpr}px ${mono}`;
        ctx.fillText(key.toUpperCase(), px + 4 * dpr, top + 12 * dpr);
        for (let s = 1; s < u.series.length; s++) {
          const v = u.data[s][idx];
          if (v === null || v === undefined || !Number.isFinite(v as number)) continue;
          const py = u.valToPos(v as number, "y", true);
          ctx.beginPath();
          ctx.fillStyle = series[s - 1]?.color ?? color;
          ctx.arc(px + 0.5, py, 3 * dpr, 0, Math.PI * 2);
          ctx.fill();
        }
        ctx.restore();
      }
    };

    const opts: uPlot.Options = {
      width: Math.max(100, box.clientWidth),
      height: Math.max(80, box.clientHeight),
      pxAlign: false,
      scales: {
        x: logX ? { time: false, distr: 3, log: 10 } : { time: false },
        y: { auto: true },
      },
      axes: [
        {
          stroke: text,
          font,
          labelFont: font,
          label: showXLabels ? xLabel : undefined,
          labelSize: showXLabels ? 18 : 0,
          size: showXLabels ? 30 : 8,
          grid: { stroke: grid, width: 1, ...(logX ? { filter: logLines } : {}) },
          ticks: { stroke: grid, width: 1, size: 4, ...(logX ? { filter: logLines } : {}) },
          values: showXLabels ? values : () => [],
          space: logX ? 30 : 60,
        },
        {
          stroke: text,
          font,
          labelFont: font,
          label: compact && yShort ? yShort : yLabel,
          labelSize: 18,
          size: 58,
          grid: { stroke: grid, width: 1 },
          ticks: { stroke: grid, width: 1, size: 4 },
          values: (_u, splits) => spiceTicks(splits),
        },
      ],
      series: [
        { label: xLabel },
        ...series.map((s) => ({
          label: s.label,
          stroke: s.color,
          width: 1.6,
          dash: s.dash,
          points: { show: false },
        })),
      ],
      legend: { show: false },
      cursor: {
        sync: syncKey ? { key: syncKey } : undefined,
        drag: { x: true, y: false, setScale: true },
        points: { size: 6, width: 1.5 },
        x: true,
        y: false,
      },
      hooks: {
        draw: [drawCursors],
        setScale: [
          (u, key) => {
            if (key !== "x" || !rangeRef.current) return;
            const min = u.scales.x.min ?? full[0];
            const max = u.scales.x.max ?? full[1];
            const isFull = Math.abs(min - full[0]) <= Math.abs(full[1] - full[0]) * 1e-9 && Math.abs(max - full[1]) <= Math.abs(full[1] - full[0]) * 1e-9;
            rangeRef.current(isFull ? null : [min, max]);
          },
        ],
      },
    };

    const data: uPlot.AlignedData = [x, ...series.map((s) => s.values)];
    const u = new uPlot(opts, data, box);
    plotRef.current = u;
    u.root.style.setProperty("--u-faint", faint);

    // A press and release in place picks; a drag zooms (uPlot's own behaviour).
    let down: { x: number; y: number } | null = null;
    const over = u.over;
    const onDown = (e: MouseEvent) => {
      down = { x: e.clientX, y: e.clientY };
    };
    const onUp = (e: MouseEvent) => {
      if (!down) return;
      const moved = Math.hypot(e.clientX - down.x, e.clientY - down.y);
      down = null;
      if (moved > 3) return;
      const idx = u.cursor.idx;
      if (idx !== null && idx !== undefined) pickRef.current(idx);
    };
    over.addEventListener("mousedown", onDown);
    over.addEventListener("mouseup", onUp);

    const isShort = () => box.clientHeight < 150;
    if (isShort() !== compactRef.current) setCompact(isShort());
    const ro =
      typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(() => {
            if (isShort() !== compactRef.current) setCompact(isShort());
            else u.setSize({ width: Math.max(100, box.clientWidth), height: Math.max(80, box.clientHeight) });
          })
        : null;
    ro?.observe(box);

    return () => {
      ro?.disconnect();
      over.removeEventListener("mousedown", onDown);
      over.removeEventListener("mouseup", onUp);
      u.destroy();
      plotRef.current = null;
    };
  }, [x, series, logX, xLabel, yLabel, yShort, compact, syncKey, showXLabels, palette]);

  // Cursor moves only redraw; they never rebuild the plot.
  useEffect(() => {
    plotRef.current?.redraw(false, false);
  }, [cursors.a, cursors.b]);

  // Zoom shared with a sibling pane.
  useEffect(() => {
    const u = plotRef.current;
    if (!u || xRange === undefined) return;
    const full: [number, number] = [x[0], x[x.length - 1]];
    const [min, max] = xRange ?? full;
    if (u.scales.x.min !== min || u.scales.x.max !== max) u.setScale("x", { min, max });
  }, [xRange, x]);

  return <div ref={boxRef} className="plot" role="img" aria-label={ariaLabel} />;
}
