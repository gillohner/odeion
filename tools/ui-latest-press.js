// The latest press wins the "starting" spinner. On a speaker zone (the Kitchen, paused), a song
// row's Play spins while its song starts. Another row's ⋯ › Start radio, or Space (play the
// Kitchen), pressed within 1.5 s of it starts again, and that start puts no spinner back on the
// row's Play (before, it spun again, as if pressed a second time, and stayed spinning).
// A picker the Play opens to finish it (a song not on your services: its 30 s preview, played in
// this browser) keeps that press: the pick, made after 1.5 s, spins the Play pressed (before, the
// pick's own click took the press away and nothing spun). But only that Play's press, and only
// with no newer one since: Space while the picker is open, or another Play pressed while the
// first one's answer was still coming, leaves no Play spinning for the pick. The phone width too.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-latest-press.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'latest-press-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };

const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, canonical: { kind: 'recording', mbid: fx.track.mbid } })); // (matched: Start radio needs a known song)
const item = (t) => ({ id: t.id, uri: `track:${t.id}`, title: t.title, artist: 'Sefa', duration_ms: 240000 });
// The Kitchen, paused on Song 3: Space plays it again
const z1 = { id: 'z1', name: 'Kitchen', output: 'upnp:z1', transport: 'paused', queue: [item(tracks[2])], queue_index: 0, now_playing: { item: item(tracks[2]), index: 0, position_ms: 1000, duration_ms: 240000 }, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
const fixtures = {
  '/library/tracks': { body: { tracks, next: null } },
  '/zones': { body: [z1] }, '/zones/z1': { body: z1 }, '/zones/z1/queue': { body: z1.queue },
  '/outputs': { body: [{ id: 'upnp:z1', name: 'Kitchen', kind: 'upnp', caps: {} }] },
};
// Every spinner put on a button: when, and on which (its id, else its first data- attribute)
const logSpins = (page) => page.addInitScript(() => {
  window.__spins = []; const owner = new WeakMap();
  const cl = Object.getOwnPropertyDescriptor(Element.prototype, 'classList');
  Object.defineProperty(Element.prototype, 'classList', { ...cl, get() { const l = cl.get.call(this); owner.set(l, this); return l; } });
  const who = (el) => (!el ? '?' : el.id ? `#${el.id}` : (() => { const d = [...el.attributes].find((a) => a.name.startsWith('data-')); return d ? `[${d.name}]` : el.tagName.toLowerCase(); })());
  const add = DOMTokenList.prototype.add;
  DOMTokenList.prototype.add = function (...c) { if (c.includes('starting')) window.__spins.push({ at: Date.now(), el: who(owner.get(this)) }); return add.apply(this, c); };
});

async function run(browser, width, how) {
  const tag = `${width} ${how}`;
  const { context, page, state } = await fx.create(browser, width, { fixtures, zone: 'z1' });
  await logSpins(page);
  // (the zone never says it plays: each start's wait lasts)
  await context.routeWebSocket(/\/v1\/events/, (w) => { w.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') w.send(JSON.stringify({ type: 'pong', id: j.id })); }); w.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [z1] })); });
  await context.route(/\/v1\/zones\/z1\/(queue|radio|transport)$/, async (r) => {
    const q = r.request(); if (q.method() === 'GET') return r.fallback();
    state.hits.push({ v: new URL(q.url()).pathname.slice(3), m: q.method(), q: '', body: q.postData(), at: Date.now() });
    return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(/\/transport$/.test(q.url()) ? z1 : {}) }).catch(() => {});
  });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    await page.waitForFunction(() => /Kitchen/.test((document.querySelector('#zonename, #zonebtn') || {}).textContent || ''));
    await page.waitForTimeout(300);
    const row = page.locator('#tracklist .track').nth(0);
    // (on a phone a tap on the row plays it, not a press of its Play: its Play is the keyboard's, Enter on it)
    if (width < 600) { await row.locator('[data-act="play"]').focus(); await page.keyboard.press('Enter'); } else { await row.hover(); await row.locator('[data-act="play"]').click(); }
    const t0 = Date.now();
    await page.waitForFunction(() => window.__spins.length > 0);
    check((await page.evaluate(() => window.__spins.map((s) => s.el))).join() === '[data-act]', `${tag}: Song 1's Play spins while it starts`);
    const mark = await page.evaluate(() => Date.now());
    const n = state.hits.length;
    if (how === 'menu') {
      const r2 = page.locator('#tracklist .track').nth(1);
      await r2.hover(); await r2.locator('[data-act="more"]').click();
      await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Start radio' }).click();
      for (let t = 0; t < 40 && !state.hits.slice(n).some((h) => h.m === 'POST' && h.v === '/zones/z1/queue'); t++) await page.waitForTimeout(50);
      check(state.hits.slice(n).some((h) => h.m === 'POST' && h.v === '/zones/z1/queue'), `${tag}: Start radio sends Song 2 to the Kitchen`);
    } else {
      // (the Play the click left focused doesn't take Space: Space plays or pauses the zone. One the
      // keyboard is on would take it, as a press of its own: on a phone the keyboard leaves it first)
      if (width < 600) await page.evaluate(() => document.activeElement.blur());
      await page.keyboard.press('Space');
      for (let t = 0; t < 40 && !state.hits.slice(n).some((h) => h.v === '/zones/z1/transport'); t++) await page.waitForTimeout(50);
      const sent = state.hits.slice(n).filter((h) => h.v === '/zones/z1/transport').map((h) => h.body);
      check(sent.some((b) => /"play"/.test(b || '')), `${tag}: Space plays the paused Kitchen (${sent.join(', ')})`);
    }
    const gap = (await page.evaluate(() => Date.now())) - t0;
    // (within the 1.5 s a Play's press lasts, or this check means nothing)
    check(gap < 1500, `${tag}: the second press came within 1.5 s of the Play (${gap} ms)`);
    await page.waitForTimeout(300);
    const later = await page.evaluate((m) => window.__spins.filter((s) => s.at >= m).map((s) => s.el), mark);
    check(later.length === 0, `${tag}: no Play spins for the second press (${JSON.stringify(later)})`);
    const still = await page.evaluate(() => [...document.querySelectorAll('.starting')].map((e) => e.outerHTML.slice(0, 60)));
    check(still.length === 0, `${tag}: no button left spinning (${JSON.stringify(still)})`);
    await page.screenshot({ path: path.join(SHOTS, `latest-press-${how}-${width}.png`) });
    check(state.errors.length === 0, `${tag}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
}

// Favourites from another service (none of yours has them): a Play opens "Not on your services"
// with a 30 s preview, played in this browser. `how`:
//   'late':  picked after the 1.5 s a press lasts: the pick is the Play's press going on
//   'space': Space pressed while the picker is open, then the pick: Space was the newer press, so
//            no Play spins (before, the Play that opened the picker spun again)
//   'other': B's Play pressed while A's answer is still coming, then A's picker opens and is picked:
//            B's press is not A's picker's, so B doesn't spin (before, B spun for A's preview)
const ELSE = 'https://sefa.bandcamp.com/track/elsewhere';
const ELSE2 = 'https://sefa.bandcamp.com/track/elsewhere-too';
async function picker(browser, width, how) {
  const tag = `${width} picker ${how}`;
  const { context, page, state } = await fx.create(browser, width, { fixtures: {
    '/favourites': { body: { uris: [ELSE, ELSE2], tracks: [], albums: [], artists: [], other: [ELSE, ELSE2] } },
    '/discover/describe': { body: [ELSE, ELSE2].map((u, i) => ({ uri: u, page: u, title: i ? 'Elsewhere too' : 'Elsewhere', artist: 'Sefa', kind: 'track', site: 'bandcamp', also_on: [], connect: [] })) },
  } });
  // (A's answer, held in the page until the test lets it go; B's never comes)
  const plan = { plays_from: null, items: [], connect: [], preview: { service: 'deezer', id: 'track:123' } };
  await page.addInitScript(({ plan, hold }) => {
    const f = window.fetch;
    window.__release = () => {};
    window.fetch = function (u, o) {
      const url = String(u && u.url || u);
      if (!/\/discover\/play\?/.test(url)) return f.call(this, u, o);
      const answer = () => new Response(JSON.stringify(plan), { status: 200, headers: { 'content-type': 'application/json' } });
      if (!hold) return Promise.resolve(answer());
      if (/elsewhere-too/.test(decodeURIComponent(url))) return new Promise(() => {});
      return new Promise((res) => { window.__release = () => res(answer()); });
    };
  }, { plan, hold: how === 'other' });
  await logSpins(page);
  try {
    await fx.go(page, '/favourites');
    const plays = page.locator('#favother .list-row [data-dplay]');
    await plays.nth(1).waitFor();
    await page.waitForTimeout(300);
    const menu = page.locator('.popover.menu:not([hidden])');
    if (how === 'other') {
      await plays.nth(0).click(); // (A: its answer is held)
      await page.waitForTimeout(100);
      await plays.nth(1).click(); // (B, pressed while A's answer is still coming)
      await page.waitForTimeout(100);
      await page.evaluate(() => window.__release());
    } else await plays.nth(0).click();
    await menu.waitFor();
    check(/Not on your services/.test(await menu.textContent()), `${tag}: the Play opens "Not on your services"`);
    check((await page.evaluate(() => window.__spins.length)) === 0, `${tag}: nothing spins while the picker is open`);
    if (how === 'late') await page.waitForTimeout(1700);
    if (how === 'space') {
      // (off the Play the click left focused: Space is the zone's play or pause, a press of its own)
      await page.evaluate(() => document.activeElement && document.activeElement.blur());
      await page.keyboard.press('Space');
      await page.waitForTimeout(100);
      check(await menu.isVisible(), `${tag}: the picker is still open after Space`);
    }
    const n = state.hits.length;
    await menu.locator('.pop-item', { hasText: '30 s preview' }).click();
    if (how === 'late') await page.waitForFunction(() => window.__spins.length > 0, null, { timeout: 2000 }).catch(() => {});
    else await page.waitForTimeout(400);
    const spins = await page.evaluate(() => window.__spins.map((s) => s.el));
    if (how === 'late') check(spins.join() === '[data-dplay]', `${tag}: the pick spins the Play pressed (${JSON.stringify(spins)})`);
    else check(spins.length === 0, `${tag}: no Play spins for the pick (${JSON.stringify(spins)})`);
    const src = await page.evaluate(() => document.querySelector('audio') && document.querySelector('audio').getAttribute('src'));
    check(/\/sources\/deezer\/stream\/123/.test(src || ''), `${tag}: the preview plays in this browser (${src})`);
    check(!state.hits.slice(n).some((h) => /^\/zones\//.test(h.v)), `${tag}: no zone is sent anything`);
    await page.screenshot({ path: path.join(SHOTS, `latest-press-picker-${how}-${width}.png`) });
    check(state.errors.length === 0, `${tag}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [1280, 1024, 768, 412])) {
      for (const how of ['menu', 'space']) {
        try { await run(browser, width, how); } catch (e) { fail++; console.log(`FAIL ${width} ${how}: ${e.message}`); }
      }
      for (const how of ['late', 'space', 'other']) {
        try { await picker(browser, width, how); } catch (e) { fail++; console.log(`FAIL ${width} picker ${how}: ${e.message}`); }
      }
    }
  } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})();
