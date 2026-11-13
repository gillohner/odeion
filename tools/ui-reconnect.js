// Real-browser check of the connection (docs/CONNECTION.md): two controllers on a playing zone, the node killed
// and restored. Measures how fast both notice, how long the resync takes after the node is back, that a change
// made during a short outage is held and goes through, and that one made during a long outage is refused.
//   npm install playwright@1.49.1 && npx playwright install chromium   (anywhere; run from there)
//   PM_NULL_OUTPUT=realtime node /path/to/tools/ui-reconnect.js <token> <http://127.0.0.1:7799> <shots dir> -- <pm-node> --config <node.toml> serve
// The script starts and kills the node itself (the command after `--`). The node needs at least one track
// longer than a minute and a `null-output` zone; PM_NULL_OUTPUT=realtime makes the null output play in real time.
// The arguments are checked before anything starts: exactly three before `--` and a node command after it, else
// a usage line and exit 2 (tools/ui-reconnect-args.js). Without the check, a missing `--` made the node command
// the whole argv, so the script started itself again and again.
const { spawn } = require('child_process');
const USAGE = 'usage: node tools/ui-reconnect.js <token> <http://host:port> <shots dir> -- <pm-node> [args...]';
const parseArgs = (argv) => {
  const sep = argv.indexOf('--', 2);
  if (sep === -1) return { error: 'no `--` before the node command' };
  if (sep !== 5) return { error: 'expected exactly <token> <url> <shots dir> before `--`' };
  const [token, base, out] = argv.slice(2, 5);
  if (!token || !out) return { error: 'the token and the shots dir must not be empty' };
  let url = null;
  try { url = new URL(base); } catch { /* checked below */ }
  if (!url || !/^https?:$/.test(url.protocol)) return { error: 'the url must be http:// or https://' };
  const node = argv.slice(sep + 1);
  if (!node.length || !node[0]) return { error: 'no node command after `--`' };
  return { token, base, out, node };
};
const CHILD_MARK = 'PM_UI_RECONNECT_CHILD'; // set for the node it starts: the script refuses to run under itself
if (require.main !== module) { module.exports = { parseArgs, USAGE }; return; }
const args = process.env[CHILD_MARK] ? { error: 'started by ui-reconnect.js itself' } : parseArgs(process.argv);
if (args.error) { console.error(`ui-reconnect.js: ${args.error}\n${USAGE}`); process.exit(2); }
const { token: TOKEN, base: BASE, out: OUT, node: NODE } = args;
const ZONE = process.env.ZONE || 'null-output';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const api = async (method, path, body) => {
  const r = await fetch(BASE + '/v1' + path, { method, headers: { authorization: 'Bearer ' + TOKEN, 'content-type': 'application/json' }, body: body && JSON.stringify(body) });
  const t = await r.text(); return t ? JSON.parse(t) : null;
};
let node = null;
const nodeRunning = () => Boolean(node && node.pid && node.exitCode === null && node.signalCode === null);
// The node runs in its own process group, so a wrapper (`sh -c …`, `cargo run …`) is stopped with what it started.
const killGroup = () => { try { process.kill(-node.pid, 'SIGKILL'); } catch { /* already gone */ } };
// Whatever way the script ends (a failed check, an exception, Ctrl+C), the node it started does not outlive it.
process.on('exit', () => { if (nodeRunning()) killGroup(); });
for (const [sig, code] of [['SIGINT', 130], ['SIGTERM', 143]]) process.on(sig, () => process.exit(code));
const startNode = async () => {
  let failedToStart = null;
  node = spawn(NODE[0], NODE.slice(1), { stdio: 'ignore', detached: true, env: { ...process.env, [CHILD_MARK]: '1' } });
  node.once('error', (e) => { failedToStart = 'the node command did not start: ' + e.code; });
  node.once('exit', () => { failedToStart = failedToStart || 'the node command exited at start'; });
  for (let i = 0; i < 100 && !failedToStart; i++) { try { await api('GET', '/info'); break; } catch { await sleep(100); } }
  // An answer from another node on the same port doesn't count: ours must still be running.
  await sleep(200);
  if (failedToStart) throw new Error(failedToStart);
  if (!nodeRunning()) throw new Error('the node command exited at start');
  try { await api('GET', '/info'); } catch { throw new Error('node did not start'); }
};
const killNode = () => new Promise((r) => { if (!nodeRunning()) return r(); node.once('exit', r); killGroup(); });
const until = async (what, fn, ms = 60000) => { const t = Date.now(); while (Date.now() - t < ms) { if (await fn()) return Date.now() - t; await sleep(25); } throw new Error('timed out: ' + what); };
const connState = (p) => p.$eval('#conn', (e) => e.dataset.state || '');
const barTitle = (p) => p.$eval('#np .np-title', (e) => e.textContent);
const toggleLabel = (p) => p.$eval('#toggle', (e) => e.getAttribute('aria-label'));
let failed = false;
const check = (ok, msg) => { console.log((ok ? 'ok    ' : 'FAILED ') + msg); if (!ok) failed = true; };

