import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import type { DatasetMeta, RunView, WaveData } from "../../ipc/types";
import { duration, eng, vectorUnit } from "../../lib/format";
import { SIMULATOR_NAMES } from "../../lib/names";
import { token, usePaletteVersion } from "../../lib/theme";
import { message } from "../../store/store";
import { useStore, useStoreApi } from "../../store/context";
import { MOD } from "../../lib/keys";
import { Plot, type Cursors, type PlotSeries } from "./Plot";
import "./Waveforms.css";

const DASHES: number[][] = [[], [7, 4], [2, 3], [9, 3, 2, 3]];

/** Plottable datasets: everything with more than one point. */
export function plottable(run: RunView | null): DatasetMeta[] {
  return run?.datasets.filter((d) => d.points > 1 && d.kind !== "op") ?? [];
}

export function signalsOf(ds: DatasetMeta | undefined): string[] {
  if (!ds) return [];
  return ds.vectors.filter((v) => v.name !== ds.axis && !["time", "frequency", "sweep"].includes(v.quantity)).map((v) => v.name);
}

export function defaultSignals(ds: DatasetMeta | undefined): string[] {
  const all = signalsOf(ds);
  const lower = all.map((s) => s.toLowerCase());
  if (ds?.kind === "ac") {
    const out = lower.indexOf("v(out)");
    return [all[out >= 0 ? out : 0]].filter(Boolean);
  }
  const wanted = ["v(in)", "v(out)"].map((n) => lower.indexOf(n)).filter((i) => i >= 0);
  if (wanted.length) return wanted.map((i) => all[i]);
  return all.filter((_, i) => i < 2);
}

interface Trace {
  key: string;
  name: string;
  step: string;
  color: string;
  dash: number[];
  y: number[];
  phase?: number[];
}

const traceLabel = (t: Trace) => (t.step ? `${t.name} ${t.step}` : t.name);

