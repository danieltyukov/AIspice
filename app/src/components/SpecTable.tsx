import type { SpecReport, SpecRow } from "../ipc/types";
import { eng } from "../lib/format";
import "./SpecTable.css";

/** The unit a spec's display string carries: "1.005 kHz" -> "Hz". */
function unitOf(display: string): string {
  const m = /^[-+]?[\d.]+(?:e[-+]?\d+)?\s*(.*)$/i.exec(display.trim());
  const rest = (m?.[1] ?? "").trim();
  return rest.replace(/^[TGMkµunpfm](?=[A-Za-z°/])/, "");
}

export function limitText(row: SpecRow): string {
  const unit = unitOf(row.display);
  const f = (v: number) => eng(v, unit, 3);
  if (row.min !== null && row.max !== null) return `${f(row.min)} to ${f(row.max)}`;
  if (row.min !== null) return `at least ${f(row.min)}`;
  if (row.max !== null) return `at most ${f(row.max)}`;
  return "none";
}

export function marginText(row: SpecRow): string {
  if (row.margin === null) return "";
  const unit = unitOf(row.display);
  const text = eng(Math.abs(row.margin), unit, 3);
  return row.margin >= 0 ? `+${text}` : `-${text}`;
}

export function SpecTable({ report, compact = false }: { report: SpecReport; compact?: boolean }) {
  return (
    <table className="data-table spec-table" data-compact={compact ? "" : undefined}>
      <thead>
        <tr>
          <th scope="col">Spec</th>
          <th scope="col" className="num">
            Value
          </th>
          {compact ? null : <th scope="col">Limit</th>}
          <th scope="col" className="num">
            Margin
          </th>
          <th scope="col">Result</th>
        </tr>
      </thead>
      <tbody>
        {report.rows.map((r) => (
          <tr key={r.name} data-pass={r.pass ? "" : undefined}>
            <td>
              {r.name}
              {compact ? <span className="spec-limit">{limitText(r)}</span> : null}
            </td>
            <td className="num">{r.display}</td>
            {compact ? null : <td className="spec-limit-cell">{limitText(r)}</td>}
            <td className="num spec-margin" data-negative={r.margin !== null && r.margin < 0 ? "" : undefined}>
              {marginText(r)}
            </td>
            <td>
              <span className="chip" data-tone={r.pass ? "pass" : "fail"}>
                {r.pass ? "Pass" : "Fail"}
              </span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
