// Spacing, layers and font weights in app.css stand on the design tokens.
//
//   node tools/refactor/tokens.js --check   fail when a value that has a token is written raw, or a
//                                           z-index is not a named layer (tools/design-lint.js runs
//                                           this); --write puts the first kind right
//   node tools/refactor/tokens.js --write   put the token definitions in (once) and move every raw
//                                           value that has a token onto it; run again on a newer
//                                           main: nothing that is already on a token changes
//   node tools/refactor/tokens.js --stats   how many values are on tokens, and what is left
//
// What moves is exact, so nothing looks different: `padding: 8px 12px` becomes
// `padding: var(--sp-2) var(--sp-3)`, `z-index: 45` becomes `var(--z-menu)`, `font-weight: 600`
// becomes `var(--fw-semibold)`. Only whole values of padding, margin and gap (and their one-sided
// forms), z-index and font-weight are looked at; comments, custom properties and anything inside
// calc() or another function stay as they are. No dependencies.
const fs = require('node:fs');
const path = require('node:path');

const CSS = path.join(__dirname, '..', '..', 'crates', 'pm-web', 'static', 'app.css');

// The spacing scale: 4 px steps, with the half steps a dense player's rows and pills are built on.
const SPACE = { '2px': '--sp-0h', '4px': '--sp-1', '6px': '--sp-1h', '8px': '--sp-2', '10px': '--sp-2h', '12px': '--sp-3', '14px': '--sp-3h', '16px': '--sp-4', '24px': '--sp-5', '32px': '--sp-6' };
// Layers, lowest first. The first six order parts inside one component; the rest order the page.
const LAYER = { '-1': '--z-below', 1: '--z-above', 2: '--z-above-2', 3: '--z-above-3', 5: '--z-dragging', 6: '--z-libnav', 15: '--z-sticky', 20: '--z-overlay', 25: '--z-scrim', 30: '--z-popover', 40: '--z-bar', 45: '--z-menu', 60: '--z-sheet', 65: '--z-toast', 70: '--z-progress' };
const WEIGHT = { 400: '--fw-regular', 500: '--fw-medium', 600: '--fw-semibold', 700: '--fw-bold' };
// (Values with no token, 18 px, 5 px, a negative margin, weight 550, are counted by --stats and
// left alone: each is a decision about one component, not a mechanical move.)

const SPACE_PROPS = /(?<![-\w])((?:padding|margin)(?:-(?:top|right|bottom|left|inline|block)(?:-(?:start|end))?)?|gap|row-gap|column-gap)(\s*:\s*)([^;{}]+)/g;
const Z_PROP = /(?<![-\w])(z-index)(\s*:\s*)([^;{}]+)/g;
const W_PROP = /(?<![-\w])(font-weight)(\s*:\s*)([^;{}]+)/g;

const DEFS = `  /* Spacing: 4 px steps and their halves (tools/refactor/tokens.js keeps padding, margin and gap on
     them; --sp-1 to --sp-6 are the 4/8 scale, the h steps lie between). */
  --sp-0h: 2px; --sp-1h: 6px; --sp-2h: 10px; --sp-3h: 14px;
  /* Layers, lowest first: parts inside one component (below … libnav), then the page's own. */
  --z-below: -1; --z-above: 1; --z-above-2: 2; --z-above-3: 3; --z-dragging: 5; --z-libnav: 6;
  --z-sticky: 15; --z-overlay: 20; --z-scrim: 25; --z-popover: 30; --z-bar: 40; --z-menu: 45; --z-sheet: 60; --z-toast: 65; --z-progress: 70;
  /* Font weights. */
  --fw-regular: 400; --fw-medium: 500; --fw-semibold: 600; --fw-bold: 700;
`;
const ANCHOR = '  --sp-1: 4px; --sp-2: 8px; --sp-3: 12px; --sp-4: 16px; --sp-5: 24px; --sp-6: 32px;\n';

// Comments blanked out (same length), so a position in the result is a position in the file
const withoutComments = (src) => src.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' '));
// The parts of a value outside any function: [[text, isTopLevel], …]
const topLevel = (value) => { const out = []; let depth = 0; let a = 0; for (let i = 0; i < value.length; i++) { const c = value[i]; if (c === '(') { if (depth === 0 && i > a) { out.push([value.slice(a, i), true]); a = i; } depth++; } else if (c === ')') { depth--; if (depth === 0) { out.push([value.slice(a, i + 1), false]); a = i + 1; } } } if (a < value.length) out.push([value.slice(a), depth === 0]); return out; };
// Each whole value outside a function goes through fn (a function's own name, the `max` of max(…),
// is no value in any table, and what is inside the brackets is not looked at)
const eachToken = (value, fn) => topLevel(value).map(([text, top]) => (top ? text.replace(/[^\s,/]+/g, (t) => fn(t)) : text)).join('');

