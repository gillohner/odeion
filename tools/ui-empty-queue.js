// Play with nothing to play. On "This browser": songs queued while nothing played (Add to queue,
// Play next) start at the first one when Play is pressed (before, the button turned to Pause and
// nothing played); with an empty queue Play, Previous and Next say "The queue is empty" and offer
// Shuffle my music when the library has songs (before, nothing happened at all), without a word
// of it when it has none, nor offline. A node zone with an empty queue (the bar and its card on
// Zones) says the same instead of sending Play, unless its queue as fetched has songs; Shuffle my
// music then plays on that zone (a card's, This browser's), whichever zone is in use when it starts.
// A paused zone still takes Play; another zone's fetched queue doesn't count for the one in use, and
// with none fetched for the zone in use yet, or an older answer landing late, Play goes to the node.
// The note names the zone whenever there is more than one place to play.
// After a reload, Play, Next and Previous in This browser pressed before the node's events are in
// (or with the node out of reach) go on from the saved song and place, not song 1. The note's Shuffle
// my music checks again when pressed, and a queue change of this app's on its way counts as songs.
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
    for (let i = 0; i < 40 && !state.hits.some((h) => h.v === '/zones/z1/queue'); i++) await sleep(50);
    await sleep(200);
    for (const id of ['#toggle', '#next', '#prev']) {
      await hideToast(page);
      await page.click(id);
      const t = await toastNow(page);
      assert.equal(t.text.replace(t.act || '', '').trim(), 'The queue is empty on Kitchen', `${width}: ${id} on an empty node zone says so (${JSON.stringify(t)})`);
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
    assert.ok(t.text.startsWith('The queue is empty on This browser'), `${width}: the note names This browser (${JSON.stringify(t)})`);
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
  // 10. A paused zone with nothing in its queue still takes Play/Pause (sent, no note); after a switch
  // to Study, Kitchen's fetched queue does not count for it: while Study's own is on its way Play goes
  // to the node (no note to tempt a shuffle); once it is in, empty, Play says so
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
    for (let i = 0; i < 40 && !zonePosts(state, 'z2').length; i++) await sleep(50);
    assert.deepEqual(zonePosts(state, 'z2'), ['/zones/z2/transport'], `${width}: Study's Play, its queue not yet in, is sent`);
    assert.ok(!/queue is empty/.test((await toastNow(page)).text), `${width}: no empty-queue note before Study's queue is in`);
    state.delay = 0;
    await sleep(1700);
    await hideToast(page);
    await page.click('#toggle');
    const t = await toastNow(page);
    assert.ok(t.text.startsWith('The queue is empty on Study'), `${width}: Study's Play says its queue is empty, Kitchen's songs aside (${JSON.stringify(t)})`);
    await sleep(300);
    assert.deepEqual(zonePosts(state, 'z2'), ['/zones/z2/transport'], `${width}: nothing more sent to Study`);
  } finally { await context.close(); }
}

