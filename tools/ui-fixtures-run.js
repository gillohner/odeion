// Run the UI fixture pages of tools/ui-fixtures.list: all of them, or one runner's share.
//
//   node tools/ui-fixtures-run.js [--shard K/N] [--jobs J] [--no-pins] [--shots DIR] [--plan] [page.js ...]
//
// --shard K/N  only the pages of runner K of N (CI: one job per runner). A page with a runner
//              number in the list runs there; the others go round robin in list order.
// --jobs J     run J pages at a time (default 1). Each page is its own node process with its
//              own Chromium, so they don't share anything but the machine.
// --no-pins    ignore the runner numbers in the list: every page goes round robin (for a number
//              of runners other than the one the list was balanced for)
// --shots DIR  where the screenshot folders go (default /tmp/pm)
// --plan       print which pages the runner would run, and run nothing
// page.js ...  only these pages of the list
//
// Playwright is resolved from the current directory, as the pages do: run it from a folder that
// has it (`npm install playwright`), e.g. `cd /tmp/pw && node <repo>/tools/ui-fixtures-run.js`.
// Exit code 1 when a page fails; the failed pages' output is printed again at the end.
const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

const REPO = path.join(__dirname, '..');
const args = process.argv.slice(2);
const opt = (name) => { const i = args.indexOf(name); if (i < 0) return null; const v = args[i + 1]; args.splice(i, 2); return v; };
const flag = (name) => { const i = args.indexOf(name); if (i < 0) return false; args.splice(i, 1); return true; };
const shardArg = opt('--shard');
const jobsArg = opt('--jobs') || process.env.UI_FIXTURES_JOBS || '1';
const noPins = flag('--no-pins');
const shots = opt('--shots') || '/tmp/pm';
const planOnly = flag('--plan');
const only = args;

const fail = (m) => { console.error(`::error::${m}`); process.exit(1); };
// "--jobs x" would be NaN: no page would run and the run would end green
if (!/^[1-9]\d*$/.test(jobsArg)) fail(`--jobs ${jobsArg}: give a number of pages to run at a time, 1 or more`);
const jobs = Number(jobsArg);
const pages = [];
for (const line of fs.readFileSync(path.join(__dirname, 'ui-fixtures.list'), 'utf8').split('\n')) {
  const t = line.trim();
  if (!t || t.startsWith('#')) continue;
  const [script, folder, pin, ...rest] = t.split(/\s+/);
  if (!folder || rest.length) fail(`ui-fixtures.list: "${t}" is not <script> <shots folder> [runner]`);
  if (pages.some((p) => p.script === script)) fail(`${script} is in the list twice`);
  if (!fs.existsSync(path.join(__dirname, script))) fail(`${script} is in the list but not in tools/`);
  if (pin !== undefined && !/^[1-9]\d*$/.test(pin)) fail(`${script}: "${pin}" is not a runner number`);
  pages.push({ script, folder, pin: pin ? Number(pin) : null });
}

// A page written against the fixtures (it loads ./ui-fixture) and left out of the list would
// never run: ui-open-place.js was a fix's test for a week without running once
for (const f of fs.readdirSync(__dirname)) {
  if (!/^ui-.*\.js$/.test(f) || f === 'ui-fixture.js' || f === 'ui-fixtures-run.js' || pages.some((p) => p.script === f)) continue;
  if (/require\(['"]\.\/ui-fixture(\.js)?['"]\)/.test(fs.readFileSync(path.join(__dirname, f), 'utf8'))) fail(`${f} uses the fixtures but is not in tools/ui-fixtures.list: add a line for it`);
}

let [shard, shards] = [1, 1];
if (shardArg) {
  const m = /^(\d+)\/(\d+)$/.exec(shardArg);
  if (!m || +m[1] < 1 || +m[1] > +m[2]) fail(`--shard ${shardArg}: give K/N with 1 <= K <= N`);
  [shard, shards] = [+m[1], +m[2]];
  if (noPins) for (const p of pages) p.pin = null;
  for (const p of pages) if (p.pin && p.pin > shards) fail(`${p.script} is set to runner ${p.pin}: there are ${shards}`);
}
let n = 0;
for (const p of pages) p.shard = shards === 1 ? 1 : p.pin || (n++ % shards) + 1;
for (const o of only) if (!pages.some((p) => p.script === o)) fail(`${o} is not in tools/ui-fixtures.list`);
const mine = pages.filter((p) => p.shard === shard && (!only.length || only.includes(p.script)));
// The pages with a runner number are the long ones: started first, so that with several at a
// time a long page is not the last to start
if (jobs > 1) mine.sort((a, b) => (b.pin ? 1 : 0) - (a.pin ? 1 : 0));

if (planOnly) { for (const p of mine) console.log(p.script); process.exit(0); }

const started = Date.now();
const secs = (ms) => (ms / 1000).toFixed(0) + ' s';
const results = [];
let next = 0;
function runOne(p) {
  return new Promise((done) => {
    const t0 = Date.now();
    const out = [];
    // Said at once: a page that never ends is then named in the log (its output comes at its end)
    console.log(`started ${p.script}`);
    const child = spawn(process.execPath, [path.join(__dirname, p.script), REPO, path.join(shots, p.folder)], { stdio: ['ignore', 'pipe', 'pipe'] });
    child.stdout.on('data', (d) => out.push(d));
    child.stderr.on('data', (d) => out.push(d));
    child.on('close', (code, signal) => {
      const r = { ...p, code: code === null ? `signal ${signal}` : code, ms: Date.now() - t0, out: Buffer.concat(out).toString('utf8') };
      results.push(r);
      // One page's output in one piece, so pages run side by side don't mix their lines
      console.log(`::group::${p.script} (${secs(r.ms)}${r.code === 0 ? '' : ', FAILED'})`);
      process.stdout.write(r.out.endsWith('\n') || !r.out ? r.out : r.out + '\n');
      console.log('::endgroup::');
      done();
    });
  });
}
async function worker() { while (next < mine.length) await runOne(mine[next++]); }

(async () => {
  console.log(`${mine.length} pages${shardArg ? ` (runner ${shard} of ${shards})` : ''}, ${jobs} at a time`);
  await Promise.all(Array.from({ length: Math.min(jobs, mine.length) }, worker));
  const failed = results.filter((r) => r.code !== 0);
  const slow = [...results].sort((a, b) => b.ms - a.ms).slice(0, 8).map((r) => `${r.script} ${secs(r.ms)}`).join(', ');
  console.log(`${results.length - failed.length} of ${results.length} pages passed in ${secs(Date.now() - started)} (pages' own time ${secs(results.reduce((s, r) => s + r.ms, 0))}); slowest: ${slow}`);
  for (const r of failed) {
    console.log(`::error::${r.script} failed (exit ${r.code})`);
    process.stdout.write(r.out.split('\n').slice(-40).join('\n') + '\n');
  }
  process.exit(failed.length ? 1 : 0);
})();
