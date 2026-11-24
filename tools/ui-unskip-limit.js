// Bringing skipped songs back (Which song is this?: the card's "Bring back N skipped", and the Undo of
// Skip all unsure and of Leave as is) says "Bringing the songs back…" at once. A node that stops
// answering didn't leave that up for good: after a minute the request is given up and the toast says
// so, with where to see what is still skipped; the card's Bring back can be pressed again. The node
// itself waits up to 10 minutes while a scan writes (then answers why), so the cut comes after 11.
// An answer in time still says how many came back. Phone (360 with the Android app, 412) and desktop (1280).
// The page's real assets from a loopback server that answers /v1 itself; the clock is Playwright's,
// so the minutes pass at once:
//   node tools/ui-unskip-limit.js [static dir] [screenshot dir]
const http = require('http');
const fs = require('fs');
const path = require('path');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const STATIC = process.argv[2] || path.join(__dirname, '..', 'crates', 'pm-web', 'static');
const ui = require('./ui-src'); // app.js is joined from its parts, as the node's build does
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'unskip-limit-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 6000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(50); } };

const LATE = /^The node stopped answering, so the songs may not be back\. Your library › Node library shows how many are still skipped\.$/;
const track = (id, no, title) => ({ id, title, artists: [{ name: 'Band' }], track_no: no, disc_no: null, duration_ms: 200000, file: { path: `/music/Band/Album/${no} ${title}.flac` } });
const album = () => ({ key: '/music/Band/Album', album: 'Album', artist: 'Band', release: null, ready: 1, conflicts: 0, songs: [{ track: track(11, 1, 'Song 1'), options: [{ id: 101, track_id: 11, recording_mbid: '00000000-0000-4000-8000-000000000001', release_mbid: null, title: 'Song 1', artist: 'Band', release_title: null, score: 0.95, source: 'album' }], status: 'single', pick: 101 }] });
const status = { phase: 'needs_review', progress: { running: false, total: 30, done: 30, identified: 20, suggested: 6, unknown: 4, errors: 0, finished_at: 1 }, counts: { total: 120, identified: 90, suggested: 6, skipped: 3, outdated: 0, unknown: 4, unchecked: 0, by: { tags: 10, album: 70, fingerprint: 8, you: 2 } }, acoustid: true, acoustid_own: true, acoustid_state: 'ok', write_ids: false };

// /v1 as a node with unsure songs; `fx.hang`: the unskip is never answered
let fx = { log: [], hang: true };
const server = http.createServer(async (req, res) => {
  const u = new URL(req.url, 'http://x');
  if (u.pathname.startsWith('/v1/')) {
    const p = u.pathname.slice(3);
    let body = '';
    for await (const c of req) body += c;
    const send = (st, json) => { res.writeHead(st, { 'content-type': 'application/json' }); res.end(json === undefined ? '' : JSON.stringify(json)); };
    const b = body ? JSON.parse(body) : null;
    fx.log.push({ method: req.method, path: p, body: b });
    if (p === '/library/suggestions/unskip' && req.method === 'POST') { if (fx.hang) return undefined; return send(200, { restored: b.all ? 3 : 1, counts: { skipped: 0 } }); }
    if (p === '/library/suggestions/skip' && req.method === 'POST') return send(200, { skipped: b.all ? 6 : 1, counts: { skipped: b.all ? 9 : 4 } });
    if (p === '/library/identify') return send(200, status);
    if (p === '/library/suggestions/albums') return send(200, { albums: [album()], songs: 1, total_albums: 1, offset: 0, next: null });
    const lists = { '/favourites': [], '/sources': [], '/zones': [], '/library/folders': [], '/library/webdav': [], '/library/hidden': [], '/presence': [] };
    if (p === '/info') return send(200, { id: 'n', name: 'Test node', version: '0', api: 'v1', library: { tracks: 120, albums: 10, artists: 3, files: 120, missing_files: 0, total_duration_ms: 1 }, zones: 0, outputs: 0, scanning: false });
    if (p === '/me') return send(200, { role: 'owner' });
    if (p === '/library/drives') return send(200, { roots: [], providers: [] });
    if (p in lists) return send(200, lists[p]);
    return send(200, {});
  }
  const name = u.pathname.slice(1);
  const file = /^[\w.-]+$/.test(name) && name !== 'index.html' && ui.has(STATIC, name) ? name : 'index.html';
  const type = { '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.webmanifest': 'application/manifest+json', '.html': 'text/html; charset=utf-8' }[path.extname(file)] || 'application/octet-stream';
  res.writeHead(200, { 'content-type': type, 'cache-control': 'no-cache' });
  res.end(ui.read(STATIC, file));
});

