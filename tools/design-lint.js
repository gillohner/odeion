// Design lint for the web UI (STEWARD-FIX-design-lint.md). No dependencies; CI runs it.
//   1. every <button> has a class from the component set;
//   2. no emoji or dingbats as icons (icons come from icon()); a line may opt out with
//      `lint-allow-glyph` when the glyph is a component's own styled text (the rating stars);
//   3. every class the UI uses has a rule in app.css;
//   4. every transition and animation takes its duration from the motion scale (--dur-fast/base/slow),
//      except a looping indicator (spin, shimmer, pulse, progress bars) and a bar's width; the
//      long-hand duration and delay properties too;
//   5. every font-size and border-radius takes a step of the type or radius scale (--fs-*, --r-*);
//      a radius of 3 px or less (bars, marks), 50%, relative units and a fluid clamp() are fine.
//   6. Odeion's own words in the page's copy: Bookmarks, never Favourites; Collections, never
//      Playlists (tools/refactor/wording.js has the table, what another product's own word is, and
//      --write to put it right).
//   node tools/design-lint.js [static dir]
const fs = require('fs');
const path = require('path');
const DIR = process.argv[2] || path.join(__dirname, '..', 'crates', 'pm-web', 'static');
// (app.js is kept as parts, crates/pm-web/ui/app: read joined, as the node serves it, and a
// problem in it is reported at its part and line)
const ui = require('./ui-src');
const files = [...fs.readdirSync(DIR).filter((f) => /\.(js|html)$/.test(f) && f !== 'app.js').map((f) => [f, fs.readFileSync(path.join(DIR, f), 'utf8')]), ['app.js', ui.appJs(DIR).toString('utf8')]];
const css = fs.readdirSync(DIR).filter((f) => f.endsWith('.css')).map((f) => fs.readFileSync(path.join(DIR, f), 'utf8')).join('\n');
const problems = [];
const where = (file, src, idx) => (file === 'app.js' && ui.hasParts(DIR) ? `app/${ui.where(idx, DIR)}` : `${file}:${src.slice(0, idx).split('\n').length}`);

// Spacing, layers and font weights stand on their tokens: padding, margin and gap take a step of
// the spacing scale (--sp-*), z-index a named layer (--z-*), font-weight a weight of the scale
// (--fw-*), wherever the value has a token (tools/refactor/tokens.js: its table says what is on the
// scale, its --write moves plain values onto it).
{
  const appCss = path.join(DIR, 'app.css');
  if (fs.existsSync(appCss)) problems.push(...require('./refactor/tokens.js').check(fs.readFileSync(appCss, 'utf8')).problems);
}

// The component set a button may belong to.
const BUTTON_CLASSES = ['btn', 'set-tab', 'np-artbtn', 'chip', 'icon-btn', 'chiplink', 'chipx', 'half', 'back', 'tile', 'link', 'nav-item', 'pop-item', 'tab', 'seg', 'row-btn', 'list-row', 'toast-x', 'close', 'crumb', 'zone-btn', 'out-btn', 'play-btn', 'sbtn', 'mini', 'qbtn', 'rowplay', 'idmark', 'card', 'quality', 'connbar-retry', 'lnk', 'upstreams', 'jobchip', 'faces', 'bnav', 'az', 'ptag', 'np-dockbtn', 'np-stab', 'np-grab'];
// Classes set only by the browser or third parties, never styled by us.
const IGNORE_CLASSES = new Set(['ic']);

// Characters that are icons, not text: emoji, symbols and pictographs, dingbats, geometric
// shapes, misc symbols. Copy punctuation (· — … “ ” › ← → × ½ ⋯) is outside these ranges.
const ICONISH = /[\u{1F000}-\u{1FAFF}\u{2600}-\u{27BF}\u{25A0}-\u{25FF}\u{2B00}-\u{2BFF}\u{1F1E6}-\u{1F1FF}]/u;

// A token glued to an interpolation (`f${n}`) is a computed class, not checked here.
const classTokens = (value) => value.replace(/\S*\$\{[^}]*\}\S*/g, ' ').split(/\s+/).filter((t) => /^[a-zA-Z][\w-]*$/.test(t));
const used = new Map(); // class -> first place
const use = (c, at) => { if (!used.has(c)) used.set(c, at); };

