// This browser's saved place names the song, not only its position in the queue. Right after a
// reload the saved song and its place come back when the node's events connect; a Shuffle or a
// Queue page Remove pressed before that moves the songs, and the place still comes back on the
// saved song (before, whatever now sat at the old index came back from the old position, and the
// saver kept it). Play or Next pressed before that goes on from the saved song too. A saved song no longer in the queue brings nothing back; a place saved before
// the song was named still comes back by its index.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-browser-place.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/browser-place-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null }));
const fixtures = {};
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };
const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));
// A 60 s mono WAV (16 kHz), served with ranges, so the saved place can be seeked to and played
const WAV = (() => { const rate = 16000, n = rate * 60, b = Buffer.alloc(44 + n * 2); b.write('RIFF'); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); return b; })();
const serveWav = (r) => {
  const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || '');
  const a = m ? Number(m[1]) : 0, z = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1;
  return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
};
const until = async (f, label, ms = 7000) => { const t = Date.now(); while (Date.now() - t < ms) { const v = await f(); if (v) return v; await sleep(50); } throw new Error('Timed out: ' + label); };
const now = (page) => page.evaluate(() => pmPlayer.now());
const queueIds = (page) => page.evaluate(() => [...document.querySelectorAll('.queue [data-qrow]')].map((r) => (r.textContent.match(/Song (\d)/) || [])[1]).map(Number));

// A reload with Songs 1-3 saved and the place `at`; the node's events connect only when the test
// says so (connect()), as after a reload on a slow link. `before` runs first (Shuffle, Remove…).
async function reload(browser, width, at, before) {
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true });
  await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify(at)]);
  let sock = null;
  await context.routeWebSocket(/\/v1\/events/, (ws) => { sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); });
  await page.route(/\/v1\/library\/tracks\/\d+\/file/, serveWav);
  const connect = async () => { await until(() => sock, 'the event connection opens'); sock.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); };
  await fx.go(page, '/queue');
  await page.waitForSelector('.queue [data-qrow]');
  await sleep(300);
  assert.equal((await now(page)).index, -1, `${width}: nothing restored yet (the events are not in)`);
  if (before) await before(page);
  await connect();
  return { page, context, state };
}

