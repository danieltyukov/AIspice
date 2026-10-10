import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent, type WheelEvent } from "react";
import type { ComponentInfo, Finding, Highlight } from "../../ipc/types";
import { sanitizeSvg } from "../../lib/sanitizeSvg";
import { useStore, useStoreApi } from "../../store/context";
import { centerOn, fit, pan, parseViewBox, viewBox, zoomAt, type Box, type View } from "./viewport";
import { PartPopover } from "./PartPopover";
import { Findings } from "./Findings";
import "./Schematic.css";

interface Popover {
  part: ComponentInfo;
  x: number;
  y: number;
}

const SVG_NS = "http://www.w3.org/2000/svg";

function bboxOf(el: Element): Box | null {
  const g = el as SVGGraphicsElement;
  if (typeof g.getBBox !== "function") return null;
  try {
    const b = g.getBBox();
    return { x: b.x, y: b.y, width: b.width, height: b.height };
  } catch {
    return null;
  }
}

/** Marks highlighted parts and draws a dashed box behind each. Works on a plain or an annotated SVG. */
function applyHighlights(svg: SVGSVGElement, highlights: Highlight[]): void {
  svg.querySelectorAll(".sch-hl-box").forEach((n) => n.remove());
  const wanted = new Map(highlights.map((h) => [h.inst.toUpperCase(), h.kind]));
  svg.querySelectorAll<SVGElement>("[data-inst]").forEach((el) => {
    const inst = (el.getAttribute("data-inst") ?? "").toUpperCase();
    const kind = wanted.get(inst) ?? el.getAttribute("data-hl");
    if (!kind) return;
    el.setAttribute("data-hl", kind);
    const box = bboxOf(el);
    if (!box) return;
    const rect = document.createElementNS(SVG_NS, "rect");
    rect.setAttribute("class", "sch-hl-box");
    rect.setAttribute("data-kind", kind);
    rect.setAttribute("x", String(box.x - 7));
    rect.setAttribute("y", String(box.y - 7));
    rect.setAttribute("width", String(box.width + 14));
    rect.setAttribute("height", String(box.height + 14));
    rect.setAttribute("rx", "5");
    el.insertBefore(rect, el.firstChild);
  });
}

