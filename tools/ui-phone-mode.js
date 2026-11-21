// Phone mode: the Android app serves this web UI from its own backend on the phone, with no node.
// Its /v1/info says mode "phone" and lists what it can do (capabilities); the page shows only
// those features and asks the phone for nothing else. Real candidate assets on the fixture origin
// (tools/ui-fixture.js); the phone's backend stood in for here, answering 503 for anything it
// doesn't serve, as the app's does. At 360 and 1280 px:
//   * phone: body.phone; Home, Your library and Zones are not offered; the search box is; a search
//     shows the TIDAL hit, with Everywhere and the phone's sources as its scopes only; Settings has
//     "This phone" with its streaming accounts and "Connect to a node" (busy at once, sent once);
//     no request went to zones, events, Home, tags, reviews, Pubky, playlists or favourites;
//   * a node (its /v1/info without mode): nothing of that is hidden, and it reads its zones.
//   node tools/ui-phone-mode.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-mode-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const PHONE_INFO = { mode: 'phone', capabilities: ['search', 'items', 'sources'], name: 'This phone', api: 'v1', version: '0.2.4', offline: false,
  library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0, total_duration_ms: 0 }, zones: 0, outputs: 0, scanning: false };
const SOURCES = [
  { id: 'tidal', name: 'TIDAL', description: 'TIDAL', status: 'signed in', enabled: true, features: ['search', 'stream'], stage: 'stable', downloadable: false, healthy: true },
  { id: 'qobuz', name: 'Qobuz', description: 'Qobuz', status: 'not signed in', enabled: true, features: ['search', 'stream'], stage: 'stable', downloadable: false, healthy: false },
];
const HIT = { kind: 'song', id: 'tidal-4242', title: 'Harbour Lights', subtitle: 'Phone Fixture Band', keys: ['isrc:XX0000000001'], uri: 'tidal:track:4242', sources: ['tidal'] };
// What the phone's backend serves (everything else: 503, as pm-offline's server does)
const SERVED = (v) => ['/info', '/me', '/zones', '/sources', '/play-order', '/search'].includes(v) || /^\/item\//.test(v) || /^\/sources\/[^/]+\//.test(v);
// Never asked of a phone that lists none of these capabilities
const FORBIDDEN = [/^\/zones/, /^\/events/, /^\/home/, /^\/tags/, /^\/reviews/, /^\/pubky/, /^\/playlists/, /^\/favourites/, /^\/search\/tags/, /^\/presence/, /^\/outputs/];

const record = async (run) => {
  run.reqs = [];
  run.page.on('request', (r) => { const u = new URL(r.url()); if (u.pathname.startsWith('/v1/')) run.reqs.push(`${r.method()} ${u.pathname.slice(3)}`); });
  // A WebSocket (the node's /v1/events) recorded as the page opens it
  await run.context.addInitScript(() => {
    window.__ws = []; window.__inv = [];
    const WS = window.WebSocket;
    window.WebSocket = function (url, p) { window.__ws.push(String(url)); return new WS(url, p); };
    window.WebSocket.prototype = WS.prototype; Object.assign(window.WebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
    // The app's Tauri bridge: the phone's commands are recorded and never settle (the app leaves the page)
    window.__TAURI__ = { core: { invoke: (cmd) => { window.__inv.push(cmd); return new Promise(() => {}); } } };
  });
};
const phoneRun = async (browser, w) => {
  const run = await fx.create(browser, w, { mobile: w < 600 });
  run.served503 = [];
  await run.context.route((u) => u.origin === fx.ORIGIN && u.pathname.startsWith('/v1/'), async (route) => {
    const req = route.request(); const u = new URL(req.url()); const v = u.pathname.slice(3);
    const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    if (!SERVED(v)) { run.served503.push(`${req.method()} ${v}`); return json({ error: 'Not available on this phone' }, 503); }
    if (v === '/info') return json(PHONE_INFO);
    if (v === '/me') return json({ role: 'owner', zones: [] });
    if (v === '/zones') return json([]);
    if (v === '/sources') return json(SOURCES);
    if (v === '/play-order') return json({ order: ['tidal', 'qobuz'], services: [{ id: 'tidal', name: 'TIDAL', connected: true }, { id: 'qobuz', name: 'Qobuz', connected: false }] });
    if (v === '/search') {
      const q = u.searchParams.get('q'); run.searched = (run.searched || []).concat(`${q}|${u.searchParams.get('in')}`);
      return route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': 'tidal,qobuz' },
        body: [{ source: 'tidal', status: 'ok', hits: [HIT], counts: [], ms: 3 }, { source: 'qobuz', status: 'ok', hits: [], counts: [], ms: 4 }].map((a) => JSON.stringify(a)).join('\n') + '\n' });
    }
    return json({ error: 'not in this fixture' }, 404);
  });
  await record(run);
  return run;
};
const hidden = (page, sel) => page.evaluate((s) => { const el = document.querySelector(s); return !el || getComputedStyle(el).display === 'none' || !!el.closest('[hidden]'); }, sel);
const shown = async (page, sel) => !(await hidden(page, sel));
const forbidden = (reqs) => reqs.filter((r) => FORBIDDEN.some((re) => re.test(r.split(' ')[1])));

