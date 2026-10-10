import { clock, relative } from "../lib/format";
import { useStore, useStoreApi } from "../store/context";
import "./Panels.css";

export function HistoryView() {
  const store = useStoreApi();
  const history = useStore((s) => s.history);
  const view = useStore((s) => s.view);
  const now = Date.now();

  return (
    <div className="panel-view">
      <div className="panel-bar">
        <div>
          <p className="panel-bar-title">History</p>
          <p className="panel-bar-note">Every edit is a snapshot. Restoring one keeps the others.</p>
        </div>
        <span className="panel-bar-note">
          {history.length > 0 ? `${history.length} snapshot${history.length === 1 ? "" : "s"}` : ""}
          {view?.can_redo ? ", redo available" : ""}
        </span>
      </div>
      <div className="panel-scroll">
        {history.length === 0 ? (
          <div className="empty">
            <p className="empty-text">No snapshots yet. They appear as the circuit is edited.</p>
          </div>
        ) : (
          <ol className="history-list">
            {history.map((snap) => (
              <li key={snap.id} className="history-row" data-current={snap.current ? "" : undefined}>
                <span className="history-mark" aria-hidden="true" />
                <div className="history-main">
                  <p className="history-summary">{snap.summary}</p>
                  <p className="history-time">
                    <time dateTime={new Date(snap.time).toISOString()}>{clock(snap.time)}</time>
                    <span>{relative(snap.time, now)}</span>
                  </p>
                </div>
                {snap.current ? (
                  <span className="chip" data-tone="pass">
                    Current
                  </span>
                ) : (
                  <button type="button" className="button" onClick={() => void store.restore(snap.id)}>
                    Restore
                  </button>
                )}
              </li>
            ))}
          </ol>
        )}
      </div>
    </div>
  );
}