export function SchematicView() {
  const store = useStoreApi();
  const view = useStore((s) => s.view);
  const viewLoading = useStore((s) => s.viewLoading);
  const viewError = useStore((s) => s.viewError);
  const highlights = useStore((s) => s.highlights);
  const locate = useStore((s) => s.locate);

  const markup = useMemo(() => sanitizeSvg(view?.svg ?? ""), [view?.svg]);
  const paneRef = useRef<HTMLDivElement>(null);
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<View>({ x: 0, y: 0, k: 1 });
  const contentRef = useRef<Box | null>(null);
  const fittedRef = useRef(true);
  const pathRef = useRef<string | null>(null);
  const sizeRef = useRef({ width: 0, height: 0 });
  const [zoom, setZoom] = useState(1);
  const [popover, setPopover] = useState<Popover | null>(null);
  const [selected, setSelected] = useState<string | null>(null);

  const svgEl = () => hostRef.current?.querySelector("svg") ?? null;

  const commit = useCallback((next: View, userAction = true) => {
    viewRef.current = next;
    if (userAction) fittedRef.current = false;
    const svg = hostRef.current?.querySelector("svg");
    const { width, height } = sizeRef.current;
    if (svg && width > 0 && height > 0) svg.setAttribute("viewBox", viewBox(next, width, height));
    setZoom(next.k);
  }, []);

  const doFit = useCallback(() => {
    const content = contentRef.current;
    const { width, height } = sizeRef.current;
    if (!content) return;
    commit(fit(content, width, height), false);
    fittedRef.current = true;
  }, [commit]);

  // New markup: inject it, read its natural size, size the SVG to the pane,
  // then keep the current view or fit. The host's children are managed here,
  // not by React, so re-renders never reset the viewBox or the highlight boxes.
  const lastMarkup = useRef<string | null>(null);
  useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) {
      lastMarkup.current = null;
      return;
    }
    if (lastMarkup.current !== markup) {
      host.innerHTML = markup;
      lastMarkup.current = markup;
      const svg = host.querySelector("svg");
      if (!svg) return;
      const natural = parseViewBox(svg.getAttribute("viewBox"));
      if (natural) contentRef.current = natural;
      svg.setAttribute("width", "100%");
      svg.setAttribute("height", "100%");
      svg.setAttribute("preserveAspectRatio", "xMidYMid meet");
      svg.setAttribute("role", "img");
      svg.setAttribute("aria-label", `Schematic of ${view?.path ?? "the circuit"}`);
    }
    const pane = paneRef.current;
    if (pane) sizeRef.current = { width: pane.clientWidth, height: pane.clientHeight };
    if (view?.path !== pathRef.current || fittedRef.current) {
      pathRef.current = view?.path ?? null;
      doFit();
    } else {
      commit(viewRef.current, false);
    }
  }, [markup, view?.path, doFit, commit]);

  useLayoutEffect(() => {
    const svg = svgEl();
    if (svg) applyHighlights(svg as SVGSVGElement, highlights);
  }, [markup, highlights]);

  useLayoutEffect(() => {
    const svg = svgEl();
    if (!svg) return;
    svg.querySelectorAll("[data-selected]").forEach((n) => n.removeAttribute("data-selected"));
    if (selected) {
      svg.querySelectorAll(`[data-inst]`).forEach((n) => {
        if (n.getAttribute("data-inst")?.toUpperCase() === selected.toUpperCase()) n.setAttribute("data-selected", "");
      });
    }
  }, [markup, selected]);

  // Close the popover when the circuit changes underneath it.
  useEffect(() => {
    setPopover(null);
    setSelected(null);
  }, [view?.path]);

  useEffect(() => {
    const pane = paneRef.current;
    if (!pane || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      sizeRef.current = { width: pane.clientWidth, height: pane.clientHeight };
      if (fittedRef.current) doFit();
      else commit(viewRef.current, false);
    });
    ro.observe(pane);
    return () => ro.disconnect();
  }, [doFit, commit]);

  const partByName = useCallback(
    (inst: string) => view?.summary.components.find((c) => c.name.toUpperCase() === inst.toUpperCase()) ?? null,
    [view],
  );

  // Locate requests from findings and chat cards.
  useEffect(() => {
    if (!locate) return;
    const svg = svgEl();
    const el = [...(svg?.querySelectorAll("[data-inst]") ?? [])].find(
      (n) => n.getAttribute("data-inst")?.toUpperCase() === locate.inst.toUpperCase(),
    );
    const part = partByName(locate.inst);
    const { width, height } = sizeRef.current;
    const box = el ? bboxOf(el) : part ? { x: part.at.x - 20, y: part.at.y - 20, width: 40, height: 40 } : null;
    if (box) commit(centerOn(viewRef.current, box, width, height));
    setSelected(locate.inst);
    if (part) setPopover({ part, x: width / 2 + 24, y: height / 2 - 12 });
    // Only a new nonce should move the view.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [locate?.nonce]);

  /* Pointer: drag pans, two pointers pinch, a click without movement picks a part. */
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const gesture = useRef<{ moved: boolean; startX: number; startY: number; dist: number } | null>(null);

  const local = (e: { clientX: number; clientY: number }) => {
    const r = paneRef.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 && e.pointerType === "mouse") return;
    if ((e.target as HTMLElement).closest(".part-popover, .sch-tools")) return;
    paneRef.current?.setPointerCapture(e.pointerId);
    const p = local(e);
    pointers.current.set(e.pointerId, p);
    if (pointers.current.size === 1) {
      gesture.current = { moved: false, startX: p.x, startY: p.y, dist: 0 };
    } else if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      gesture.current = { moved: true, startX: (a.x + b.x) / 2, startY: (a.y + b.y) / 2, dist: Math.hypot(a.x - b.x, a.y - b.y) };
    }
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const prev = pointers.current.get(e.pointerId);
    if (!prev || !gesture.current) return;
    const p = local(e);
    pointers.current.set(e.pointerId, p);
    if (pointers.current.size >= 2) {
      const [a, b] = [...pointers.current.values()];
      const dist = Math.hypot(a.x - b.x, a.y - b.y);
      const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
      if (gesture.current.dist > 0) {
        let next = zoomAt(viewRef.current, dist / gesture.current.dist, mid.x, mid.y);
        next = pan(next, mid.x - gesture.current.startX, mid.y - gesture.current.startY);
        commit(next);
      }
      gesture.current = { moved: true, startX: mid.x, startY: mid.y, dist };
      return;
    }
    const dx = p.x - prev.x;
    const dy = p.y - prev.y;
    if (!gesture.current.moved && Math.hypot(p.x - gesture.current.startX, p.y - gesture.current.startY) < 4) return;
    if (!gesture.current.moved) {
      gesture.current.moved = true;
      paneRef.current?.setAttribute("data-panning", "");
    }
    commit(pan(viewRef.current, dx, dy));
  };

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    pointers.current.delete(e.pointerId);
    paneRef.current?.removeAttribute("data-panning");
    if (paneRef.current?.hasPointerCapture(e.pointerId)) paneRef.current.releasePointerCapture(e.pointerId);
    if (pointers.current.size > 0) return;
    gesture.current = null;
    if (!g || g.moved) return;
    // A click: find the part under the pointer.
    const target = document.elementFromPoint(e.clientX, e.clientY);
    const el = target?.closest("[data-inst]");
    const inst = el?.getAttribute("data-inst");
    if (el && inst) {
      const part = partByName(inst);
      // Beside the part rather than under the pointer, so its value stays visible.
      const r = el.getBoundingClientRect();
      const p = local({ clientX: r.right, clientY: r.top });
      setSelected(inst);
      setPopover(part ? { part, x: p.x + 14, y: p.y - 6 } : null);
    } else {
      setSelected(null);
      setPopover(null);
    }
  };

  const onWheel = (e: WheelEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest(".part-popover")) return;
    const p = local(e);
    // Trackpad pinch arrives as a wheel event with ctrlKey set.
    const speed = e.ctrlKey ? 0.01 : 0.0015;
    const delta = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    commit(zoomAt(viewRef.current, Math.exp(-delta * speed), p.x, p.y));
  };

  // React attaches wheel listeners as passive; this one must be able to prevent page zoom.
  useEffect(() => {
    const pane = paneRef.current;
    if (!pane) return;
    const block = (e: globalThis.WheelEvent) => e.preventDefault();
    pane.addEventListener("wheel", block, { passive: false });
    return () => pane.removeEventListener("wheel", block);
  }, []);

  const zoomBy = (factor: number) => {
    const { width, height } = sizeRef.current;
    commit(zoomAt(viewRef.current, factor, width / 2, height / 2));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const step = e.shiftKey ? 160 : 40;
    switch (e.key) {
      case "ArrowLeft":
        commit(pan(viewRef.current, step, 0));
        break;
      case "ArrowRight":
        commit(pan(viewRef.current, -step, 0));
        break;
      case "ArrowUp":
        commit(pan(viewRef.current, 0, step));
        break;
      case "ArrowDown":
        commit(pan(viewRef.current, 0, -step));
        break;
      case "+":
      case "=":
        zoomBy(1.25);
        break;
      case "-":
      case "_":
        zoomBy(0.8);
        break;
      case "f":
      case "F":
      case "0":
        doFit();
        break;
      case "Escape":
        if (popover) {
          setPopover(null);
          setSelected(null);
        } else store.clearHighlights();
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  const findings: Finding[] = view?.summary.findings ?? [];
  const added = highlights.filter((h) => h.kind === "added").length;
  const changed = highlights.filter((h) => h.kind === "changed").length;

  if (!view) {
    return (
      <div className="empty">
        {viewError ? (
          <>
            <p className="empty-title">This schematic could not be read</p>
            <p className="empty-text error-text">{viewError}</p>
            <button type="button" className="button" onClick={() => void store.refreshView()}>
              Try again
            </button>
          </>
        ) : viewLoading ? (
          <p className="empty-text">Reading the schematic...</p>
        ) : (
          <>
            <p className="empty-title">No circuit selected</p>
            <p className="empty-text">Pick a circuit from the list, or create a new one.</p>
          </>
        )}
      </div>
    );
  }

  return (
    <div className="schematic">
      <div
        ref={paneRef}
        className="sch-pane"
        tabIndex={0}
        role="application"
        aria-roledescription="schematic viewer"
        aria-label={`Schematic of ${view.path}. Arrow keys pan, plus and minus zoom, F fits. Click a part for details.`}
        data-testid="schematic-pane"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onWheel={onWheel}
        onKeyDown={onKeyDown}
      >
        {markup ? (
          <div ref={hostRef} className="sch-host" />
        ) : (
          <div className="empty">
            <p className="empty-text">The drawing for this circuit is not available. The netlist and summary still are.</p>
          </div>
        )}

        <div className="sch-tools" role="toolbar" aria-label="Zoom">
          <button type="button" className="button button-quiet" onClick={() => zoomBy(0.8)} aria-label="Zoom out" title="Zoom out (-)">
            &minus;
          </button>
          <span className="sch-zoom mono" aria-live="polite" data-testid="zoom-level">
            {Math.round(zoom * 100)}%
          </span>
          <button type="button" className="button button-quiet" onClick={() => zoomBy(1.25)} aria-label="Zoom in" title="Zoom in (+)">
            +
          </button>
          <button type="button" className="button button-quiet" onClick={doFit} title="Fit to view (F)">
            Fit
          </button>
        </div>

        {highlights.length > 0 ? (
          <div className="sch-legend" role="status">
            {added > 0 ? (
              <span className="sch-legend-item" data-kind="added">
                {added} added
              </span>
            ) : null}
            {changed > 0 ? (
              <span className="sch-legend-item" data-kind="changed">
                {changed} changed
              </span>
            ) : null}
            <button type="button" className="link" onClick={() => store.clearHighlights()}>
              Clear
            </button>
          </div>
        ) : null}

        {popover ? (
          <PartPopover
            part={popover.part}
            x={popover.x}
            y={popover.y}
            bounds={sizeRef.current}
            highlight={highlights.find((h) => h.inst.toUpperCase() === popover.part.name.toUpperCase())?.kind}
            onClose={() => {
              setPopover(null);
              setSelected(null);
              paneRef.current?.focus();
            }}
          />
        ) : null}
      </div>

      {findings.length > 0 ? <Findings findings={findings} onLocate={(inst) => store.locate(inst)} /> : null}
    </div>
  );
}
