// Usage insights in the web app (usage.js, docs/guides/usage-insights.md), on the fixture origin
// (tools/ui-fixture.js: the app's own files, the node mocked, usage routes answered like usage.rs):
// - switch off: the page reads GET /v1/usage/me once and sends nothing else, shows no notice;
// - switch on: the notice shows once, out of the way of the top bar and the player; a real press
//   and a page opened are recorded, only closed-list values, with the sign-in in a header, never in
//   the address; a page's own el.click() and socket redraws record nothing;
// - nothing typed or named leaves the page (canaries in the search box and in the fixture's names),
//   and nothing of usage is kept in storage;
// - Stop for me drops what waits and nothing more is sent; Clear mine is its own button and doesn't
//   stop; a refusal (409 off, 410 gone) stops sending, a new alias only after the next own press;
//   401 ends it; leaving the page sends with keepalive and the header, never sendBeacon;
// - a flood is capped (queue and per-minute rate) and the drop is counted;
// - offline mode records nothing; a listener sees their own controls and never the node's switch.
//   node tools/ui-usage.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'usage-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const CANARY = 'CANARYzq81';
const canaryTrack = { ...fx.track, title: `${CANARY} song`, album: { id: 7, title: `${CANARY} album` } };

// The closed lists, from usage.rs itself: a value outside them would be refused by the node
const RS = fs.readFileSync(path.join(__dirname, '..', 'crates', 'pm-api', 'src', 'usage.rs'), 'utf8');
const snake = (s) => s.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();
const enumOf = (name) => {
  const body = RS.split(`pub enum ${name} {`)[1].split('\n}')[0];
  return new Set(body.split('\n').map((l) => l.trim()).filter((l) => /^[A-Z][A-Za-z0-9]*,?$/.test(l)).map((l) => snake(l.replace(',', ''))));
};
const E = { view: enumOf('View'), area: enumOf('Area'), control: enumOf('Control'), surface: enumOf('Surface'), via: enumOf('Via'), how: enumOf('How'), class: new Set([...enumOf('Class'), ...enumOf('ErrorClass')]), outcome: new Set([...enumOf('Outcome'), 'http_4xx', 'http_5xx']), ms: enumOf('Ms') };
const FIELDS = { nav: ['view', 'how'], load: ['view', 'outcome', 'ms'], api: ['area', 'class', 'outcome', 'ms'], action: ['control', 'view', 'surface', 'via', 'to'], exception: ['class'] };
const closed = (ev) => FIELDS[ev.kind] && Object.entries(ev).every(([k, v]) => k === 'kind' || (FIELDS[ev.kind].includes(k) && (k === 'to' ? E.view.has(v) : E[k] && E[k].has(v))));

