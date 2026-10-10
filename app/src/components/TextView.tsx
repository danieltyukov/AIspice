import { useState } from "react";
import "./TextView.css";

/** Monospaced text with line numbers and a copy button: the netlist and the simulator log. */
export function TextView({ text, label, empty }: { text: string; label: string; empty: string }) {
  const [copied, setCopied] = useState(false);
  if (!text) {
    return (
      <div className="empty">
        <p className="empty-text">{empty}</p>
      </div>
    );
  }
  const lines = text.replace(/\n$/, "").split("\n");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  };
  return (
    <div className="textview">
      <div className="textview-bar">
        <span className="textview-meta">
          {label}, {lines.length} lines
        </span>
        <button type="button" className="button" onClick={() => void copy()} aria-live="polite">
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <div className="textview-scroll" tabIndex={0} aria-label={label}>
        <pre className="textview-pre">
          {lines.map((line, i) => (
            <div key={i} className="textview-line">
              <span className="textview-no" aria-hidden="true">
                {i + 1}
              </span>
              <code className="textview-code" data-kind={line.startsWith(".") ? "directive" : line.startsWith("*") || line.startsWith(";") ? "comment" : undefined}>
                {line || " "}
              </code>
            </div>
          ))}
        </pre>
      </div>
    </div>
  );
}