async function lateQueueAnswer(browser, width) {
  // 11. Kitchen's queue answer held back and landing after a newer one: it is dropped. (a) Switched to
  // Study (stopped, its event late: empty; its fetched queue has a song): Play on Study is sent, no
  // note. (b) Back to Kitchen, whose newer fetch has a song: the held, older, empty answer landing
  // last doesn't make Play say "empty" and offer a shuffle that would replace Kitchen's song.
  const song = [{ id: 1, uri: 'track:1', title: 'Song 1' }];
  const k = { ...Z1, queue: [], queue_index: null }; // (both zones' events late: empty)
  const { page, context, state } = await twoZones(browser, width, [k, Z2], { '/zones/z2/queue': { body: song } });
  let held = 0, heldSent = 0;
  // (waits until the held answer has reached the page, so Play is pressed after it, not on a clock)
  const heldLanded = async (n) => { for (let i = 0; i < 200 && heldSent < n; i++) await sleep(50); assert.equal(heldSent, n, `${width}: the held answer was sent`); await sleep(300); };
  await page.route('**/v1/zones/z1/queue', async (r) => {
    // The first of Kitchen's fetches answers 2.5 s late with an old, empty queue; later ones at once
    if (held++ === 0) { await sleep(2500); await r.fulfill({ status: 200, contentType: 'application/json', body: '[]' }).catch(() => {}); heldSent++; return; }
    return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(song) });
  });
  try {
    await fx.go(page, '/zones');
    await page.locator('[data-zcard="z2"] [data-zsel="z2"]').click();
    await page.waitForFunction(() => /Study/.test(document.querySelector('#zonebtn')?.textContent || ''));
    await heldLanded(1);
    await hideToast(page);
    await page.click('#toggle');
    for (let i = 0; i < 40 && !zonePosts(state, 'z2').length; i++) await sleep(50);
    assert.deepEqual(zonePosts(state, 'z2'), ['/zones/z2/transport'], `${width}: Study's Play is sent, Kitchen's late answer aside`);
    assert.ok(!/queue is empty/.test((await toastNow(page)).text), `${width}: no empty-queue note on Study`);
    // (b) Reloaded on Kitchen (its fetch held), to Study and back: a newer fetch has the song
    held = 0;
    await fx.go(page, '/zones');
    await page.waitForFunction(() => /Kitchen/.test(document.querySelector('#zonebtn')?.textContent || ''));
    await page.locator('[data-zcard="z2"] [data-zsel="z2"]').click();
    await page.waitForFunction(() => /Study/.test(document.querySelector('#zonebtn')?.textContent || ''));
    await page.locator('[data-zcard="z1"] [data-zsel="z1"]').click();
    await page.waitForFunction(() => /Kitchen/.test(document.querySelector('#zonebtn')?.textContent || ''));
    assert.ok(held >= 2, `${width}: Kitchen's queue was asked for again while the first answer was held (${held})`);
    await heldLanded(2);
    await hideToast(page);
    await page.click('#toggle');
    for (let i = 0; i < 40 && !zonePosts(state, 'z1').length; i++) await sleep(50);
    assert.deepEqual(zonePosts(state, 'z1'), ['/zones/z1/transport'], `${width}: Kitchen's Play is sent, its older empty answer dropped`);
    assert.ok(!/queue is empty/.test((await toastNow(page)).text), `${width}: no empty-queue note on Kitchen`);
    await page.screenshot({ path: path.join(shots, `late-queue-answer-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

// A 60 s mono WAV (16 kHz), served with ranges, so the saved place can be seeked to and played
const WAV = (() => { const rate = 16000, n = rate * 60, b = Buffer.alloc(44 + n * 2); b.write('RIFF'); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); return b; })();
const serveWav = (r) => {
  const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || '');
  const a = m ? Number(m[1]) : 0, z = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1;
  return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
};

async function playAfterReload(browser, width, button) {
  // 12. A reload with a saved queue in This browser (Song 3 at 0:30) and Play (or Next) pressed
  // before the node's events connect: Play goes on with Song 3 from 0:30 and the saved place still
  // names it; Next goes to the song after it; Previous (past 0:03) starts it again from the top.
  // (Before, all three started Song 1 and the saver kept that.)
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true });
  const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));
  await context.addInitScript(([q]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', JSON.stringify({ index: 2, pos: 30 })); }, [JSON.stringify(saved)]);
  if (button === '#next') await context.addInitScript(([q]) => { localStorage.setItem('pm.browserAt', JSON.stringify({ index: 0, pos: 30 })); }, [null]);
  // (the event connection opens and stays silent: no snapshot, so nothing is restored by it)
  await context.routeWebSocket(/\/v1\/events/, () => {});
  await page.route(/\/v1\/library\/tracks\/\d+\/file/, serveWav);
  try {
    await fx.go(page, '/queue');
    await page.waitForSelector('#toggle');
    await sleep(500);
    assert.equal(await page.evaluate(() => pmPlayer.now().index), -1, `${width}: nothing restored yet (the events are not in)`);
    await hideToast(page);
    await page.click(button);
    const want = button === '#next' ? 2 : 3;
    for (let i = 0; i < 100; i++) { const n = await page.evaluate(() => pmPlayer.now()); if (!n.paused && n.time > 0) break; await sleep(50); }
    const now = await page.evaluate(() => pmPlayer.now());
    assert.match(now.src, new RegExp(`/v1/library/tracks/${want}/file`), `${width}: ${button} plays Song ${want} (${now.src})`);
    assert.equal(now.paused, false, `${width}: ${button}: it plays`);
    if (button === '#toggle') assert.ok(now.time >= 29 && now.time < 40, `${width}: Play goes on from 0:30 (${now.time})`);
    if (button === '#prev') assert.ok(now.time < 10, `${width}: Previous starts Song 3 from the top (${now.time})`);
    await sleep(2500); // (the saver writes every 2 s)
    const at = await page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserAt') || 'null'));
    assert.equal(at && at.index, want - 1, `${width}: the saved place names Song ${want} (${JSON.stringify(at)})`);
    if (button === '#toggle') assert.ok(at.pos >= 30, `${width}: the saved position is 0:30 or later (${JSON.stringify(at)})`);
    if (button === '#toggle') assert.ok(!state.hits.some((h) => h.v === '/library/history'), `${width}: the saved song, counted when it first started, is not counted again`);
    assert.ok(!/queue is empty/.test((await toastNow(page)).text), `${width}: no empty-queue note`);
    if (button === '#toggle') await page.screenshot({ path: path.join(shots, `play-after-reload-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function playAfterReloadNoNode(browser, width) {
  // 12b. The same with the node out of reach (the song's file can't be read): Play says the song
  // did not start, and the saved place still names Song 3 at 0:30 (not 0:00)
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true });
  const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));
  await context.addInitScript(([q]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', JSON.stringify({ index: 2, pos: 30 })); }, [JSON.stringify(saved)]);
  await context.routeWebSocket(/\/v1\/events/, () => {});
  await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => r.abort());
  try {
    await fx.go(page, '/queue');
    await page.waitForSelector('#toggle');
    await sleep(500);
    await hideToast(page);
    await page.click('#toggle');
    for (let i = 0; i < 60 && !/did not start|can.t play/.test((await toastNow(page)).text); i++) await sleep(50);
    assert.match((await toastNow(page)).text, /Song 3. did not start/, `${width}: Play says Song 3 did not start (${JSON.stringify(await toastNow(page))})`);
    await sleep(4500); // (two saver ticks and the one retry)
    const at = await page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserAt') || 'null'));
    assert.deepEqual(at, { index: 2, pos: 30 }, `${width}: the saved place is kept (${JSON.stringify(at)})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function shuffleRechecked(browser, width) {
  // 13. The empty-queue note stays a while: a song queued on the zone meanwhile (from another device)
  // is not replaced by the note's Shuffle my music, which says the queue has songs now instead
  const Z = { ...zoneBase, id: 'z1', name: 'Kitchen', output: 'upnp:k' };
  const song = { id: 1, uri: 'track:1', title: 'Song 1' };
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/queue': { body: [] }, '/zones/z1/load': { body: Z }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
  const { page, context, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1' });
  let sock = null;
  await context.routeWebSocket(/\/v1\/events/, (ws) => { sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
  try {
    await fx.go(page, '/queue');
    await page.waitForFunction(() => /Kitchen/.test(document.querySelector('#zonebtn')?.textContent || ''));
    for (let i = 0; i < 40 && !state.hits.some((h) => h.v === '/zones/z1/queue'); i++) await sleep(50);
    await sleep(200);
    await hideToast(page);
    await page.click('#toggle');
    assert.equal((await toastNow(page)).act, 'Shuffle my music', `${width}: the note offers Shuffle my music`);
    // (another device adds a song while the note is up)
    zf['/zones/z1/queue'].body = [song];
    sock.send(JSON.stringify({ type: 'zone', seq: 1, state: { ...Z, queue: [song], queue_index: 0, queue_version: 2 } }));
    await page.waitForFunction(() => /Song 1/.test(document.querySelector('main')?.textContent || ''));
    const asked = state.hits.length;
    await page.click('#toast .toast-act');
    await sleep(400);
    assert.ok(!state.hits.slice(asked).some((h) => h.v === '/library/tracks' && /limit=200/.test(h.q)), `${width}: no shuffle asked for`);
    assert.deepEqual(zonePosts(state, 'z1', asked), [], `${width}: Kitchen's queue is not replaced`);
    assert.equal((await toastNow(page)).text, 'The queue has songs now on Kitchen: press Play', `${width}: says the queue has songs now`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function pendingAdd(browser, width) {
  // 14. A node zone with an empty queue and this app's Add to queue still on its way: Play is sent
  // to the node (no note offering a shuffle that would replace the song being added)
  const Z = { ...zoneBase, id: 'z2', name: 'Study', output: 'upnp:s' };
  const { page, context, state } = await twoZones(browser, width, [{ ...Z, id: 'z1', name: 'Kitchen', output: 'upnp:k' }], { '/zones/z1/queue': { body: [] } });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    for (let i = 0; i < 40 && !state.hits.some((h) => h.v === '/zones/z1/queue'); i++) await sleep(50);
    await sleep(200);
    state.delay = (v, m) => (v === '/zones/z1/queue' && m === 'POST' ? 3000 : 0);
    const row = page.locator('#tracklist .track').nth(0);
    await row.hover(); await row.locator('[data-act="add"]').click();
    await sleep(200);
    await hideToast(page);
    await page.click('#toggle');
    await sleep(200);
    assert.ok(!/queue is empty/.test((await toastNow(page)).text), `${width}: no empty-queue note while the Add is on its way`);
    assert.equal(zonePosts(state, 'z1').filter((v) => v === '/zones/z1/transport').length, 1, `${width}: Play is sent (${JSON.stringify(zonePosts(state, 'z1'))})`);
    state.delay = 0;
    await sleep(3100);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
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
      for (const [name, fn] of [['empty browser queue', browserZone], ['queued then Play', queuedThenPlay], ['no songs', noSongs], ['empty node zone', nodeZone], ['node zone, late event', nodeZoneLateEvent], ['shuffle on the card\'s zone', shuffleOnTheCardsZone], ['shuffle keeps its zone', shuffleKeepsItsZone], ['paused zone, switched zone', pausedAndSwitched], ['late queue answer', lateQueueAnswer], ['Play after a reload', (b, w) => playAfterReload(b, w, '#toggle')], ['Next after a reload', (b, w) => playAfterReload(b, w, '#next')], ['Previous after a reload', (b, w) => playAfterReload(b, w, '#prev')], ['Play after a reload, node out of reach', playAfterReloadNoNode], ['Shuffle checked again', shuffleRechecked], ['Add on its way', pendingAdd], ['offline', offline]]) {
        try { await fn(browser, width); console.log(`PASS ${width} ${name}`); } catch (e) { failed++; console.log(`FAIL ${width} ${name}: ${e.message}`); }
      }
    }
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all passed');
})();
