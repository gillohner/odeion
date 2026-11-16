// The Queue page's head on a phone: Radio and Shuffle, and one ⋯ for Save as playlist, Clear queue and
// Move to (the other devices), all on one row. Before, the five buttons took three rows (188 px) at
// 360 px before the first song. The desktop layout keeps its five buttons and has no ⋯.
// Checked at 360/412 px (and 360 px in the Android app) for a node zone, a listener's node zone and
// this browser's queue; each ⋯ item does what its button did.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-queue-head-phone.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/queue-head-phone-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (f, label, ms = 5000) => { const t = Date.now(); while (Date.now() - t < ms) { const v = await f(); if (v) return v; await sleep(50); } throw new Error('Timed out: ' + label); };
const tracks = [1, 2, 3, 4].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null }));
const items = tracks.map((t, k) => ({ id: 101 + k, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));
// (the Living room has a queue of its own: a tap that reached it would clear, save or move it)
const items2 = tracks.slice(0, 2).map((t, k) => ({ ...items[k], id: 201 + k }));
const rows = (page) => page.evaluate(() => document.querySelectorAll('.queue [data-qrow]').length);
// The head's visible buttons, how many rows they take, and their sizes
const head = (page) => page.evaluate(() => {
  const bs = [...document.querySelectorAll('.page-head .actions > *')].filter((e) => e.getClientRects().length);
  const box = (e) => e.getBoundingClientRect();
  return { ids: bs.map((e) => e.id).join(','), rows: new Set(bs.map((e) => Math.round(box(e).top))).size, small: bs.filter((e) => box(e).width < 44 || box(e).height < 44).map((e) => e.id), over: document.documentElement.scrollWidth - innerWidth };
});
const menu = (page) => page.evaluate(() => { const m = document.querySelector('.popover.menu'); return m && !m.hidden ? [...m.children].map((c) => c.classList.contains('pop-title') ? '# ' + c.textContent.trim() : ((c.querySelector(':scope > span') || c).firstChild.textContent || '').trim()).join(' | ') : ''; });

async function zoneQueue(browser, width, opts = {}) {
  const Z = (id, name) => ({ id, name, output: 'upnp:' + id, transport: 'paused', now_playing: { item: items[0], position_ms: 1000, duration_ms: 60000 }, queue: id === 'z1' ? items : items2, queue_index: 0, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } });
  const z1 = Z('z1', 'Kitchen'); const z2 = Z('z2', 'Living room');
  const fixtures = { '/zones': { body: [z1, z2] }, '/zones/z1': { body: z1 }, '/zones/z2': { body: z2 }, '/zones/z1/queue': { body: items }, '/zones/z2/queue': { body: items2 },
    '/outputs': { body: [{ id: 'upnp:z1', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }, { id: 'upnp:z2', name: 'Living room', kind: 'upnp', caps: {} }] },
    '/playlists/from-queue': { body: { playlist: { summary: { id: 'q1', name: 'Queue · kitchen' } }, skipped: [] } } };
  const run = await fx.create(browser, width, { fixtures, zone: opts.browser ? 'browser' : 'z1', tracks, role: opts.role || 'owner', ...(width < 600 ? { mobile: true } : {}) });
  if (opts.android) await run.context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  if (opts.browser) await run.context.addInitScript((q) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', JSON.stringify({ index: 0, id: 101, pos: 0 })); }, JSON.stringify(items));
  run.fixtures = fixtures;
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { run.ws = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [z1, z2] })); });
  await run.context.route(/\/v1\/zones\/z1\/queue$/, (r) => {
    if (r.request().method() !== 'DELETE') return r.fallback();
    run.state.hits.push({ v: '/zones/z1/queue', m: 'DELETE', at: Date.now() }); // (this route answers before the fixture's, which logs the others)
    fixtures['/zones/z1/queue'].body = []; z1.queue = []; z1.now_playing = null;
    return r.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
  });
  await fx.go(run.page, '/queue');
  await until(async () => (await rows(run.page)) === 4, `${width}: the queue shows its 4 songs`);
  await sleep(300);
  return run;
}
const hit = (state, m, re) => state.hits.some((h) => h.m === m && re.test(h.v));

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  const check = async (label, f) => { try { await f(); console.log('ok   ' + label); } catch (e) { failed++; console.log('FAIL ' + label + '\n     ' + e.message.split('\n').slice(0, 4).join(' ')); } };
  try {
    for (const [w, android] of [[360, false], [412, false], [360, true]]) {
      const tag = `${w}${android ? ' (Android app)' : ''}`;
      await check(`${tag}: a node zone's head is Radio, Shuffle and ⋯, on one row of 44 px targets`, async () => {
        const { page, context, state } = await zoneQueue(browser, w, { android });
        try {
          const h = await head(page);
          assert.equal(h.ids, 'qradio,qshuffle,qmenu', 'visible head buttons');
          assert.equal(h.rows, 1, 'rows the head takes');
          assert.deepEqual(h.small, [], 'targets under 44 px');
          assert.ok(h.over <= 0, `no sideways scroll (${h.over} px)`);
          await page.screenshot({ path: path.join(shots, `after-queue-${w}${android ? '-android' : ''}.png`) });
          await page.click('#qmenu');
          await until(async () => (await menu(page)).includes('Clear queue'), `the ⋯ menu opens (${await menu(page)})`);
          assert.equal(await menu(page), `# Queue | Save as playlist | Clear queue | # Move to | ${android ? 'Living room | This phone' : 'This browser | Living room'}`, 'the ⋯ menu');
          await page.screenshot({ path: path.join(shots, `after-queue-menu-${w}${android ? '-android' : ''}.png`) });
          // Save as playlist: the zone's queue, as the button did
          await page.locator('[data-mi]', { hasText: 'Save as playlist' }).click();
          await until(async () => hit(state, 'POST', /^\/playlists\/from-queue$/), 'Save as playlist asks the node');
          const save = state.hits.find((x) => x.v === '/playlists/from-queue');
          assert.deepEqual(JSON.parse(save.body), { zone: 'z1' }, 'saved from the zone');
          await until(async () => page.evaluate(() => /Saved “Queue · kitchen”/.test(document.body.innerText)), 'the toast says it was saved');
          // Clear queue: the queue empties
          await page.click('#qmenu');
          await page.locator('[data-mi]', { hasText: 'Clear queue' }).click();
          await until(async () => hit(state, 'DELETE', /^\/zones\/z1\/queue$/), 'Clear queue asks the node');
          await until(async () => (await rows(page)) === 0, 'the queue empties');
          assert.equal(await page.locator('#qmenu').count(), 0, 'no ⋯ on an empty queue');
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    await check('360: Move to › another device moves the queue, as the Play to menu does', async () => {
      const { page, context, state } = await zoneQueue(browser, 360);
      try {
        await page.click('#qmenu');
        await page.locator('[data-mi]', { hasText: 'Living room' }).click();
        await until(async () => state.hits.some((h) => h.m !== 'GET' && /z2|transfer|move/.test(h.v + (h.body || ''))), `the move asks the node (${state.hits.filter((h) => h.m !== 'GET').map((h) => h.m + ' ' + h.v).join(', ')})`);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check('360: a listener\'s ⋯ has Clear queue and Move to, not Save as playlist', async () => {
      const { page, context, state } = await zoneQueue(browser, 360, { role: 'listener' });
      try {
        const h = await head(page);
        assert.equal(h.ids, 'qradio,qshuffle,qmenu', 'visible head buttons');
        await page.click('#qmenu');
        await until(async () => (await menu(page)).includes('Clear queue'), `the ⋯ menu opens (${await menu(page)})`);
        assert.equal(await menu(page), '# Queue | Clear queue | # Move to | This browser | Living room', 'the ⋯ menu');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check('360: this browser\'s queue: Shuffle and ⋯ (Save as playlist, Move to)', async () => {
      const { page, context, state } = await zoneQueue(browser, 360, { browser: true });
      try {
        const h = await head(page);
        assert.equal(h.ids, 'qshuffle,qmenu', 'visible head buttons');
        await page.click('#qmenu');
        await until(async () => (await menu(page)).includes('Save as playlist'), `the ⋯ menu opens (${await menu(page)})`);
        const m = await menu(page);
        assert.ok(!m.includes('Clear queue'), `no Clear in this browser (${m})`);
        assert.match(m, /^# Queue \| Save as playlist \| # Move to \| .*Kitchen/, 'the ⋯ menu');
        await page.screenshot({ path: path.join(shots, 'after-queue-browser-menu-360.png') });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check('360: by keyboard, ⋯ › Clear queue leaves the focus on Radio (as the Clear button did)', async () => {
      const { page, context, state } = await zoneQueue(browser, 360);
      try {
        await page.focus('#qmenu'); await page.keyboard.press('Enter');
        const item = page.locator('[data-mi]', { hasText: 'Clear queue' }).first();
        await item.waitFor(); await item.focus(); await page.keyboard.press('Enter');
        await until(async () => (await rows(page)) === 0, 'the queue empties');
        await sleep(300);
        assert.equal(await page.evaluate(() => document.activeElement && (document.activeElement.id || document.activeElement.tagName)), 'qradio', 'focus');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check('360: by keyboard, ⋯ › Save as playlist leaves the focus on the ⋯', async () => {
      const { page, context, state } = await zoneQueue(browser, 360);
      try {
        await page.focus('#qmenu'); await page.keyboard.press('Enter');
        const item = page.locator('[data-mi]', { hasText: 'Save as playlist' }).first();
        await item.waitFor(); await item.focus(); await page.keyboard.press('Enter');
        await until(async () => hit(state, 'POST', /^\/playlists\/from-queue$/), 'Save as playlist asks the node');
        await sleep(300);
        assert.equal(await page.evaluate(() => document.activeElement && (document.activeElement.id || document.activeElement.tagName)), 'qmenu', 'focus');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check('360: a listener on this browser\'s queue: ⋯ is Move to only, with no empty Queue heading', async () => {
      const { page, context, state } = await zoneQueue(browser, 360, { browser: true, role: 'listener' });
      try {
        assert.equal((await head(page)).ids, 'qshuffle,qmenu', 'visible head buttons');
        await page.click('#qmenu');
        await until(async () => (await menu(page)).includes('Kitchen'), `the ⋯ menu opens (${await menu(page)})`);
        assert.match(await menu(page), /^# Move to \| [^#]*Kitchen[^#]*$/, 'the ⋯ menu');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // The ⋯ menu stays open across a redraw; the zone can change meanwhile (here the host moves the
    // music to the Living room and the page follows): its items are for the zone it was opened on
    // and do nothing for the new one (before, Clear queue emptied the Living room's queue)
    const follow = async (run) => {
      run.fixtures['/presence'] = { body: [{ mine: true, key: 'host', name: 'Host', zone: { id: 'z2', name: 'Living room' }, your_zone: 'z1' }] };
      run.fixtures['/me/presence'] = { body: { visible: 'hidden' } };
      run.ws.send(JSON.stringify({ type: 'zone_removed', seq: 1, zone: 'z9' })); // (another zone: asks /presence again)
      await until(async () => run.page.evaluate(() => /Living room/.test(document.querySelector('.page-head') ? document.querySelector('.page-head').textContent : '')), 'the page follows to the Living room');
    };
    for (const what of ['Clear queue', 'Save as playlist', 'This browser']) {
      await check(`360: ⋯ › ${what} after the page followed to another device does nothing to that device's queue`, async () => {
        const run = await zoneQueue(browser, 360); const { page, context, state } = run;
        try {
          await page.click('#qmenu');
          await until(async () => (await menu(page)).includes(what), `the ⋯ menu opens (${await menu(page)})`);
          await follow(run);
          assert.ok((await menu(page)).includes(what), 'the menu is still open');
          const n = state.hits.length;
          await page.locator('[data-mi]', { hasText: what }).click();
          await sleep(500);
          const sent = state.hits.slice(n).filter((h) => h.m !== 'GET');
          assert.deepEqual(sent.map((h) => h.m + ' ' + h.v), [], 'nothing asked of the node');
          await until(async () => page.evaluate(() => /Nothing changed: the Queue now shows Living room/.test(document.body.innerText)), 'the toast says why');
          if (what === 'Clear queue') await page.screenshot({ path: path.join(shots, 'after-queue-followed-clear-360.png') });
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // (the node's POST is shared while on its way; the list of playlists read after it was not: a tap
    // then made a second playlist)
    await check('360: a second Save as playlist while the first is finishing makes no second playlist', async () => {
      const run = await zoneQueue(browser, 360); const { page, context, state } = run;
      try {
        let release; const held = new Promise((r) => { release = r; }); let asked = false;
        await context.route(/\/v1\/playlists$/, async (r) => { if (r.request().method() !== 'GET' || !hit(state, 'POST', /^\/playlists\/from-queue$/)) return r.fallback(); asked = true; await held; return r.fulfill({ status: 200, contentType: 'application/json', body: '[]' }); });
        await page.click('#qmenu');
        await until(async () => (await menu(page)).includes('Save as playlist'), `the ⋯ menu opens (${await menu(page)})`);
        await page.locator('[data-mi]', { hasText: 'Save as playlist' }).click();
        await until(async () => asked, 'the first save reads the playlists again');
        // (the ⋯ is busy: it doesn't open; were it to, its Save as playlist is pressed again)
        await page.click('#qmenu'); await sleep(200);
        if ((await menu(page)).includes('Save as playlist')) await page.locator('[data-mi]', { hasText: 'Save as playlist' }).click();
        await sleep(300); release(); await sleep(500);
        assert.equal(state.hits.filter((h) => h.m === 'POST' && h.v === '/playlists/from-queue').length, 1, 'saves sent');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    // The page's buttons and rows are for the zone it shows, until it is drawn again: here the page
    // follows the host to the Living room, whose queue is slow to come, so the page still shows the
    // Kitchen meanwhile. A tap then sends nothing (before: Clear emptied the Living room's queue, Save
    // saved it, Move to moved it, a row's number played the Living room's song at that place).
    const followHeld = async (run) => {
      let release; const held = new Promise((r) => { release = r; }); let asked = false;
      await run.context.route(/\/v1\/zones\/z2\/queue$/, async (r) => { if (r.request().method() !== 'GET') return r.fallback(); asked = true; await held; return r.fallback(); });
      run.fixtures['/presence'] = { body: [{ mine: true, key: 'host', name: 'Host', zone: { id: 'z2', name: 'Living room' }, your_zone: 'z1' }] };
      run.fixtures['/me/presence'] = { body: { visible: 'hidden' } };
      run.ws.send(JSON.stringify({ type: 'zone_removed', seq: 1, zone: 'z9' }));
      await until(async () => asked, 'the page asks for the Living room\'s queue');
      assert.match(await run.page.evaluate(() => document.querySelector('.page-head').textContent), /Kitchen/, 'the page still shows the Kitchen');
      return release;
    };
    const stale = [
      [1280, 'Clear', async (page) => page.click('#qclear')],
      [1280, 'Save as playlist', async (page) => page.click('#qsave')],
      [1280, 'Move to…', async (page) => page.click('#qtransfer')],
      [1280, 'Move to › This browser (the menu opened before)', async (page) => page.locator('[data-mi]', { hasText: 'This browser' }).click(), async (page) => { await page.click('#qtransfer'); await until(async () => (await menu(page)).includes('This browser'), 'the Move to menu opens'); }],
      [1280, 'Radio', async (page) => page.click('#qradio')],
      [1280, 'a row\'s Play from here', async (page) => page.click('[data-qi="1"]')],
      [1280, 'a row\'s Remove', async (page) => page.click('[data-qrm="1"]', { force: true })],
      [360, '⋯', async (page) => page.click('#qmenu')],
      [360, 'a row\'s ⋯', async (page) => page.click('[data-qmore="1"]')],
      // (twice: one redraw, not two racing ones)
      [1280, 'Clear, twice', async (page) => { await page.click('#qclear'); await sleep(100); await page.click('#qclear'); }],
    ];
    for (const [w, what, tap, before] of stale) {
      await check(`${w}: ${what} while the page still shows the device it left does nothing to the new one`, async () => {
        const run = await zoneQueue(browser, w); const { page, context, state } = run;
        try {
          if (before) await before(page);
          const release = await followHeld(run);
          const n = state.hits.length;
          await tap(page);
          await sleep(400);
          const sent = state.hits.slice(n).filter((h) => h.m !== 'GET');
          assert.deepEqual(sent.map((h) => h.m + ' ' + h.v), [], 'nothing asked of the node');
          assert.equal(await menu(page), '', 'no menu opened');
          await until(async () => page.evaluate(() => /Nothing changed: the Queue now shows Living room/.test(document.body.innerText)), 'the toast says why');
          release();
          await until(async () => page.evaluate(() => /Living room/.test(document.querySelector('.page-head').textContent)) && (await rows(page)) === 2, 'the page then shows the Living room\'s queue');
          if (what === 'Clear') await page.screenshot({ path: path.join(shots, 'after-queue-stale-clear-1280.png') });
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // The page drawn again for the Living room asks for its queue; a zone event right after asks
    // again, and the page's own answer, now the older one, is dropped. Before, the page then drew the
    // Kitchen's songs (still in hand) under the Living room's name: a row's Play played the Living
    // room's song at that place, Remove sent the Kitchen's queue ids to the Living room. Now the
    // Kitchen's page stays (its taps say why and send nothing) until the newer answer draws the page.
    for (const w of [1280, 360]) {
      await check(`${w}: the page's queue fetch overtaken by a newer one: the old device's songs are never drawn under the new name`, async () => {
        const run = await zoneQueue(browser, w); const { page, context, state } = run;
        try {
          const held = []; let holding = true; // (each GET of the Living room's queue, held until released)
          await context.route(/\/v1\/zones\/z2\/queue$/, async (r) => { if (r.request().method() !== 'GET' || !holding) return r.fallback(); await new Promise((res) => held.push(res)); return r.fallback(); });
          run.fixtures['/presence'] = { body: [{ mine: true, key: 'host', name: 'Host', zone: { id: 'z2', name: 'Living room' }, your_zone: 'z1' }] };
          run.fixtures['/me/presence'] = { body: { visible: 'hidden' } };
          run.ws.send(JSON.stringify({ type: 'zone_removed', seq: 1, zone: 'z9' }));
          await until(async () => held.length >= 1, 'the app asks for the Living room\'s queue');
          await sleep(200); const a = held.length;
          // (a tap on the Kitchen's page: nothing sent, the page drawn again, which asks too)
          await page.click(w < 600 ? '[data-qmore="1"]' : '[data-qi="1"]');
          await until(async () => held.length > a, 'the page drawn again asks for the Living room\'s queue');
          const ownAnswer = held[held.length - 1]; const b = held.length;
          run.ws.send(JSON.stringify({ type: 'zone', seq: 2, state: { ...run.fixtures['/zones/z2'].body, queue_version: 2 } }));
          await until(async () => held.length > b, 'the zone event asks again');
          ownAnswer(); // (the page's own answer, now not the newest)
          await sleep(400);
          const shown = await page.evaluate(() => ({ head: document.querySelector('.page-head').textContent, rows: document.querySelectorAll('.queue [data-qrow]').length }));
          if (w >= 600) await page.screenshot({ path: path.join(shots, 'queue-overtaken-1280.png') });
          assert.ok(!(/Living room/.test(shown.head) && shown.rows === 4), `the Kitchen's 4 songs under the Living room's name (${shown.head}, ${shown.rows} rows)`);
          const n = state.hits.length;
          if (shown.rows) await page.click(w < 600 ? '[data-qmore="2"]' : '[data-qi="2"]', { force: true });
          if (shown.rows && w >= 600) await page.click('[data-qrm="2"]', { force: true });
          await sleep(300);
          assert.deepEqual(state.hits.slice(n).filter((h) => h.m !== 'GET').map((h) => h.m + ' ' + h.v), [], 'nothing asked of the node');
          assert.equal(await menu(page), '', 'no menu opened');
          holding = false; held.forEach((f) => f());
          await until(async () => page.evaluate(() => /Living room/.test(document.querySelector('.page-head').textContent)) && (await rows(page)) === 2, 'the page then shows the Living room\'s queue', 8000);
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    // The zone shown goes (removed on another device): the page is drawn again for this browser at
    // once. Before, it kept the Kitchen's songs and buttons, and its Clear sent DELETE /zones/browser/queue.
    for (const w of [1280, 360]) {
      await check(`${w}: the zone shown removed elsewhere: the page shows this browser's queue at once`, async () => {
        const run = await zoneQueue(browser, w); const { page, context, state } = run;
        try {
          run.ws.send(JSON.stringify({ type: 'zone_removed', seq: 1, zone: 'z1' }));
          await until(async () => page.evaluate(() => !/Kitchen/.test(document.querySelector('.page-head').textContent)), 'the page is drawn again');
          assert.equal(await page.locator('#qclear').count(), 0, 'no Clear for this browser\'s queue');
          assert.ok(!state.hits.some((h) => h.m !== 'GET'), 'nothing asked of the node');
          assert.deepEqual(state.errors, [], 'no page errors');
        } finally { await context.close(); }
      });
    }
    await check('1280: Save as playlist while the first is still on its way says so (after the page was drawn again)', async () => {
      const run = await zoneQueue(browser, 1280); const { page, context, state } = run;
      try {
        let release; const held = new Promise((r) => { release = r; }); let asked = false;
        await context.route(/\/v1\/playlists\/from-queue$/, async (r) => { asked = true; await held; return r.fallback(); });
        await page.click('#qsave');
        await until(async () => asked, 'the first save asks the node');
        // (the page drawn again meanwhile, here by Radio: its new Save button is not busy)
        run.fixtures['/zones/z1/radio'] = { body: {} };
        await page.click('#qradio');
        await until(async () => page.evaluate(() => { const b = document.querySelector('#qsave'); return b && !b.getAttribute('aria-busy') && /Radio on: similar/.test(document.body.innerText); }), 'the page is drawn again');
        await page.click('#qsave');
        await until(async () => page.evaluate(() => /Still saving the queue as a playlist/.test(document.body.innerText)), 'the second tap says the first is still saving');
        release(); await sleep(400);
        assert.equal(state.hits.filter((h) => h.m === 'POST' && h.v === '/playlists/from-queue').length, 1, 'saves sent');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check('1280: the desktop head keeps its five buttons and has no ⋯', async () => {
      const { page, context, state } = await zoneQueue(browser, 1280);
      try {
        const h = await head(page);
        assert.equal(h.ids, 'qradio,qshuffle,qtransfer,qsave,qclear', 'visible head buttons');
        await page.screenshot({ path: path.join(shots, 'after-queue-1280.png') });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} FAIL`); process.exit(1); }
  console.log('all passed');
})();
