// Synthetic browser check of opt-in local diagnostics (docs/guides/local-diagnostics.md): the real
// web app (crates/pm-api/static) served from a made-up origin, with every /v1 request and the event
// socket answered here by fixtures. No node is started and nothing leaves this machine. Every check
// is an assertion: the script exits non-zero if any fails.
//   npm install playwright@1.49.1 && npx playwright install chromium   (anywhere; run from there)
//   node /path/to/tools/ui-diagnostics.js [<repository dir>] [<screenshots dir>]
// The event schema (views, areas, outcomes…) is read from crates/pm-api/src/diagnostics.rs, so the
// page and the node can't drift apart unnoticed. Takes about three minutes (the page sends every 10 s).
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('fs');
const os = require('os');
const path = require('path');

const REPO = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(os.tmpdir(), 'ui-diagnostics-shots'));
const STATIC = path.join(REPO, 'crates/pm-api/static');
const ORIGIN = 'http://diagnostics.test';
const TOKEN = 'synthetic-owner-token-SECRETCANARY-1';
const CANARY = 'CANARY';
// A person's key has to look like one (52 lowercase letters and digits)
const PERSON = ('canaryperson' + 'x'.repeat(52)).slice(0, 52);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let failed = 0;
const check = (ok, msg, detail) => {
  console.log((ok ? 'ok     ' : 'FAILED ') + msg + (!ok && detail !== undefined ? ` (${typeof detail === 'string' ? detail : JSON.stringify(detail)})` : ''));
  if (!ok) failed += 1;
  return ok;
};

// ---- the schema, from the node's own source ----
const rs = fs.readFileSync(path.join(REPO, 'crates/pm-api/src/diagnostics.rs'), 'utf8');
const variants = (name) => {
  const m = rs.match(new RegExp(`pub enum ${name} \\{([\\s\\S]*?)\\n\\}`));
  if (!m) throw new Error(`no enum ${name} in diagnostics.rs`);
  const out = []; let rename = null;
  for (const raw of m[1].split('\n')) {
    const line = raw.trim();
    const r = line.match(/rename = "([^"]+)"/); if (r) { rename = r[1]; continue; }
    const v = line.match(/^([A-Z][A-Za-z0-9]*)\b/); if (!v) continue;
    out.push(rename || v[1].replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase()); rename = null;
  }
  return new Set(out);
};
const ENUMS = { view: variants('View'), area: variants('Area'), outcome: variants('Outcome'), ms: variants('Ms'), class: variants('ErrorClass') };
const SHAPES = { nav: ['view'], load: ['view', 'outcome', 'ms'], api: ['area', 'outcome', 'ms'], exception: ['class'] };
const schemaError = (ev) => {
  if (!ev || typeof ev !== 'object' || !SHAPES[ev.kind]) return 'unknown kind';
  const keys = Object.keys(ev).filter((k) => k !== 'kind').sort();
  if (JSON.stringify(keys) !== JSON.stringify([...SHAPES[ev.kind]].sort())) return `fields ${keys}`;
  for (const k of keys) if (!ENUMS[k].has(ev[k])) return `${k}=${ev[k]}`;
  return null;
};

// ---- the fake node ----
const LIMITS = { max_events: 50000, max_bytes: 52428800, retention_hours: 72, max_sessions: 32, per_session_per_min: 120, global_per_min: 2000, max_batch: 32, max_body_bytes: 16384, session_minutes: 60, node_switch_hours: 24, cleanup_seconds: 60 };
const fresh = (role = 'owner') => ({
  role, token: TOKEN, enabled: false, generation: 0, sessions: new Map(), events: [], dropped: 0,
  expiresIn: 3600, startDelay: 0, acceptStaleStart: false, stallEvents: false,
  diagRequests: [], schemaErrors: [], deletes: [], inflight: 0, maxInflight: 0, eventPosts: 0,
  fixtures: {}, apiHits: [], held: [], posts: [], rateLimitNext: 0,
});
let node = fresh();
const status = () => ({ enabled: node.enabled, generation: node.generation, expires_in_s: node.enabled ? 86400 : null, events: node.events.length, bytes: node.events.length * 80, sessions: node.sessions.size, dropped: { evicted: 0, rate_limited: 0, by_browsers: node.dropped }, limits: LIMITS });
const exportOf = () => ({ manifest: { schema: 'pm-diagnostics/1', app_version: '0.0.0-fixture', exported_at: 1, limits: LIMITS, dropped: { evicted: 0, rate_limited: 0, by_browsers: node.dropped } }, sessions: [], aggregates: {}, events: node.events.map((e, i) => ({ at: i, session: 's1', ...e.ev })) });
const strictKeys = (o, allowed, required) => o && typeof o === 'object' && !Array.isArray(o) && Object.keys(o).every((k) => allowed.includes(k)) && required.every((k) => k in o);

