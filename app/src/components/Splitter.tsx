import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import "./Splitter.css";

export interface SplitterProps {
  label: string;
  /** "vertical" divides columns (drag left and right); "horizontal" divides rows. */
  orientation: "vertical" | "horizontal";
  value: number;
  min: number;
  max: number;
  /** Pixels per arrow press; Shift multiplies by four. */
  step?: number;
  /** True when moving the pointer right (or down) should shrink the value. */
  invert?: boolean;
  onChange: (value: number) => void;
  onReset?: () => void;
}

/*
 * A resize handle that works by pointer and by keyboard: arrows move it, Home
 * and End jump to the limits, Enter or a double click restores the default.
 */
export function Splitter({ label, orientation, value, min, max, step = 16, invert = false, onChange, onReset }: SplitterProps) {
  const drag = useRef<{ start: number; value: number } | null>(null);
  const clamp = (v: number) => Math.min(max, Math.max(min, Math.round(v)));
  const pos = (e: PointerEvent) => (orientation === "vertical" ? e.clientX : e.clientY);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { start: pos(e), value };
    document.body.dataset.resizing = orientation;
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const delta = pos(e) - drag.current.start;
    onChange(clamp(drag.current.value + (invert ? -delta : delta)));
  };

  const end = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    drag.current = null;
    delete document.body.dataset.resizing;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const amount = step * (e.shiftKey ? 4 : 1);
    const grow = orientation === "vertical" ? (invert ? "ArrowLeft" : "ArrowRight") : invert ? "ArrowUp" : "ArrowDown";
    const shrink = orientation === "vertical" ? (invert ? "ArrowRight" : "ArrowLeft") : invert ? "ArrowDown" : "ArrowUp";
    let next: number | null = null;
    if (e.key === grow) next = value + amount;
    else if (e.key === shrink) next = value - amount;
    else if (e.key === "Home") next = min;
    else if (e.key === "End") next = max;
    else if (e.key === "Enter" && onReset) {
      e.preventDefault();
      onReset();
      return;
    }
    if (next !== null) {
      e.preventDefault();
      onChange(clamp(next));
    }
  };

  return (
    <div
      className="splitter"
      data-orientation={orientation}
      role="separator"
      aria-label={label}
      aria-orientation={orientation}
      aria-valuenow={Math.round(value)}
      aria-valuemin={Math.round(min)}
      aria-valuemax={Math.round(max)}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
      onKeyDown={onKeyDown}
      onDoubleClick={onReset}
    />
  );
}