for (const [file, src] of files) {
  // 1. buttons
  for (const m of src.matchAll(/<button\b([^>]*)>/g)) {
    const attrs = m[1];
    const cls = /class="([^"]*)"/.exec(attrs);
    const tokens = cls ? classTokens(cls[1]) : [];
    const dynamic = cls && /\$\{/.test(cls[1]);
    if (!tokens.some((t) => BUTTON_CLASSES.includes(t)) && !(dynamic && /\$\{[^}]*'(btn|chip|icon-btn)\b/.test(cls[1]))) {
      problems.push(`${where(file, src, m.index)}  button without a component class: <button${attrs.slice(0, 80)}>`);
    }
  }
  for (const m of src.matchAll(/createElement\('button'\)/g)) {
    const after = src.slice(m.index, m.index + 300);
    if (!/className\s*=|classList\.add/.test(after)) problems.push(`${where(file, src, m.index)}  created button without a class`);
  }
  // 2. icon glyphs
  src.split('\n').forEach((line, i) => {
    if (line.includes('lint-allow-glyph') || /^\s*(\/\/|\*|<!--)/.test(line)) return;
    const g = ICONISH.exec(line);
    if (g) problems.push(`${file}:${i + 1}  emoji or symbol used as an icon: ${JSON.stringify(g[0])} (use icon())`);
  });
  // 3. classes used
  for (const m of src.matchAll(/class="([^"]*)"/g)) classTokens(m[1]).forEach((c) => use(c, where(file, src, m.index)));
  for (const m of src.matchAll(/classList\.(add|toggle|remove)\(([^)]*)\)/g)) {
    // toggle(name, force): only the first argument is a class
    const names = [...m[2].matchAll(/'([\w-]+)'/g)].map((x) => x[1]);
    (m[1] === 'toggle' ? names.slice(0, 1) : names).forEach((c) => use(c, where(file, src, m.index)));
  }
  for (const m of src.matchAll(/className\s*=\s*'([^']*)'/g)) classTokens(m[1]).forEach((c) => use(c, where(file, src, m.index)));
  // ternaries inside class attributes: `${x ? 'on' : ''}`
  for (const m of src.matchAll(/class="[^"]*"/g)) {
    // only the branches of a ternary, not the strings it compares
    for (const e of m[0].matchAll(/\$\{([^}]*)\}/g)) for (const s of e[1].matchAll(/[?:]\s*'([a-zA-Z][\w-]*(?: [a-zA-Z][\w-]*)*)'/g)) s[1].split(' ').forEach((c) => use(c, where(file, src, m.index)));
  }
}
const hasRule = (c) => new RegExp(`\\.${c.replace(/[-]/g, '\\-')}(?![\\w-])`).test(css);
for (const [c, at] of [...used.entries()].sort()) {
  if (IGNORE_CLASSES.has(c) || hasRule(c)) continue;
  problems.push(`${at}  class "${c}" has no rule in app.css`);
}

