// This browser's player from outside the page and when a start goes wrong.
// - The keyboard's or lock screen's media keys (navigator.mediaSession) take the bar's own steps:
//   right after a reload, before the node's events bring the saved place back, Play goes on with
//   the saved song from where it was and Next moves from it (before, Play did nothing and Next and
//   Previous started song 1); Previous past 0:03 starts the song again, as the bar's does (before,
//   it went to the song before).
// - The automatic retry after a song did not start opens it where it was to start (a resumed song
//   no longer starts again at 0:00), and the retry is not counted as a second play.
// - A retry due after another song was started in its place (a new queue within the 2 s) leaves
//   that song alone: not started again, not counted twice.
// - The empty-queue note's Shuffle my music, pressed while an earlier one still reads the library,
//   starts no second shuffle that would replace the first one's songs; Play meanwhile shows no note.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-browser-keys.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/browser-keys-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null, duration_ms: 60000 }));
const fixtures = { '/library/tracks': { body: { tracks, next: null } } };
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };
const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));

// A 60 s mono WAV (16 kHz), served with ranges, so a place can be seeked to and played
const WAV = (() => { const rate = 16000, n = rate * 60, b = Buffer.alloc(44 + n * 2); b.write('RIFF'); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); return b; })();
const serveWav = (r) => {
  const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || '');
  const a = m ? Number(m[1]) : 0, z = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1;
  return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
};
const toastNow = (page) => page.evaluate(() => { const t = document.querySelector('#toast'); return { text: t.textContent.trim(), shown: t.classList.contains('show'), act: t.querySelector('.toast-act')?.textContent.trim() || null }; });
const hideToast = (page) => page.evaluate(() => { const t = document.querySelector('#toast'); t.classList.remove('show'); t.textContent = ''; });
const now = (page) => page.evaluate(() => pmPlayer.now());
const playing = async (page) => { for (let i = 0; i < 100; i++) { const n = await now(page); if (!n.paused && n.time > 0) return n; await sleep(50); } return now(page); };
const songOf = (n) => Number((/\/v1\/library\/tracks\/(\d+)\/file/.exec(n.src || '') || [])[1] || 0);
const history = (state) => state.hits.filter((h) => h.v === '/library/history');

// The page's media-session handlers, kept where the test can press them (the browser's own stay set)
const keepKeys = () => {
  const ms = navigator.mediaSession; if (!ms) return;
  const keys = {}; const set = ms.setActionHandler.bind(ms);
  ms.setActionHandler = (k, f) => { keys[k] = f; try { set(k, f); } catch { /* unsupported */ } };
  window.__keys = keys;
};
// (what a handler returns is not waited for: the old Play key's play() of nothing never settled)
const press = (page, key) => page.evaluate((k) => { window.__keys[k](); }, key);

// A reload with This browser's queue saved (Song 3 at 0:30, or `at`); the event connection opens and
// stays silent, so nothing is restored by it
async function reloaded(browser, width, at = { index: 2, pos: 30 }, file = serveWav) {
  const run = await fx.create(browser, width, { fixtures, realAudio: true });
  await run.context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify(at)]);
  await run.context.addInitScript(keepKeys);
  await run.context.routeWebSocket(/\/v1\/events/, () => {});
  await run.page.route(/\/v1\/library\/tracks\/\d+\/file/, file);
  await fx.go(run.page, '/queue');
  await run.page.waitForSelector('#toggle');
  await sleep(500);
  assert.equal((await now(run.page)).index, -1, `${width}: nothing restored yet (the events are not in)`);
  assert.equal(await run.page.evaluate(() => typeof (window.__keys && window.__keys.nexttrack)), 'function', `${width}: the page set its media keys`);
  return run;
}