// The node's usage routes, as usage.rs answers them (state in u)
const mockUsage = async (context, u) => {
  const me = () => ({ schema: 'pm-usage/1', saved: true, node_on: u.on, generation: u.generation, sharing: u.sharing, choice: u.choice, notice_seen: u.noticeSeen, notice_version: 1, role: u.role, limits: { raw_retention_days: 14, aggregate_retention_days: 90 } });
  const j = (r, body, status = 200) => r.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  await context.route(/\/v1\/(usage|node\/usage)(\/|$|\?)/, async (r) => {
    const req = r.request(); const url = new URL(req.url()); const p = url.pathname.slice(3); const m = req.method();
    const body = req.postData() || '';
    u.log.push({ p, m, url: req.url(), body, auth: req.headers().authorization || '' });
    if (u.force && u.force[p]) { const f = u.force[p]; if (!f.once || !f.used) { f.used = true; return j(r, { error: f.error }, f.status); } }
    if (p === '/usage/me' && m === 'GET') return j(r, me());
    if (p === '/usage/me' && m === 'PUT' && u.delayPut) await new Promise((res) => setTimeout(res, u.delayPut));
    if (p === '/usage/me' && m === 'PUT') { const b = JSON.parse(body); if (b.share && b.choice !== u.choice) return j(r, { error: 'usage insights changed since; look again' }, 409); u.sharing = !!b.share; u.choice += 1; return j(r, me()); }
    if (p === '/usage/me/notice') { u.noticeSeen = 1; return j(r, me()); }
    if (p === '/usage/me/clear') { u.cleared += 1; return j(r, { cleared: u.events.length }); }
    if (p === '/usage/sessions') { if (!u.on) return j(r, { error: 'usage insights are off on this node' }, 409); u.aliases += 1; return j(r, { alias: `a${String(u.aliases).padStart(15, '0')}`, idle_s: 1800, expires_in_s: 43200, max_batch: 50 }); }
    if (p === '/usage/sessions/close') return r.fulfill({ status: 204, body: '' });
    if (p === '/usage/events') { const b = JSON.parse(body); u.batches.push(b); u.events.push(...b.events); return r.fulfill({ status: 204, body: '' }); }
    if (p === '/node/usage' && m === 'GET') return j(r, { enabled: u.on, saved: true, cleanup_pending: false, generation: u.generation, aliases: u.aliases, waiting: 0, events: u.events.length, bytes: 0, coverage: { owner: 'instrumented', listeners: 'instrumented', guests: 'not instrumented' }, limits: { raw_retention_days: 14, aggregate_retention_days: 90 } });
    if (p === '/node/usage' && m === 'PUT') { const b = JSON.parse(body); u.on = !!b.enabled; u.generation += 1; return j(r, { enabled: u.on, saved: true, generation: u.generation, coverage: { guests: 'not instrumented' } }); }
    if (p === '/node/usage/clear') return r.fulfill({ status: 204, body: '' });
    return j(r, { error: 'not found' }, 404);
  });
};
const newU = (o = {}) => ({ on: true, sharing: true, choice: 1, generation: 1, noticeSeen: 0, role: 'owner', aliases: 0, cleared: 0, log: [], batches: [], events: [], ...o });
// A spy on fetch and sendBeacon, set before the page's scripts (what leaving the page sends)
const SPY = () => {
  window.__fetches = []; const f = window.fetch.bind(window);
  window.fetch = (input, init = {}) => { try { const h = init.headers || {}; window.__fetches.push({ url: String(input), keepalive: !!init.keepalive, auth: !!(h.authorization || h.Authorization) }); } catch { /* */ } return f(input, init); };
  window.__beacons = 0; if (navigator.sendBeacon) navigator.sendBeacon = () => { window.__beacons += 1; return true; };
};
const open = async (browser, width, u, opts = {}) => {
  const run = await fx.create(browser, width, { touch: width < 600, role: u.role, token: u.role === 'listener' ? 'pml_fixture-listener' : 'fixture-owner', fixtures: { '/library/tracks': { body: { tracks: [canaryTrack], next: null } }, '/library/tracks/1': { body: canaryTrack } }, ...opts });
  await run.page.setViewportSize({ width, height: width < 600 ? 844 : 900 });
  await run.context.addInitScript(SPY);
  await mockUsage(run.context, u);
  // The node's events socket, kept to send a library change later
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { run.ws = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
  return run;
};
// The page goes to the background: what waits is sent now (usage.js flushes on hidden)
const hide = (page) => page.evaluate(() => { Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true }); document.dispatchEvent(new Event('visibilitychange')); Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true }); });
const usageReq = (u, p) => u.log.filter((x) => x.p === p);
const until = async (fn, ms = 4000) => { const t = Date.now(); while (Date.now() - t < ms) { if (await fn()) return true; await sleep(100); } return false; };
const noOverlap = (page) => page.evaluate(() => {
  const n = document.querySelector('.usage-notice'); if (!n) return 'no notice';
  const a = n.getBoundingClientRect();
  for (const sel of ['.player', '#me', '#help', '.topbar', '.bnav', '#sidebar']) {
    for (const el of document.querySelectorAll(sel)) {
      if (!el.getClientRects().length) continue;
      const b = el.getBoundingClientRect();
      if (a.left < b.right - 1 && b.left < a.right - 1 && a.top < b.bottom - 1 && b.top < a.bottom - 1) return `overlaps ${sel}`;
    }
  }
  return a.width > 0 && a.height > 0 && a.right <= document.documentElement.clientWidth + 1 ? '' : 'not fully shown';
});

