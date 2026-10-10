/* A small line diff for the mock backend's edit results (unified format). */

type Op = { kind: " " | "-" | "+"; text: string; a: number; b: number };

function ops(a: string[], b: string[]): Op[] {
  const n = a.length;
  const m = b.length;
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: Op[] = [];
  let i = 0;
  let j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && a[i] === b[j]) {
      out.push({ kind: " ", text: a[i], a: i, b: j });
      i++;
      j++;
    } else if (j < m && (i === n || lcs[i][j + 1] >= lcs[i + 1][j])) {
      out.push({ kind: "+", text: b[j], a: i, b: j });
      j++;
    } else {
      out.push({ kind: "-", text: a[i], a: i, b: j });
      i++;
    }
  }
  return out;
}

export function unifiedDiff(before: string, after: string, file: string, context = 2): string {
  const a = before.replace(/\n$/, "").split("\n");
  const b = after.replace(/\n$/, "").split("\n");
  const all = ops(a, b);
  const changed = all.map((o, k) => (o.kind === " " ? -1 : k)).filter((k) => k >= 0);
  if (changed.length === 0) return "";

  const hunks: Array<[number, number]> = [];
  for (const k of changed) {
    const start = Math.max(0, k - context);
    const end = Math.min(all.length - 1, k + context);
    const last = hunks[hunks.length - 1];
    if (last && start <= last[1] + 1) last[1] = Math.max(last[1], end);
    else hunks.push([start, end]);
  }

  const lines = [`--- a/${file}`, `+++ b/${file}`];
  for (const [start, end] of hunks) {
    const slice = all.slice(start, end + 1);
    const aStart = slice[0].a + 1;
    const bStart = slice[0].b + 1;
    const aLen = slice.filter((o) => o.kind !== "+").length;
    const bLen = slice.filter((o) => o.kind !== "-").length;
    lines.push(`@@ -${aStart},${aLen} +${bStart},${bLen} @@`);
    for (const o of slice) lines.push(`${o.kind}${o.text}`);
  }
  return lines.join("\n");
}
