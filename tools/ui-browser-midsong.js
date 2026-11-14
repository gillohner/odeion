// This browser's player when a song breaks off mid-song (a dropped connection): it picks the song
// up where it was, once per play.
// - A song started from its row in the 1.5 s before the pick-up (the same place in its new queue)
//   keeps playing: the pick-up does not put the broken song back under it (before, it did, and the
//   bar named the new song while the old one played).
// - A later song that breaks off at the same place in the queue is picked up too (before, a song
//   at a place where an earlier one had broken off was never picked up).
// - The same song started again from the Queue in the 1.5 s before the pick-up plays from its
//   start (before, the pick-up pulled it back to where it broke off), and is picked up itself when
//   it breaks off later.
// - Pause pressed in the 1.5 s before the pick-up stays paused (before, 1.5 s later the song went
//   back to where it broke off and played); Previous past 0:03 and a seek there play from the place
//   asked for (before, they were pulled back to where the song broke off).
// - An earlier song taken out of the queue in the 1.5 s doesn't stop the pick-up (before, the song
//   on, now one place up, was never picked up).
// - A saved song that doesn't open at all where it was (the node sent no audio) is opened again
//   once, by the start's own retry (before, the pick-up opened it a second time as well).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-browser-midsong.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/browser-midsong-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null, duration_ms: 60000 }));
const fixtures = { '/library/tracks': { body: { tracks, next: null } } };
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };

// A 60 s mono WAV (16 kHz), served with ranges, so a place can be seeked to and played
const WAV = (() => { const rate = 16000, n = rate * 60, b = Buffer.alloc(44 + n * 2); b.write('RIFF'); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); return b; })();
const serveWav = (r) => {
  const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || '');
  const a = m ? Number(m[1]) : 0, z = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1;
  return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
};
const now = (page) => page.evaluate(() => pmPlayer.now());
const playing = async (page) => { for (let i = 0; i < 100; i++) { const n = await now(page); if (!n.paused && n.time > 0) return n; await sleep(50); } return now(page); };
const past = async (page, s) => { for (let i = 0; i < 100; i++) { const n = await now(page); if (!n.paused && n.time > s) return n; await sleep(50); } return now(page); };
const songOf = (src) => Number((/\/v1\/library\/tracks\/(\d+)\/file/.exec(src || '') || [])[1] || 0);

// Every song the page's <audio> is given, in order (the pick-up gives it the broken song again)
const watchSrc = () => {
  const d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
  window.__srcs = [];
  Object.defineProperty(HTMLMediaElement.prototype, 'src', { configurable: true, get() { return d.get.call(this); }, set(v) { window.__srcs.push(String(v)); d.set.call(this, v); } });
};
const srcs = (page) => page.evaluate(() => window.__srcs.slice());
// The stream breaking off, as the page's player hears it
const breakOff = (page) => page.evaluate(() => document.querySelector('#browser').dispatchEvent(new Event('error')));

async function open(browser, width) {
  const run = await fx.create(browser, width, { fixtures, realAudio: true });
  await run.context.addInitScript(watchSrc);
  await run.page.route(/\/v1\/library\/tracks\/\d+\/file/, serveWav);
  await fx.go(run.page, '/tracks');
  await run.page.waitForSelector('#tracklist .track');
  run.playRow = async (n) => { const row = run.page.locator('#tracklist .track').nth(n); await row.hover(); await row.locator('[data-act="play"]').click(); };
  return run;
}

