/** A unified diff with added and removed lines marked by colour and by their +/- prefix. */
export function DiffBlock({ diff, label = "Diff" }: { diff: string; label?: string }) {
  if (!diff.trim()) return <p className="panel-note">No textual change.</p>;
  const lines = diff.replace(/\n$/, "").split("\n");
  return (
    <pre className="diff" aria-label={label} tabIndex={0}>
      {lines.map((line, i) => {
        const kind = line.startsWith("+++") || line.startsWith("---")
          ? "file"
          : line.startsWith("@@")
            ? "hunk"
            : line.startsWith("+")
              ? "add"
              : line.startsWith("-")
                ? "del"
                : line.startsWith("~")
                  ? "change"
                  : "ctx";
        return (
          <span key={i} className="diff-line" data-kind={kind}>
            {line || " "}
          </span>
        );
      })}
    </pre>
  );
}