(async () => {
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const BASE = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch();
  try {
    for (const [width, mobile, android] of [[360, true, true], [412, true, false], [1280, false, false]]) {
      const at = `${width}${android ? ' Android' : ''}:`;
      const ctx = await browser.newContext({ viewport: { width, height: mobile ? 800 : 900 }, reducedMotion: 'reduce', hasTouch: mobile, isMobile: mobile, serviceWorkers: 'block' });
      await ctx.addInitScript(() => { localStorage.setItem('pm.token', 't'); localStorage.setItem('pm.zone', 'browser'); });
      if (android) await ctx.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
      const outside = [];
      await ctx.route((url) => url.origin !== BASE && !/^(data|blob):/.test(url.protocol), (r) => { outside.push(r.request().url()); return r.abort(); });
      // connectToServer first: otherwise Playwright 1.49 opens the routed socket after this handler
      // (WebSocketRoute._afterHandle → ensureOpened) outside any await, and a reconnect landing while
      // the context closes rejects with a TargetClosedError nobody catches (exit 1 after every check
      // passed). The loopback server has no upgrade listener and drops it; the page still sees a close.
      await ctx.routeWebSocket(/\/v1\/events/, (ws) => { ws.connectToServer(); return ws.close(); });
      const page = await ctx.newPage();
      await page.clock.install();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      const asked = () => fx.log.filter((x) => x.method === 'POST' && x.path === '/library/suggestions/unskip').length;
      const toastText = () => page.locator('#toast').textContent();
      const toastIs = (re, kind) => page.evaluate(([src, k]) => { const t = document.querySelector('#toast'); return new RegExp(src).test(t.textContent) && t.classList.contains('show') && (!k || t.classList.contains(k)); }, [re.source, kind]);
      const press = async (sel) => { const l = page.locator(sel); await l.scrollIntoViewIfNeeded(); return mobile ? l.tap() : l.click(); };
      // 11 minutes pass: still bringing them back at 10 min 58 s (the node may still be waiting out a
      // scan), given up after 11
      const minute = async (what) => {
        await page.clock.fastForward(658000);
        check(await toastIs(/^Bringing the songs back…$/, 'wait'), `${at} ${what}: 10 min 58 s on, the toast still says it is bringing the songs back ("${await toastText()}")`);
        await page.clock.fastForward(3000);
        check(await until(() => toastIs(LATE, 'error'), 3000), `${at} ${what}: after 11 minutes it says the node stopped answering, and where to see what is still skipped ("${await toastText()}")`);
      };
      try {
        // 1. The card's Bring back 3 skipped (a second press confirms it)
        fx = { log: [], hang: true };
        await page.goto(`${BASE}/library/node`);
        await page.locator('#idunskip').waitFor();
        await press('#idunskip'); await sleep(700); await press('#idunskip');
        check(await until(async () => asked() === 1), `${at} the card's Bring back asks the node`);
        check(await until(() => toastIs(/^Bringing the songs back…$/, 'wait'), 2000), `${at} …and says at once it is bringing the songs back`);
        await minute('the card\'s Bring back');
        check(await until(async () => page.evaluate(() => { const b = document.querySelector('#idunskip'); return !!b && !b.disabled && b.getAttribute('aria-busy') !== 'true' && /^Bring back 3 skipped$/.test(b.textContent.trim()); }), 3000), `${at} …and Bring back can be pressed again (${await page.locator('#idunskip').textContent().catch(() => 'gone')})`);
        await page.screenshot({ path: path.join(SHOTS, `card-late-${width}.png`) });
        // …which, answered in time, brings them back
        fx.hang = false;
        await press('#idunskip'); await sleep(700); await press('#idunskip');
        check(await until(async () => /3 songs back for review/.test(await toastText())), `${at} …pressed again with the node answering, it says 3 songs are back`);

        // 2. The card's Skip all unsure, then its Undo
        fx = { log: [], hang: true };
        await page.goto(`${BASE}/library/node`);
        await page.locator('#idskipall').waitFor();
        await press('#idskipall');
        await page.locator('#toast .toast-act').waitFor();
        await press('#toast .toast-act');
        check(await until(async () => asked() === 1) && await until(() => toastIs(/^Bringing the songs back…$/, 'wait'), 2000), `${at} the card's Skip all Undo asks the node and says it is bringing the songs back`);
        await minute('the card\'s Undo');

        // 3. The review page's Leave as is, then its Undo
        fx = { log: [], hang: true };
        await page.goto(`${BASE}/identify`);
        await page.locator('section.idalbum [data-skip]').waitFor();
        await press('section.idalbum [data-skip]');
        await page.locator('#toast .toast-act').waitFor();
        await press('#toast .toast-act');
        check(await until(async () => asked() === 1) && await until(() => toastIs(/^Bringing the songs back…$/, 'wait'), 2000), `${at} the review page's Undo asks the node and says it is bringing the songs back`);
        await minute('the review page\'s Undo');
        await page.screenshot({ path: path.join(SHOTS, `review-late-${width}.png`) });
        // …answered in time, it says how many came back
        fx = { log: [], hang: false };
        await page.goto(`${BASE}/identify`);
        await page.locator('section.idalbum [data-skip]').waitFor();
        await press('section.idalbum [data-skip]');
        await page.locator('#toast .toast-act').waitFor();
        await press('#toast .toast-act');
        check(await until(async () => /1 song back for review/.test(await toastText())), `${at} …answered in time, the review page's Undo says 1 song is back ("${await toastText()}")`);

        const wide = await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);
        check(wide, `${at} nothing wider than the screen`);
        check(!errors.length, `${at} no page errors ${errors.slice(0, 3).join(' | ')}`);
        check(!outside.length, `${at} nothing asked outside the loopback server ${outside.slice(0, 3).join(' ')}`);
      } catch (e) {
        check(false, `${at} ran to the end (${e.message.split('\n')[0]})`);
      } finally {
        await ctx.close();
      }
    }
  } finally {
    await browser.close();
    server.close();
  }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