const reply = (route, code, body) => route.fulfill({ status: code, contentType: 'application/json', body: body === undefined ? '' : JSON.stringify(body) }).catch(() => {});
const later = (ms, f) => (ms ? sleep(ms).then(f) : f());

async function diagRoute(route, req, rest) {
  const m = req.method(); const auth = req.headers().authorization || '';
  const body = req.postData() || '';
  node.diagRequests.push({ m, rest, url: req.url(), body, auth });
  if (auth !== 'Bearer ' + node.token || node.role !== 'owner') return reply(route, 401, { error: 'missing or wrong bearer token' });
  let b = null; try { b = body ? JSON.parse(body) : null; } catch { return reply(route, 400, { error: 'that is not in the diagnostics schema' }); }
  if (m === 'GET' && rest === '') return reply(route, 200, status());
  if (m === 'PUT' && rest === '') {
    if (!strictKeys(b, ['enabled', 'generation'], ['enabled'])) return reply(route, 400, { error: 'schema' });
    if (!b.enabled) { node.generation += 1; node.enabled = false; node.sessions.clear(); return reply(route, 200, status()); }
    if (b.generation !== node.generation) return reply(route, 409, { error: 'diagnostics were turned off or on since this page showed them; look again' });
    if (!node.enabled) { node.enabled = true; node.generation += 1; }
    return reply(route, 200, status());
  }
  if (m === 'POST' && rest === '/sessions') {
    if (!strictKeys(b, ['viewport', 'generation'], ['viewport', 'generation']) || !['phone', 'tablet', 'desktop'].includes(b.viewport)) return reply(route, 400, { error: 'schema' });
    const shown = b.generation;
    return later(node.startDelay, () => {
      if (!node.enabled) return reply(route, 409, { error: 'diagnostics are off on this node' });
      if (shown !== node.generation && !node.acceptStaleStart) return reply(route, 409, { error: 'stale' });
      const id = [...Array(16)].map(() => '0123456789abcdef'[Math.floor(Math.random() * 16)]).join('');
      node.sessions.set(id, { generation: node.generation, viewport: b.viewport });
      return reply(route, 200, { session: id, expires_in_s: node.expiresIn, max_batch: 32 });
    });
  }
  if (m === 'DELETE' && rest.startsWith('/sessions/')) { const id = rest.split('/')[2]; node.deletes.push(id); node.sessions.delete(id); return reply(route, 204); }
  if (m === 'POST' && rest === '/events') {
    node.eventPosts += 1;
    if (!strictKeys(b, ['session', 'events', 'dropped'], ['session', 'events']) || !Array.isArray(b.events) || b.events.length > 32 || (b.dropped !== undefined && !(Number.isInteger(b.dropped) && b.dropped >= 0 && b.dropped <= 65535))) { node.schemaErrors.push({ batch: b }); return reply(route, 400, { error: 'schema' }); }
    for (const ev of b.events) { const e = schemaError(ev); if (e) node.schemaErrors.push({ ev, e }); }
    if (!b.events.length) { node.schemaErrors.push({ empty: b }); return reply(route, 400, { error: 'a batch needs events' }); }
    if (node.stallEvents) return; // never answered: the page has to give up by itself
    if (node.rateLimitNext > 0) { node.rateLimitNext -= 1; node.posts.push({ status: 429, body: b }); node.rateLimited = (node.rateLimited || 0) + b.events.length; return reply(route, 429, { error: 'too many diagnostics events; wait a minute' }); }
    node.posts.push({ status: 204, body: b });
    if (!node.enabled) return reply(route, 409, { error: 'diagnostics are off on this node' });
    const s = node.sessions.get(b.session); if (!s || s.generation !== node.generation) return reply(route, 410, { error: 'this diagnostics session has ended' });
    for (const ev of b.events) node.events.push({ session: b.session, ev });
    node.dropped += b.dropped || 0;
    return reply(route, 204);
  }
  if (m === 'POST' && rest === '/clear') { node.events = []; node.dropped = 0; return reply(route, 204); }
  if (m === 'GET' && rest === '/export') return route.fulfill({ status: 200, contentType: 'application/json', headers: { 'content-disposition': 'attachment; filename="odeion-diagnostics.json"' }, body: JSON.stringify(exportOf()) });
  return reply(route, 404, { error: 'not here' });
}

