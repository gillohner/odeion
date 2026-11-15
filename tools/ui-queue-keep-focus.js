// The Queue page keeps the keyboard's place when an edit draws it again: Move up / Move down,
// Alt+↑/↓ on a row's grip and the phone's ⋯ › Move down leave the focus on the moved song's row
// (on the same control, or the row's other one where that control is gone, e.g. Move down on the
// last row); Remove leaves it on the next song's row; Radio keeps its own button. Before, every one
// of these sent the focus back to the page, and a keyboard user started again from the top.
// The focus goes with the song, not the row number: a song another device puts above it moves the
// focus down with it. Clear, or removing the only song, leaves it on the empty page's Shuffle my
// music, Radio or the page's heading. A failed edit doesn't move the focus on a later redraw, nor
// does a phone ⋯ edit that lands after another page has been opened.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-queue-keep-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/queue-keep-focus-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (f, label, ms = 5000) => { const t = Date.now(); while (Date.now() - t < ms) { const v = await f(); if (v) return v; await sleep(50); } throw new Error('Timed out: ' + label); };
const tracks = [1, 2, 3, 4].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null }));
const fixtures = {};
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };
const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));
const queueIds = (page) => page.evaluate(() => [...document.querySelectorAll('.queue [data-qrow]')].map((r) => Number((r.textContent.match(/Song (\d)/) || [])[1])).join(','));
// Where the focus is: a row control as "<kind> <row>", a head button by its id, else the tag
const focused = (page) => page.evaluate(() => {
  const a = document.activeElement; if (!a || a === document.body) return 'BODY';
  for (const k of ['qup', 'qdown', 'qrm', 'qgrip', 'qi', 'qmore']) if (a.dataset && a.dataset[k] != null) return `${k} ${a.dataset[k]}`;
  return a.id || a.tagName;
});
// The song on the focused control's row
const focusedSong = (page) => page.evaluate(() => { const r = document.activeElement && document.activeElement.closest('[data-qrow]'); return r ? Number((r.textContent.match(/Song (\d)/) || [])[1]) : null; });

