// This browser's saved place names the song, not only its position in the queue. Right after a
// reload the saved song and its place come back when the node's events connect; a Shuffle or a
// Queue page Remove pressed before that moves the songs, and the place still comes back on the
// saved song (before, whatever now sat at the old index came back from the old position, and the
// saver kept it). Play or Next pressed before that goes on from the saved song too. A saved song no longer in the queue brings nothing back; a place saved before
// the song was named still comes back by its index. A queue change is saved at once when the page
// is hidden or goes (not only every 2 s), except after the session ended. A saved song whose file
// doesn't open on the reload (the node busy) keeps its place, and Play starts it from there with the
// start's own retry and message (before, Play did nothing and said nothing, and the place was lost).
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
    // 6. The queue is saved at once when the page goes (a reload, a phone's app ended in the
    // background), not only every 2 s: a song removed just before is still gone after the reload.
    // (Songs 1-3 are seeded once per tab, so the reload reads what the page itself saved.)
    const seededOnce = async (width, opts = {}) => {
      const r = await fx.create(browser, width, { fixtures, ...opts });
      await r.context.addInitScript((q) => { if (!sessionStorage.getItem('seeded')) { sessionStorage.setItem('seeded', '1'); localStorage.setItem('pm.browserQueue', q); } }, JSON.stringify(saved));
      await fx.go(r.page, '/queue');
      await r.page.waitForSelector('.queue [data-qrow]');
      await until(async () => (await queueIds(r.page)).length === 3, 'Songs 1-3 are in the queue');
      await sleep(2300); // (a saver tick has written Songs 1-3)
      return r;
    };
    const savedIds = (page) => page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').map((x) => x.id));
    for (const width of [1280, 360]) {
      await check(`${width}: a song removed just before a reload is still gone after it`, async () => {
        const { page, context, state } = await seededOnce(width, width < 600 ? { mobile: true } : {});
        try {
          const t0 = Date.now();
          // (a phone's Remove is in the row's ⋯)
          if (width < 600) { await page.locator('.list.queue [data-qmore]').first().tap(); await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from queue' }).tap(); } else await page.click('[data-qrm="0"]');
          await until(async () => JSON.stringify(await queueIds(page)) === '[2,3]', 'Song 1 is removed');
          assert.deepEqual(await savedIds(page), [1, 2, 3], 'no saver tick has saved the Remove yet');
          await page.reload();
          assert.ok(Date.now() - t0 < 1500, `the reload came before a saver tick (${Date.now() - t0} ms)`);
          await page.waitForSelector('.queue [data-qrow]');
          await sleep(300);
          assert.deepEqual(await queueIds(page), [2, 3], `${width}: the queue after the reload is as left`);
          if (width === 360) await page.screenshot({ path: path.join(shots, 'removed-then-reloaded-360.png') });
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    await check('1280: a song removed just before the page is hidden is saved at once', async () => {
      const { page, context, state } = await seededOnce(1280);
      try {
        await page.click('[data-qrm="0"]');
        await until(async () => JSON.stringify(await queueIds(page)) === '[2,3]', 'Song 1 is removed');
        const ids = await page.evaluate(() => {
          Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
          document.dispatchEvent(new Event('visibilitychange'));
          return JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').map((x) => x.id);
        });
        assert.deepEqual(ids, [2, 3], `saved as soon as the page is hidden (${JSON.stringify(ids)})`);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 7. A session that ends forgets the account's queue: the save as the page goes does not write
    // it back for whoever logs in next
    await check('1280: an ended session\'s queue is not saved back as the page reloads', async () => {
      const { page, context } = await seededOnce(1280);
      try {
        assert.deepEqual(await savedIds(page), [1, 2, 3], 'Songs 1-3 saved before');
        await page.route(/\/v1\/library\/tracks(\?|$)/, (r) => r.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify({ error: 'missing or wrong bearer token' }) }), { times: 1 });
        // (a change not saved yet, so the save as the page goes would have something to write)
        await page.click('[data-qrm="0"]');
        await until(async () => JSON.stringify(await queueIds(page)) === '[2,3]', 'Song 1 is removed');
        await page.evaluate(() => { window.oldPage = true; });
        await page.click('.nav-item[data-view="tracks"]');
        // (the page reloads on the 401; read the storage from the new page, before its own first save)
        await until(() => page.evaluate(() => !window.oldPage && document.readyState !== 'loading').catch(() => false), 'the page reloads');
        const ids = await savedIds(page);
        assert.deepEqual(ids, [], `the old queue is gone after the reload (${JSON.stringify(ids)})`);
      } finally { await context.close(); }
    });
    // 8. The saved song's file doesn't open on the reload (the node busy: 503). The saved place stays
    // 0:30 through saver ticks; Play asks for the file again at once and says so ("did not start …
    // Trying again"), and once the node answers the song plays from 0:30. Not counted again. On a
    // phone too (the bar's Play is the mini player's).
    for (const width of [1280, 360]) {
      await check(`${width}: a saved song whose file failed on the reload plays from its place on Play`, async () => {
        const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true, ...(width < 600 ? { mobile: true } : {}) });
        await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 1, id: 2, pos: 30 })]);
        let busy = true; let asked = 0;
        await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => { asked++; return busy ? r.fulfill({ status: 503, contentType: 'text/plain', body: 'busy' }) : serveWav(r); });
        try {
          await fx.go(page, '/queue');
          await until(async () => (await now(page)).index === 1, 'Song 2 comes back to the bar');
          await until(() => page.evaluate(() => !!document.querySelector('#browser').error), 'its file does not open');
          await sleep(2300); // (a saver tick)
          assert.deepEqual(await page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserAt') || 'null')), { index: 1, id: 2, pos: 30 }, 'the saved place is still 0:30');
          // (the bar says where Play will start it: 0:30, half way along, not 0:00)
          const bar = await page.evaluate(() => ({ pos: document.querySelector('#pos').textContent, seek: Number(document.querySelector('#seek').value) }));
          assert.equal(bar.pos, '0:30', `the bar shows 0:30 (${JSON.stringify(bar)})`);
          assert.ok(bar.seek >= 490 && bar.seek <= 510, `the seek slider is half way (${JSON.stringify(bar)})`);
          if (width === 360) await page.screenshot({ path: path.join(shots, 'failed-restore-bar-360.png') });
          const before = asked;
          await page.click('#toggle');
          await until(async () => asked > before, 'Play asks for the file again');
          const t = await until(() => page.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).find((x) => /did not start/.test(x))), 'Play says the song did not start');
          assert.match(t, /Song 2.*503.*Trying again/, `the message names the song and the node's answer (${t})`);
          busy = false; // (the node answers again; the retry 2 s later opens it)
          const p = await until(async () => { const v = await now(page); return !v.paused && v.time > 30 ? v : null; }, 'the song plays', 9000);
          assert.equal(p.index, 1, `Song 2 plays (${JSON.stringify(p)})`);
          assert.ok(p.time < 34, `from 0:30 (${p.time})`);
          if (width === 360) await page.screenshot({ path: path.join(shots, 'failed-restore-then-play-360.png') });
          assert.equal(state.hits.filter((h) => h.v === '/library/history' && h.m === 'POST').length, 0, 'not counted as a second play');
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // 8b. A saved song whose file opens (as usual) and plays is an ordinary song from then on: when its
    // stream breaks off later (here at about 0:33), Pause then Play go on from where it broke, not
    // from the place it came back at (0:30). The file is served in pieces up to 0:33, the next piece
    // answers late with a 503; then the node answers again.
    await check('1280: a saved song that opened and played, then broke off, goes on from where it broke', async () => {
      const { page, context, state } = await fx.create(browser, 1280, { fixtures, realAudio: true });
      await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 1, id: 2, pos: 30 })]);
      const CUT = 44 + 32000 * 33; let healed = false;
      await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => {
        const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || ''); const a = m ? Number(m[1]) : 0;
        if (!healed && a >= CUT) return sleep(4000).then(() => r.fulfill({ status: 503, contentType: 'text/plain', body: 'busy' })).catch(() => {});
        const end = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1; const z = healed ? end : Math.min(end, a + 200000, CUT - 1);
        return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
      });
      try {
        await fx.go(page, '/queue');
        await until(async () => (await now(page)).time >= 29, 'Song 2 comes back at 0:30');
        await page.click('#toggle');
        await until(() => page.evaluate(() => { const a = document.querySelector('#browser'); return !!a.error && a.currentTime > 31; }), 'the stream breaks off after 0:31', 20000);
        healed = true;
        if (!(await now(page)).paused) { await page.click('#toggle'); await sleep(200); }
        const broke = (await now(page)).time;
        await page.click('#toggle');
        // (its first moment playing again: about where it broke, or about 0:30)
        const p = await until(async () => { const v = await now(page); return !v.paused && v.time > 0 ? v : null; }, 'Play plays it again', 9000);
        assert.ok(broke > 31.5 && p.time >= broke - 1, `from where it broke (${broke.toFixed(1)} s), not 0:30 (${p.time.toFixed(1)} s)`);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 8c. A seek on a saved song whose file didn't open moves where Play starts it (and the saved place)
    await check('1280: a seek on a saved song whose file failed moves where Play starts it', async () => {
      const { page, context, state } = await fx.create(browser, 1280, { fixtures, realAudio: true });
      await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 1, id: 2, pos: 30 })]);
      let busy = true;
      await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => (busy ? r.fulfill({ status: 503, contentType: 'text/plain', body: 'busy' }) : serveWav(r)));
      try {
        await fx.go(page, '/queue');
        await until(() => page.evaluate(() => pmPlayer.now().index === 1 && !!document.querySelector('#browser').error), 'Song 2 comes back, its file not opening');
        // (the bar's seek slider to 0:10 of the song's 1:00; the lock screen's seekto takes the same step)
        const seeked = await page.evaluate(() => { const s = document.querySelector('#seek'); if (!s) return false; s.value = String(Math.round(1000 * 10 / 60)); s.dispatchEvent(new Event('input', { bubbles: true })); s.dispatchEvent(new Event('change', { bubbles: true })); return s.value; });
        assert.ok(seeked, 'the bar has a seek slider');
        await sleep(2300); // (a saver tick)
        const at = await page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserAt') || 'null'));
        assert.ok(at && at.pos >= 8 && at.pos <= 12, `the saved place follows the seek to about 0:10 (${JSON.stringify(at)})`);
        busy = false;
        await page.click('#toggle');
        const p = await until(async () => { const v = await now(page); return !v.paused && v.time > 8 ? v : null; }, 'Play plays it');
        assert.ok(p.time < 14, `from about 0:10, not 0:30 (${p.time.toFixed(1)} s)`);
        // (and it stays there once the file is open: not pulled back to 0:30 by the restore)
        await sleep(1200);
        const q = await now(page);
        assert.ok(!q.paused && q.time > p.time && q.time < 15, `still going on from 0:10 a second later (${q.time.toFixed(1)} s)`);
        await sleep(2300); // (a saver tick)
        const kept = await page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserAt') || 'null'));
        assert.ok(kept && kept.pos < 16, `the saved place follows the song (${JSON.stringify(kept)})`);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 9. Move here (this browser) from a zone paused at 0:30 whose song's file doesn't open here (the
    // node busy: 503): the bar shows 0:30, Play asks for the file again and says so, and once the node
    // answers the song plays from 0:30 (before, the bar showed 0:00 and Play did nothing, said nothing)
    for (const width of [1280, 360]) {
      await check(`${width}: a song moved here paused whose file failed plays from its place on Play`, async () => {
        const q = saved.map(({ uri, title, artist, album, duration_ms }) => ({ uri, title, artist, album, duration_ms }));
        const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'paused', now_playing: { item: q[1], index: 1, position_ms: 30000, duration_ms: 60000 }, queue: q, queue_index: 1, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
        const zf = { '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
        const { page, context, state } = await fx.create(browser, width, { fixtures: { ...fixtures, ...zf }, realAudio: true, ...(width < 600 ? { mobile: true } : {}) });
        await context.addInitScript(() => localStorage.setItem('pm.zone', 'z1'));
        await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
        let busy = true; let asked = 0;
        await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => { asked++; return busy ? r.fulfill({ status: 503, contentType: 'text/plain', body: 'busy' }) : serveWav(r); });
        // (the Queue page reads Kitchen's queue; the move empties it)
        await page.route(/\/v1\/zones\/z1\/queue(\?|$)/, (r) => r.fulfill({ contentType: 'application/json', body: JSON.stringify(r.request().method() === 'DELETE' ? {} : q) }));
        try {
          await fx.go(page, '/queue');
          await until(() => page.evaluate(() => document.querySelector('#zonename').textContent === 'Kitchen'), 'Kitchen is the zone');
          await page.waitForSelector(width >= 600 ? '#zonebtn' : '#qtransfer');
          // (a desktop's way: the zone menu's Move here; a phone's: the Queue page's Move to…)
          if (width >= 600) { await page.click('#zonebtn'); await page.locator('#zonepop [data-move="browser"]').click(); } else { await page.click('#qtransfer'); await page.locator('.pop-item[data-mi]', { hasText: /this|browser/i }).first().click(); }
          await until(async () => (await now(page)).index === 1, 'Song 2 is in this browser\'s bar');
          await until(() => page.evaluate(() => !!document.querySelector('#browser').error), 'its file does not open');
          if (width >= 600) await page.keyboard.press('Escape');
          await sleep(300);
          const bar = await page.evaluate(() => ({ pos: document.querySelector('#pos').textContent, label: document.querySelector('#toggle').getAttribute('aria-label') }));
          assert.deepEqual(bar, { pos: '0:30', label: 'Play' }, `the bar shows Song 2 paused at 0:30 (${JSON.stringify(bar)})`);
          if (width === 360) await page.screenshot({ path: path.join(shots, 'failed-move-bar-360.png') });
          const before = asked;
          await page.click('#toggle');
          await until(async () => asked > before, 'Play asks for the file again');
          const t = await until(() => page.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).find((x) => /did not start/.test(x))), 'Play says the song did not start');
          assert.match(t, /Song 2.*503.*Trying again/, `the message names the song and the node's answer (${t})`);
          busy = false;
          const p = await until(async () => { const v = await now(page); return !v.paused && v.time > 30 ? v : null; }, 'the song plays', 9000);
          assert.equal(p.index, 1, `Song 2 plays (${JSON.stringify(p)})`);
          assert.ok(p.time < 34, `from 0:30 (${p.time})`);
          if (width === 360) await page.screenshot({ path: path.join(shots, 'failed-move-then-play-360.png') });
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // 8d. Previous on a saved song held at 0:30 whose file didn't open goes back to its start, as
    // on any song past 0:03 (before, it went to the song before: the bar said 0:30, the player 0:00)
    await check('1280: Previous on a saved song whose file failed goes back to its start', async () => {
      const { page, context, state } = await fx.create(browser, 1280, { fixtures, realAudio: true });
      await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 1, id: 2, pos: 30 })]);
      await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => r.fulfill({ status: 503, contentType: 'text/plain', body: 'busy' }));
      try {
        await fx.go(page, '/queue');
        await until(() => page.evaluate(() => pmPlayer.now().index === 1 && !!document.querySelector('#browser').error), 'Song 2 comes back, its file not opening');
        await page.click('#prev');
        await sleep(300);
        const after = await page.evaluate(() => ({ index: pmPlayer.now().index, pos: document.querySelector('#pos').textContent }));
        assert.deepEqual(after, { index: 1, pos: '0:00' }, `still Song 2, at its start (${JSON.stringify(after)})`);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 9b. A saved song whose file didn't open on the reload, moved to a zone: the zone gets it paused at
    // its place (before, stopped at 0:00: the place was lost on the way)
    await check('1280: a saved song whose file failed, moved to a zone, goes there paused at its place', async () => {
      const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'stopped', now_playing: null, queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
      const zf = { '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
      const { page, context, state } = await fx.create(browser, 1280, { fixtures: { ...fixtures, ...zf }, realAudio: true });
      await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 1, id: 2, pos: 30 })]);
      await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
      await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => r.fulfill({ status: 503, contentType: 'text/plain', body: 'busy' }));
      let sent = null;
      await page.route(/\/v1\/zones\/z1\/load(\?|$)/, (r) => { sent = JSON.parse(r.request().postData() || 'null'); return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...Z, transport: 'paused', queue: sent.items, queue_index: sent.index, now_playing: { item: sent.items[sent.index], index: sent.index, position_ms: sent.position_ms, duration_ms: 60000 } }) }); });
      try {
        await fx.go(page, '/queue');
        await until(() => page.evaluate(() => pmPlayer.now().index === 1 && !!document.querySelector('#browser').error), 'Song 2 comes back, its file not opening');
        await page.click('#zonebtn');
        await page.locator('#zonepop [data-move="z1"]').click();
        await until(() => sent, 'the move is asked for');
        assert.deepEqual({ index: sent.index, position_ms: sent.position_ms, transport: sent.transport }, { index: 1, position_ms: 30000, transport: 'paused' }, `Kitchen gets Song 2 paused at 0:30 (${JSON.stringify(sent)})`);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all browser-place checks passed');
})().catch((e) => { console.error(e); process.exit(1); });