const types = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml', '.webmanifest': 'application/manifest+json', '.png': 'image/png' };
async function handle(route) {
  const req = route.request(); const u = new URL(req.url());
  if (u.origin !== ORIGIN) return route.abort(); // nothing leaves this machine
  const p = u.pathname;
  if (!p.startsWith('/v1/')) {
    const f = path.join(STATIC, p === '/' || !path.extname(p) ? 'index.html' : p.slice(1));
    if (!f.startsWith(STATIC) || !fs.existsSync(f)) return route.fulfill({ status: 404, body: '' });
    return route.fulfill({ status: 200, contentType: types[path.extname(f)] || 'application/octet-stream', body: fs.readFileSync(f) });
  }
  const v = p.slice(3);
  if (v.startsWith('/node/diagnostics')) return diagRoute(route, req, v.slice('/node/diagnostics'.length));
  node.apiHits.push({ m: req.method(), v, q: u.search });
  const fx = node.fixtures[v];
  if (fx) {
    if (fx.abort) return route.abort('connectionrefused');
    if (fx.hold) { node.held.push(() => reply(route, 200, fx.body)); return; } // answered when the test says
    return later(fx.delay || 0, () => reply(route, fx.status || 200, typeof fx.body === 'function' ? fx.body() : fx.body));
  }
  if (v === '/me') return reply(route, 200, node.role === 'owner' ? { role: 'owner', key: null, zones: null } : { role: 'listener', key: 'listenerkey', zones: null });
  if (v === '/info') return reply(route, 200, { name: 'Synthetic node', version: '0.0.0-fixture', library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0 } });
  if (v === '/outputs' || v === '/zones' || v === '/sources' || v === '/playlists' || v === '/presence') return reply(route, 200, []);
  if (v === '/favourites') return reply(route, 200, { uris: [] });
  if (v === '/node/settings') return reply(route, 200, { contact: '', discovery: false, token_replaceable: true });
  if (v === '/me/token' && req.method() === 'DELETE') return reply(route, 200, { signed_out: true, ended: false });
  if (v === '/node/token' && req.method() === 'POST') { node.token = TOKEN.replace('-1', '-2'); node.sessions.clear(); return reply(route, 200, { token: node.token }); }
  if (v === '/library/albums') return reply(route, 200, []);
  if (v === '/library/tracks') return reply(route, 200, { tracks: [], next: null });
  if (v === '/library/genres') return reply(route, 200, []);
  return reply(route, 404, { error: `synthetic: nothing at ${CANARY}-free path` });
}

// The event socket: a snapshot, pongs, and whatever a test pushes (another person's activity)
const sockets = [];
let seq = 0;
async function newContext(browser, viewport, role = 'owner', extra = {}) {
  const ctx = await browser.newContext({ viewport, ...extra });
  await ctx.addInitScript(([t, owner]) => { if (!sessionStorage.getItem('seeded')) { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); if (!owner) localStorage.setItem('pm.role', 'listener'); sessionStorage.setItem('seeded', '1'); } }, [role === 'owner' ? TOKEN : 'pml_synthetic-listener', role === 'owner']);
  await ctx.route('**/*', handle);
  await ctx.routeWebSocket(/\/v1\/events/, (ws) => {
    sockets.push(ws);
    ws.onMessage((m) => { let j = null; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); });
    ws.send(JSON.stringify({ type: 'snapshot', seq: (seq = 0), boot: 'b1', library_rev: 0, zones: [] }));
  });
  return ctx;
}
const push = (ev) => { for (const ws of sockets) { try { ws.send(JSON.stringify({ ...ev, seq: ++seq })); } catch { /* closed */ } } };

// In-app navigation, as Back/Forward does (setView): the page is not reloaded
const go = (page, p) => page.evaluate((to) => { history.pushState(null, '', to); dispatchEvent(new PopStateEvent('popstate', { state: null })); }, p);
const eventsOf = (sid) => node.events.filter((e) => !sid || e.session === sid).map((e) => e.ev);
const liveSid = () => [...node.sessions.keys()].pop();
async function until(what, f, ms = 30000) { const t = Date.now(); while (Date.now() - t < ms) { if (await f()) return true; await sleep(100); } return check(false, `timed out: ${what}`); }
// A recorded navigation to Songs, then its batch: whatever was queued before it has been sent too
async function sentinel(page, sid) {
  const n = eventsOf(sid).length;
  await go(page, '/tracks');
  await until('the Songs navigation arrives', () => eventsOf(sid).slice(n).some((e) => e.kind === 'load' && e.view === 'tracks'), 30000);
  return eventsOf(sid).slice(n);
}
async function openSettings(page) {
  await go(page, '/settings/about');
  await page.waitForSelector('#diaggate', { timeout: 15000 });
  await sleep(400); // Settings draws the box twice (a placeholder, then the status)
  await page.locator('#diagbox').scrollIntoViewIfNeeded().catch(() => {});
}
async function startHere(page) {
  await openSettings(page);
  if (!(await page.locator('#diaggate').isChecked())) { await page.locator('#diaggate').check(); await page.waitForSelector('#diagstart:not([disabled])'); }
  const before = node.sessions.size;
  await page.locator('#diagstart').click();
  await page.waitForSelector('#diagbadge', { timeout: 15000 });
  await until('a session at the node', () => node.sessions.size > before);
  return liveSid();
}
const overlap = (a, b) => a && b && a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
const shot = async (page, name) => { fs.mkdirSync(SHOTS, { recursive: true }); await page.screenshot({ path: path.join(SHOTS, name) }); };
const allDiagText = () => node.diagRequests.map((r) => r.url + '\n' + r.body).join('\n');

