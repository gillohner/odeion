// Play with nothing to play. On "This browser": songs queued while nothing played (Add to queue,
// Play next) start at the first one when Play is pressed (before, the button turned to Pause and
// nothing played); with an empty queue Play, Previous and Next say "The queue is empty" and offer
// Shuffle my music when the library has songs (before, nothing happened at all), without a word
// of it when it has none, nor offline. A node zone with an empty queue (the bar and its card on
// Zones) says the same instead of sending Play, unless its queue as fetched has songs.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-empty-queue.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/empty-queue-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null }));
const fixtures = { '/library/tracks': { body: { tracks, next: null } } };
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };

const bar = (page) => page.evaluate(() => ({ toggle: document.querySelector('#toggle').getAttribute('aria-label'), src: document.querySelector('audio')?.getAttribute('src') || '' }));
const toastNow = (page) => page.evaluate(() => { const t = document.querySelector('#toast'); return { text: t.textContent.trim(), shown: t.classList.contains('show'), act: t.querySelector('.toast-act')?.textContent.trim() || null }; });
const hideToast = (page) => page.evaluate(() => { const t = document.querySelector('#toast'); t.classList.remove('show'); t.textContent = ''; });

async function browserZone(browser, width) {
  const { page, state } = await fx.create(browser, width, { fixtures });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    // 1. Empty queue: Play, Next and Previous each say so, with the shuffle on offer; nothing starts
    for (const id of ['#toggle', '#next', '#prev']) {
      await hideToast(page);
      await page.click(id);
      const t = await toastNow(page);
      assert.ok(t.shown && t.text.startsWith('The queue is empty'), `${width}: ${id} on an empty queue says so (${JSON.stringify(t)})`);
      assert.equal(t.act, 'Shuffle my music', `${width}: ${id} offers Shuffle my music`);
      const b = await bar(page);
      assert.equal(b.toggle, 'Play', `${width}: ${id} leaves Play as it was`);
      assert.equal(b.src, '', `${width}: ${id} starts nothing`);
    }
    await page.screenshot({ path: path.join(shots, `empty-queue-${width}.png`) });
    // 2. Shuffle my music in the note plays the library, shuffled
    const asked = state.hits.length;
    await page.click('#toast .toast-act');
    await page.waitForFunction(() => document.querySelector('audio')?.getAttribute('src'));
    assert.ok(state.hits.slice(asked).some((h) => h.v === '/library/tracks' && /limit=200/.test(h.q)), `${width}: Shuffle my music asks for the library's songs`);
    const after = await bar(page);
    assert.match(after.src, /\/v1\/library\/tracks\/[123]\//, `${width}: a library song is playing (${after.src})`);
    assert.equal(await page.evaluate(() => document.querySelector('#shufflebtn').getAttribute('aria-pressed')), 'true', `${width}: shuffle is on`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await page.context().close(); }
}

async function queuedThenPlay(browser, width) {
  const { page, state } = await fx.create(browser, width, { fixtures });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    // 3. Add to queue (and Play next) with nothing playing, then Play: the first queued song plays
    const row = (i) => page.locator('#tracklist .track').nth(i);
    await row(1).hover(); await row(1).locator('[data-act="add"]').click();
    await row(2).hover(); await row(2).locator('[data-act="next"]').click();
    await sleep(300);
    const before = await bar(page);
    assert.equal(before.src, '', `${width}: queueing alone starts nothing`);
    await hideToast(page);
    await page.click('#toggle');
    await page.waitForFunction(() => document.querySelector('audio')?.getAttribute('src'));
    const b = await bar(page);
    // (Play next put Song 3 first, then Song 2)
    assert.match(b.src, /\/v1\/library\/tracks\/3\//, `${width}: Play starts the queue at its first song (${b.src})`);
    const t = await toastNow(page);
    assert.ok(!/queue is empty/.test(t.text), `${width}: no empty-queue note while songs are queued`);
    await page.screenshot({ path: path.join(shots, `queued-then-play-${width}.png`) });
    // Next goes on to the other queued song
    await page.click('#next');
    await page.waitForFunction(() => /\/tracks\/2\//.test(document.querySelector('audio')?.getAttribute('src') || ''));
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await page.context().close(); }
}

async function noSongs(browser, width) {
  // 4. A library with no songs: the note says what to do, with no shuffle on offer
  const { page } = await fx.create(browser, width, { tracks: [], fixtures: { '/library/tracks': { body: { tracks: [], next: null } } } });
  try {
    await fx.go(page, '/queue');
    await page.waitForSelector('#toggle');
    await page.waitForFunction(() => /0 songs/.test(document.body.textContent));
    await page.click('#toggle');
    const t = await toastNow(page);
    assert.equal(t.text, 'The queue is empty: search for something to play', `${width}: no songs, no shuffle (${JSON.stringify(t)})`);
    assert.equal(t.act, null, `${width}: no button`);
  } finally { await page.context().close(); }
}

async function nodeZone(browser, width) {
  // 5. A node zone with an empty queue: the same note, no Play sent for the zone to refuse
  const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'stopped', now_playing: null, queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/queue': { body: [] }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
  const { page, context, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1' });
  try {
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
    await fx.go(page, '/queue');
    await page.waitForFunction(() => /Kitchen/.test(document.querySelector('#zonebtn')?.textContent || ''));
    for (const id of ['#toggle', '#next', '#prev']) {
      await hideToast(page);
      await page.click(id);
      const t = await toastNow(page);
      assert.equal(t.text.replace(t.act || '', '').trim(), 'The queue is empty', `${width}: ${id} on an empty node zone says so (${JSON.stringify(t)})`);
      assert.equal(t.act, 'Shuffle my music', `${width}: ${id} offers Shuffle my music`);
    }
    // The zone's card on Zones: the same
    await fx.go(page, '/zones');
    const card = page.locator('[data-zcard="z1"]');
    await card.waitFor();
    for (const zt of ['toggle', 'next', 'previous']) {
      await hideToast(page);
      await card.locator(`[data-zt="${zt}"]`).click();
      const t = await toastNow(page);
      assert.ok(t.shown && t.text.startsWith('The queue is empty'), `${width}: the Zones card's ${zt} says so (${JSON.stringify(t)})`);
    }
    await sleep(300);
    assert.equal(state.hits.filter((h) => h.m === 'POST' && /\/zones\/z1\/transport/.test(h.v)).length, 0, `${width}: no transport sent to the empty zone`);
  } finally { await context.close(); }
}

async function nodeZoneLateEvent(browser, width) {
  // 6. The zone's event still says empty, but its queue as fetched has a song (an Add whose event
  // is late): Play goes to the node
  const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'stopped', now_playing: null, queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/queue': { body: [{ id: 1, uri: 'track:1', title: 'Song 1' }] }, '/zones/z1/transport': { body: Z }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
  const { page, context, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1' });
  try {
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
    await fx.go(page, '/queue');
    await page.waitForFunction(() => /Song 1/.test(document.querySelector('main')?.textContent || ''));
    await hideToast(page);
    await page.click('#toggle');
    await sleep(500);
    const posts = state.hits.filter((h) => h.m === 'POST' && /\/zones\/z1\/transport/.test(h.v));
    assert.equal(posts.length, 1, `${width}: Play is sent (${posts.length})`);
    assert.ok(!/queue is empty/.test((await toastNow(page)).text), `${width}: no empty-queue note`);
  } finally { await context.close(); }
}

async function offline(browser, width) {
  // 7. Offline (the Android app with no node): no Shuffle my music, the downloads have no song list
  const { page } = await fx.create(browser, width, { offline: true, fixtures });
  try {
    await fx.go(page, '/queue');
    await page.waitForSelector('#toggle');
    await page.waitForFunction(() => document.querySelector('main')?.textContent.trim());
    await hideToast(page);
    await page.click('#toggle');
    const t = await toastNow(page);
    assert.equal(t.text, 'The queue is empty: play one of your downloads', `${width}: offline note (${JSON.stringify(t)})`);
    assert.equal(t.act, null, `${width}: offline, no button`);
  } finally { await page.context().close(); }
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  try {
    for (const width of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [1280, 1024, 768])) {
      for (const [name, fn] of [['empty browser queue', browserZone], ['queued then Play', queuedThenPlay], ['no songs', noSongs], ['empty node zone', nodeZone], ['node zone, late event', nodeZoneLateEvent], ['offline', offline]]) {
        try { await fn(browser, width); console.log(`PASS ${width} ${name}`); } catch (e) { failed++; console.log(`FAIL ${width} ${name}: ${e.message}`); }
      }
    }
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all passed');
})();