// This browser's queue: Songs 1-4 (or the ones given), the Queue page open
async function browserQueue(browser, width, list = saved) {
  const run = await fx.create(browser, width, { fixtures, tracks, ...(width < 600 ? { mobile: true } : {}) });
  await run.context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(list), JSON.stringify({ index: 0, id: list[0].id, pos: 0 })]);
  await fx.go(run.page, '/queue');
  const want = list.map((s) => s.id).join(',');
  await until(async () => (await queueIds(run.page)) === want, `${width}: the queue shows Songs ${want}`);
  return run;
}
// A node zone "Kitchen" with Songs 1-4 whose queue edits (Move, Remove, Clear) the fixture carries out;
// opts.hold(kind) may return a promise the answer waits for
async function zoneQueue(browser, width, opts = {}) {
  const z = { items: saved.map((s, k) => ({ id: 101 + k, uri: s.uri, title: s.title, artist: s.artist, album: s.album, duration_ms: s.duration_ms })), fail: false, sock: null };
  const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'paused', now_playing: { item: z.items[0], position_ms: 1000, duration_ms: 60000 }, queue: z.items, queue_index: 0, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/queue': { body: z.items }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
  const run = await fx.create(browser, width, { fixtures: zf, zone: 'z1', tracks, ...(width < 600 ? { mobile: true } : {}) });
  const set = (items) => { zf['/zones/z1/queue'].body = z.items = items; };
  z.set = set; z.Z = Z;
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { z.sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
  await run.context.route(/\/v1\/zones\/z1\/queue(\/\d+)?$/, async (r) => {
    const m = r.request().method(); const id = Number(r.request().url().split('/').pop());
    if (m === 'GET') return r.fallback();
    if (opts.hold) await opts.hold(m);
    if (z.fail) return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'the zone said no' }) });
    if (m === 'DELETE' && !id) set([]);
    else if (m === 'DELETE') set(z.items.filter((x) => x.id !== id));
    else { const to = JSON.parse(r.request().postData() || '{}').to_index; const items = [...z.items]; const k = items.findIndex((x) => x.id === id); const [it] = items.splice(k, 1); items.splice(to, 0, it); set(items); }
    return r.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
  });
  await run.context.route(/\/v1\/zones\/z1\/radio$/, (r) => { Z.radio = JSON.parse(r.request().postData() || '{}').on; return r.fulfill({ status: 200, contentType: 'application/json', body: '{}' }); });
  await fx.go(run.page, '/queue');
  await until(async () => (await queueIds(run.page)) === '1,2,3,4', `${width}: the zone's queue shows`);
  return { ...run, z };
}
// Another device changes the zone's queue (the page hears it on the socket)
const pushQueue = (z, items, v) => { z.set(items); z.sock.send(JSON.stringify({ type: 'zone', seq: v, state: { ...z.Z, queue: items, queue_version: v } })); };

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  const check = async (label, f) => { try { await f(); console.log('ok   ' + label); } catch (e) { failed++; console.log('FAIL ' + label + '\n     ' + e.message.split('\n')[0]); } };
  try {
    // 1. This browser's queue, by keyboard: each edit leaves the focus on the moved song's row
    const CASES = [
      // [what, control focused, key, queue after, focus after, song focused after]
      ['Move down', '[data-qdown="0"]', 'Enter', '2,1,3,4', 'qdown 1', 1],
      ['Move up', '[data-qup="2"]', 'Enter', '1,3,2,4', 'qup 1', 3],
      ['Remove', '[data-qrm="1"]', 'Enter', '1,3,4', 'qrm 1', 3],
      ['Alt+↓ on the grip', '[data-qgrip="1"]', 'Alt+ArrowDown', '1,3,2,4', 'qgrip 2', 2],
      ['Alt+↑ on the grip', '[data-qgrip="2"]', 'Alt+ArrowUp', '1,3,2,4', 'qgrip 1', 3],
      // (Move down onto the last row: that row has no Move down, its Move up takes the focus)
      ['Move down onto the last row', '[data-qdown="2"]', 'Enter', '1,2,4,3', 'qup 3', 3],
      // (Remove on the last row: the focus goes to the row now last)
      ['Remove the last row', '[data-qrm="3"]', 'Enter', '1,2,3', 'qrm 2', 3],
    ];
    for (const width of [1280, 1024, 768]) {
      for (const [what, sel, key, after, want, song] of CASES) {
        await check(`${width}: ${what} keeps the focus on the song's row`, async () => {
          const { page, context, state } = await browserQueue(browser, width);
          try {
            await page.focus(sel);
            await page.keyboard.press(key);
            await until(async () => (await queueIds(page)) === after, `the queue becomes ${after} (${await queueIds(page)})`);
            await sleep(200);
            assert.equal(await focused(page), want, 'focus');
            assert.equal(await focusedSong(page), song, 'the focused row is the song moved (or the next one)');
            if (what === 'Move down' && width !== 1024) await page.screenshot({ path: path.join(shots, `after-move-down-${width}.png`) });
            // and the next press goes on from there (Move down twice moves the song two rows)
            if (what === 'Move down') {
              await page.keyboard.press('Enter');
              await until(async () => (await queueIds(page)) === '2,3,1,4', `a second Move down moves Song 1 again (${await queueIds(page)})`);
              await sleep(200);
              assert.equal(await focused(page), 'qdown 2', 'focus after the second press');
            }
            assert.deepEqual(state.errors, [], 'no page errors');
          } finally { await context.close(); }
        });
      }
    }
    // 2. Phone: the row's ⋯ › Move down; the focus goes back to the ⋯ of the song's new row
    for (const width of [360, 412]) {
      await check(`${width}: ⋯ › Move down leaves the focus on the song's ⋯`, async () => {
        const { page, context, state } = await browserQueue(browser, width);
        try {
          assert.equal(await page.evaluate(() => document.documentElement.dataset.shell), 'phone', 'the phone layout');
          await page.focus('[data-qmore="0"]');
          await page.keyboard.press('Enter');
          const item = page.locator('[data-mi]', { hasText: 'Move down' }).first();
          await item.waitFor();
          await item.focus(); await page.keyboard.press('Enter');
          await until(async () => (await queueIds(page)) === '2,1,3,4', `Song 1 moves down (${await queueIds(page)})`);
          await sleep(300);
          assert.equal(await focused(page), 'qmore 1', 'focus');
          assert.equal(await focusedSong(page), 1, 'on Song 1');
          await page.screenshot({ path: path.join(shots, `after-phone-move-down-${width}.png`) });
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'no sideways scroll');
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // 2b. Phone: ⋯ › Remove leaves the focus on the next song's ⋯
    await check('360: ⋯ › Remove leaves the focus on the next song\'s ⋯', async () => {
      const { page, context, state } = await browserQueue(browser, 360);
      try {
        await page.focus('[data-qmore="1"]'); await page.keyboard.press('Enter');
        const item = page.locator('[data-mi]', { hasText: 'Remove from queue' }).first();
        await item.waitFor(); await item.focus(); await page.keyboard.press('Enter');
        await until(async () => (await queueIds(page)) === '1,3,4', `Song 2 goes (${await queueIds(page)})`);
        await sleep(300);
        assert.equal(await focused(page), 'qmore 1', 'focus');
        assert.equal(await focusedSong(page), 3, 'on Song 3');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 2c. Removing the only song: the empty page's Shuffle my music takes the focus (the next thing
    // to press), or with no songs in the library the page's heading; not nowhere
    for (const [what, lib, want] of [['the library has songs', tracks, 'qmine'], ['the library is empty', [], 'heading']]) {
      await check(`1280: Remove of the only song leaves the focus on ${want === 'qmine' ? 'Shuffle my music' : 'the heading'} (${what})`, async () => {
        const run = await fx.create(browser, 1280, { fixtures, tracks: lib });
        const { page, context, state } = run;
        try {
          await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved.slice(0, 1)), JSON.stringify({ index: 0, id: saved[0].id, pos: 0 })]);
          await fx.go(page, '/queue');
          await until(async () => (await queueIds(page)) === '1', 'the queue shows Song 1');
          await page.focus('[data-qrm="0"]'); await page.keyboard.press('Enter');
          await until(async () => (await queueIds(page)) === '', `the queue empties (${await queueIds(page)})`);
          await sleep(300);
          const at = await page.evaluate(() => (document.activeElement && document.activeElement.matches('.page-head h2') ? 'heading' : (document.activeElement || {}).id || 'BODY'));
          assert.equal(at, want, 'focus');
          if (want === 'qmine') {
            await page.screenshot({ path: path.join(shots, 'after-remove-only-song-1280.png') });
            // and pressing it: the songs come, the button goes, the focus stays on the page (the heading)
            await page.keyboard.press('Enter');
            await until(async () => (await queueIds(page)).split(',').filter(Boolean).length === tracks.length, `the shuffled songs show (${await queueIds(page)})`);
            await sleep(300);
            assert.notEqual(await focused(page), 'BODY', 'the focus stays on the page after Shuffle my music');
          }
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // 3. A node zone: Move down (in the zone's turn, then the queue asked for again) and Radio keep
    // the focus; a failed Move doesn't send the focus anywhere on a later redraw
    await check('1280: a node zone\'s Move down, a failed Move, and Radio keep the focus', async () => {
      const { page, context, state, z } = await zoneQueue(browser, 1280);
      try {
        await page.focus('[data-qdown="0"]'); await page.keyboard.press('Enter');
        await until(async () => (await queueIds(page)) === '2,1,3,4', `Song 1 moves down (${await queueIds(page)})`);
        await sleep(300);
        assert.equal(await focused(page), 'qdown 1', 'focus after the zone\'s Move down');
        // a refused Move: the queue stays, and when another device then changes it the focus stays on its row
        z.fail = true;
        await page.keyboard.press('Enter');
        await until(async () => state.errors.length || (await page.evaluate(() => /said no/.test(document.body.innerText))), 'the refusal is shown');
        z.fail = false;
        pushQueue(z, [...z.items, { ...z.items[0], id: 105, title: 'Song 9' }], 2);
        await until(async () => (await queueIds(page)).split(',').length === 5, `the new song shows (${await queueIds(page)})`);
        await sleep(300);
        assert.equal(await focused(page), 'qdown 1', 'the failed Move\'s target doesn\'t take the focus');
        await page.focus('#qradio'); await page.keyboard.press('Enter');
        await until(async () => page.evaluate(() => /Radio on/.test(document.querySelector('#qradio').textContent)), 'Radio is on');
        await sleep(200);
        assert.equal(await focused(page), 'qradio', 'focus after Radio');
        await page.screenshot({ path: path.join(shots, 'after-zone-radio-1280.png') });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 3b. The focus goes with the song: another device puts a song above it, and the focus moves down
    // with the song (not onto whatever is now in its old row)
    await check('1280: a song added above the focused one keeps the focus on its song', async () => {
      const { page, context, state, z } = await zoneQueue(browser, 1280);
      try {
        await page.focus('[data-qrm="1"]');
        pushQueue(z, [{ ...z.items[3], id: 109 }, ...z.items], 2);
        await until(async () => (await queueIds(page)) === '4,1,2,3,4', `the new song shows (${await queueIds(page)})`);
        await sleep(300);
        assert.equal(await focused(page), 'qrm 2', 'focus');
        assert.equal(await focusedSong(page), 2, 'still on Song 2');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 3c. Clear: the button goes with the songs; the focus stays on the page, on Radio
    await check('1280: Clear leaves the focus on Radio', async () => {
      const { page, context, state } = await zoneQueue(browser, 1280);
      try {
        await page.focus('#qclear'); await page.keyboard.press('Enter');
        await until(async () => (await queueIds(page)) === '', `the queue empties (${await queueIds(page)})`);
        await sleep(300);
        assert.equal(await focused(page), 'qradio', 'focus');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // 3d. Phone, a node zone: ⋯ › Remove still on its way when another page is opened; coming back
    // to the Queue later doesn't put the focus on a row's ⋯
    await check('360: a ⋯ edit that lands after leaving the Queue doesn\'t take the focus later', async () => {
      let release = null; const held = new Promise((r) => { release = r; });
      const { page, context, state } = await zoneQueue(browser, 360, { hold: (m) => (m === 'DELETE' ? held : null) });
      try {
        await page.focus('[data-qmore="1"]'); await page.keyboard.press('Enter');
        const item = page.locator('[data-mi]', { hasText: 'Remove from queue' }).first();
        await item.waitFor(); await item.focus(); await page.keyboard.press('Enter');
        await sleep(200);
        await page.evaluate(() => { history.pushState(null, '', '/songs'); dispatchEvent(new PopStateEvent('popstate', { state: null })); });
        await until(async () => page.evaluate(() => !document.querySelector('.queue [data-qrow]') && location.pathname === '/songs'), 'Songs opens');
        release(); await sleep(400);
        await page.evaluate(() => { document.activeElement && document.activeElement.blur(); history.pushState(null, '', '/queue'); dispatchEvent(new PopStateEvent('popstate', { state: null })); });
        await until(async () => (await queueIds(page)) === '1,3,4', `the queue shows without Song 2 (${await queueIds(page)})`);
        await sleep(400);
        assert.notEqual((await focused(page)).split(' ')[0], 'qmore', `the focus isn't on a row's ⋯ (${await focused(page)})`);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all queue-keep-focus checks passed');
})().catch((e) => { console.error(e); process.exit(1); });