// The place comes back on Song 3 (now at `index`), paused at 0:30; Play goes on from there and the
// saver keeps naming Song 3
async function backOnSong3(page, width, index, label, shot) {
  await until(async () => (await now(page)).index >= 0, `${width} ${label}: the place comes back`);
  const n = await until(async () => { const v = await now(page); return v.time >= 29 ? v : null; }, `${width} ${label}: the saved position comes back`);
  assert.equal(n.index, index, `${width} ${label}: the bar is on Song 3's row (${n.index})`);
  assert.match(n.src, /\/v1\/library\/tracks\/3\/file/, `${width} ${label}: the bar holds Song 3 (${n.src})`);
  assert.equal(n.paused, true, `${width} ${label}: paused, as it was left`);
  assert.ok(n.time >= 29 && n.time < 31, `${width} ${label}: at 0:30 (${n.time})`);
  await page.click('#toggle');
  const p = await until(async () => { const v = await now(page); return !v.paused && v.time > 30 ? v : null; }, `${width} ${label}: Play plays`);
  assert.match(p.src, /\/v1\/library\/tracks\/3\/file/, `${width} ${label}: Play plays Song 3 (${p.src})`);
  await sleep(2500); // (the saver writes every 2 s)
  const at = await page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserAt') || 'null'));
  assert.equal(at && at.id, 3, `${width} ${label}: the saved place names Song 3 (${JSON.stringify(at)})`);
  assert.equal(at.index, index, `${width} ${label}: at its row now (${JSON.stringify(at)})`);
  assert.ok(at.pos >= 30, `${width} ${label}: from 0:30 or later (${JSON.stringify(at)})`);
  if (shot) await page.screenshot({ path: path.join(shots, `${shot}-${width}.png`) });
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  const check = async (label, f) => { try { await f(); console.log('ok   ' + label); } catch (e) { failed++; console.log('FAIL ' + label + '\n     ' + e.message.split('\n')[0]); } };
  try {
    for (const width of [1280, 768]) {
      // 1. Shuffle before the place comes back (the order fixed for the test: Song 3 moves to row 2)
      await check(`${width}: Shuffle before the restore, the place comes back on the saved song`, async () => {
        const { page, context, state } = await reload(browser, width, { index: 2, id: 3, pos: 30 }, async (pg) => {
          await pg.evaluate(() => { Math.random = () => 0; });
          // (the bar's Shuffle at 1280, the Queue page's where the bar has none)
          await pg.click(width >= 1024 ? '#shufflebtn' : '#qshuffle');
          await until(async () => { const ids = await queueIds(pg); return ids.length === 3 && ids[2] !== 3; }, 'the shuffle moves Song 3 from row 3');
        });
        try {
          const ids = await queueIds(page);
          await backOnSong3(page, width, ids.indexOf(3), 'shuffled', 'shuffled-then-restored');
          assert.deepEqual(state.errors, [], `${width}: no page errors`);
        } finally { await context.close(); }
      });
      // 2. A Queue page Remove before the place comes back: Song 3 is row 2 now, and the old row 3 is gone
      await check(`${width}: Remove before the restore, the place comes back on the saved song`, async () => {
        const { page, context, state } = await reload(browser, width, { index: 2, id: 3, pos: 30 }, async (pg) => {
          await pg.click('[data-qrm="0"]');
          await until(async () => JSON.stringify(await queueIds(pg)) === '[2,3]', 'Song 1 is removed');
        });
        try {
          await backOnSong3(page, width, 1, 'removed', width === 1280 ? 'removed-then-restored' : null);
          assert.deepEqual(state.errors, [], `${width}: no page errors`);
        } finally { await context.close(); }
      });
    }
    // 2b. A Queue page Move before the place comes back: Song 3 moved up to row 2
    await check('1280: Move before the restore, the place comes back on the saved song', async () => {
      const { page, context, state } = await reload(browser, 1280, { index: 2, id: 3, pos: 30 }, async (pg) => {
        await pg.click('[data-qup="2"]');
        await until(async () => JSON.stringify(await queueIds(pg)) === '[1,3,2]', 'Song 3 moves up');
      });
      try {
        await backOnSong3(page, 1280, 1, 'moved');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 2c. Shuffle, then Play or Next, all before the place comes back (the node's events not in yet):
    // Play plays the saved song from 0:30, Next the song after it in the new order; the restore on
    // connection does not take it back, and the saver keeps naming the song that plays
    for (const [button, label] of [['#toggle', 'Play'], ['#next', 'Next']]) {
      await check(`1280: Shuffle then ${label} before the restore goes on from the saved song`, async () => {
        let want = null;
        const { page, context, state } = await reload(browser, 1280, { index: 2, id: 3, pos: 30 }, async (pg) => {
          await pg.evaluate(() => { Math.random = () => 0; });
          await pg.click('#shufflebtn');
          await until(async () => { const ids = await queueIds(pg); return ids.length === 3 && ids[2] !== 3; }, 'the shuffle moves Song 3 from row 3');
          const ids = await queueIds(pg);
          const i = button === '#next' ? ids.indexOf(3) + 1 : ids.indexOf(3);
          assert.ok(i < ids.length, `a song after Song 3 (${ids})`);
          want = { index: i, id: ids[i] };
          await pg.click(button);
          const p = await until(async () => { const v = await now(pg); return !v.paused && v.index >= 0 ? v : null; }, `${label} plays`);
          assert.equal(p.index, want.index, `${label} plays row ${want.index} (${JSON.stringify(p)})`);
          assert.match(p.src, new RegExp(`/v1/library/tracks/${want.id}/file`), `${label} plays Song ${want.id} (${p.src})`);
          if (button === '#toggle') assert.ok(p.time >= 29 && p.time < 33, `Play goes on from 0:30 (${p.time})`);
          else assert.ok(p.time < 5, `Next starts its song from the top (${p.time})`);
        });
        try {
          await sleep(2600); // (the events are in; a saver tick)
          const n = await now(page);
          assert.equal(n.index, want.index, `still on row ${want.index} after the events connect (${JSON.stringify(n)})`);
          assert.equal(n.paused, false, 'still playing');
          const at = await page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserAt') || 'null'));
          assert.equal(at && at.id, want.id, `the saved place names Song ${want.id} (${JSON.stringify(at)})`);
          if (button === '#toggle') await page.screenshot({ path: path.join(shots, 'shuffled-then-play-1280.png') });
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // 3. The saved song itself removed before the place comes back: nothing comes back (not Song 2
    // from 0:30, which sits at the old row now), and the queue is as left
    await check('1280: the saved song removed before the restore brings nothing back', async () => {
      const { page, context, state } = await reload(browser, 1280, { index: 1, id: 2, pos: 30 }, async (pg) => {
        await pg.click('[data-qrm="1"]');
        await until(async () => JSON.stringify(await queueIds(pg)) === '[1,3]', 'Song 2 is removed');
      });
      try {
        await sleep(800);
        const n = await now(page);
        assert.equal(n.index, -1, `nothing comes back (${JSON.stringify(n)})`);
        assert.equal(n.src.includes('/file'), false, `no song is loaded (${n.src})`);
        assert.deepEqual(await queueIds(page), [1, 3], 'the queue is as left');
        await sleep(2200); // (a saver tick: with nothing in the bar the saved place is not rewritten)
        assert.deepEqual(await page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserAt') || 'null')), { index: 1, id: 2, pos: 30 }, 'the saved place is left as it was');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 4. A place saved before the song was named (index and position only) still comes back
    await check('1280: a place saved without the song still comes back by its index', async () => {
      const { page, context, state } = await reload(browser, 1280, { index: 2, pos: 30 });
      try {
        await backOnSong3(page, 1280, 2, 'old place');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 5. A saved index that is not a whole number is not used
    await check('1280: a saved index that is not a whole number brings nothing back', async () => {
      const { page, context, state } = await reload(browser, 1280, { index: '2', pos: 30 });
      try {
        await sleep(800);
        assert.equal((await now(page)).index, -1, 'nothing comes back');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all browser-place checks passed');
})().catch((e) => { console.error(e); process.exit(1); });
