// The Queue page with nothing queued offers Shuffle my music when the library has songs, the same
// button as the empty-queue note of the player's Play (before, the page only said "The queue is
// empty": Play in the player, then the note's button, 2 taps). One tap plays the library shuffled
// on the zone the page is for (This browser, or a node zone), and the page then lists the songs.
// No button with no songs in the library, nor offline (the downloads have no song list to shuffle),
// nor on a node zone not known to be empty (its queue could not be fetched, or it is paused on a
// song): one tap must not replace music it still has. Pressed after songs came, it says so.
// On a phone the button is a full 44 px tap target.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-queue-empty-shuffle.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/queue-empty-shuffle-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null }));
const fixtures = { '/library/tracks': { body: { tracks, next: null } } };
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };

// The empty state's words and its button, as drawn
const emptyNow = (page) => page.evaluate(() => {
  const e = document.querySelector('#main .empty'); const b = document.querySelector('#main .empty #qmine');
  const r = b && b.getBoundingClientRect();
  return { shown: !!e, title: e ? e.querySelector('h3')?.textContent.trim() : null, button: b ? b.textContent.trim() : null, h: r ? r.height : 0, inView: r ? r.top >= 0 && r.bottom <= innerHeight : false };
});
// (drawn: the empty state, the rows, or the notice of a queue that could not be read)
const queueReady = (page) => page.waitForFunction(() => document.querySelector('#main .empty, #main .list.queue, #main [data-failed]') && /^Queue/.test(document.querySelector('#main .page-head h2')?.textContent || ''));