async function staleMidSong(browser, width) {
  // 1. Song 2 breaks off mid-song, and before its pick-up is due Song 3 is started from its row
  // (the same place in its new queue): Song 3 keeps playing on from where it is and the broken
  // Song 2 is not put back (before, Song 2 played again under Song 3's name on the bar)
  const { page, context, state, playRow } = await open(browser, width);
  try {
    await playRow(1);
    const a = await past(page, 1);
    assert.equal(songOf(a.src), 2, `${width}: Song 2 plays (${a.src})`);
    await breakOff(page);
    const said = Date.now();
    await playRow(2);
    // (inside the 1.5 s: a slow runner that got here after the pick-up would pass without the check)
    assert.ok(Date.now() - said < 1200, `${width}: Song 3 started before Song 2's pick-up was due (${Date.now() - said} ms)`);
    const n = await playing(page);
    assert.equal(songOf(n.src), 3, `${width}: Song 3 plays (${n.src})`);
    assert.equal(n.index, 0, `${width}: at the place Song 2 had (${n.index})`);
    const given = (await srcs(page)).length;
    await sleep(2200); // (past the broken song's pick-up)
    const later = await now(page);
    assert.equal(songOf(later.src), 3, `${width}: Song 3 is still on (${later.src})`);
    assert.equal(later.paused, false, `${width}: and plays`);
    assert.ok(later.time > n.time + 1, `${width}: on from where it was (${n.time} -> ${later.time})`);
    assert.deepEqual((await srcs(page)).slice(given), [], `${width}: the player is given no other song`);
    assert.equal(await page.locator('#np .np-title').textContent(), 'Song 3', `${width}: the bar names Song 3`);
    await page.screenshot({ path: path.join(shots, `stale-mid-song-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function laterSongPickedUp(browser, width) {
  // 2. Song 2 breaks off and is picked up where it was; then Song 3 is started from its row (the
  // same place in its new queue) and breaks off too: it is picked up as well (before, never)
  const { page, context, state, playRow } = await open(browser, width);
  try {
    await playRow(1);
    await past(page, 1);
    let given = (await srcs(page)).length;
    await breakOff(page);
    await sleep(1900);
    const up = await playing(page);
    assert.deepEqual((await srcs(page)).slice(given).map(songOf), [2], `${width}: Song 2 is picked up`);
    assert.equal(songOf(up.src), 2, `${width}: and plays (${up.src})`);
    assert.ok(up.time >= 1, `${width}: from where it broke off (${up.time})`);
    await playRow(2);
    const n = await past(page, 1);
    assert.equal(songOf(n.src), 3, `${width}: Song 3 plays (${n.src})`);
    assert.equal(n.index, 0, `${width}: at the place Song 2 had (${n.index})`);
    given = (await srcs(page)).length;
    await breakOff(page);
    await sleep(1900);
    const later = await playing(page);
    assert.deepEqual((await srcs(page)).slice(given).map(songOf), [3], `${width}: Song 3 is picked up too`);
    assert.equal(songOf(later.src), 3, `${width}: and plays (${later.src})`);
    assert.ok(later.time >= 1, `${width}: from where it broke off (${later.time})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function samePlayedAgain(browser, width) {
  // 3. Song 2 breaks off at 0:04, and before its pick-up is due it is started again from the Queue:
  // it plays from its start and is not pulled back to 0:04 (before, it was); when it breaks off
  // later in that new play, it is picked up (before, never)
  const { page, context, state, playRow } = await open(browser, width);
  try {
    await playRow(1);
    const a = await past(page, 4);
    assert.equal(songOf(a.src), 2, `${width}: Song 2 plays (${a.src})`);
    await page.evaluate(() => document.querySelector('.nav-item[data-view="queue"]').click());
    await page.waitForSelector('[data-qi="0"]');
    await breakOff(page);
    const said = Date.now();
    await page.evaluate(() => document.querySelector('[data-qi="0"]').click());
    assert.ok(Date.now() - said < 1200, `${width}: started again before the pick-up was due (${Date.now() - said} ms)`);
    await sleep(1900); // (past the pick-up)
    const n = await playing(page);
    assert.equal(songOf(n.src), 2, `${width}: Song 2 plays (${n.src})`);
    assert.ok(n.time < 3.5, `${width}: from its start, not pulled back to 0:04 (${n.time})`);
    await past(page, 2.5);
    const given = (await srcs(page)).length;
    await breakOff(page);
    await sleep(1900);
    const later = await playing(page);
    assert.deepEqual((await srcs(page)).slice(given).map(songOf), [2], `${width}: it is picked up when it breaks off again`);
    assert.ok(later.time >= 2.5, `${width}: from where it broke off (${later.time})`);
    await page.screenshot({ path: path.join(shots, `same-played-again-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function pausedStaysPaused(browser, width) {
  // 4. Song 2 breaks off, and before its pick-up is due Pause is pressed on the bar: it stays paused
  // where it is and is given to the player no more (before, 1.5 s later it played again)
  const { page, context, state, playRow } = await open(browser, width);
  try {
    await playRow(1);
    await past(page, 1);
    const given = (await srcs(page)).length;
    await breakOff(page);
    const said = Date.now();
    await page.click('#toggle');
    assert.ok(Date.now() - said < 1200, `${width}: paused before the pick-up was due (${Date.now() - said} ms)`);
    const p = await now(page);
    assert.equal(p.paused, true, `${width}: Pause pauses`);
    await sleep(2200); // (past the pick-up)
    const later = await now(page);
    assert.equal(later.paused, true, `${width}: still paused after the pick-up was due`);
    assert.ok(Math.abs(later.time - p.time) < 0.3, `${width}: where it was paused (${p.time} -> ${later.time})`);
    assert.deepEqual((await srcs(page)).slice(given), [], `${width}: the song is not opened again`);
    await page.screenshot({ path: path.join(shots, `paused-stays-paused-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function previousInTheWait(browser, width) {
  // 5. Song 2 breaks off at 0:04 and Previous is pressed before the pick-up is due: it plays from
  // its start (before, 1.5 s later it went back to 0:04)
  const { page, context, state, playRow } = await open(browser, width);
  try {
    await playRow(1);
    await past(page, 4);
    await breakOff(page);
    const said = Date.now();
    await page.click('#prev');
    assert.ok(Date.now() - said < 1200, `${width}: Previous before the pick-up was due (${Date.now() - said} ms)`);
    await sleep(2200); // (past the pick-up)
    const n = await playing(page);
    assert.equal(songOf(n.src), 2, `${width}: Song 2 plays (${n.src})`);
    assert.ok(n.time < 3.5, `${width}: from its start, not pulled back to 0:04 (${n.time})`);
    await page.screenshot({ path: path.join(shots, `previous-in-the-wait-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function seekInTheWait(browser, width) {
  // 5b. Song 2 breaks off at 0:02 and the bar's seek is moved to 0:30 before the pick-up is due:
  // the song is opened again at 0:30 at once and plays on from there (before, 1.5 s later it went
  // back to 0:02)
  const { page, context, state, playRow } = await open(browser, width);
  try {
    await playRow(1);
    await past(page, 2);
    const given = (await srcs(page)).length;
    await breakOff(page);
    const said = Date.now();
    await page.evaluate(() => { const s = document.querySelector('#seek'); s.value = '500'; s.dispatchEvent(new Event('change')); });
    assert.ok(Date.now() - said < 1200, `${width}: the seek before the pick-up was due (${Date.now() - said} ms)`);
    await sleep(2200); // (past the pick-up)
    const later = await playing(page);
    assert.equal(songOf(later.src), 2, `${width}: Song 2 still plays (${later.src})`);
    assert.ok(later.time >= 30 && later.time < 34, `${width}: on from 0:30, not pulled back to 0:02 (${later.time})`);
    assert.deepEqual((await srcs(page)).slice(given).map(songOf), [2], `${width}: opened again once, at the seek`);
    await page.screenshot({ path: path.join(shots, `seek-in-the-wait-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function earlierSongRemoved(browser, width) {
  // 6. Song 2 (second in the queue) breaks off, and before its pick-up is due Song 1 is removed
  // from the Queue: Song 2, now first, is still picked up (before, it never was)
  const { page, context, state, playRow } = await open(browser, width);
  try {
    await playRow(0);
    await playing(page);
    await page.click('#next');
    const a = await past(page, 1);
    assert.equal(songOf(a.src), 2, `${width}: Song 2 plays (${a.src})`);
    assert.equal(a.index, 1, `${width}: second in the queue (${a.index})`);
    await page.evaluate(() => document.querySelector('.nav-item[data-view="queue"]').click());
    await page.waitForSelector('[data-qrm="0"]', { state: 'attached' });
    const given = (await srcs(page)).length;
    await breakOff(page);
    const said = Date.now();
    await page.evaluate(() => document.querySelector('[data-qrm="0"]').click());
    assert.ok(Date.now() - said < 1200, `${width}: removed before the pick-up was due (${Date.now() - said} ms)`);
    await sleep(1900);
    const up = await playing(page);
    assert.equal(up.index, 0, `${width}: Song 2 is first now (${up.index})`);
    assert.deepEqual((await srcs(page)).slice(given).map(songOf), [2], `${width}: Song 2 is picked up`);
    assert.ok(up.time >= 1, `${width}: from where it broke off (${up.time})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function savedNotOpened(browser, width) {
  // 7. After a reload Song 2 was left at 0:30, and its file comes back as an error page: Play opens
  // it, the start's retry opens it once more, and nothing else does (before, the pick-up opened it
  // a third time)
  const run = await fx.create(browser, width, { fixtures, realAudio: true });
  const { page, context, state } = run;
  try {
    const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: '', album: '', duration_ms: 60000 }));
    await context.addInitScript(watchSrc);
    await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 1, id: 2, pos: 30 })]);
    // (the node's events never connect: Play goes on from the saved place)
    await context.routeWebSocket(/\/v1\/events/, () => {});
    await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => r.fulfill({ status: 200, contentType: 'text/html', body: '<html>busy</html>' }));
    await fx.go(page, '/queue');
    await page.waitForSelector('.queue [data-qrow]');
    await page.click('#toggle');
    await sleep(3500); // (past the pick-up and the start's retry)
    const opened = (await srcs(page)).map(songOf).filter((n) => n === 2).length;
    assert.equal(opened, 2, `${width}: Song 2 opened twice, by Play and the start's retry (${opened})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  try {
    for (const width of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [1280, 1024, 768])) {
      for (const [name, fn] of [['a song started meanwhile keeps playing', staleMidSong], ['a later song at the same place is picked up', laterSongPickedUp], ['the same song started again', samePlayedAgain], ['Pause in the wait stays paused', pausedStaysPaused], ['Previous in the wait', previousInTheWait], ['a seek in the wait', seekInTheWait], ['an earlier song removed in the wait', earlierSongRemoved], ['a saved song that does not open', savedNotOpened]]) {
        try { await Promise.race([fn(browser, width), sleep(60000).then(() => { throw new Error('timed out after 60 s'); })]); console.log(`PASS ${width} ${name}`); } catch (e) { failed++; console.log(`FAIL ${width} ${name}: ${e.message}`); }
      }
    }
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all passed');
})();
