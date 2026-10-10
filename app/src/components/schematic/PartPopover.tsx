import { useEffect, useRef } from "react";
import type { ComponentInfo, HighlightKind } from "../../ipc/types";

export interface PartPopoverProps {
  part: ComponentInfo;
  x: number;
  y: number;
  bounds: { width: number; height: number };
  highlight?: HighlightKind;
  onClose: () => void;
}

const WIDTH = 248;

/** A small card beside a clicked part: value, symbol, pins and the net on each. */
export function PartPopover({ part, x, y, bounds, highlight, onClose }: PartPopoverProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    ref.current?.focus();
  }, [part.name]);

  const height = ref.current?.offsetHeight ?? 180;
  const left = Math.max(8, Math.min(x, bounds.width - WIDTH - 8));
  const top = Math.max(8, Math.min(y, bounds.height - height - 8));
  const attrs = Object.entries(part.attrs ?? {});

  return (
    <div
      ref={ref}
      className="part-popover"
      role="dialog"
      aria-label={`${part.name} details`}
      tabIndex={-1}
      style={{ left, top, width: WIDTH }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="pop-head">
        <p className="pop-name mono">{part.name}</p>
        {highlight ? (
          <span className="chip" data-tone={highlight === "added" ? "pass" : "info"}>
            {highlight}
          </span>
        ) : null}
        <button type="button" className="button button-quiet pop-close" onClick={onClose} aria-label="Close details">
          Close
        </button>
      </div>
      {part.description ? <p className="pop-desc">{part.description}</p> : null}
      <dl className="pop-grid">
        <div className="pop-pair">
          <dt>Value</dt>
          <dd className="mono">{part.value ?? "none"}</dd>
        </div>
        <div className="pop-pair">
          <dt>Symbol</dt>
          <dd className="mono">{part.symbol}</dd>
        </div>
        {attrs.map(([k, v]) => (
          <div key={k} className="pop-pair">
            <dt>{k}</dt>
            <dd className="mono">{v}</dd>
          </div>
        ))}
      </dl>
      {part.pins.length > 0 ? (
        <table className="pop-pins">
          <thead>
            <tr>
              <th scope="col">Pin</th>
              <th scope="col">Net</th>
            </tr>
          </thead>
          <tbody>
            {part.pins.map((p) => (
              <tr key={p.pin}>
                <td className="mono">{p.pin}</td>
                <td className="mono">{p.net}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </div>
  );
}