export function WaveformsView() {
  const store = useStoreApi();
  const active = useStore((s) => s.active);
  const runs = useStore((s) => s.runs);
  const simulating = useStore((s) => s.simulating);
  const run = active ? (runs[active] ?? null) : null;
  const palette = usePaletteVersion();

  const datasets = useMemo(() => plottable(run), [run]);
  const [dsChoice, setDsChoice] = useState(0);
  const ds = datasets[dsChoice] ?? datasets[0];
  const all = useMemo(() => signalsOf(ds), [ds]);
  const selKey = `${run?.circuit}:${ds?.plotname}`;
  const [picked, setPicked] = useState<Record<string, string[]>>({});
  const selected = useMemo(() => {
    const p = picked[selKey];
    return p ? p.filter((s) => all.includes(s)) : defaultSignals(ds);
  }, [picked, selKey, all, ds]);
  const [step, setStep] = useState("all");
  const [wave, setWave] = useState<WaveData | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cursors, setCursors] = useState<Cursors>({ a: null, b: null });
  const [which, setWhich] = useState<"a" | "b">("a");
  const [xRange, setXRange] = useState<[number, number] | null>(null);

  // A new run resets the cursors, zoom and step filter.
  useEffect(() => {
    setCursors({ a: null, b: null });
    setWhich("a");
    setXRange(null);
    setStep("all");
  }, [run?.run_id]);

  const signalKey = selected.join("\u0000");
  useEffect(() => {
    if (!run || !ds || selected.length === 0) {
      setWave(null);
      return;
    }
    let live = true;
    setLoading(true);
    setError(null);
    store.backend
      .waveform(run.run_id, ds.index, selected, 4000)
      .then((w) => {
        if (live) {
          setWave(w);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (live) {
          setError(message(err));
          setLoading(false);
        }
      });
    return () => {
      live = false;
    };
    // signalKey stands in for the selected array's contents.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run?.run_id, ds?.index, signalKey, store]);

  const steps = ds?.steps ?? [];
  const traces = useMemo<Trace[]>(() => {
    if (!wave) return [];
    const colors = Array.from({ length: 8 }, (_, i) => token(`--plot-${i + 1}`, "#888"));
    const shown = wave.series.filter((s) => selected.includes(s.name) && (step === "all" || s.step === step || s.step === ""));
    const stepNames = [...new Set(shown.map((s) => s.step))];
    const byStep = selected.length === 1 && stepNames.length > 1;
    return shown.map((s) => {
      const si = selected.indexOf(s.name);
      const ti = Math.max(0, stepNames.indexOf(s.step));
      return {
        key: `${s.name}|${s.step}`,
        name: s.name,
        step: s.step,
        color: colors[(byStep ? ti : si) % colors.length],
        dash: byStep ? [] : DASHES[ti % DASHES.length],
        y: s.y,
        phase: s.phase,
      };
    });
    // palette is read through token(); it only has to trigger a recompute.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wave, signalKey, step, palette]);

  const isAc = wave?.kind === "ac";
  const magSeries = useMemo<PlotSeries[]>(() => traces.map((t) => ({ label: traceLabel(t), color: t.color, dash: t.dash, values: t.y })), [traces]);
  const phaseSeries = useMemo<PlotSeries[]>(
    () => traces.filter((t) => t.phase).map((t) => ({ label: traceLabel(t), color: t.color, dash: t.dash, values: t.phase! })),
    [traces],
  );

  const pick = (idx: number) => {
    setCursors((c) => ({ ...c, [which]: idx }));
    setWhich((w) => (w === "a" ? "b" : "a"));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!wave || wave.x.length === 0) return;
    const n = wave.x.length;
    const move = (delta: number) => {
      setCursors((c) => {
        const cur = c[which] ?? Math.floor(n / 2);
        return { ...c, [which]: Math.min(n - 1, Math.max(0, cur + delta)) };
      });
    };
    if (e.key === "ArrowRight") move(e.shiftKey ? 10 : 1);
    else if (e.key === "ArrowLeft") move(e.shiftKey ? -10 : -1);
    else if (e.key === "a" || e.key === "A") setWhich("a");
    else if (e.key === "b" || e.key === "B") setWhich("b");
    else if (e.key === "Delete" || e.key === "Backspace") setCursors({ a: null, b: null });
    else return;
    e.preventDefault();
  };

  const toggle = (name: string) => {
    const next = selected.includes(name) ? selected.filter((s) => s !== name) : [...selected, name];
    setPicked((p) => ({ ...p, [selKey]: next }));
  };

  if (!active) {
    return (
      <div className="empty">
        <p className="empty-title">No circuit selected</p>
      </div>
    );
  }

  if (!run) {
    return (
      <div className="empty">
        <p className="empty-title">{simulating ? "Simulating..." : "No simulation yet"}</p>
        <p className="empty-text">
          Press Simulate above, or {MOD}+R, to see the waveforms, measurements and log.
        </p>
      </div>
    );
  }

  const colorOf = (name: string) => traces.find((t) => t.name === name)?.color;
  const xUnit = wave?.x_unit ?? "";
  const xName = wave?.x_name ?? ds?.axis ?? "x";
  const yUnit = selected.length > 0 && selected.every((s) => vectorUnit(s) === vectorUnit(selected[0])) ? vectorUnit(selected[0]) : "";

  return (
    <div className="waves">
      <div className="wave-bar" role="toolbar" aria-label="Waveform options">
        {datasets.length > 1 ? (
          <select className="select" aria-label="Analysis" value={dsChoice} onChange={(e) => setDsChoice(Number(e.target.value))}>
            {datasets.map((d, i) => (
              <option key={d.index} value={i}>
                {d.plotname}
              </option>
            ))}
          </select>
        ) : null}
        {steps.length > 0 ? (
          <label className="wave-steps">
            <span>Step</span>
            <select className="select" aria-label="Step" value={step} onChange={(e) => setStep(e.target.value)}>
              <option value="all">All {steps.length} overlaid</option>
              {steps.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        {all.length > 0 ? (
          <div className="wave-signals" role="group" aria-label="Signals">
            {all.map((name) => {
              const on = selected.includes(name);
              const color = on ? colorOf(name) : undefined;
              return (
                <button key={name} type="button" className="signal" aria-pressed={on} onClick={() => toggle(name)}>
                  <span className="signal-swatch" style={color ? { background: color, borderColor: color } : undefined} />
                  <span className="mono">{name}</span>
                </button>
              );
            })}
          </div>
        ) : null}
        <span className="wave-meta">
          {SIMULATOR_NAMES[run.simulator]}, {duration(run.duration_ms)}
          {ds ? `, ${ds.points} points` : ""}
        </span>
      </div>

      <div className="wave-body">
        <div className="wave-plots" tabIndex={0} onKeyDown={onKeyDown} aria-label="Plot. Click to place cursor A, then B. Arrow keys move the active cursor." data-testid="plot-area">
          {run.errors.length > 0 ? (
            <div className="empty">
              <p className="empty-title">The simulation failed</p>
              {run.errors.map((e) => (
                <p key={e} className="empty-text error-text">
                  {e}
                </p>
              ))}
              <button type="button" className="button" onClick={() => store.setTab("log")}>
                Open the log
              </button>
            </div>
          ) : datasets.length === 0 ? (
            <div className="empty">
              <p className="empty-text">This run has no plottable data. Operating point values are listed beside.</p>
            </div>
          ) : selected.length === 0 ? (
            <div className="empty">
              <p className="empty-text">Pick a signal above to plot it.</p>
            </div>
          ) : error ? (
            <div className="empty">
              <p className="empty-text error-text">{error}</p>
            </div>
          ) : wave && wave.x.length > 0 ? (
            isAc ? (
              <>
                <div className="plot-pane">
                  <Plot
                    x={wave.x}
                    series={magSeries}
                    logX={wave.log_x}
                    xLabel={`${xName} (${xUnit})`}
                    yLabel="Magnitude (dB)"
                    yShort="dB"
                    cursors={cursors}
                    onPick={pick}
                    syncKey={`bode-${run.run_id}`}
                    xRange={xRange}
                    onXRange={setXRange}
                    showXLabels={false}
                    palette={palette}
                    ariaLabel="Magnitude in dB against frequency"
                  />
                </div>
                <div className="plot-pane">
                  <Plot
                    x={wave.x}
                    series={phaseSeries}
                    logX={wave.log_x}
                    xLabel={`${xName} (${xUnit})`}
                    yLabel={"Phase (°)"}
                    yShort="deg"
                    cursors={cursors}
                    onPick={pick}
                    syncKey={`bode-${run.run_id}`}
                    xRange={xRange}
                    onXRange={setXRange}
                    palette={palette}
                    ariaLabel="Phase in degrees against frequency"
                  />
                </div>
              </>
            ) : (
              <div className="plot-pane">
                <Plot
                  x={wave.x}
                  series={magSeries}
                  logX={wave.log_x}
                  xLabel={`${xName} (${xUnit})`}
                  yLabel={yUnit ? `${yUnit === "V" ? "Voltage" : yUnit === "A" ? "Current" : "Value"} (${yUnit})` : "Value"}
                  cursors={cursors}
                  onPick={pick}
                  palette={palette}
                  ariaLabel={`${selected.join(", ")} against ${xName}`}
                />
              </div>
            )
          ) : (
            <div className="empty">
              <p className="empty-text">{loading ? "Loading waveforms..." : "No data."}</p>
            </div>
          )}
        </div>

        <aside className="wave-side" aria-label="Cursors and measurements">
          <section className="wave-card" aria-labelledby="cursor-title">
            <div className="wave-card-head">
              <h3 className="section-title" id="cursor-title">
                Cursors
              </h3>
              <div className="segmented" role="group" aria-label="Active cursor">
                <button type="button" className="segment" aria-pressed={which === "a"} onClick={() => setWhich("a")}>
                  A
                </button>
                <button type="button" className="segment" aria-pressed={which === "b"} onClick={() => setWhich("b")}>
                  B
                </button>
              </div>
              <button
                type="button"
                className="button button-quiet"
                onClick={() => setCursors({ a: null, b: null })}
                disabled={cursors.a === null && cursors.b === null}
              >
                Clear
              </button>
            </div>
            {wave && (cursors.a !== null || cursors.b !== null) ? (
              <Readout wave={wave} traces={traces} cursors={cursors} />
            ) : (
              <p className="panel-note">Click the plot to place cursor A, then B.</p>
            )}
          </section>

          <section className="wave-card" aria-labelledby="meas-title">
            <h3 className="section-title" id="meas-title">
              Measurements
            </h3>
            {run.measurements.length === 0 ? (
              <p className="panel-note">No measurements in this run.</p>
            ) : (
              <dl className="meas-list">
                {run.measurements.map((m) => (
                  <div key={m.name} className="meas-row" title={m.note ?? undefined}>
                    <dt className="mono">{m.name}</dt>
                    <dd className="mono">{m.display}</dd>
                  </div>
                ))}
              </dl>
            )}
            {Object.keys(run.op).length > 0 ? (
              <>
                <h3 className="section-title meas-sub">Operating point</h3>
                <dl className="meas-list">
                  {Object.entries(run.op).map(([k, v]) => (
                    <div key={k} className="meas-row">
                      <dt className="mono">{k}</dt>
                      <dd className="mono">{eng(v, vectorUnit(k), 4)}</dd>
                    </div>
                  ))}
                </dl>
              </>
            ) : null}
            {run.warnings.length > 0 ? (
              <ul className="wave-warnings">
                {run.warnings.map((w) => (
                  <li key={w}>{w}</li>
                ))}
              </ul>
            ) : null}
          </section>
        </aside>
      </div>
    </div>
  );
}

function Readout({ wave, traces, cursors }: { wave: WaveData; traces: Trace[]; cursors: Cursors }) {
  const { a, b } = cursors;
  const ac = wave.kind === "ac";
  const xa = a !== null ? wave.x[a] : null;
  const xb = b !== null ? wave.x[b] : null;
  const fmt = (v: number | null | undefined, unit: string) => (v === null || v === undefined ? "" : eng(v, unit, 4));
  const delta = (va: number | null | undefined, vb: number | null | undefined) =>
    va === null || va === undefined || vb === null || vb === undefined ? null : vb - va;
  const rows: Array<{ key: string; label: string; color?: string; unit: string; va: number | null; vb: number | null }> = [];
  for (const t of traces) {
    const name = t.step ? `${t.name} ${t.step}` : t.name;
    rows.push({ key: t.key, label: ac ? `${name} mag` : name, color: t.color, unit: ac ? "dB" : vectorUnit(t.name), va: a !== null ? t.y[a] : null, vb: b !== null ? t.y[b] : null });
    if (ac && t.phase) {
      rows.push({ key: `${t.key}-ph`, label: `${name} phase`, color: t.color, unit: "°", va: a !== null ? t.phase[a] : null, vb: b !== null ? t.phase[b] : null });
    }
  }
  const dx = delta(xa, xb);
  // Each quantity takes two rows, its name above its three values, so the
  // numbers get the full width of a narrow side panel.
  const pair = (key: string, label: string, color: string | undefined, va: string, vb: string, d: string) => [
    <tr key={`${key}-h`} className="readout-name">
      <th scope="rowgroup" colSpan={3}>
        {color ? <span className="readout-swatch" style={{ background: color }} /> : null}
        {label}
      </th>
    </tr>,
    <tr key={`${key}-v`}>
      <td>{va}</td>
      <td>{vb}</td>
      <td>{d}</td>
    </tr>,
  ];
  return (
    <table className="readout" data-testid="cursor-readout">
      <thead>
        <tr>
          <th scope="col">A</th>
          <th scope="col">B</th>
          <th scope="col">B - A</th>
        </tr>
      </thead>
      <tbody>
        {pair("x", wave.x_name, undefined, fmt(xa, wave.x_unit), fmt(xb, wave.x_unit), fmt(dx, wave.x_unit))}
        {!ac && dx ? pair("inv", "1 / (B - A)", undefined, "", "", fmt(1 / Math.abs(dx), "Hz")) : null}
        {rows.map((r) => pair(r.key, r.label, r.color, fmt(r.va, r.unit), fmt(r.vb, r.unit), fmt(delta(r.va, r.vb), r.unit)))}
      </tbody>
    </table>
  );
}