(async () => {
  await startNode();
  let tracks = [];
  for (let i = 0; i < 40 && !tracks.length; i++) {
    tracks = (await api('GET', '/library/tracks')).tracks.filter((t) => t.duration_ms > 60000);
    if (!tracks.length) await sleep(500); // the node scans its config folder at start
  }
  if (!tracks.length) throw new Error('needs a track longer than a minute');
  await api('POST', `/zones/${ZONE}/queue`, { track_ids: [tracks[0].id], replace: true, play: true });
  const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
  const b = await chromium.launch();
  const pages = [];
  for (const [w, h, name] of [[1280, 800, 'desktop'], [390, 844, 'phone']]) {
    const ctx = await b.newContext({ viewport: { width: w, height: h }, colorScheme: name === 'phone' ? 'light' : 'dark' });
    await ctx.addInitScript(([t, z]) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', z); }, [TOKEN, ZONE]);
    const p = await ctx.newPage(); p.name = name;
    p.on('pageerror', (e) => check(false, `${name}: page error ${e.message}`));
    await p.goto(BASE + '/');
    pages.push(p);
  }
  const [A, B] = pages;
  for (const p of pages) await until(`${p.name} connected`, async () => (await connState(p)) === 'connected');
  for (const p of pages) await until(`${p.name} shows the track`, async () => (await barTitle(p)) === tracks[0].title, 5000);
  await until('latency shown', async () => /\d+ ms/.test(await A.$eval('#conn', (e) => e.textContent)), 15000);
  console.log('      connected, latency:', await A.$eval('#conn', (e) => e.textContent));

  // 1. Short outage: paused, kill, press play on A while down, restore within the hold time. (The button
  // sends what it shows: play while paused, pause while playing.)
  await A.click('#toggle');
  for (const p of pages) await until(`${p.name} shows paused`, async () => (await toggleLabel(p)) === 'Play', 5000);
  let t0 = Date.now();
  await killNode();
  const noticed = await Promise.all(pages.map((p) => until(`${p.name} notices`, async () => (await connState(p)) !== 'connected', 30000)));
  check(Math.max(...noticed) < 2000, `both controllers notice the node is gone (${noticed.join(' / ')} ms)`);
  await sleep(2600 - (Date.now() - t0));
  for (const p of pages) await p.screenshot({ path: `${OUT}/conn-reconnecting-${p.name}.png` });
  const pressed = A.click('#toggle'); // play, held until the node is back
  await sleep(500);
  await startNode();
  const tUp = Date.now();
  const resync = await Promise.all(pages.map((p) => until(`${p.name} back`, async () => (await connState(p)) === 'connected', 30000)));
  // While an outage is young (its first 20 s) or a change is held, the controllers retry at most
  // 1.5 s apart (app.js scheduleRetry, the fast phase): the retry that finds the node back is at most
  // 1.5 s away, so 3 s covers it and the connect. That holds only while the outage stays inside the
  // fast phase: checked first, so a slower start fails on its own line.
  check(tUp - t0 < 18000, `the outage stays inside the controllers' fast retry phase (${tUp - t0} ms)`);
  check(Math.max(...resync) < 3000, `both controllers resync after the restart (${resync.join(' / ')} ms after the node was back; ${tUp - t0} ms outage)`);
  await pressed;
  // The node restores the queue stopped; the held play starts it again, and both controllers show it.
  const played = await Promise.all(pages.map((p) => until(`${p.name} sees the held play`, async () => (await toggleLabel(p)) === 'Pause', 10000).catch(() => -1)));
  const z = (await api('GET', `/zones/${ZONE}`));
  check(z.transport === 'playing' && played.every((t) => t >= 0), `the change held during the outage went through and both show it (node: ${z.transport})`);
  for (const p of pages) check((await barTitle(p)) === tracks[0].title, `${p.name} shows the same track as the node`);

  // 2. Two controllers stay in sync: pause on B, A follows.
  t0 = Date.now(); await B.click('#toggle');
  const follow = await until('A follows B', async () => (await toggleLabel(A)) === 'Play', 5000);
  check(follow < 1000, `a change on one controller shows on the other (${follow} ms)`);

  // 3. Long outage: a change is refused with a message after the hold time; after the restart, Retry now (or a
  //    retry already due) reconnects at once.
  await killNode();
  await until('A notices', async () => (await connState(A)) !== 'connected', 30000);
  await sleep(21000);
  for (const p of pages) await p.screenshot({ path: `${OUT}/conn-offline-${p.name}.png` });
  check((await connState(A)) === 'offline', `after 20 s the state is offline (${await A.$eval('#conn', (e) => e.textContent)})`);
  t0 = Date.now(); await A.click('#toggle');
  const refused = await until('refusal toast', async () => /Not connected/.test(await A.$eval('#toast', (e) => e.className.includes('show') ? e.textContent : '')), 12000);
  check(refused > 7000, `a change during a long outage is refused after the hold time (${refused} ms)`);
  check((await api('GET', `/zones/${ZONE}`).catch(() => null)) === null, 'the node is still down (nothing was sent)');
  await startNode();
  // Offline past the fast phase, the controller retries by itself at growing gaps (1-2 s, then 2-4 s, ...), and
  // startNode() returns only once the node answers, so a retry may already have landed and hidden the bar. The bound
  // is measured from the node answering: Retry now is clicked through the DOM (no visibility wait, since the bar can
  // hide the moment the controller is back; a click on an up link is a no-op), unless the controller is already back.
  t0 = Date.now();
  const alreadyBack = (await connState(A)) === 'connected';
  if (!alreadyBack) await A.$eval('#connretry', (b) => b.click());
  const retry = await until('reconnect after the restart', async () => (await connState(A)) === 'connected', 5000);
  check(retry < 2000, `${alreadyBack ? 'the controller was back by itself when the node answered' : 'Retry now reconnects at once'} (${retry} ms)`);
  const zb = await api('GET', `/zones/${ZONE}`);
  check(zb.transport !== 'playing', `the refused change was not applied later (node: ${zb.transport})`);
  await B.waitForTimeout(100);
  await b.close();
  await killNode();
  process.exitCode = failed ? 1 : 0;
})().catch((e) => { console.error(e); process.exit(1); }); // the exit handler stops the node