// 5. type and radius on their scales
{
  const cssFiles = fs.readdirSync(DIR).filter((f) => f.endsWith('.css'));
  for (const f of cssFiles) {
    const src = fs.readFileSync(path.join(DIR, f), 'utf8');
    const re = /(font-size|border(?:-[a-z]+)*-radius|font):([^;{}]*)/g; let m;
    while ((m = re.exec(src))) {
      const decl = m[2]; const at = `${f}:${src.slice(0, m.index).split('\n').length}`;
      if (/clamp\(|\d(vw|vh|vmin)\b/.test(decl)) continue; // a fluid display size (the dashboard) scales with the screen
      if (/radius$/.test(m[1])) {
        const raw = decl.match(/(\d*\.?\d+)px/g) || [];
        if (raw.some((v) => parseFloat(v) > 3)) problems.push(`${at}  border-radius off the scale: use var(--r-sm|md|lg|pill): ${decl.trim()}`);
      } else if (/(^|[^\w.-])\d*\.?\d+px\b/.test(decl.replace(/\/[\d.]+/, ''))) {
        problems.push(`${at}  ${m[1]} off the type scale: use var(--fs-2xs|xs|sm|md|lg|xl|2xl|3xl|display|art): ${decl.trim().slice(0, 50)}`);
      }
    }
  }
}

// A form's submit handler hands its work to the page's busy mark: a block-bodied handler that
// starts a save and returns nothing leaves the button without progress and lets a second Enter
// send again (every onsubmit goes through busy(), which can only wait for a promise it is given).
// `return` the save's promise, or make the handler async. (What it cannot see: an async handler
// that starts a save without awaiting it, or a body that returns early and then starts one.)
for (const [file, src] of files) {
  if (!file.endsWith('.js')) continue;
  for (const m of src.matchAll(/\.onsubmit\s*=\s*(async\s*)?\(([\w\s]*)\)\s*=>\s*\{/g)) {
    if (m[1]) continue;
    let i = m.index + m[0].length; let depth = 1;
    while (i < src.length && depth) { const ch = src[i]; if (ch === '{') depth++; else if (ch === '}') depth--; i++; }
    const body = src.slice(m.index + m[0].length, i - 1);
    if (!/\breturn\b|\bawait\b/.test(body)) problems.push(`${where(file, src, m.index)}  a form's submit handler returns nothing: return the save's promise (or make it async), so its button shows progress and a second Enter does not send again: ${body.trim().slice(0, 70)}`);
  }
}

// 4. durations on the motion scale
{
  const cssFiles = fs.readdirSync(DIR).filter((f) => f.endsWith('.css'));
  const LOOP = /\b(spin|shimmer|conn-pulse|navprog|nudge)\b|\binfinite\b|(?<![-\w])width\b/;
  for (const f of cssFiles) {
    const src = fs.readFileSync(path.join(DIR, f), 'utf8');
    const re = /(transition|animation|transition-duration|animation-duration|transition-delay|animation-delay):([^;{}]*)/g; let m;
    while ((m = re.exec(src))) {
      // (each comma-separated part on its own: one on the scale doesn't excuse another)
      // (commas inside cubic-bezier() or steps() don't split)
      for (const part of m[2].split(/,(?![^(]*\))/)) {
        const decl = part.trim();
        if (!decl || LOOP.test(decl) || /^none\b/.test(decl)) continue;
        const first = decl.match(/(^|[^\w.])(-?\d*\.?\d+m?s)\b/); // the duration is the first time (a negative one too); a delay after a scale duration is fine
        if (first && !/var\(--dur-/.test(decl.slice(0, first.index + 1))) problems.push(`${f}:${src.slice(0, m.index).split('\n').length}  ${m[1]} with a raw duration: use var(--dur-fast|base|slow): ${decl.slice(0, 60)}`);
      }
    }
  }
}

// 6. Odeion's own words
{
  const wording = require('./refactor/wording');
  // The script is the parts beside DIR (DIR/../ui/app) or, in a tree from before the split, DIR/app.js
  const script = ui.hasParts(DIR) ? ui.parts(DIR).map((p) => path.join(DIR, '..', 'ui', 'app', p.name)) : [path.join(DIR, 'app.js')];
  if (!script.some((x) => fs.existsSync(x))) problems.push('app.js: neither the script nor its parts found, so its words were not checked');
  const isScript = (x) => /^\d\d-[a-z0-9-]+\.js$/.test(path.basename(x)) || path.basename(x) === 'app.js';
  const pages = [...wording.PAGE_FILES.filter((x) => !isScript(x)).map((x) => path.join(DIR, path.basename(x))), ...script];
  for (const f of pages.filter((x) => fs.existsSync(x))) {
    const name = script.includes(f) && ui.hasParts(DIR) ? `app/${path.basename(f)}` : path.basename(f);
    for (const h of wording.run(f, 'check').found) problems.push(`${name}:${h.line}  says “${h.word}” (…${h.around}…): Odeion says Bookmarks and Collections; node tools/refactor/wording.js --write`);
  }
}

if (problems.length) {
  for (const p of problems) console.log(p);
console.log(`\n${problems.length} design problem${problems.length === 1 ? '' : 's'}`);
  process.exit(1);
}
console.log(`design lint: ok (${used.size} classes, every button classed, no icon glyphs)`);
