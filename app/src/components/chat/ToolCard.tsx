import { useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ChatToolCall, ToolData } from "../../ipc/types";
import { duration, eng } from "../../lib/format";
import { sanitizeSvgElement } from "../../lib/sanitizeSvg";
import { useStore, useStoreApi } from "../../store/context";
import { SpecTable } from "../SpecTable";
import { DiffBlock } from "./DiffBlock";
import { toolTitle } from "./toolTitle";

export function ToolCard({ call, streaming }: { call: ChatToolCall; streaming: boolean }) {
  const { title, tone } = toolTitle(call, streaming);
  const kind = call.output?.data?.kind;
  const [open, setOpen] = useState<boolean | null>(null);
  const expanded = open ?? (kind === "edit" || tone === "error");
  const bodyId = useId();
  const hasBody = tone !== "running";

  return (
    <div className="tool-card" data-tone={tone} data-kind={kind ?? call.name}>
      <button
        type="button"
        className="tool-head"
        aria-expanded={hasBody ? expanded : undefined}
        aria-controls={hasBody ? bodyId : undefined}
        onClick={() => hasBody && setOpen(!expanded)}
        disabled={!hasBody}
      >
        <span className="tool-title">{title}</span>
        {call.duration_ms !== undefined && call.duration_ms >= 1000 && tone !== "running" && kind !== "run" ? (
          <span className="tool-time mono">{duration(call.duration_ms)}</span>
        ) : null}
        {hasBody ? <span className="tool-toggle">{expanded ? "Hide" : "Details"}</span> : null}
      </button>
      {tone === "running" ? <span className="tool-progress" aria-hidden="true" /> : null}
      {hasBody && expanded ? (
        <div className="tool-body" id={bodyId}>
          <ToolDetails call={call} />
        </div>
      ) : null}
    </div>
  );
}

function ToolDetails({ call }: { call: ChatToolCall }) {
  const out = call.output;
  if (!out) return <p className="panel-note">The turn stopped before this step finished.</p>;
  const data = out.data ?? null;
  const texts = out.content.filter((c) => c.type === "text").map((c) => (c.type === "text" ? c.text : ""));
  const images = out.content.filter((c) => c.type === "image");
  return (
    <>
      {data ? <DataDetails data={data} /> : null}
      {!data || out.is_error
        ? texts.map((t, i) => (
            <pre key={i} className={out.is_error ? "tool-text tool-error" : "tool-text"}>
              {t}
            </pre>
          ))
        : null}
      {images.map((img, i) =>
        img.type === "image" && /^image\/(png|jpeg|gif|webp)$/.test(img.media_type) ? (
          <img key={i} className="tool-image" alt="Tool output" src={`data:${img.media_type};base64,${img.data_base64}`} />
        ) : null,
      )}
      <details className="tool-input">
        <summary>Input</summary>
        <pre>{JSON.stringify(call.input, null, 2)}</pre>
      </details>
    </>
  );
}

