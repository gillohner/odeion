// The Queue page follows the song that plays without being drawn again: when only the song on
// changes (the next one starts here, or the node's zone moves on), its row's mark moves in place, so
// a focused row button (keyboard, TalkBack) keeps its focus and a drag in progress still lands.
// Before, every song change redrew the whole page: focus went back to the top and a drag was cut
// off (the row was gone, the drop never came). A change of the songs themselves still redraws.
// Song rows elsewhere (.track) say which one plays to a screen reader too (aria-current).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-queue-mark.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/queue-mark-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (f, label, ms = 7000) => { const t = Date.now(); while (Date.now() - t < ms) { const v = await f(); if (v) return v; await sleep(50); } throw new Error('Timed out: ' + label); };
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null }));
const fixtures = {};
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };
const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));
const WAV = (() => { const rate = 8000, n = rate * 60, b = Buffer.alloc(44 + n * 2); b.write('RIFF'); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); return b; })();
const serveWav = (r) => {
  const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || '');
  const a = m ? Number(m[1]) : 0, z = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1;
  return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
};
const now = (page) => page.evaluate(() => pmPlayer.now());
const queueIds = (page) => page.evaluate(() => [...document.querySelectorAll('.queue [data-qrow]')].map((r) => Number((r.textContent.match(/Song (\d)/) || [])[1])));
// Every row's mark, as the page draws it for the song on: .cur, aria-current, the play icon in
// place of its number, and the title in the accent colour; '-' for a row not marked at all
const marks = (page) => page.evaluate(() => {
  const list = document.querySelector('.queue'); if (!list) return [];
  const probe = document.createElement('span'); probe.style.color = 'var(--accent-strong)'; list.append(probe);
  const accent = getComputedStyle(probe).color; probe.remove();
  return [...document.querySelectorAll('.queue [data-qrow]')].map((r) => {
    const num = r.querySelector('[data-qi] .num');
    const on = [r.classList.contains('cur'), r.getAttribute('aria-current') === 'true', !!r.querySelector('[data-qi] svg') && !num, getComputedStyle(r.querySelector('.name')).color === accent];
    if (on.every((x) => !x)) return num && num.textContent === String(Number(r.dataset.qrow) + 1) ? '-' : 'no number';
    return on.every(Boolean) ? 'on' : `part ${on.map(Number).join('')}`;
  });
});
// The queue list is the one drawn at first (not drawn again), and where the focus is
const tag = (page) => page.evaluate(() => { document.querySelector('.list.queue').dataset.first = '1'; });
const sameList = (page) => page.evaluate(() => document.querySelector('.list.queue')?.dataset.first === '1');
// How many times a queue list was put on the page since it loaded
const countDraws = (context) => context.addInitScript(() => { window.__queueDraws = 0; new MutationObserver((ms) => { for (const m of ms) for (const a of m.addedNodes) if (a.nodeType === 1 && (a.matches('.list.queue') || (a.querySelector && a.querySelector('.list.queue')))) window.__queueDraws++; }).observe(document, { childList: true, subtree: true }); });
const draws = (page) => page.evaluate(() => window.__queueDraws);
const focused = (page) => page.evaluate(() => { const a = document.activeElement; return a && a.dataset ? (a.dataset.qmore != null ? 'qmore ' + a.dataset.qmore : a.dataset.qgrip != null ? 'qgrip ' + a.dataset.qgrip : a.id || a.tagName) : null; });

