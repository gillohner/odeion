// Cut the web UI's script (app.js) into its ordered parts, crates/pm-web/ui/app/NN-name.js.
// The node's build (crates/pm-web/build.rs) and the tools (tools/ui-src.js) join the parts end to
// end in name order into the one /app.js the node serves, so the cut changes no byte of it: this
// script checks that before it writes anything.
//
// The cut is by text, not by line number: each part starts at the one line that begins with its
// anchor. So it can be run again on a newer app.js (a branch made before the split, or main as it
// is when this lands) and gives the same parts with that file's changes in them.
//
//   node tools/refactor/web-ui-split.js <app.js> [--base <app.js>] [--check]
//
// <app.js>: the whole script: the file as it was before the split, or a branch's copy of it
// (`git show <branch>:<where that branch has app.js> > /tmp/app.js`).
// --base <app.js>: needed when the parts exist and differ from <app.js> (a branch's copy is cut
// into them): the script as it was where that branch started. The parts here have to join to
// exactly that file, or nothing is written: cutting a branch's copy over parts that others have
// changed since would undo their changes without a word. So this runs at the commit of the
// split (where the parts are the base), and the result is then rebased onto main, which merges
// part by part (crates/pm-web/ui/README.md has the steps).
// Without a plan of its own it uses the parts that exist (each part's first line is its anchor),
// so parts added or renamed later are kept; the first split uses PLAN below.
// --check: say what would be written, write nothing.
const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..', '..');
const PARTS = path.join(ROOT, 'crates', 'pm-web', 'ui', 'app');

// The first split: part name, and how its first line begins. Every cut is between two
// statements of the script's one closure, or between two pages (`case`s) of renderPage.
const PLAN = [
  ['00-core', '/* Odeion web UI. Talks '],
  ['01-toast-busy-api', '  // One place for the t'],
  ['02-library-add', '  // ---- Library › Add '],
  ['03-dialogs', '  // Wrapped phone dialo'],
  ['04-copy-jobs', '  // ---- Copy to your h'],
  ['05-login', '  // ---- Log in / Log o'],
  ['06-engines', '  // ---- zones & playba'],
  ['07-zones-volume', '  // The YouTube panel: '],
  ['08-signin-setup', '  // Play on a Spotify C'],
  ['09-play-queue', '  const hitItem = (h, al'],
  ['10-zones-outputs', "  // The bar's volume sl"],
  ['11-shell', "  // The tabs' Library: "],
  ['12-player-bar', '  // ---- player bar ---'],
  ['13-now-playing', '  // ---- Now Playing ov'],
  ['14-routing', '  // ---- views --------'],
  ['15-search-run-keys', "  const search = $('#sea"],
  ['16-rows-tags', '  // ---- templates ----'],
  ['17-menus', '  // A small anchored me'],
  ['18-items-select', '  // ---- one row, one t'],
  ['19-search-view', "  const SOURCE_RANK = ['"],
  ['20-home-lists', '  // ---- Home (STEWARD-'],
  ['21-setup-nav-render', '  // ---- first run (ste'],
  ['22-pages-library', "        case 'home': {"],
  ['23-pages-playlists', "        case 'playlists'"],
  ['24-pages-sources', "        case 'sources': "],
  ['25-pages-item', "        case 'item': {"],
  ['26-pages-queue-zones-library', "        case 'queue': {"],
  ['27-pages-settings', "        case 'settings':"],
  ['28-connection-identify-events', '        default:'],
];

const argv = process.argv.slice(2);
const baseAt = argv.indexOf('--base');
const baseFile = baseAt >= 0 ? argv.splice(baseAt, 2)[1] : null;
const check = argv.includes('--check');
const file = argv.filter((a) => a !== '--check')[0];
if (!file || (baseAt >= 0 && !baseFile)) { console.error('usage: node tools/refactor/web-ui-split.js <app.js> [--base <app.js>] [--check]'); process.exit(2); }
const whole = fs.readFileSync(file);
const lines = whole.toString('utf8').split('\n');

// The plan: the parts that exist (name, first line), else PLAN
const existing = fs.existsSync(PARTS) ? fs.readdirSync(PARTS).filter((f) => /^\d\d-[a-z0-9-]+\.js$/.test(f)).sort() : [];
const plan = existing.length
  ? existing.map((f) => [f.replace(/\.js$/, ''), fs.readFileSync(path.join(PARTS, f), 'utf8').split('\n')[0]])
  : PLAN;

const fail = (m) => { console.error('web-ui-split: ' + m); process.exit(1); };
// Parts that exist and are not this file: only a file that descends from them may replace them
if (existing.length) {
  const now = Buffer.concat(existing.map((f) => fs.readFileSync(path.join(PARTS, f))));
  if (!now.equals(whole)) {
    if (!baseFile) fail(`the parts in ${path.relative(ROOT, PARTS)} are not ${file}. To cut a branch's copy into them, give the script as it was where the branch started: --base <app.js> (see crates/pm-web/ui/README.md)`);
    if (!now.equals(fs.readFileSync(baseFile))) fail(`the parts here are not ${baseFile}: they have changed since that branch started, and writing the branch's copy over them would undo those changes. Check out the commit of the split (where the parts join to ${baseFile}), run this there, commit, and rebase onto main`);
  }
}
const starts = plan.map(([name, anchor]) => {
  const hits = [];
  lines.forEach((l, i) => { if (l.startsWith(anchor)) hits.push(i); });
  if (hits.length !== 1) fail(`the first line of ${name} (${JSON.stringify(anchor)}) is on ${hits.length} lines of ${file}; give that part's start a line of its own`);
  return hits[0];
});
if (starts[0] !== 0) fail(`${plan[0][0]} must start at the first line`);
for (let i = 1; i < starts.length; i++) if (starts[i] <= starts[i - 1]) fail(`${plan[i][0]} starts before ${plan[i - 1][0]}: the parts are out of order`);

const parts = plan.map(([name], i) => {
  const to = i + 1 < plan.length ? starts[i + 1] : lines.length;
  let text = lines.slice(starts[i], to).join('\n');
  if (i + 1 < plan.length) text += '\n';
  return { name: name + '.js', text, lines: to - starts[i] - (i + 1 < plan.length ? 0 : 1) };
});
if (!Buffer.concat(parts.map((p) => Buffer.from(p.text))).equals(whole)) fail('the parts joined are not the file: nothing written');

for (const p of parts) console.log(`${p.name}\t${p.lines} lines\t${Buffer.byteLength(p.text)} bytes${p.lines > 1200 ? '\t(long: over 1,200 lines)' : ''}`);
if (check) { console.log(`ok: ${parts.length} parts would be written, joined they are ${file} byte for byte`); process.exit(0); }
fs.mkdirSync(PARTS, { recursive: true });
for (const f of existing) if (!parts.some((p) => p.name === f)) fs.unlinkSync(path.join(PARTS, f));
for (const p of parts) fs.writeFileSync(path.join(PARTS, p.name), p.text);
console.log(`ok: ${parts.length} parts in ${path.relative(ROOT, PARTS)}, joined they are ${file} byte for byte`);
