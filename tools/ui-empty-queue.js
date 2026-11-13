// Play with nothing to play. On "This browser": songs queued while nothing played (Add to queue,
// Play next) start at the first one when Play is pressed (before, the button turned to Pause and
// nothing played); with an empty queue Play, Previous and Next say "The queue is empty" and offer
// Shuffle my music when the library has songs (before, nothing happened at all), without a word
// of it when it has none, nor offline. A node zone with an empty queue (the bar and its card on
// Zones) says the same instead of sending Play, unless its queue as fetched has songs; Shuffle my
// music then plays on that zone (a card's, This browser's), whichever zone is in use when it starts.
// A paused zone still takes Play; another zone's fetched queue doesn't count for the one in use.
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

// Two node zones and This browser for 8-10: Kitchen, the zone in use, has a song queued (stopped);
// Study has nothing; Hall is paused with nothing in its queue and nothing playing
const zoneBase = { transport: 'stopped', now_playing: null, queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
const Z1 = { ...zoneBase, id: 'z1', name: 'Kitchen', output: 'upnp:k', queue: [{ id: 1, uri: 'track:1', title: 'Song 1' }], queue_index: 0 };
const Z2 = { ...zoneBase, id: 'z2', name: 'Study', output: 'upnp:s' };
const Z3 = { ...zoneBase, id: 'z3', name: 'Hall', output: 'upnp:h', transport: 'paused' };
const twoZones = async (browser, width, zones, extra = {}) => {
  const zf = { ...fixtures, '/zones': { body: zones }, '/outputs': { body: zones.map((z) => ({ id: z.output, name: z.name, kind: 'upnp', caps: { hw_volume: 'percent' } })) } };
  for (const z of zones) Object.assign(zf, { [`/zones/${z.id}`]: { body: z }, [`/zones/${z.id}/queue`]: { body: z.queue }, [`/zones/${z.id}/load`]: { body: { ...z, transport: 'playing' } }, [`/zones/${z.id}/transport`]: { body: z } });
  const run = await fx.create(browser, width, { fixtures: { ...zf, ...extra }, zone: 'z1' });
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
  return run;
};
const zonePosts = (state, zid, from = 0) => state.hits.slice(from).filter((h) => h.m === 'POST' && h.v.startsWith(`/zones/${zid}/`)).map((h) => h.v);

async function shuffleOnTheCardsZone(browser, width) {
  // 8. Zones, a card that is not the zone in use: its Play on an empty queue, then Shuffle my music,
  // plays on that card's zone; the zone in use keeps its queue. The same for This browser's card.
  const { page, context, state } = await twoZones(browser, width, [Z1, Z2]);
  try {
    await fx.go(page, '/zones');
    await page.locator('[data-zcard="z2"]').waitFor();
    await hideToast(page);
    await page.locator('[data-zcard="z2"] [data-zt="toggle"]').click();
    let t = await toastNow(page);
    assert.equal(t.act, 'Shuffle my music', `${width}: Study's Play on an empty queue offers Shuffle my music (${JSON.stringify(t)})`);
    await page.screenshot({ path: path.join(shots, `card-note-${width}.png`) });
    await page.click('#toast .toast-act');
    for (let i = 0; i < 40 && !zonePosts(state, 'z2').length; i++) await sleep(50);
    assert.deepEqual(zonePosts(state, 'z2'), ['/zones/z2/load'], `${width}: Shuffle my music loads Study`);
    assert.deepEqual(zonePosts(state, 'z1'), [], `${width}: Kitchen, the zone in use, is left as it was`);
    assert.ok(t.text.startsWith('The queue is empty on Study'), `${width}: the note names Study, not the zone in use (${JSON.stringify(t)})`);
    await page.screenshot({ path: path.join(shots, `card-shuffle-${width}.png`) });
    // This browser's card while Kitchen is in use: the shuffle plays here, in this browser
    const from = state.hits.length;
    await hideToast(page);
    await page.locator('[data-zcard="browser"] [data-zt="toggle"]').click();
    t = await toastNow(page);
    assert.equal(t.act, 'Shuffle my music', `${width}: This browser's Play on an empty queue offers Shuffle my music (${JSON.stringify(t)})`);
    await page.click('#toast .toast-act');
    await page.waitForFunction(() => document.querySelector('audio')?.getAttribute('src'));
    await sleep(300);
    assert.deepEqual(zonePosts(state, 'z1', from), [], `${width}: This browser's shuffle leaves Kitchen as it was`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function shuffleKeepsItsZone(browser, width) {
  // 9. Shuffle my music for Study, and Kitchen chosen while the library's songs are still being
  // read: the songs go to Study, the zone it was offered for
  const { page, context, state } = await twoZones(browser, width, [Z1, Z2]);
  try {
    await fx.go(page, '/zones');
    await page.locator('[data-zcard="z2"] [data-zsel="z2"]').click();
    await page.waitForFunction(() => /Study/.test(document.querySelector('#zonebtn')?.textContent || ''));
    await sleep(300);
    await hideToast(page);
    await page.click('#toggle');
    assert.equal((await toastNow(page)).act, 'Shuffle my music', `${width}: Study's Play on the bar offers Shuffle my music`);
    state.delay = (v) => (v === '/library/tracks' ? 1500 : 0);
    await page.click('#toast .toast-act');
    await page.locator('[data-zcard="z1"] [data-zsel="z1"]').click();
    await page.waitForFunction(() => /Kitchen/.test(document.querySelector('#zonebtn')?.textContent || ''));
    for (let i = 0; i < 100 && !zonePosts(state, 'z2').length; i++) await sleep(50);
    assert.deepEqual(zonePosts(state, 'z2'), ['/zones/z2/load'], `${width}: the songs go to Study`);
    assert.deepEqual(zonePosts(state, 'z1'), [], `${width}: Kitchen, chosen meanwhile, is left as it was`);
  } finally { await context.close(); }
}

async function pausedAndSwitched(browser, width) {
  // 10. A paused zone with nothing in its queue still takes Play/Pause (sent, no note); and the queue
  // fetched for Kitchen does not count for Study after a switch, while Study's own is on its way
  const late = { ...Z1, queue: [] }; // (Kitchen's event says empty; its fetched queue has Song 1)
  const { page, context, state } = await twoZones(browser, width, [late, Z2, Z3], { '/zones/z1/queue': { body: [{ id: 1, uri: 'track:1', title: 'Song 1' }] } });
  try {
    await fx.go(page, '/zones');
    await page.locator('[data-zcard="z3"]').waitFor();
    await hideToast(page);
    await page.locator('[data-zcard="z3"] [data-zt="toggle"]').click();
    for (let i = 0; i < 40 && !zonePosts(state, 'z3').length; i++) await sleep(50);
    assert.equal(zonePosts(state, 'z3').filter((v) => v === '/zones/z3/transport').length, 1, `${width}: the paused zone's Play is sent`);
    assert.ok(!/queue is empty/.test((await toastNow(page)).text), `${width}: no empty-queue note for a paused zone`);
    // (Kitchen's queue, Song 1, is fetched when the page opens: Kitchen is the zone in use)
    for (let i = 0; i < 40 && !state.hits.some((h) => h.v === '/zones/z1/queue'); i++) await sleep(50);
    await sleep(200);
    state.delay = (v) => (v === '/zones/z2/queue' ? 1500 : 0);
    await fx.go(page, '/zones');
    await page.locator('[data-zcard="z2"] [data-zsel="z2"]').click();
    await page.waitForFunction(() => /Study/.test(document.querySelector('#zonebtn')?.textContent || ''));
    await hideToast(page);
    await page.click('#toggle');
    const t = await toastNow(page);
    assert.ok(t.text.startsWith('The queue is empty'), `${width}: Study's Play says its queue is empty, Kitchen's songs aside (${JSON.stringify(t)})`);
    await sleep(300);
    assert.deepEqual(zonePosts(state, 'z2'), [], `${width}: nothing sent to Study`);
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
      for (const [name, fn] of [['empty browser queue', browserZone], ['queued then Play', queuedThenPlay], ['no songs', noSongs], ['empty node zone', nodeZone], ['node zone, late event', nodeZoneLateEvent], ['shuffle on the card\'s zone', shuffleOnTheCardsZone], ['shuffle keeps its zone', shuffleKeepsItsZone], ['paused zone, switched zone', pausedAndSwitched], ['offline', offline]]) {
        try { await fn(browser, width); console.log(`PASS ${width} ${name}`); } catch (e) { failed++; console.log(`FAIL ${width} ${name}: ${e.message}`); }
      }
    }
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all passed');
})();