const rewrite = (src) => {
  const bare = withoutComments(src); const edits = []; const left = { spacing: [], layer: [], weight: [] }; let moved = 0; let on = 0;
  const lineOf = (i) => src.slice(0, i).split('\n').length;
  const each = (re, table, kind, isRaw) => {
    re.lastIndex = 0; let m;
    while ((m = re.exec(bare))) {
      const at = m.index + m[1].length + m[2].length; const value = m[3];
      let changed = false;
      const next = eachToken(value, (t) => {
        if (Object.prototype.hasOwnProperty.call(table, t)) { changed = true; moved++; return `var(${table[t]})`; }
        if (isRaw(t)) left[kind].push({ line: lineOf(m.index), prop: m[1], value: t });
        return t;
      });
      on += (value.match(/var\(--(?:sp|z|fw)-/g) || []).length;
      if (changed) edits.push([at, at + value.length, next]);
    }
  };
  each(SPACE_PROPS, SPACE, 'spacing', (t) => /^-?\d*\.?\d+px$/.test(t) && t !== '0px');
  each(Z_PROP, LAYER, 'layer', (t) => /^-?\d+$/.test(t) && t !== '0'); // (0 only makes a stacking context: no layer)
  each(W_PROP, WEIGHT, 'weight', (t) => /^\d+$/.test(t));
  edits.sort((x, y) => x[0] - y[0]);
  let out = ''; let last = 0; for (const [a, b, text] of edits) { out += src.slice(last, a) + text; last = b; } out += src.slice(last);
  return { out, moved, on, left };
};
// The token definitions, put in after the 4/8 scale's line (once)
const withDefs = (src) => {
  if (src.includes('--z-below:')) return src;
  if (!src.includes(ANCHOR)) throw new Error('app.css: the line that defines --sp-1 to --sp-6 is not as expected; add the token definitions by hand (DEFS in this file)');
  return src.replace(ANCHOR, ANCHOR + DEFS);
};
// The definitions themselves are custom properties (their names start with --), which the property
// patterns skip; the values that have a token and are written raw are what --check reports.
const check = (src) => {
  const problems = [];
  if (!src.includes('--z-below:')) problems.push('app.css: the spacing, layer and weight tokens are not defined (node tools/refactor/tokens.js --write)');
  // (a var() of a token that is not defined would leave its declaration without a value)
  const bareAll = withoutComments(src); const defined = new Set((bareAll.match(/--(?:sp|z|fw)-[a-z0-9-]+(?=\s*:)/g) || []));
  for (const name of new Set(bareAll.match(/(?<=var\(\s*)--(?:sp|z|fw)-[a-z0-9-]+(?=[),\s])/g) || [])) if (!defined.has(name)) problems.push(`app.css: var(${name}) is used and not defined in :root`);
  const r = rewrite(src);
  if (r.moved) {
    const bare = withoutComments(src); const seen = [];
    const find = (re, table, word) => { re.lastIndex = 0; let m; while ((m = re.exec(bare))) { eachToken(m[3], (t) => { if (Object.prototype.hasOwnProperty.call(table, t)) seen.push(`app.css:${src.slice(0, m.index).split('\n').length}  ${m[1]}: ${t} has a token: use var(${table[t]}) (${word})`); return t; }); } };
    find(SPACE_PROPS, SPACE, 'spacing scale'); find(Z_PROP, LAYER, 'layers'); find(W_PROP, WEIGHT, 'font weights');
    problems.push(...seen.slice(0, 40)); if (seen.length > 40) problems.push(`… and ${seen.length - 40} more (node tools/refactor/tokens.js --write moves them all)`);
  }
  for (const l of r.left.layer) problems.push(`app.css:${l.line}  z-index: ${l.value} is not a layer: use one of ${Object.values(LAYER).map((n) => `var(${n})`).join(', ')}, or add a named layer to tools/refactor/tokens.js and :root`);
  return { problems, ...r };
};

if (require.main === module) {
  const args = process.argv.slice(2); const file = args.find((a) => !a.startsWith('--')) || CSS;
  const src = fs.readFileSync(file, 'utf8');
  if (args.includes('--write')) {
    const r = rewrite(withDefs(src));
    if (r.out !== src) fs.writeFileSync(file, r.out);
    console.log(`tokens: ${r.moved} values moved onto tokens (${r.left.spacing.length} spacing values and ${r.left.weight.length} weights have no token)`);
  } else if (args.includes('--stats')) {
    const r = rewrite(src); const count = (list) => { const c = {}; for (const l of list) c[l.value] = (c[l.value] || 0) + 1; return Object.entries(c).sort((x, y) => y[1] - x[1]).map(([v, n]) => `${v} ×${n}`).join(', '); };
    console.log(`on tokens: ${r.on}\nraw with a token: ${r.moved}\nspacing off the scale: ${r.left.spacing.length} (${count(r.left.spacing)})\nlayers without a name: ${r.left.layer.length}\nweights off the scale: ${r.left.weight.length} (${count(r.left.weight)})`);
  } else {
    const r = check(src);
    if (r.problems.length) { for (const p of r.problems) console.log(p); process.exit(1); }
    console.log(`tokens: ok (${r.on} spacing, layer and weight values on tokens; ${r.left.spacing.length} spacing and ${r.left.weight.length} weight values off the scale)`);
  }
}
module.exports = { check, rewrite, withDefs, SPACE, LAYER, WEIGHT };