(async () => {
  const browser = await chromium.launch();
  try {
    // ---- off: one read, nothing else ----
    {
      const u = newU({ on: false });
      const { page, context } = await open(browser, 1280, u);
      await fx.go(page, '/library'); await page.waitForSelector('.nav-item[data-view]'); await sleep(500);
      await page.click('.nav-item[data-view="albums"]'); await sleep(300); await hide(page); await sleep(400);
      check(usageReq(u, '/usage/me').length === 1 && u.log.length === 1, `off: GET /v1/usage/me only (${u.log.map((x) => x.m + ' ' + x.p).join(', ')})`);
      check(!(await page.$('.usage-notice')), 'off: no notice');
      await context.close();
    }
    // ---- on, owner, every viewport: the notice, recording, closed values, canaries ----
    for (const width of [390, 768, 1280, 1440]) {
      for (const scheme of width === 1280 ? ['light', 'dark'] : ['light']) {
        const u = newU();
        const run = await open(browser, width, u); const { page, context, state } = run;
        await page.emulateMedia({ colorScheme: scheme });
        await fx.go(page, '/library');
        const shown = await page.waitForSelector('.usage-notice', { timeout: 5000 }).then(() => true, () => false);
        check(shown, `${width}/${scheme}: the notice shows once usage insights are on`);
        if (!shown) { await context.close(); continue; }
        const txt = await page.$eval('.usage-notice', (n) => n.innerText);
        check(/never what you type, search, play or name/.test(txt) && /What’s collected/.test(txt) && /Turn off for me/.test(txt) && /Party guests are not covered/.test(txt), `${width}: the notice says what is and isn't recorded, with What's collected and Turn off for me`);
        const ov = await noOverlap(page); check(ov === '', `${width}/${scheme}: the notice covers nothing (${ov || 'clear'})`);
        await page.screenshot({ path: path.join(SHOTS, `notice-${width}-${scheme}.png`) });
        if (width !== 1280 || scheme !== 'light') { await context.close(); continue; }
        // Real presses, a typed canary, a page's own click, then the page goes to the background
        await page.click('[data-usage="dismiss"]');
        check(await until(() => usageReq(u, '/usage/me/notice').length === 1), 'OK marks the notice seen on the node');
        check(!(await page.$('.usage-notice')), 'and the notice goes');
        await page.click('#search'); await page.keyboard.type(`${CANARY} typed`); await page.keyboard.press('Escape');
        await page.click('.nav-item[data-view="albums"]'); await sleep(400);
        await page.click('.nav-item[data-view="tracks"]').catch(() => page.click('.nav-item[data-view="library"]')); await sleep(400);
        await hide(page);
        check(await until(() => u.events.length > 0), `on: events reach the node (${u.events.length})`);
        const actions = u.events.filter((e) => e.kind === 'action').length;
        // Nobody there: a page's own el.click() and a library change (the page redraws) are not use
        await sleep(10500); await hide(page); await sleep(500);
        const before = u.events.length;
        await page.evaluate(() => { document.querySelector('.nav-item[data-view="albums"]').click(); }); await sleep(600);
        run.ws.send(JSON.stringify({ type: 'library_changed', seq: 1 })); await sleep(800);
        await hide(page); await sleep(500);
        check(actions > 0 && u.events.length === before, `a page's own el.click() and a socket redraw record nothing, no page, load or request; real presses did (${actions} presses; ${before} → ${u.events.length}: ${u.events.slice(before).map((e) => e.kind).join(',')})`);
        const kinds = new Set(u.events.map((e) => e.kind));
        check(kinds.has('nav') && kinds.has('action') && kinds.has('api'), `pages, presses and requests are recorded (${[...kinds].join(', ')})`);
        const navs0 = u.events.filter((e) => e.kind === 'nav');
        check(navs0.filter((e) => e.how === 'initial').length === 1 && navs0[0].how === 'initial' && navs0.some((e) => e.how === 'push' && e.view === 'albums'), `the page a load starts on is "initial", a page opened is "push" (${navs0.map((e) => `${e.view}:${e.how}`).join(' ')})`);
        const bad = u.events.filter((e) => !closed(e));
        check(bad.length === 0, `every event is closed-list values only (${JSON.stringify(bad.slice(0, 2))})`);
        const sessions = usageReq(u, '/usage/sessions');
        check(sessions.length === 1 && JSON.parse(sessions[0].body).viewport === 'desktop', `one alias, viewport desktop (${sessions.map((s) => s.body).join(' ')})`);
        const usageLog = u.log.map((x) => x.url + ' ' + x.body).join('\n');
        check(!usageLog.includes(CANARY) && !usageLog.includes('typed'), 'nothing typed or named (canaries) is in any usage request');
        check(!/fixture-owner|token=/.test(usageLog), 'the sign-in is never in a usage address or body');
        check(u.log.every((x) => x.auth === 'Bearer fixture-owner'), 'every usage request has the sign-in in its header');
        check(!/"(path|url|text|value|label|id|name|query|message)"/.test(u.batches.map((b) => JSON.stringify(b)).join('')), 'no path, text, value, label, id or message field is sent');
        const stored = await page.evaluate(async () => {
          const all = [...Object.entries(localStorage), ...Object.entries(sessionStorage)].map(([k, v]) => `${k}=${v}`).join('\n');
          const cached = self.caches ? (await Promise.all((await caches.keys()).map(async (n) => (await (await caches.open(n)).keys()).map((r) => r.url)))).flat() : [];
          const idb = indexedDB.databases ? (await indexedDB.databases()).map((d) => d.name) : [];
          return { all, cached, idb };
        });
        check(!/usage|"kind"|a0000/.test(stored.all) && !stored.cached.some((x) => /\/v1\/usage/.test(x)) && !stored.idb.some((n) => /usage/i.test(n || '')), `nothing of usage is kept in storage (${stored.idb.join(',')})`);
        // Socket redraws: the library changes, the page redraws, nothing new is recorded
        // (only the person's own pages and presses are; checked by the count of nav and action)
        // The account menu says it is on
        await page.click('#me'); await sleep(200);
        check(await page.locator('.popover.menu >> text=Usage insights on').count() > 0, 'the account menu says "Usage insights on"');
        await page.keyboard.press('Escape');
        // Leaving the page: keepalive with the header, never sendBeacon
        await page.click('.nav-item[data-view="albums"]'); await sleep(200);
        await page.evaluate(() => { window.__fetches = []; window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: false })); });
        await sleep(400);
        const left = await page.evaluate(() => ({ f: window.__fetches.filter((x) => /\/v1\/usage\/events/.test(x.url)), b: window.__beacons }));
        check(left.f.length === 1 && left.f[0].keepalive && left.f[0].auth && !/token=/.test(left.f[0].url) && left.b === 0, `leaving sends once with keepalive and the header, no sendBeacon (${JSON.stringify(left)})`);
        // Settings › You › Privacy: Stop and Clear are separate; Stop drops what waits, nothing after
        await fx.go(page, '/settings/privacy'); await page.waitForSelector('[data-usage="stop"]');
        check(await page.locator('[data-usage="clear"]').count() === 1, 'Privacy has Stop for me and Clear mine, two buttons');
        await page.screenshot({ path: path.join(SHOTS, 'privacy-1280.png') });
        await page.click('[data-usage="clear"]');
        const dlg = page.locator('dialog[open] button', { hasText: 'Clear mine' });
        await dlg.first().click().catch(() => {});
        check(await until(() => u.cleared === 1), 'Clear mine asks, then clears');
        check(u.sharing === true && usageReq(u, '/usage/me').filter((x) => x.m === 'PUT').length === 0, 'Clear mine does not stop');
        await page.click('.nav-item[data-view="albums"]'); await sleep(200); // waits in the queue
        await fx.go(page, '/settings/privacy').catch(() => {}); await page.waitForSelector('[data-usage="stop"]');
        const sent = u.events.length; const batches = u.batches.length;
        // The node is slow to answer the Stop: the page holds it at once all the same
        u.delayPut = 1500;
        await page.click('[data-usage="stop"]');
        for (const v of ['albums', 'artists']) { await page.click(`.nav-item[data-view="${v}"]`); await sleep(150); }
        await hide(page);
        check(await until(() => u.sharing === false), 'Stop for me tells the node');
        u.delayPut = 0;
        await fx.go(page, '/settings/privacy'); await page.waitForSelector('[data-usage="share"]');
        for (const v of ['albums', 'artists']) { await page.click(`.nav-item[data-view="${v}"]`); await sleep(200); }
        await hide(page); await sleep(500);
        check(u.batches.length === batches && u.events.length === sent, `after Stop nothing more is sent, also before the node answered (${batches} → ${u.batches.length} batches)`);
        check(state.errors.length === 0, `no page errors (${state.errors.join(' | ')})`);
        await context.close();
      }
    }
    // ---- refusals: 409 off stops; 410 gone reopens only after an own press; 401 ends ----
    {
      const u = newU({ noticeSeen: 1 });
      const { page, context } = await open(browser, 1280, u);
      await fx.go(page, '/library'); await page.waitForSelector('.nav-item[data-view]'); await sleep(400);
      u.force = { '/usage/events': { status: 410, error: 'that alias has ended', once: true } };
      await page.click('.nav-item[data-view="albums"]'); await sleep(200); await hide(page);
      check(await until(() => u.force['/usage/events'].used), '410: the batch is refused');
      await sleep(400);
      const aliases = u.aliases;
      await page.evaluate(() => window.PM_USAGE.nav('artists', 'push')); await hide(page); await sleep(500);
      check(u.aliases === aliases, `after 410 a page's own navigation opens no new alias (${u.aliases})`);
      await page.click('.nav-item[data-view="artists"]'); await sleep(200); await hide(page);
      check(await until(() => u.aliases === aliases + 1 && u.events.length > 0), `the next own press opens a new alias and sends (${u.aliases}, ${u.events.length})`);
      // 409 off: nothing more
      u.on = false; u.force = { '/usage/events': { status: 409, error: 'usage insights are off on this node' } };
      await page.click('.nav-item[data-view="albums"]'); await sleep(200); await hide(page); await sleep(600);
      const n = u.log.length;
      for (const v of ['artists', 'albums']) { await page.click(`.nav-item[data-view="${v}"]`); await sleep(200); }
      await hide(page); await sleep(600);
      check(u.log.filter((x, i) => i >= n && x.p !== '/usage/me').length === 0, `409 off: nothing more is sent (${u.log.slice(n).map((x) => x.p).join(',')})`);
      await context.close();
    }
    {
      const u = newU({ noticeSeen: 1 });
      const { page, context } = await open(browser, 1280, u);
      await fx.go(page, '/library'); await page.waitForSelector('.nav-item[data-view]'); await sleep(400);
      u.force = { '/usage/events': { status: 401, error: 'sign in' } };
      await page.click('.nav-item[data-view="albums"]'); await sleep(200); await hide(page); await sleep(600);
      const n = u.log.length;
      for (const v of ['artists', 'albums']) { await page.click(`.nav-item[data-view="${v}"]`); await sleep(200); }
      await hide(page); await sleep(600);
      check(u.log.length === n, `401: the recorder ends, nothing more (${u.log.slice(n).map((x) => x.p).join(',')})`);
      await context.close();
    }
    // ---- a flood: the queue and the per-minute rate cap it, and the drop is counted ----
    {
      const u = newU({ noticeSeen: 1 });
      const { page, context } = await open(browser, 1280, u);
      await fx.go(page, '/library'); await page.waitForSelector('.nav-item[data-view]'); await sleep(400);
      await page.click('.nav-item[data-view="albums"]'); await sleep(200);
      await page.evaluate(() => { for (let i = 0; i < 400; i++) window.PM_USAGE.nav(i % 2 ? 'albums' : 'artists', 'push'); });
      for (let i = 0; i < 4; i++) { await hide(page); await sleep(400); }
      const dropped = u.batches.reduce((s, b) => s + (b.dropped || 0), 0);
      check(u.events.length <= 110 && u.events.length > 50, `at most 110 events a minute are sent (${u.events.length})`);
      check(dropped > 0, `the dropped ones are counted (${dropped})`);
      check(u.batches.every((b) => b.events.length <= 50 && JSON.stringify(b).length <= 16 * 1024), 'every batch is within max_batch and 16 KiB');
      await context.close();
    }
    // ---- offline mode and a listener ----
    {
      const u = newU();
      const { page, context } = await open(browser, 390, u, { offline: true });
      await fx.go(page, '/downloads'); await sleep(1200);
      check(u.log.length === 0 && !(await page.$('.usage-notice')), `offline: no usage request, no notice (${u.log.length})`);
      await context.close();
    }
    {
      const u = newU({ role: 'listener', noticeSeen: 1 });
      const { page, context } = await open(browser, 390, u);
      await fx.go(page, '/settings'); await page.waitForSelector('#usageme [data-usage="stop"]', { timeout: 6000 }).catch(() => {});
      check(await page.locator('#usageme [data-usage="stop"]').count() === 1 && await page.locator('#usageme [data-usage="clear"]').count() === 1, 'a listener has Stop for me and Clear mine in Settings');
      check(usageReq(u, '/node/usage').length === 0 && !(await page.$('#usagenode')), 'a listener never sees or reads the node switch');
      await page.focus('#usageme [data-usage="stop"]'); await page.keyboard.press('Enter');
      await page.waitForSelector('#usageme [data-usage="share"]');
      check(await page.evaluate(() => document.activeElement && document.activeElement.matches('[data-usage="share"]')), 'after Stop for me the focus is on Share again (not lost)');
      check(/Stopped for you/.test(await page.$eval('#usageme .usage-msg', (m) => m.textContent)), 'and the status line says it');
      await page.screenshot({ path: path.join(SHOTS, 'listener-settings-390.png'), fullPage: true });
      await context.close();
    }
    // ---- the owner's switch: off as shipped, on asks first ----
    {
      const u = newU({ on: false });
      const { page, context } = await open(browser, 1280, u);
      await fx.go(page, '/settings/usage'); await page.waitForSelector('[data-usage-node="on"]');
      await page.click('[data-usage-node="on"]');
      await page.locator('dialog[open] button', { hasText: 'Turn on' }).first().click();
      check(await until(() => u.on === true), 'Turn on asks, then turns the node switch on');
      await page.waitForSelector('[data-usage-node="off"]');
      await page.screenshot({ path: path.join(SHOTS, 'node-usage-1280.png') });
      check(await page.locator('[data-usage-node="clear"]').count() === 1, 'Clear all is its own button');
      await page.focus('[data-usage-node="clear"]'); await page.keyboard.press('Enter');
      await page.waitForSelector('dialog[open]'); await page.keyboard.press('Escape');
      await until(() => page.evaluate(() => !document.querySelector('dialog[open]') && !!document.querySelector('[data-usage-node="clear"]:not([disabled])')));
      check(await page.evaluate(() => document.activeElement && document.activeElement.matches('[data-usage-node="clear"]')) && u.on === true, 'a cancelled Clear all leaves the focus on Clear all (never on Turn off)');
      await context.close();
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} failed` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
