#!/usr/bin/env node
// Checks the colour tokens in app/src/tokens.css against the contrast and
// separation rules the comments in that file promise. Exits non-zero on a
// failure, so it can run in CI as it is.
//
//   node assets/brand/tools/check_tokens.mjs
//
// It reads the stylesheet rather than a copy of its values, folds each dark
// block over the light palette, and checks that the two dark blocks agree.
//
// Rules:
//   text      4.5:1 against every ground it is drawn on (WCAG 1.4.3, AA)
//   graphics  3:1 against the ground (WCAG 1.4.11): wires, symbols, traces
//   traces    adjacent --plot-N pairs at least 15 apart in OKLab (x100) for
//             normal vision and 8 apart under simulated protanopia and
//             deuteranopia (Machado 2009, severity 1); the first three
//             --plot tokens hold to the same floors for every pair.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const css = readFileSync(fileURLToPath(new URL('../../../app/src/tokens.css', import.meta.url)), 'utf8')
  .replace(/\/\*[\s\S]*?\*\//g, '');

function block(selector) {
  const re = new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^{}]*)\\}`);
  const m = css.match(re);
  if (!m) throw new Error(`no block for ${selector}`);
  const out = {};
  for (const [, k, v] of m[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out[k] = v.trim();
  return out;
}

const light = block(':root');
const systemDark = block(":root:not([data-theme='light'])");
const explicitDark = block(":root[data-theme='dark']");
const dark = { ...light, ...explicitDark };

let failed = 0;
const fail = (msg) => {
  failed++;
  console.log(`  FAIL ${msg}`);
};

if (JSON.stringify(systemDark) !== JSON.stringify(explicitDark)) {
  fail('the system-dark and explicit-dark blocks differ');
}

const hex = (v) => {
  const h = v.replace('#', '');
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
};
const lin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const lum = (v) => {
  const [r, g, b] = hex(v).map(lin);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => {
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

function oklab([r, g, b]) {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}
const MACHADO = {
  protan: [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
  deutan: [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.01182, 0.04294, 0.968881]],
};
function seen(v, kind) {
  const c = hex(v).map(lin);
  if (!kind) return c;
  return MACHADO[kind].map((row) => Math.min(1, Math.max(0, row[0] * c[0] + row[1] * c[1] + row[2] * c[2])));
}
const dE = (a, b, kind) => {
  const [x, y] = [oklab(seen(a, kind)), oklab(seen(b, kind))];
  return 100 * Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]);
};

const TEXT = ['--text', '--muted', '--faint', '--accent', '--success', '--warning', '--danger', '--info'];
const GROUNDS = ['--bg', '--surface', '--surface-2'];
const SCH_TEXT = ['--sch-label', '--sch-directive', '--sch-comment', '--sch-text'];
const SCH_GRAPHIC = ['--sch-wire', '--sch-symbol', '--sch-junction', '--sch-hl-added', '--sch-hl-changed'];
const PLOTS = Array.from({ length: 8 }, (_, i) => `--plot-${i + 1}`);

for (const [name, t] of [['light', light], ['dark', dark]]) {
  console.log(`\n${name}`);
  for (const fg of TEXT) {
    const rs = GROUNDS.map((g) => ratio(t[fg], t[g]));
    console.log(`  ${fg.padEnd(16)} ${t[fg]}  ${GROUNDS.map((g, i) => `${g} ${rs[i].toFixed(2)}`).join('  ')}`);
    rs.forEach((r, i) => r < 4.5 && fail(`${fg} on ${GROUNDS[i]} is ${r.toFixed(2)}:1, under 4.5`));
  }
  const onAccent = ratio(t['--on-accent'], t['--accent']);
  const onHover = ratio(t['--on-accent'], t['--accent-hover']);
  console.log(`  --on-accent on --accent ${onAccent.toFixed(2)}, on --accent-hover ${onHover.toFixed(2)}`);
  if (onAccent < 4.5) fail('--on-accent on --accent under 4.5');
  if (onHover < 4.5) fail('--on-accent on --accent-hover under 4.5');
  const tint = ratio(t['--text'], t['--accent-tint']);
  console.log(`  --text on --accent-tint ${tint.toFixed(2)}`);
  if (tint < 4.5) fail('--text on --accent-tint under 4.5');

  const sg = t['--sch-bg'];
  for (const fg of SCH_TEXT) {
    const r = ratio(t[fg], sg);
    console.log(`  ${fg.padEnd(16)} ${t[fg]}  on --sch-bg ${r.toFixed(2)}`);
    if (r < 4.5) fail(`${fg} on --sch-bg is ${r.toFixed(2)}:1, under 4.5`);
  }
  for (const fg of SCH_GRAPHIC) {
    const r = ratio(t[fg], sg);
    console.log(`  ${fg.padEnd(16)} ${t[fg]}  on --sch-bg ${r.toFixed(2)}`);
    if (r < 3) fail(`${fg} on --sch-bg is ${r.toFixed(2)}:1, under 3`);
  }
  const ws = dE(t['--sch-wire'], t['--sch-symbol']);
  const wsCvd = Math.min(dE(t['--sch-wire'], t['--sch-symbol'], 'protan'), dE(t['--sch-wire'], t['--sch-symbol'], 'deutan'));
  console.log(`  --sch-wire against --sch-symbol: dE ${ws.toFixed(1)}, protan/deutan ${wsCvd.toFixed(1)}`);
  if (ws < 15 || wsCvd < 8) fail('--sch-wire and --sch-symbol are too close to tell apart');

  const pg = t['--plot-bg'];
  const plots = PLOTS.map((p) => t[p]);
  plots.forEach((c, i) => {
    const r = ratio(c, pg);
    if (r < 3) fail(`${PLOTS[i]} on --plot-bg is ${r.toFixed(2)}:1, under 3`);
  });
  const adj = plots.slice(1).map((c, i) => [i, i + 1]);
  const first3 = [[0, 1], [0, 2], [1, 2]];
  const worst = (pairs, kind) => Math.min(...pairs.map(([i, j]) => dE(plots[i], plots[j], kind)));
  const normal = worst(adj, null);
  const cvd = Math.min(worst(adj, 'protan'), worst(adj, 'deutan'));
  const normal3 = worst(first3, null);
  const cvd3 = Math.min(worst(first3, 'protan'), worst(first3, 'deutan'));
  const minContrast = Math.min(...plots.map((c) => ratio(c, pg)));
  console.log(`  plot traces on --plot-bg ${pg}: contrast >= ${minContrast.toFixed(2)}`);
  console.log(`    adjacent pairs: normal dE >= ${normal.toFixed(1)}, protan/deutan dE >= ${cvd.toFixed(1)}`);
  console.log(`    first three, every pair: normal dE >= ${normal3.toFixed(1)}, protan/deutan dE >= ${cvd3.toFixed(1)}`);
  if (normal < 15 || normal3 < 15) fail('two neighbouring traces are under 15 apart for normal vision');
  if (cvd < 8 || cvd3 < 8) fail('two neighbouring traces are under 8 apart under colour vision deficiency');
}

console.log(failed ? `\n${failed} failure(s)` : '\nall checks pass');
process.exit(failed ? 1 : 0);
