import { useStore, useStoreApi } from "../store/context";
import { SpecTable } from "./SpecTable";
import "./Panels.css";

export function SpecsView() {
  const store = useStoreApi();
  const active = useStore((s) => s.active);
  const specs = useStore((s) => s.specs);
  const loading = useStore((s) => s.specsLoading);
  const error = useStore((s) => s.specsError);
  const report = active ? specs[active] : undefined;

  return (
    <div className="panel-view">
      <div className="panel-bar">
        <div>
          <p className="panel-bar-title">Specs</p>
          <p className="panel-bar-note">Measured from a fresh simulation and compared with each limit.</p>
        </div>
        <button type="button" className="button button-primary" onClick={() => void store.checkSpecs()} disabled={!active || loading}>
          {loading ? "Checking..." : "Check specs"}
        </button>
      </div>
      <div className="panel-scroll">
        {error ? (
          <div className="empty">
            <p className="empty-title">The check did not finish</p>
            <p className="empty-text error-text">{error}</p>
          </div>
        ) : report === undefined ? (
          <div className="empty">
            <p className="empty-title">Not checked yet</p>
            <p className="empty-text">Check specs to simulate the circuit and see every measurement against its limit.</p>
          </div>
        ) : report === null ? (
          <div className="empty">
            <p className="empty-title">No specs for this circuit</p>
            <p className="empty-text">Ask the agent to write a spec table for it, with a limit for each measurement.</p>
          </div>
        ) : (
          <div className="panel-content">
            <div className="spec-summary" data-pass={report.all_pass ? "" : undefined}>
              <span className="chip" data-tone={report.all_pass ? "pass" : "fail"}>
                {report.all_pass ? "All pass" : `${report.rows.filter((r) => !r.pass).length} failing`}
              </span>
              <span>{report.summary}</span>
            </div>
            <SpecTable report={report} />
          </div>
        )}
      </div>
    </div>
  );
}