function DataDetails({ data }: { data: ToolData }) {
  const store = useStoreApi();
  const undone = useStore((s) => s.undone);
  switch (data.kind) {
    case "edit": {
      const isUndone = undone.includes(data.snapshot);
      return (
        <div className="tool-section">
          <p className="tool-summary">{data.summary}</p>
          {data.applied.length > 0 ? (
            <ul className="tool-list">
              {data.applied.map((a) => (
                <li key={a}>{a}</li>
              ))}
            </ul>
          ) : null}
          {data.warnings.length > 0 ? (
            <ul className="tool-warnings">
              {data.warnings.map((w) => (
                <li key={w}>
                  <span className="tool-warn-label">Warning</span> {w}
                </li>
              ))}
            </ul>
          ) : null}
          <DiffBlock diff={data.diff} label={`Changes to ${data.circuit}`} />
          <div className="tool-actions">
            <button type="button" className="button" onClick={() => void store.undoEdit(data)} disabled={isUndone}>
              {isUndone ? "Undone" : "Undo"}
            </button>
            {data.highlights[0] ? (
              <button
                type="button"
                className="link"
                onClick={() => {
                  if (store.get().active !== data.circuit) void store.selectCircuit(data.circuit);
                  store.locate(data.highlights[0].inst);
                }}
              >
                Show in schematic
              </button>
            ) : null}
          </div>
        </div>
      );
    }
    case "run": {
      const r = data.run;
      return (
        <div className="tool-section">
          {r.errors.length > 0 ? (
            <ul className="tool-errors">
              {r.errors.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
          ) : null}
          {r.measurements.length > 0 ? (
            <dl className="tool-kv">
              {r.measurements.slice(0, 5).map((m) => (
                <div key={m.name}>
                  <dt className="mono">{m.name}</dt>
                  <dd className="mono">{m.display}</dd>
                </div>
              ))}
            </dl>
          ) : null}
          {r.warnings.length > 0 ? <p className="panel-note">{r.warnings.join(" ")}</p> : null}
          <div className="tool-actions">
            <button type="button" className="link" onClick={() => store.openRun(r)}>
              {r.errors.length > 0 ? "Open the log" : "Open waveforms"}
            </button>
          </div>
        </div>
      );
    }
    case "specs":
      return <SpecTable report={data.report} compact />;
    case "schematic":
      return (
        <div className="tool-section">
          <dl className="tool-kv">
            {data.summary.components.slice(0, 12).map((c) => (
              <div key={c.name}>
                <dt className="mono">{c.name}</dt>
                <dd className="mono">{c.value ?? c.symbol}</dd>
              </div>
            ))}
          </dl>
          <p className="panel-note">
            {data.summary.analysis ? `Analysis: ${data.summary.analysis}. ` : "No analysis directive. "}
            {data.summary.findings.length === 0 ? "No rule findings." : `${data.summary.findings.length} rule findings.`}
          </p>
        </div>
      );
    case "measure":
      return (
        <dl className="tool-kv">
          {data.measurements.map((m) => (
            <div key={m.name}>
              <dt className="mono">{m.name}</dt>
              <dd className="mono">{m.display}</dd>
            </div>
          ))}
        </dl>
      );
    case "lint":
      return data.findings.length === 0 ? (
        <p className="panel-note">No findings.</p>
      ) : (
        <ul className="tool-list">
          {data.findings.map((f, i) => (
            <li key={i}>
              <span className="mono">{f.rule}</span> {f.message}
            </li>
          ))}
        </ul>
      );
    case "plot":
      return <PlotSvg svg={data.svg} />;
    case "optimize":
      return (
        <div className="tool-section">
          <dl className="tool-kv">
            {Object.entries(data.best).map(([k, v]) => (
              <div key={k}>
                <dt className="mono">{k}</dt>
                <dd className="mono">{v}</dd>
              </div>
            ))}
          </dl>
          <SpecTable report={data.report} compact />
        </div>
      );
    case "montecarlo":
      return <pre className="tool-text">{data.report}</pre>;
    case "poles_zeros": {
      const roots = [
        ...data.poles.map((r, i) => ({ label: `p${i + 1}`, r })),
        ...data.zeros.map((r, i) => ({ label: `z${i + 1}`, r })),
      ];
      if (roots.length === 0) return <p className="panel-note">No finite poles or zeros.</p>;
      return (
        <dl className="tool-kv">
          {roots.map(({ label, r }) => (
            <div key={label}>
              <dt className="mono">{label}</dt>
              <dd className="mono">
                {eng(r.re_hz, "Hz")}
                {r.im_hz !== 0 ? ` ${r.im_hz < 0 ? "-" : "+"} j${eng(Math.abs(r.im_hz), "Hz")}` : ""}
                {r.q !== null ? `, Q ${r.q.toPrecision(4)}` : ""}
              </dd>
            </div>
          ))}
        </dl>
      );
    }
    case "generic":
      return <pre className="tool-text">{JSON.stringify(data.value, null, 2)?.slice(0, 4000)}</pre>;
  }
}

/** A plot from a tool result, through the same SVG policy as the schematic. */
function PlotSvg({ svg }: { svg: string }) {
  const clean = useMemo(() => sanitizeSvgElement(svg), [svg]);
  const host = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (host.current && clean) host.current.replaceChildren(clean);
  }, [clean]);
  if (!clean) return <p className="panel-note">The plot could not be shown.</p>;
  return <div className="tool-plot" ref={host} role="img" aria-label="Plot from the tool result" />;
}
