import type { Finding } from "../../ipc/types";

const LABEL = { error: "Error", warning: "Warning", info: "Note" } as const;

/** Rule check findings under the drawing. A finding that names a part locates it. */
export function Findings({ findings, onLocate }: { findings: Finding[]; onLocate: (inst: string) => void }) {
  const errors = findings.filter((f) => f.severity === "error").length;
  const warnings = findings.filter((f) => f.severity === "warning").length;
  const summary = [errors ? `${errors} error${errors > 1 ? "s" : ""}` : null, warnings ? `${warnings} warning${warnings > 1 ? "s" : ""}` : null]
    .filter(Boolean)
    .join(", ");
  return (
    <section className="findings" aria-label="Rule checks">
      <p className="findings-head">
        <span className="section-title">Checks</span>
        <span className="findings-count">{summary || `${findings.length} note${findings.length > 1 ? "s" : ""}`}</span>
      </p>
      <ul className="findings-list">
        {findings.map((f, i) => {
          const part = f.parts?.[0];
          const body = (
            <>
              <span className="finding-sev" data-severity={f.severity}>
                {LABEL[f.severity]}
              </span>
              <span className="finding-msg">{f.message}</span>
              <span className="finding-rule mono">{f.rule}</span>
            </>
          );
          return (
            <li key={`${f.rule}-${i}`}>
              {part ? (
                <button type="button" className="finding" onClick={() => onLocate(part)} title={`Show ${part} in the schematic`}>
                  {body}
                </button>
              ) : (
                <div className="finding">{body}</div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
