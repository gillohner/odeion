// The palette's contrast (TASK-ui-motion-polish-1106 §4b): every text colour on every surface it is
// drawn on, in the dark and the light theme, at WCAG AA (4.5:1 for text, 3:1 for large text, UI
// borders and icons). Reads app.css's :root blocks; no browser, no dependencies. CI runs it.
//   node tools/palette-contrast.js [app.css]
const fs = require('fs');
const path = require('path');
const file = process.argv[2] || path.join(__dirname, '..', 'crates', 'pm-api', 'static', 'app.css');
const css = fs.readFileSync(file, 'utf8');

// The variables of a :root block, in order, with var() references kept for resolving
const block = (sel) => {
  const i = css.indexOf(sel);
  if (i < 0) throw new Error(`no ${sel} in ${file}`);
  const body = css.slice(css.indexOf('{', i) + 1, css.indexOf('}', i));
  const vars = {};
  for (const m of body.matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)) vars[m[1]] = m[2].trim();
  return vars;
};
const dark = block(':root {');
const lightPicked = block(':root[data-theme="light"]');
const light = { ...dark, ...lightPicked };
// The light theme is written twice (picked in Settings, and following the system): both must say the same
const sysIdx = css.indexOf('@media (prefers-color-scheme: light)');
let drift = 0;
if (sysIdx >= 0) {
  const body = css.slice(sysIdx, css.indexOf('}', css.indexOf('{', css.indexOf('{', sysIdx) + 1)) + 1);
  const sys = {}; for (const m of body.matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)) sys[m[1]] = m[2].trim();
  for (const k of new Set([...Object.keys(sys), ...Object.keys(lightPicked)])) {
    if (sys[k] == null || lightPicked[k] == null) { drift++; console.log(`FAIL light: --${k} is in ${sys[k] == null ? 'the picked theme only' : 'the system theme only'}`); }
    else if (sys[k] !== lightPicked[k]) { drift++; console.log(`FAIL light: --${k} differs between the picked theme (${lightPicked[k]}) and the system one (${sys[k]})`); }
  }
}

// sRGB from #rgb, #rrggbb, rgb()/rgba(), hsl() (what the palette uses)
const parse = (v, vars) => {
  v = v.trim();
  const ref = v.match(/^var\(--([\w-]+)(?:,\s*([^)]+))?\)$/);
  if (ref) return parse(vars[ref[1]] ?? ref[2] ?? '', vars);
  let m;
  if ((m = v.match(/^#([0-9a-f]{3})$/i))) return [...m[1]].map((c) => parseInt(c + c, 16)).concat([1]);
  if ((m = v.match(/^#([0-9a-f]{6})([0-9a-f]{2})?$/i))) return [0, 2, 4].map((k) => parseInt(m[1].slice(k, k + 2), 16)).concat([m[2] ? parseInt(m[2], 16) / 255 : 1]);
  if ((m = v.match(/^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+))?\s*\)$/))) return [+m[1], +m[2], +m[3], m[4] != null ? +m[4] : 1];
  if ((m = v.match(/^hsla?\(\s*([\d.]+)\s*,?\s*([\d.]+)%\s*,?\s*([\d.]+)%\s*(?:[,/]\s*([\d.]+))?\s*\)$/))) {
    const h = +m[1] / 360; const s = +m[2] / 100; const l = +m[3] / 100;
    const f = (n) => { const k = (n + h * 12) % 12; const a = s * Math.min(l, 1 - l); return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1)))); };
    return [f(0), f(8), f(4), m[4] != null ? +m[4] : 1];
  }
  return null;
};
// A translucent colour over its background
const over = (c, bg) => (c[3] >= 1 ? c : [0, 1, 2].map((i) => Math.round(c[i] * c[3] + bg[i] * (1 - c[3]))).concat([1]));
const lum = ([r, g, b]) => { const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };

// What is drawn on what: text on the surfaces it sits on; the accent's text on the accent; the
// accent as a border or mark on the page (3:1); the status colours as words on the page and cards
const PAIRS = [
  ['fg', ['bg', 'surface', 'surface-2', 'surface-3'], 4.5],
  ['fg-2', ['bg', 'surface', 'surface-2', 'surface-3'], 4.5],
  ['muted', ['bg', 'surface', 'surface-2', 'surface-3'], 4.5],
  ['on-accent', ['accent'], 4.5],
  ['accent-strong', ['bg', 'surface', 'surface-2', 'accent-soft'], 4.5],
  ['accent', ['bg', 'surface'], 3],
  ['line-strong', ['bg', 'surface', 'surface-2'], 3],
  ['ok', ['bg', 'surface'], 4.5],
  ['warn', ['bg', 'surface'], 4.5],
  ['bad', ['bg', 'surface'], 4.5],
];
let problems = drift; let checked = 0;
for (const [name, vars] of [['dark', dark], ['light', light]]) {
  for (const [text, bgs, min] of PAIRS) {
    for (const bgName of bgs) {
      const t = parse(vars[text] || '', vars); const bg0 = parse(vars[bgName] || '', vars); const page = parse(vars.bg, vars);
      if (!t || !bg0) { problems++; console.log(`FAIL ${name}: ${text} on ${bgName}: a colour this script can't read (${vars[text]} / ${vars[bgName]}); use hex, rgb() or hsl()`); continue; }
      const bg = over(bg0, page); const r = ratio(over(t, bg), bg); checked++;
      const ok = r >= min;
      if (!ok) problems++;
      console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${text} on ${bgName} ${r.toFixed(2)}:1 (needs ${min})`);
    }
  }
}
console.log(problems ? `\n${problems} of ${checked} pairs under AA` : `\npalette: ${checked} pairs at AA`);
process.exit(problems ? 1 : 0);