// This browser plays Songs 1-3 (the place saved on Song 1), the Queue page open
async function browserQueue(browser, width, opts = {}) {
  const run = await fx.create(browser, width, { fixtures, realAudio: true, tracks, ...opts });
  await run.context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 0, id: 1, pos: 0 })]);
  await run.page.route(/\/v1\/library\/tracks\/\d+\/file/, serveWav);
  await fx.go(run.page, '/queue');
  await until(async () => (await now(run.page)).index === 0, `${width}: Song 1's place comes back`);
  await until(async () => (await marks(run.page)).join() === 'on,-,-', `${width}: Song 1's row is marked`);
  return run;
}
const next = (page) => page.evaluate(() => document.querySelector('#next').click());

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  const check = async (label, f) => { try { await f(); console.log('ok   ' + label); } catch (e) { failed++; console.log('FAIL ' + label + '\n     ' + e.message.split('\n')[0]); } };
  try {
    // 1. This browser goes on to the next song while a row's ⋯ has the focus: the mark moves to
    // Song 2's row in place, and the focus stays on that ⋯ (before: the page was drawn again and the
    // focus fell back to the page)
    for (const width of [360, 412, 1280]) {
      await check(`${width}: next song here, the mark moves and the focus stays`, async () => {
        const { page, context, state } = await browserQueue(browser, width, width < 600 ? { mobile: true } : {});
        try {
          if (width < 600) assert.equal(await page.evaluate(() => document.documentElement.dataset.shell), 'phone', `${width}: the phone layout`);
          await tag(page);
          const target = width < 600 ? '[data-qmore="2"]' : '[data-qgrip="2"]';
          await page.focus(target);
          const before = await focused(page);
          await next(page);
          await until(async () => (await now(page)).index === 1, `${width}: Song 2 plays`);
          await until(async () => (await marks(page)).join() === '-,on,-', `${width}: Song 2's row is marked (${(await marks(page)).join()})`, 3000);
          assert.equal(await focused(page), before, `${width}: the focus stays on ${before}`);
          assert.equal(await sameList(page), true, `${width}: the queue is not drawn again for a song change`);
          if (width < 600) await page.screenshot({ path: path.join(shots, `next-song-focus-kept-${width}.png`) });
          // and back to Song 1 (Previous at the song's start): the mark goes back, the focus stays
          await page.evaluate(() => { const a = document.querySelector('#browser'); try { a.currentTime = 0; } catch { /* */ } document.querySelector('#prev').click(); });
          await until(async () => (await now(page)).index === 0, `${width}: Song 1 again`);
          await until(async () => (await marks(page)).join() === 'on,-,-', `${width}: Song 1's row is marked again (${(await marks(page)).join()})`, 3000);
          assert.equal(await focused(page), before, `${width}: the focus still stays`);
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `${width}: no sideways scroll`);
          assert.deepEqual(state.errors, [], `${width}: no page errors`);
        } finally { await context.close(); }
      });
    }
    // 2. A drag on the phone's grip while the next song starts: the drag goes on and the drop moves
    // the row (before: the redraw took the row away mid-drag, the drop was lost and the page stayed
    // in its dragging state)
    for (const width of [360, 412]) {
      await check(`${width}: a drag across a song change still lands`, async () => {
        const { page, context, state } = await browserQueue(browser, width);
        try {
          const g = await page.locator('[data-qgrip="2"]').boundingBox();
          const h = await page.evaluate(() => document.querySelector('[data-qrow="1"]').offsetHeight);
          const x = g.x + g.width / 2; const y = g.y + g.height / 2;
          await page.mouse.move(x, y); await page.mouse.down();
          await page.mouse.move(x, y - h * 0.4, { steps: 4 });
          await next(page);
          await until(async () => (await now(page)).index === 1, `${width}: Song 2 plays`);
          await sleep(200);
          await page.mouse.move(x, y - h * 1.1, { steps: 4 });
          await page.mouse.up();
          await until(async () => JSON.stringify(await queueIds(page)) === '[1,3,2]', `${width}: Song 3 lands above Song 2 (${JSON.stringify(await queueIds(page))})`, 3000);
          assert.equal(await page.evaluate(() => document.body.classList.contains('qdrag-active')), false, `${width}: the drag is over`);
          await until(async () => (await marks(page)).join() === '-,-,on', `${width}: Song 2 is still the one marked, on its new row (${(await marks(page)).join()})`, 3000);
          assert.equal((await now(page)).index, 2, `${width}: and still plays`);
          assert.deepEqual(state.errors, [], `${width}: no page errors`);
        } finally { await context.close(); }
      });
    }
    // 3. New songs played here while the Queue page is open: the page shows them (a change of the
    // songs still draws the page again)
    await check('360: new songs played here show on the open Queue page', async () => {
      const { page, context, state } = await browserQueue(browser, 360, { mobile: true });
      try {
        await page.evaluate(() => pmPlayer.play([{ uri: 'track:3', track_id: 3, title: 'Song 3', artist: 'Sefa', duration_ms: 60000 }, { uri: 'track:2', track_id: 2, title: 'Song 2', artist: 'Sefa', duration_ms: 60000 }]));
        await until(async () => JSON.stringify(await queueIds(page)) === '[3,2]', `the queue shows Songs 3 and 2 (${JSON.stringify(await queueIds(page))})`);
        await until(async () => (await marks(page)).join() === 'on,-', `Song 3's row is marked (${(await marks(page)).join()})`, 3000);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 4. The Queue page of a node zone is drawn once as it opens (before: the page's own ask for the
    // queue took over the one made at load, and that dropped answer drew the page a second time, a
    // moment later, taking the focus from a row reached by then). A node zone moves on to its next
    // song (its queue's version changes with the song on): the queue is asked for again, and as its
    // songs are the same only the mark moves; when another device removed a song, the page is drawn
    // with the queue as it is now
    for (const width of [360, 1280]) {
      await check(`${width}: the node zone's next song moves the mark in place; a removed song redraws`, async () => {
        const items = saved.map((s, k) => ({ id: 101 + k, uri: s.uri, title: s.title, artist: s.artist, album: s.album, duration_ms: s.duration_ms }));
        const np = (k) => ({ item: items[k], position_ms: 1000, duration_ms: 60000 });
        const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'playing', now_playing: np(0), queue: items, queue_index: 0, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
        const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/queue': { body: items }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
        const { page, context, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1', tracks, ...(width < 600 ? { mobile: true } : {}) });
        let sock = null;
        await context.routeWebSocket(/\/v1\/events/, (ws) => { sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
        await countDraws(context);
        try {
          await fx.go(page, '/queue');
          await until(async () => (await marks(page)).join() === 'on,-,-', `Song 1's row is marked (${(await marks(page)).join()})`);
          await sleep(500);
          assert.equal(await draws(page), 1, 'the Queue page is drawn once as it opens');
          await tag(page);
          const target = width < 600 ? '[data-qmore="2"]' : '[data-qgrip="2"]';
          await page.focus(target);
          const before = await focused(page);
          const asked = () => state.hits.filter((x) => x.v === '/zones/z1/queue').length;
          const a0 = asked();
          sock.send(JSON.stringify({ type: 'zone', seq: 1, state: { ...Z, now_playing: np(1), queue_index: 1, queue_version: 2 } }));
          await until(async () => asked() > a0, 'the queue is asked for again');
          await until(async () => (await marks(page)).join() === '-,on,-', `Song 2's row is marked (${(await marks(page)).join()})`, 3000);
          await sleep(300);
          assert.equal(await focused(page), before, `the focus stays on ${before}`);
          assert.equal(await sameList(page), true, 'the queue is not drawn again for a song change');
          if (width < 600) await page.screenshot({ path: path.join(shots, `zone-next-song-${width}.png`) });
          // (another device removes Song 1: same song on, now row 1 of 2)
          zf['/zones/z1/queue'].body = items.slice(1);
          sock.send(JSON.stringify({ type: 'zone', seq: 2, state: { ...Z, queue: items.slice(1), now_playing: np(1), queue_index: 0, queue_version: 3 } }));
          await until(async () => JSON.stringify(await queueIds(page)) === '[2,3]', `the page shows the queue as it is now (${JSON.stringify(await queueIds(page))})`);
          await until(async () => (await marks(page)).join() === 'on,-', `Song 2's row is marked, now the first (${(await marks(page)).join()})`, 3000);
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // 5. Song rows (the Songs page) say which one plays to a screen reader too, as the queue and
    // song lists elsewhere do
    await check('360: the Songs page row of the song on has aria-current', async () => {
      const { page, context, state } = await browserQueue(browser, 360, { mobile: true });
      try {
        await page.evaluate(() => { history.pushState(null, '', '/tracks'); dispatchEvent(new PopStateEvent('popstate')); });
        await page.waitForSelector('.track[data-tid="1"]');
        const cur = () => page.evaluate(() => [...document.querySelectorAll('.track[data-tid]')].map((r) => `${r.dataset.tid}:${r.classList.contains('cur') ? 'cur' : ''}${r.getAttribute('aria-current') === 'true' ? '+aria' : ''}`).join(' '));
        await until(async () => (await cur()) === '1:cur+aria 2: 3:', `Song 1's row is current (${await cur()})`, 3000);
        await next(page);
        await until(async () => (await cur()) === '1: 2:cur+aria 3:', `Song 2's row is current now (${await cur()})`, 3000);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all queue-mark checks passed');
})().catch((e) => { console.error(e); process.exit(1); });