(async () => {
  const browser = await chromium.launch();
  for (const w of [360, 1280]) {
    const at = `${w}px`;
    // ---- The phone's own backend ----
    const run = await phoneRun(browser, w); const p = run.page;
    await fx.go(p, '/');
    await p.waitForFunction(() => document.body.classList.contains('phone'), null, { timeout: 5000 }).catch(() => {});
    await sleep(600);
    check(await p.evaluate(() => document.body.classList.contains('phone')), `${at}: phone: body has the phone class`);
    for (const v of ['home', 'library', 'zones']) check(await hidden(p, `.nav-item[data-view="${v}"]`), `${at}: phone: the ${v} nav item is hidden`);
    for (const v of ['home', 'library', 'zones']) check(await hidden(p, `[data-bnav="${v}"]`), `${at}: phone: the ${v} tab is hidden`);
    check(await hidden(p, '#zonebtn'), `${at}: phone: no zone button`);
    check(await shown(p, '#search') && (await p.locator('#search').boundingBox()) !== null, `${at}: phone: the search box is there`);
    check(await p.evaluate(() => location.pathname === '/search' || location.pathname === '/'), `${at}: phone: lands on Search (${await p.evaluate(() => location.pathname)})`);
    await p.fill('#search', 'harbour'); await p.press('#search', 'Enter');
    const hit = await p.waitForSelector('#sresults [data-skind="song"] .srow', { timeout: 5000 }).then(() => true).catch(() => false);
    const text = hit ? await p.locator('#sresults').innerText() : '';
    check(hit && /Harbour Lights/.test(text), `${at}: phone: a search shows the TIDAL hit`);
    const scopes = await p.$$eval('#scopechips [data-scope-in]', (bs) => bs.map((b) => b.dataset.scopeIn));
    check(JSON.stringify(scopes) === JSON.stringify(['everywhere', 'tidal', 'qobuz']), `${at}: phone: the search scopes are Everywhere and the phone's sources (${scopes.join(', ')})`);
    check(!(await p.$('#stags')), `${at}: phone: no tags section in the results`);
    await p.screenshot({ path: path.join(SHOTS, `phone-search-${w}.png`) });
    // Settings, opened as its own page
    await fx.go(p, '/settings');
    await p.waitForFunction(() => /running on this phone/i.test((document.querySelector('#main') || {}).innerText || ''), null, { timeout: 5000 }).catch(() => {});
    const set = await p.locator('#main').innerText();
    const heads = await p.$$eval('#main .set-h', (hs) => hs.map((h) => h.textContent.trim()));
    check(heads[0] === 'This phone' && /running on this phone without a node/i.test(set), `${at}: phone: Settings shows the "This phone" group (${heads.join(', ')})`);
    check(/TIDAL/.test(set) && /Qobuz/.test(set), `${at}: phone: Settings lists the streaming accounts`);
    check(!/Favourites|Playlists/.test(set), `${at}: phone: Settings says Bookmarks and Collections, never Favourites or Playlists`);
    const leave = p.locator('#main button', { hasText: 'Connect to a node' });
    check((await leave.count()) === 1, `${at}: phone: Settings has "Connect to a node"`);
    await p.screenshot({ path: path.join(SHOTS, `phone-settings-${w}.png`) });
    if (await leave.count()) {
      await leave.click();
      const busyNow = await leave.evaluate((b) => b.disabled || b.getAttribute('aria-busy') === 'true');
      await leave.click({ force: true }).catch(() => {}); await sleep(200);
      const inv = await p.evaluate(() => window.__inv);
      check(busyNow, `${at}: phone: "Connect to a node" shows it is busy at once`);
      check(JSON.stringify(inv) === JSON.stringify(['phone_leave']), `${at}: phone: "Connect to a node" asks the app once (${JSON.stringify(inv)})`);
    }
    const bad = forbidden(run.reqs); const ws = await p.evaluate(() => window.__ws);
    check(!bad.length, `${at}: phone: nothing asked of zones, Home, tags, reviews, Pubky, playlists or favourites (${bad.join(', ')})`);
    check(!ws.length, `${at}: phone: no events socket (${ws.join(', ')})`);
    check(!run.served503.length, `${at}: phone: nothing asked that the phone doesn't serve (${[...new Set(run.served503)].join(', ')})`);
    check(!run.state.errors.length, `${at}: phone: no page errors (${run.state.errors.join(' | ')})`);
    await run.context.close();

    // ---- A node: nothing hidden ----
    const node = await fx.create(browser, w, { mobile: w < 600 }); await record(node);
    await fx.go(node.page, '/'); await sleep(900);
    const n = node.page;
    check(!(await n.evaluate(() => document.body.classList.contains('phone'))), `${at}: node: no phone class`);
    for (const v of ['home', 'library', 'zones']) check(await shown(n, `.nav-item[data-view="${v}"]`), `${at}: node: the ${v} nav item is offered`);
    if (w < 600) for (const v of ['home', 'search', 'library', 'zones']) check(await shown(n, `[data-bnav="${v}"]`), `${at}: node: the ${v} tab is offered`);
    if (w >= 600) check(await shown(n, '#zonebtn'), `${at}: node: the zone button is there`);
    check(node.reqs.some((r) => r === 'GET /zones'), `${at}: node: it reads its zones`);
    check((await n.evaluate(() => window.__ws)).some((u) => /\/v1\/events/.test(u)), `${at}: node: it opens its events socket`);
    await fx.go(n, '/settings'); await sleep(900);
    check(!(await n.locator('#main button', { hasText: 'Connect to a node' }).count()), `${at}: node: no "Connect to a node" in its Settings`);
    check(!node.state.errors.length, `${at}: node: no page errors (${node.state.errors.join(' | ')})`);
    await node.context.close();
  }
  await browser.close();
  if (failed) { console.error(`${failed} check(s) failed`); process.exit(1); }
  console.log('phone mode shows what the phone can do, and asks it for nothing else');
})();