async function browserZone(browser, width) {
  // 1. This browser, nothing queued: the page offers Shuffle my music; one tap plays the library
  const { page, state } = await fx.create(browser, width, { fixtures, mobile: width < 600 });
  try {
    await fx.go(page, '/queue');
    await queueReady(page);
    const e = await emptyNow(page);
    assert.equal(e.title, 'The queue is empty', `${width}: the page says the queue is empty (${JSON.stringify(e)})`);
    assert.equal(e.button, 'Shuffle my music', `${width}: and offers Shuffle my music (${JSON.stringify(e)})`);
    assert.ok(e.inView, `${width}: the button is on screen without scrolling`);
    if (width < 600) assert.ok(e.h >= 44, `${width}: a 44 px tap target on a phone (${e.h})`);
    await page.screenshot({ path: path.join(shots, `browser-empty-${width}.png`) });
    const asked = state.hits.length;
    await page.click('#qmine');
    await page.waitForFunction(() => document.querySelector('audio')?.getAttribute('src'));
    assert.ok(state.hits.slice(asked).some((h) => h.v === '/library/tracks' && /limit=200/.test(h.q)), `${width}: one tap asks for the library's songs`);
    assert.match(await page.evaluate(() => document.querySelector('audio').getAttribute('src')), /\/v1\/library\/tracks\/[123]\//, `${width}: a library song is playing`);
    assert.equal(await page.evaluate(() => document.querySelector('#shufflebtn')?.getAttribute('aria-pressed')), 'true', `${width}: shuffle is on`);
    await page.waitForFunction(() => document.querySelectorAll('#main .list.queue .list-row').length === 3);
    assert.equal(await page.locator('#main .empty').count(), 0, `${width}: the page lists the songs, the empty state is gone`);
    await page.screenshot({ path: path.join(shots, `browser-shuffled-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await page.context().close(); }
}

async function nodeZone(browser, width) {
  // 2. A node zone with nothing queued: the button shuffles onto that zone
  const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'stopped', now_playing: null, queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/queue': { body: [] }, '/zones/z1/load': { body: { ...Z, transport: 'playing' } }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
  const { page, context, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1', mobile: width < 600 });
  try {
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
    await fx.go(page, '/queue');
    await queueReady(page);
    await page.waitForFunction(() => /Kitchen/.test(document.querySelector('#main .page-head')?.textContent || ''));
    const e = await emptyNow(page);
    assert.equal(e.button, 'Shuffle my music', `${width}: the zone's empty queue offers Shuffle my music (${JSON.stringify(e)})`);
    await page.screenshot({ path: path.join(shots, `zone-empty-${width}.png`) });
    const asked = state.hits.length;
    await page.click('#qmine');
    for (let i = 0; i < 60 && !state.hits.slice(asked).some((h) => h.m === 'POST' && h.v === '/zones/z1/load'); i++) await sleep(50);
    const load = state.hits.slice(asked).find((h) => h.m === 'POST' && h.v === '/zones/z1/load');
    assert.ok(load, `${width}: the shuffle goes to Kitchen (${JSON.stringify(state.hits.slice(asked).map((h) => h.m + ' ' + h.v))})`);
    const sent = JSON.parse(load.body);
    assert.equal(sent.items.length, 3, `${width}: with the library's songs (${load.body})`);
    assert.equal(sent.shuffle, true, `${width}: shuffled`);
    assert.equal(await page.evaluate(() => document.querySelector('audio')?.getAttribute('src') || ''), '', `${width}: nothing plays in this browser`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function zoneNotKnownEmpty(browser, width, what, Z, queueAnswer) {
  // 5-6. A node zone that draws no rows but may have music to lose: its queue could not be fetched,
  // or it is paused on a song with nothing after it. No button, as the player's note gives none
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/queue': queueAnswer, '/zones/z1/load': { body: { ...Z, transport: 'playing' } }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
  const { page, context, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1', mobile: width < 600 });
  try {
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
    await fx.go(page, '/queue');
    await queueReady(page);
    await page.waitForFunction(() => /Kitchen/.test(document.querySelector('#main .page-head')?.textContent || ''));
    for (let i = 0; i < 40 && !state.hits.some((h) => h.v === '/zones/z1/queue'); i++) await sleep(50);
    await sleep(300);
    const e = await emptyNow(page);
    assert.equal(e.button, null, `${width}: ${what}: no Shuffle my music (${JSON.stringify(e)})`);
    // (a queue that could not be read is not called empty: the page says so, with Try again)
    if (queueAnswer.status === 500) assert.deepEqual(await page.evaluate(() => [!!document.querySelector('#main .empty'), /Couldn.t read the queue/.test(document.querySelector('#main [data-failed]')?.textContent || ''), !!document.querySelector('#main [data-failed] [data-retry]')]), [false, true, true], `${width}: ${what}: said, not shown as an empty queue`);
    assert.equal(state.hits.filter((h) => h.m === 'POST' && h.v === '/zones/z1/load').length, 0, `${width}: ${what}: nothing replaced`);
  } finally { await context.close(); }
}
const zoneBase = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'stopped', now_playing: null, queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
const fetchFailed = (b, w) => zoneNotKnownEmpty(b, w, 'the queue could not be fetched', zoneBase, { status: 500, body: { error: 'busy' } });
const pausedOnASong = (b, w) => zoneNotKnownEmpty(b, w, 'paused on a song', { ...zoneBase, transport: 'paused', now_playing: { item: { id: 1, uri: 'track:1', title: 'Song 1', artist: 'Sefa', duration_ms: 240000 }, index: 0, position_ms: 1000, duration_ms: 240000 } }, { body: [] });

async function noSongs(browser, width) {
  // 3. A library with no songs: the words only, no button
  const { page } = await fx.create(browser, width, { tracks: [], fixtures: { '/library/tracks': { body: { tracks: [], next: null } } }, mobile: width < 600 });
  try {
    await fx.go(page, '/queue');
    await queueReady(page);
    const e = await emptyNow(page);
    assert.ok(e.shown && e.button === null, `${width}: no songs, no Shuffle my music (${JSON.stringify(e)})`);
  } finally { await page.context().close(); }
}

async function offline(browser, width) {
  // 4. Offline (the Android app with no node): no button either
  const { page } = await fx.create(browser, width, { offline: true, fixtures, mobile: width < 600 });
  try {
    await fx.go(page, '/queue');
    await page.waitForFunction(() => document.querySelector('#main')?.textContent.trim());
    await sleep(300);
    assert.equal(await page.locator('#qmine').count(), 0, `${width}: offline, no Shuffle my music`);
  } finally { await page.context().close(); }
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  try {
    for (const width of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [360, 412, 1280])) {
      for (const [name, fn] of [['This browser', browserZone], ['node zone', nodeZone], ['queue not fetched', fetchFailed], ['paused on a song', pausedOnASong], ['no songs', noSongs], ['offline', offline]]) {
        try { await fn(browser, width); console.log(`PASS ${width} ${name}`); } catch (e) { failed++; console.log(`FAIL ${width} ${name}: ${e.message}`); }
      }
    }
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all passed');
})();