async function playKey(browser, width) {
  // 1. The Play key right after a reload: Song 3 plays from 0:30 (before, nothing played), and is
  // not counted again; then Previous (past 0:03) starts Song 3 again, as the bar's Previous does
  const { page, context, state } = await reloaded(browser, width);
  try {
    await press(page, 'play');
    const n = await playing(page);
    assert.equal(songOf(n), 3, `${width}: the Play key plays the saved Song 3 (${n.src})`);
    assert.equal(n.paused, false, `${width}: it plays`);
    assert.ok(n.time >= 29 && n.time < 40, `${width}: from 0:30 (${n.time})`);
    assert.equal(history(state).length, 0, `${width}: the saved song is not counted again`);
    await page.screenshot({ path: path.join(shots, `play-key-${width}.png`) });
    await press(page, 'previoustrack');
    await sleep(300);
    const p = await now(page);
    assert.equal(songOf(p), 3, `${width}: Previous past 0:03 stays on Song 3 (${p.src})`);
    assert.ok(p.time < 3, `${width}: and starts it again (${p.time})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function nextKey(browser, width) {
  // 2. The Next key right after a reload (Song 1 saved): Song 2 plays (before, Song 1)
  const { page, context, state } = await reloaded(browser, width, { index: 0, pos: 30 });
  try {
    await press(page, 'nexttrack');
    const n = await playing(page);
    assert.equal(songOf(n), 2, `${width}: the Next key plays the song after the saved one (${n.src})`);
    assert.ok(n.time < 10, `${width}: from its start (${n.time})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function previousKey(browser, width) {
  // 3. The Previous key right after a reload (Song 3 saved at 0:30): Song 3 from the top, as the
  // bar's Previous (before, Song 1)
  const { page, context, state } = await reloaded(browser, width);
  try {
    await press(page, 'previoustrack');
    const n = await playing(page);
    assert.equal(songOf(n), 3, `${width}: the Previous key plays the saved Song 3 (${n.src})`);
    assert.ok(n.time < 10, `${width}: from the top (${n.time})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function retryKeepsPlace(browser, width) {
  // 4. Play after a reload while the node is briefly out of reach: the song did not start, it is
  // tried again, and the retry opens Song 3 at 0:30 (before, at 0:00), still not counted
  let failing = true;
  const file = (r) => (failing ? r.abort() : serveWav(r));
  const { page, context, state } = await reloaded(browser, width, { index: 2, pos: 30 }, file);
  try {
    await hideToast(page);
    await page.click('#toggle');
    for (let i = 0; i < 60 && !/Trying again/.test((await toastNow(page)).text); i++) await sleep(50);
    assert.match((await toastNow(page)).text, /Song 3. did not start.*Trying again/, `${width}: says Song 3 did not start and is tried again`);
    failing = false;
    await sleep(2200); // (the retry comes after 2 s)
    const n = await playing(page);
    assert.equal(songOf(n), 3, `${width}: the retry plays Song 3 (${n.src})`);
    assert.equal(n.paused, false, `${width}: it plays`);
    assert.ok(n.time >= 29 && n.time < 40, `${width}: the retry goes on from 0:30 (${n.time})`);
    assert.equal(history(state).length, 0, `${width}: the saved song is not counted (${history(state).length})`);
    await page.screenshot({ path: path.join(shots, `retry-keeps-place-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function retryCountsOnce(browser, width) {
  // 5. A song started from its row whose first start fails: the retry plays it and it is counted
  // once (before, twice)
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true });
  let failing = true;
  await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => (failing ? r.abort() : serveWav(r)));
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    const row = page.locator('#tracklist .track').nth(1);
    await row.hover(); await row.locator('[data-act="play"]').click();
    for (let i = 0; i < 60 && !/Trying again/.test((await toastNow(page)).text); i++) await sleep(50);
    assert.match((await toastNow(page)).text, /did not start.*Trying again/, `${width}: says the song did not start and is tried again`);
    failing = false;
    await sleep(2200);
    const n = await playing(page);
    assert.equal(n.paused, false, `${width}: the retry plays (${n.src})`);
    const h = history(state);
    assert.equal(h.length, 1, `${width}: the song is counted once (${JSON.stringify(h.map((x) => x.body))})`);
    assert.equal(JSON.parse(h[0].body).track_id, songOf(n), `${width}: the playing song is the one counted`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function shuffleOnce(browser, width) {
  // 6. An empty queue in This browser: Play's note, Shuffle my music (the library slow to read),
  // then Play again and the first note's Shuffle my music once more while it reads: no note
  // offering a second shuffle, one read of the library, and the song it starts stays
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true });
  await page.route(/\/v1\/library\/tracks\/\d+\/file/, serveWav);
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    await hideToast(page);
    await page.click('#toggle');
    assert.equal((await toastNow(page)).act, 'Shuffle my music', `${width}: the note offers Shuffle my music`);
    // (the note's button, kept to be pressed again as a second quick press would)
    await page.evaluate(() => { window.__shuffleAgain = document.querySelector('#toast .toast-act'); });
    state.delay = (v) => (v === '/library/tracks' ? 1500 : 0);
    const asked = state.hits.length;
    await page.click('#toast .toast-act');
    await sleep(200);
    await hideToast(page);
    await page.click('#toggle');
    await sleep(100);
    const t = await toastNow(page);
    assert.ok(!(t.shown && /queue is empty/.test(t.text)), `${width}: Play while the shuffle reads shows no empty-queue note (${JSON.stringify(t)})`);
    await page.evaluate(() => window.__shuffleAgain.click());
    await sleep(100);
    assert.ok(!/has songs now/.test((await toastNow(page)).text), `${width}: the second press says nothing of songs not in yet (${JSON.stringify(await toastNow(page))})`);
    state.delay = 0;
    const first = await playing(page);
    assert.ok(songOf(first) > 0, `${width}: the shuffle plays a library song (${first.src})`);
    await sleep(2000);
    const later = await now(page);
    const reads = state.hits.slice(asked).filter((h) => h.v === '/library/tracks' && /limit=200/.test(h.q));
    assert.equal(reads.length, 1, `${width}: the library is read once (${reads.length})`);
    assert.equal(later.src, first.src, `${width}: the song the shuffle started stays (${first.src} -> ${later.src})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function retryStale(browser, width) {
  // 7. A song whose first start fails, and within the 2 s before its retry another song started
  // from its row (the same place in its new queue): the new song keeps playing from where it is,
  // is counted once, and the failed one's retry does nothing (before, it started the new song again
  // at 0:00 and counted it twice)
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true });
  await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => (/\/tracks\/2\/file/.test(r.request().url()) ? r.abort() : serveWav(r)));
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    const playRow = async (n) => { const row = page.locator('#tracklist .track').nth(n); await row.hover(); await row.locator('[data-act="play"]').click(); };
    await playRow(1);
    for (let i = 0; i < 60 && !/Trying again/.test((await toastNow(page)).text); i++) await sleep(50);
    assert.match((await toastNow(page)).text, /Song 2. did not start.*Trying again/, `${width}: says Song 2 did not start and is tried again`);
    const said = Date.now();
    await playRow(2);
    // (inside the 2 s: a slow runner that got here after the retry would pass without the check)
    assert.ok(Date.now() - said < 1700, `${width}: Song 3 started before Song 2's retry was due (${Date.now() - said} ms)`);
    const n = await playing(page);
    assert.equal(songOf(n), 3, `${width}: Song 3 plays (${n.src})`);
    assert.equal(n.index, 0, `${width}: at the place Song 2 had (${n.index})`);
    await sleep(2600); // (past the failed song's retry)
    const later = await now(page);
    assert.equal(songOf(later), 3, `${width}: Song 3 is still on (${later.src})`);
    assert.equal(later.paused, false, `${width}: and plays`);
    assert.ok(later.time > n.time + 1.5, `${width}: on from where it was, not started again (${n.time} -> ${later.time})`);
    const h3 = history(state).filter((x) => JSON.parse(x.body).track_id === 3);
    assert.equal(h3.length, 1, `${width}: Song 3 is counted once (${h3.length})`);
    await page.screenshot({ path: path.join(shots, `retry-stale-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  try {
    for (const width of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [1280, 1024, 768])) {
      for (const [name, fn] of [['Play key after a reload', playKey], ['Next key after a reload', nextKey], ['Previous key after a reload', previousKey], ['retry keeps the place', retryKeepsPlace], ['retry counted once', retryCountsOnce], ['one shuffle at a time', shuffleOnce], ['a stale retry leaves the new song', retryStale]]) {
        try { await Promise.race([fn(browser, width), sleep(60000).then(() => { throw new Error('timed out after 60 s'); })]); console.log(`PASS ${width} ${name}`); } catch (e) { failed++; console.log(`FAIL ${width} ${name}: ${e.message}`); }
      }
    }
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all passed');
})();
