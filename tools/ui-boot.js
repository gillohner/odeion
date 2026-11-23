// The page shows a loader the moment it opens, and its first three reads go together.
// Real candidate assets on the fixture origin (tools/ui-fixture.js), every answer delayed on purpose
// (a slow link: remote access, a phone's data):
//   * the loader is in the page itself: it is there when the page has loaded, before the node has
//     answered anything, at phone and desktop widths, and the first page takes its place;
//   * /info, /me and /zones are asked for together (each used to wait for the one before: three
//     round trips before the first page was asked for);
//   * the first page is drawn after about two round trips, not four;
//   * logged out, only the first read is made and Log in shows;
//   * a slow start says so in words ("Loading…" for a screen reader, "Still loading…" after 2 s);
//   * a kept token the node no longer takes reloads the page once, not once per refusal;
//   * a node that can't be reached says so in place of the loader, with nothing left unhandled.
//   node tools/ui-boot.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'boot-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const DELAY = 700;
const first = (state, v) => state.hits.find((h) => h.v === v);
const until = async (fn, ms = 7000) => { const t0 = Date.now(); for (;;) { const v = await fn(); if (v) return v; if (Date.now() - t0 > ms) return v; await sleep(40); } };

(async () => {
  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const { context, page, state } = await fx.create(browser, width, { delay: DELAY, mobile: width < 600 });
    const t0 = Date.now();
    await fx.go(page, '/tracks');
    // (the page has loaded; no answer can have come yet)
    const at = Date.now() - t0;
    const boot = await page.evaluate(() => { const m = document.getElementById('main'); const k = m.firstElementChild; const r = k ? k.getBoundingClientRect() : null; return { cls: k ? k.className : '', busy: k ? k.getAttribute('aria-busy') : null, shapes: m.querySelectorAll('.skeleton').length, h: r ? r.height : 0, w: r ? r.width : 0 }; });
    check(at < DELAY && /\bnavskel\b/.test(boot.cls) && boot.busy === 'true' && boot.shapes >= 3 && boot.h > 100 && boot.w > 200, `${width}px: the loader is on the page before the node's first answer (${at} ms in; ${boot.shapes} shapes, ${Math.round(boot.w)} × ${Math.round(boot.h)})`);
    await page.screenshot({ path: path.join(SHOTS, `boot-${width}.png`) });
    const drawn = await until(() => page.evaluate(() => !!document.querySelector('#main .track')));
    const took = Date.now() - t0;
    const info = first(state, '/info'); const me = first(state, '/me'); const zones = first(state, '/zones');
    check(!!(info && me && zones) && Math.abs(me.at - info.at) < DELAY / 2 && Math.abs(zones.at - info.at) < DELAY / 2, `${width}px: /info, /me and /zones are asked for together (${me && info ? me.at - info.at : '?'} and ${zones && info ? zones.at - info.at : '?'} ms after /info; each answer takes ${DELAY})`);
    check(drawn && took < DELAY * 3.5, `${width}px: the first page is drawn after about two round trips (${took} ms; three and a page's own were ${DELAY * 4})`);
    check(await page.evaluate(() => !document.getElementById('bootskel')), `${width}px: the page took the loader's place`);
    check(!state.errors.length, `${width}px: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  {
    // Logged out: the first refusal alone opens Log in
    const { context, page, state } = await fx.create(browser, 1280, { token: '', fixtures: { '/info': { status: 401, body: { error: 'missing or wrong bearer token' } }, '/me': { status: 401, body: { error: 'missing or wrong bearer token' } }, '/zones': { status: 401, body: { error: 'missing or wrong bearer token' } } } });
    await page.addInitScript(() => { try { localStorage.removeItem('pm.token'); } catch { /* none */ } });
    await fx.go(page, '/');
    const login = await until(() => page.evaluate(() => !document.getElementById('loginpage').hidden), 4000);
    check(login, 'logged out: Log in shows');
    check(!first(state, '/zones') && !first(state, '/me'), `logged out: only the first read was made (${state.hits.map((h) => h.v).filter((v) => ['/info', '/me', '/zones'].includes(v)).join(' ')})`);
    check(!state.errors.length, `logged out: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  {
    // A slow start says so in words: "Loading…" for a screen reader from the start, "Still loading…" on the page after 2 s
    const { context, page, state } = await fx.create(browser, 390, { delay: 3200, mobile: true });
    await fx.go(page, '/tracks');
    // (a status region reads out what is put into it after the page has opened: empty at first, then the word)
    const region = () => page.evaluate(() => { const s = document.getElementById('navstatus'); return s ? { role: s.getAttribute('role'), text: s.textContent, outside: !document.getElementById('main').contains(s) } : null; });
    const atOpen = await region();
    const saidLoading = await until(async () => { const r = await region(); return !!r && r.text === 'Loading…'; }, 1500);
    const words = await page.evaluate(() => { const n = document.querySelector('#bootskel .slow-note'); return { note: !!n, hidden: n ? n.hidden : null }; });
    check(!!atOpen && atOpen.role === 'status' && atOpen.outside && atOpen.text === '' && saidLoading && words.note && words.hidden === true, `slow start: "Loading…" is put into a status region outside the page after it opened, and no "Still loading…" yet (${JSON.stringify(atOpen)}, ${JSON.stringify(words)})`);
    const still = await until(() => page.evaluate(() => { const n = document.querySelector('#bootskel .slow-note'); return !!n && !n.hidden && n.getClientRects().length > 0; }), 3000);
    check(still, 'slow start: "Still loading…" shows on the loader after 2 s');
    await page.screenshot({ path: path.join(SHOTS, 'boot-still-loading-390.png') });
    check(!state.errors.length, `slow start: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  {
    // A kept token the node no longer takes: three refusals come together, and the page reloads once for Log in
    const refused = { status: 401, body: { error: 'missing or wrong bearer token' } };
    const { context, page, state } = await fx.create(browser, 1280, { fixtures: { '/info': refused, '/me': refused, '/zones': refused } });
    // (the fixture puts its token back on every load, so the page after the reload is a plain one
    // here: this counts the reloads the first page asks for and reads the reason it left)
    let docs = 0;
    await page.route((u) => u.pathname === '/tracks', async (route) => { if (route.request().resourceType() !== 'document' || ++docs === 1) return route.fallback(); return route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>reloaded</title><p id="reloaded">reloaded</p>' }); });
    await fx.go(page, '/tracks');
    const reloaded = await until(() => page.evaluate(() => !!document.getElementById('reloaded')).catch(() => false), 5000);
    await sleep(600);
    check(reloaded && docs === 2, `token no longer taken: the page reloaded once (${docs - 1} reloads after three refusals together)`);
    const left = await page.evaluate(() => sessionStorage.getItem('pm.loginWhy'));
    check(left === 'ended', `token no longer taken: Log in will say the session ended (${left})`);
    const asked = state.hits.map((h) => h.v).filter((v) => ['/info', '/me', '/zones'].includes(v));
    check(asked.length === 3, `token no longer taken: the three reads were made once (${asked.join(' ')})`);
    check(!state.errors.length, `token no longer taken: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  {
    // A node that can't be reached: the loader gives way to the message
    const { context, page, state } = await fx.create(browser, 390, { mobile: true, fixtures: { '/info': { status: 502, body: { error: 'bad gateway' } }, '/zones': { status: 502, body: { error: 'bad gateway' } }, '/me': { status: 502, body: { error: 'bad gateway' } } } });
    const rejections = []; await page.exposeFunction('__rejected', (m) => rejections.push(m));
    await page.addInitScript(() => window.addEventListener('unhandledrejection', (e) => window.__rejected(String(e.reason && e.reason.message || e.reason))));
    await fx.go(page, '/');
    const said = await until(() => page.evaluate(() => /Cannot reach the node/.test(document.getElementById('main').innerText)), 4000);
    check(said, 'node away: the page says it cannot reach the node');
    check(await page.evaluate(() => !document.querySelector('#main .navskel') && document.getElementById('navstatus').textContent === ''), 'node away: the loader is gone, and its "Loading…"');
    await sleep(300);
    check(!rejections.length && !state.errors.length, `node away: nothing left unhandled (${[...rejections, ...state.errors].join(' | ') || 'none'})`);
    await page.screenshot({ path: path.join(SHOTS, 'boot-node-away-390.png') });
    await context.close();
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