(async () => {
  const browser = await chromium.launch();
  try {
    // ---- 1. Off by default: nothing is recorded, and the notice says what Start would do ----
    let ctx = await newContext(browser, { width: 1280, height: 860 });
    let page = await ctx.newPage();
    const pageErrors = [];
    page.on('pageerror', (e) => pageErrors.push(e.name));
    await page.goto(ORIGIN + '/settings/about');
    await page.waitForSelector('#diaggate', { timeout: 20000 });
    await page.locator('#diagbox').scrollIntoViewIfNeeded();
    check(!(await page.locator('#diaggate').isChecked()), 'the node switch starts off');
    check(await page.locator('#diagstart').isDisabled(), 'Start is disabled while the node switch is off');
    const notice = (await page.locator('#diagbox').innerText()).replace(/\s+/g, ' ');
    for (const [what, re] of [['purpose', /find slow pages, failed requests/], ['what, not why', /show what happened, not why/], ['node-local, no automatic sending', /stays on this node: nothing is sent anywhere unless you download/], ['viewport', /phone, tablet or desktop/], ['app version', /app version/], ['session limit', /at most 60 minutes/], ['retention and its cleanup', /for 72 hours, removed within 1 minute after that, and all gone after a restart/], ['event/byte caps', /at most 50,000 events or 50 MiB/], ['listeners excluded', /listeners and guests are not recorded/], ['manual download', /Download saves them as a file for you to check and share by hand/], ['Clear vs Off', /Clear deletes them now; turning diagnostics off only stops new ones/], ['what is never taken', /Never anything you type or search, titles, names, addresses, keys/]]) check(re.test(notice), `the notice before Start states: ${what}`, notice.slice(0, 200));
    check(!/all activity|everything you do|complete record of/i.test(notice), 'the notice makes no blanket tracking promise');
    await shot(page, '01-desktop-settings-off.png');
    await go(page, '/tracks'); await sleep(500); await go(page, '/settings/about'); await sleep(500);
    check(node.diagRequests.every((r) => r.m === 'GET' && r.rest === ''), 'while off the page only reads the status', node.diagRequests.map((r) => r.m + r.rest));
    check(!(await page.locator('#diagbadge').count()), 'no recording bar while off');

    // ---- 2. On, Start: the bar, navigation, slow/failing/broken pages, sensitive arguments ----
    await page.waitForSelector('#diaggate');
    await page.locator('#diaggate').check();
    await page.waitForSelector('#diagstart:not([disabled])');
    const put = node.diagRequests.filter((r) => r.m === 'PUT').pop();
    check(put && JSON.parse(put.body).generation === 0, 'On names the generation it was shown', put && put.body);
    check(!(await page.locator('#diagbadge').count()) && node.sessions.size === 0, 'turning the node switch on records nothing by itself');
    await page.locator('#diagstart').click();
    await page.waitForSelector('#diagbadge', { timeout: 15000 });
    const startReq = node.diagRequests.filter((r) => r.rest === '/sessions').pop();
    check(startReq && JSON.parse(startReq.body).generation === 1, 'Start names the generation it was shown', startReq && startReq.body);
    const A = liveSid();
    check(!!A, 'Start made a session');
    await page.locator('#diagbox').scrollIntoViewIfNeeded();
    await shot(page, '02-desktop-recording.png');
    // A slow page that fails at the node
    node.fixtures['/library/albums'] = { delay: 1500, status: 500, body: { error: `synthetic failure ${CANARY}-500` } };
    await go(page, '/albums');
    await page.waitForSelector('.notice.warn', { timeout: 10000 });
    {
      const bar = await page.locator('#diagbadge').boundingBox(); const first = await page.locator('#main > *').first().boundingBox();
      check(!overlap(bar, first), 'desktop: the bar does not cover the page’s first row');
    }
    await shot(page, '03-desktop-recording-failed-page.png');
    // A page whose own code breaks (a TypeError on a malformed answer), and a read with no answer
    node.fixtures['/library/genres'] = { body: null };
    await go(page, '/genres'); await sleep(800);
    node.fixtures['/radio/stations'] = { abort: true };
    await go(page, '/radio'); await sleep(800);
    // An error thrown outside any page's drawing (a timer): not a page this browser opened
    await page.evaluate(() => { setTimeout(() => { null.CANARY_field; }, 0); Promise.reject(new Error('CANARY rejection')); });
    // Navigation whose arguments are private: a search, a tag, a playlist, an album id, a person
    node.fixtures['/library/search'] = { body: { tracks: [{ id: 1, title: `${CANARY} title` }], albums: [], artists: [] } };
    for (const p of [`/search?q=${CANARY}-query-secret`, `/tag/${CANARY}-tag`, `/playlist/${CANARY}-playlist`, `/album/lib-${CANARY}1`, `/person/${PERSON}`]) { await go(page, p); await sleep(600); }
    let got = await sentinel(page, A);
    const evA = eventsOf(A);
    check(evA.some((e) => e.kind === 'nav' && e.view === 'albums'), 'navigation is recorded (albums)');
    check(evA.some((e) => e.kind === 'load' && e.view === 'albums' && e.outcome === 'http_5xx' && e.ms === 'lt3000'), 'a slow page that failed: load albums http_5xx lt3000', evA.filter((e) => e.view === 'albums'));
    check(evA.some((e) => e.kind === 'load' && e.view === 'genres' && e.outcome === 'failed'), 'a page whose code broke: load genres failed', evA.filter((e) => e.view === 'genres'));
    check(evA.some((e) => e.kind === 'exception' && e.class === 'type_error'), 'its error class: type_error');
    check(evA.some((e) => e.kind === 'api' && e.area === 'library' && e.outcome === 'ok'), 'the page’s own read: api library ok');
    check(evA.some((e) => e.kind === 'api' && e.area === 'radio' && e.outcome === 'unreachable'), 'a read with no answer: api radio unreachable', evA.filter((e) => e.kind === 'api'));
    check(evA.filter((e) => e.kind === 'exception').length === 1, 'errors outside a page being drawn are not recorded', evA.filter((e) => e.kind === 'exception'));
    for (const v of ['search', 'tag', 'playlist', 'item', 'person']) check(evA.some((e) => e.kind === 'nav' && e.view === v), `a private navigation is recorded only as its kind of page: ${v}`);
    check(!allDiagText().includes(CANARY) && !allDiagText().includes(PERSON), 'no argument, title, query, message or address reaches a diagnostics request');
    check(!node.diagRequests.some((r) => r.body.includes(TOKEN) || r.url.includes(TOKEN)), 'the token travels only in the Authorization header');
    check(!node.diagRequests.some((r) => r.url.includes('?')), 'no diagnostics request carries a query');
    check(node.schemaErrors.length === 0, 'every event sent is in the node’s schema', node.schemaErrors);

    // ---- 3. Activity from the node's event socket (other people) is never recorded ----
    // Each time: a page opened by hand and sent, then socket events, then the sentinel; anything
    // recorded between the two would sit in the queue before the sentinel's navigation
    const between = (base) => { const after = eventsOf(A).slice(base); return after.slice(0, after.findIndex((e) => e.kind === 'nav' && e.view === 'tracks')); };
    node.fixtures['/library/albums'] = { body: [] };
    await go(page, '/albums'); await sleep(800);
    await sentinel(page, A);
    let base = eventsOf(A).length; let hits = node.apiHits.length;
    push({ type: 'library_changed' }); await sleep(800);
    push({ type: 'library_changed' }); await sleep(800);
    await sentinel(page, A);
    check(node.apiHits.slice(hits).filter((h) => h.v === '/info').length >= 2 && node.apiHits.slice(hits).some((h) => h.v === '/library/tracks'), 'library changes from the socket did make the page fetch and redraw', node.apiHits.slice(hits).map((h) => h.v));
    check(between(base).length === 0, 'library changes from the socket recorded nothing', between(base));
    // On Zones (opened by hand, its own events sent first), other people's zones change
    const nz = eventsOf(A).length;
    await go(page, '/zones');
    await until('the Zones page is sent', () => eventsOf(A).slice(nz).some((e) => e.kind === 'load' && e.view === 'zones'), 30000);
    base = eventsOf(A).length; hits = node.apiHits.length;
    const zone = (transport, v) => ({ type: 'zone', state: { id: 'z-other', name: 'Kitchen', transport, queue_version: v, now_playing: null, error: null, volume: 0.5, queue: [], output: { name: 'Kitchen', kind: 'null' } } });
    push(zone('playing', 3)); await sleep(800);
    push(zone('paused', 4)); await sleep(800);
    push({ type: 'zone_removed', zone: 'z-other' }); await sleep(800);
    const zoneHits = node.apiHits.slice(hits).map((h) => h.v);
    await sentinel(page, A);
    check(zoneHits.length > 0, 'zone changes from the socket did make the page fetch and redraw', zoneHits);
    check(between(base).length === 0, 'zone changes from the socket recorded nothing', between(base));

    // ---- 3b. A batch refused for the rates (429) is not counted again by the browser ----
    {
      const p0 = node.posts.length; node.rateLimitNext = 1;
      await go(page, '/albums'); await sleep(500);
      await until('a batch is refused with 429', () => node.posts.slice(p0).some((x) => x.status === 429), 25000);
      await sentinel(page, A);
      const after = node.posts.slice(p0); const ok = after.filter((x) => x.status === 204);
      check(ok.length > 0 && ok.every((x) => (x.body.dropped || 0) === 0), 'after a 429 the browser does not report the refused events as its own drops', after.map((x) => [x.status, x.body.events.length, x.body.dropped]));
    }

    // ---- 4. Stale completions: a read held until the new session has started is never in it ----
    // The read is the Radio page's own (area radio, which nothing else in the new session makes),
    // answered only after Stop+Start, a new token or Off/On; the whole new session is checked
    for (const how of ['Stop then Start', 'a new token then Start', 'Off and On then Start']) {
      node.fixtures['/radio/stations'] = { hold: true, body: [] };
      const was = liveSid();
      await go(page, '/radio');
      await until('the Radio read is held', () => node.held.length > 0, 10000);
      if (how === 'Stop then Start') { await page.locator('#diagstop').click(); await until('the session is stopped at the node', () => node.deletes.includes(was)); }
      if (how === 'a new token then Start') {
        await go(page, '/settings/about'); await page.waitForSelector('#tokennew', { timeout: 10000 });
        await page.locator('#tokennew').click(); await page.getByRole('button', { name: 'Make a new token' }).last().click();
        await until('the bar goes with the old token', async () => !(await page.locator('#diagbadge').count()));
      }
      if (how === 'Off and On then Start') { await openSettings(page); await page.locator('#diaggate').uncheck(); await until('the bar goes when switched off', async () => !(await page.locator('#diagbadge').count())); }
      check(!(await page.locator('#diagbadge').count()), `${how}: the bar is gone`);
      const sid = await startHere(page);
      check(sid && sid !== was, `${how}: a new session`);
      const answered = node.held.length; node.held.splice(0).forEach((f) => f());
      check(answered > 0, `${how}: the old read is answered only now, in the new session's time`);
      await sleep(1000);
      await sentinel(page, sid);
      const stale = eventsOf(sid).filter((e) => e.area === 'radio' || e.view === 'radio');
      check(stale.length === 0, `${how}: nothing of the old Radio page is in the new session`, eventsOf(sid));
    }
    delete node.fixtures['/radio/stations'];

    // ---- 5. A Start still on its way when the switch goes off and on again: never activates ----
    await page.locator('#diagstop').click(); await sleep(300);
    await openSettings(page);
    node.startDelay = 2500; node.acceptStaleStart = true; // even if a node let it through
    const dels = node.deletes.length; const sessionsBefore = node.sessions.size; const reqs0 = node.diagRequests.length;
    await page.locator('#diagstart').click();
    await sleep(300);
    await page.locator('#diaggate').uncheck(); await sleep(400);
    await page.waitForSelector('#diaggate'); await page.locator('#diaggate').check(); await sleep(3000);
    check(!(await page.locator('#diagbadge').count()), 'a Start pending across Off and On does not start recording');
    const madeLate = [...node.sessions.keys()].slice(sessionsBefore);
    check(madeLate.every((id) => node.deletes.slice(dels).includes(id)) && node.deletes.length > dels, 'the session the node made for it is ended by the page', { madeLate, deletes: node.deletes.slice(dels), reqs: node.diagRequests.slice(reqs0).map((r) => `${r.m} ${r.rest} ${r.body}`) });
    const looks = node.diagRequests.slice(reqs0).filter((r) => r.m === 'GET' && r.rest === '').length;
    check(looks <= 12, 'Settings asks for the status a bounded number of times meanwhile (no loop)', looks);
    node.startDelay = 0; node.acceptStaleStart = false;

    // ---- 6. A stalled node: one send at a time, each given up after a while, cancelled on Stop ----
    const S = await startHere(page);
    let inflight = 0; let maxInflight = 0; const failedSends = [];
    const isEvents = (r) => r.url().endsWith('/v1/node/diagnostics/events');
    page.on('request', (r) => { if (isEvents(r)) { inflight += 1; maxInflight = Math.max(maxInflight, inflight); } });
    page.on('requestfailed', (r) => { if (isEvents(r)) { inflight -= 1; failedSends.push({ at: Date.now(), why: r.failure() && r.failure().errorText }); } });
    page.on('requestfinished', (r) => { if (isEvents(r)) inflight -= 1; });
    node.stallEvents = true;
    for (const p of ['/albums', '/tracks', '/zones', '/albums']) { await go(page, p); await sleep(300); }
    await until('a send starts', () => inflight > 0, 15000);
    const t0 = Date.now();
    await until('the stalled send is given up', () => failedSends.length > 0, 12000);
    check(Date.now() - t0 <= 9500, 'a stalled send is given up within its time limit', Date.now() - t0);
    for (const p of ['/genres', '/tracks']) { await go(page, p); await sleep(300); }
    await until('another send starts', () => inflight > 0, 15000);
    const nFailed = failedSends.length;
    await page.locator('#diagstop').click();
    await until('Stop cancels the send on its way', () => failedSends.length > nFailed, 3000);
    check(maxInflight === 1, 'never more than one send at a time', maxInflight);
    node.stallEvents = false;
    const postsAfterStop = node.eventPosts; await sleep(11000);
    check(node.eventPosts === postsAfterStop, 'nothing is sent after Stop (the queue went with it)');
    check(!eventsOf(S).length, 'nothing from the stalled session was kept');

    // ---- 7. The browser's own deadline holds offline ----
    node.expiresIn = 3; // the node grants 3 s; the page never takes more than an hour, nor more than given
    const E = await startHere(page);
    await ctx.setOffline(true);
    await go(page, '/downloads'); await sleep(400); await go(page, '/queue'); await sleep(400);
    await until('the bar goes at the deadline, offline', async () => !(await page.locator('#diagbadge').count()), 6000);
    await go(page, '/downloads'); await sleep(300); await go(page, '/settings'); await sleep(300);
    await ctx.setOffline(false); node.expiresIn = 3600;
    await sleep(12000);
    check(eventsOf(E).length === 0, 'nothing recorded offline is sent after the deadline', eventsOf(E));

    // ---- 8. Download and Clear ----
    await openSettings(page);
    await until('the Download button is enabled', async () => !(await page.locator('#diagdl').isDisabled()), 10000);
    const [dl] = await Promise.all([page.waitForEvent('download'), page.locator('#diagdl').click()]);
    const file = fs.readFileSync(await dl.path(), 'utf8');
    const ex = JSON.parse(file);
    check(dl.suggestedFilename() === 'odeion-diagnostics.json' && ex.manifest && ex.manifest.schema === 'pm-diagnostics/1' && ex.events.length === node.events.length, 'Download saves the export as a file', dl.suggestedFilename());
    check(!file.includes(CANARY) && !file.includes(TOKEN), 'the downloaded file has no canary and no token');
    await page.locator('#diagclear').click();
    await until('Clear empties the node', () => node.events.length === 0 && node.diagRequests.some((r) => r.rest === '/clear'));
    await page.waitForSelector('#diagdl[disabled]', { timeout: 5000 }).catch(() => {});
    check(/0 events/.test(await page.locator('#diagbox').innerText()), 'Settings shows nothing kept after Clear');
    check(await page.locator('#diaggate').isChecked(), 'Clear leaves the switch as it was (Clear is not Off)');
    check(node.schemaErrors.length === 0, 'every event sent in the whole run is in the schema', node.schemaErrors);
    check(!allDiagText().includes(CANARY), 'no canary in any diagnostics request of the whole run');
    check(!pageErrors.includes('SyntaxError') && !pageErrors.includes('ReferenceError'), 'the app itself raised no syntax or reference error', pageErrors);

    // ---- 8b. Reload and Log out end this browser's session, at the node too ----
    {
      const R = await startHere(page);
      await go(page, '/albums'); await sleep(300);
      await page.reload();
      await page.waitForSelector('#main', { timeout: 15000 }); await sleep(1500);
      check(!(await page.locator('#diagbadge').count()), 'after a reload this browser is not recording');
      await until('the reloaded page ended its session at the node', () => node.deletes.includes(R), 5000);
      await go(page, '/tracks'); await sleep(300); await go(page, '/albums'); await sleep(300);
      const L = await startHere(page);
      const tokenThen = node.token;
      await go(page, '/tracks'); await sleep(300);
      await page.locator('#me').click();
      await page.getByText('Log out', { exact: true }).last().click();
      await until('Log out ends the session at the node', () => node.deletes.includes(L), 5000);
      const del = node.diagRequests.filter((r) => r.m === 'DELETE' && r.rest === `/sessions/${L}`).pop();
      check(del && del.auth === 'Bearer ' + tokenThen, 'Log out ends it with the token it started with', del && del.auth.slice(0, 14));
      check(!(await page.locator('#diagbadge').count()), 'after Log out this browser is not recording');
      const posts = node.eventPosts; await sleep(11000);
      check(node.eventPosts === posts, 'nothing is sent after a reload or Log out');
      check(eventsOf(R).every((e) => e.view !== 'tracks'), 'pages opened after the reload are not in the ended session', eventsOf(R));
    }
    await ctx.close();

    // ---- 9. Phone and tablet: the bar has its own row, errors stay visible ----
    for (const [name, vp] of [['phone', { width: 390, height: 844 }], ['tablet', { width: 820, height: 1180 }]]) {
      node = fresh(); sockets.length = 0;
      ctx = await newContext(browser, vp, 'owner', { isMobile: name === 'phone', hasTouch: true });
      page = await ctx.newPage();
      await page.goto(ORIGIN + '/settings/about');
      await page.waitForSelector('#diaggate', { timeout: 20000 });
      await page.locator('#diagbox').scrollIntoViewIfNeeded();
      await shot(page, `04-${name}-settings-off.png`);
      await startHere(page);
      check((await page.evaluate(() => document.documentElement.dataset.shell)) === name, `${name}: the ${name} layout`);
      const sent = JSON.parse(node.diagRequests.filter((r) => r.rest === '/sessions').pop().body);
      check(sent.viewport === name, `${name}: the viewport class sent is ${name}`, sent);
      node.fixtures['/library/albums'] = { status: 500, body: { error: 'synthetic failure' } };
      await go(page, '/albums');
      await page.waitForSelector('.notice.warn', { timeout: 10000 });
      const bar = await page.locator('#diagbadge').boundingBox();
      const err = await page.locator('#main .notice.warn').first().boundingBox();
      const top = await page.locator('.topbar').boundingBox();
      const player = await page.locator('.player').first().boundingBox().catch(() => null);
      check(bar && err && !overlap(bar, err) && err.y >= bar.y + bar.height - 1, `${name}: the error notice is below the bar, not under it`, { bar, err });
      check(!overlap(bar, top) && !overlap(bar, player), `${name}: the bar covers neither the top bar nor the player`, { bar, top, player });
      check(await page.locator('#diagstop').isVisible(), `${name}: Stop is on the bar`);
      // Now playing (an overlay over the page) opens below the bar: the bar stays visible
      await page.keyboard.press('n');
      await page.waitForSelector('#nowplaying:not([hidden])', { timeout: 5000 }).catch(() => {});
      if (await page.locator('#nowplaying:not([hidden])').count()) {
        const np = await page.locator('#nowplaying').boundingBox(); const b = await page.locator('#diagbadge').boundingBox();
        const onTop = await page.evaluate(({ x, y }) => !!document.elementFromPoint(x, y).closest('#diagbadge'), { x: b.x + b.width / 2, y: b.y + b.height / 2 });
        check(onTop && !overlap(b, np), `${name}: with Now playing open the bar is still shown, above it`, { b, np, onTop });
        await sleep(500); // its opening animation
        await shot(page, `06-${name}-now-playing.png`);
        await page.keyboard.press('Escape');
      } else check(false, `${name}: Now playing opened for the check`);
      await shot(page, `05-${name}-recording-failed-page.png`);
      await ctx.close();
    }

    // ---- 10. Listeners: no diagnostics at all ----
    node = fresh('listener'); sockets.length = 0;
    ctx = await newContext(browser, { width: 1280, height: 860 }, 'listener');
    page = await ctx.newPage();
    await page.goto(ORIGIN + '/albums'); await sleep(2500);
    for (const p of ['/tracks', '/settings', '/queue']) { await go(page, p); await sleep(700); }
    check(!(await page.locator('#diagbox').count()) && !(await page.locator('#diagbadge').count()), 'a listener sees no diagnostics controls');
    check(node.diagRequests.length === 0, 'a listener’s page never calls diagnostics', node.diagRequests.map((r) => r.m + r.rest));
    await ctx.close();
  } finally {
    await browser.close();
  }
  console.log(failed ? `\n${failed} check(s) FAILED` : '\nall diagnostics checks passed');
  console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error('ui-diagnostics: the run broke:', e); process.exit(2); });
