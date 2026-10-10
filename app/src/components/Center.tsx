import { useRef, useState, type KeyboardEvent } from "react";
import { MOD } from "../lib/keys";
import { readJson, writeJson } from "../lib/storage";
import { useStore, useStoreApi } from "../store/context";
import { TABS, type Tab } from "../store/store";
import { HistoryView } from "./HistoryView";
import { SimulateControl } from "./SimulateControl";
import { SpecsView } from "./SpecsView";
import { Splitter } from "./Splitter";
import { TextView } from "./TextView";
import { SchematicView } from "./schematic/SchematicView";
import { WaveformsView } from "./waves/WaveformsView";
import "./Center.css";

function TabBody({ tab }: { tab: Tab }) {
  const view = useStore((s) => s.view);
  const active = useStore((s) => s.active);
  const runs = useStore((s) => s.runs);
  const run = active ? runs[active] : undefined;
  switch (tab) {
    case "schematic":
      return <SchematicView />;
    case "waveforms":
      return <WaveformsView />;
    case "netlist":
      return <TextView text={view?.netlist ?? ""} label="Netlist" empty="No netlist yet. Select a circuit." />;
    case "log":
      return <TextView text={run?.log ?? ""} label="Simulator log" empty="Run a simulation to see the simulator's log here." />;
    case "specs":
      return <SpecsView />;
    case "history":
      return <HistoryView />;
  }
}

export function Center() {
  const store = useStoreApi();
  const tab = useStore((s) => s.tab);
  const split = useStore((s) => s.split);
  const view = useStore((s) => s.view);
  const active = useStore((s) => s.active);
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(() => readJson<number>("splitTop", 0.5));

  const onTabKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const i = TABS.findIndex(([id]) => id === tab);
    let next = -1;
    if (e.key === "ArrowRight") next = (i + 1) % TABS.length;
    else if (e.key === "ArrowLeft") next = (i - 1 + TABS.length) % TABS.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = TABS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    store.setTab(TABS[next][0]);
    tabRefs.current[next]?.focus();
  };

  const showSplit = split && tab !== "schematic";
  const height = bodyRef.current?.clientHeight ?? 600;
  const setTopPx = (px: number) => {
    const f = Math.min(0.8, Math.max(0.2, px / Math.max(1, height)));
    setTop(f);
    writeJson("splitTop", f);
  };

  return (
    <main className="center" aria-label="Circuit">
      <div className="center-head">
        <div className="tabs" role="tablist" aria-label="Views" onKeyDown={onTabKey}>
          {TABS.map(([id, label], i) => (
            <button
              key={id}
              ref={(el) => {
                tabRefs.current[i] = el;
              }}
              type="button"
              role="tab"
              id={`tab-${id}`}
              className="tab"
              aria-selected={tab === id}
              aria-controls="center-panel"
              tabIndex={tab === id ? 0 : -1}
              data-pinned={showSplit && id === "schematic" ? "" : undefined}
              onClick={() => store.setTab(id)}
              title={`${label} (${MOD}+${i + 1})`}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="center-tools">
          <button
            type="button"
            className="button button-quiet"
            aria-pressed={split}
            onClick={() => store.toggleSplit()}
            title={`Keep the schematic above the other tabs (${MOD}+\\)`}
          >
            Split
          </button>
        </div>
      </div>

      {active ? (
        <div className="center-toolbar" role="toolbar" aria-label="Circuit actions">
          <p className="center-path mono" title={active}>
            {active}
          </p>
          <SimulateControl />
          <span className="center-sep" aria-hidden="true" />
          <button type="button" className="button" onClick={() => void store.undo()} disabled={!view?.can_undo} title={`Undo (${MOD}+Z)`}>
            Undo
          </button>
          <button type="button" className="button" onClick={() => void store.redo()} disabled={!view?.can_redo} title={`Redo (${MOD}+Shift+Z)`}>
            Redo
          </button>
          <button type="button" className="button" onClick={() => void store.openInLtspice()} disabled={!view}>
            Open in LTspice
          </button>
        </div>
      ) : null}

      <div ref={bodyRef} className="center-body" id="center-panel" role="tabpanel" aria-labelledby={`tab-${tab}`}>
        {showSplit ? (
          <div className="split">
            <div className="split-top" style={{ flexBasis: `${top * 100}%` }}>
              <SchematicView />
            </div>
            <Splitter
              label="Resize schematic"
              orientation="horizontal"
              value={Math.round(top * height)}
              min={Math.round(height * 0.2)}
              max={Math.round(height * 0.8)}
              onChange={setTopPx}
              onReset={() => setTopPx(height / 2)}
            />
            <div className="split-bottom">
              <TabBody tab={tab} />
            </div>
          </div>
        ) : (
          <TabBody tab={tab} />
        )}
      </div>
    </main>
  );
}
