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
const rows = (page) => page.evaluate(() => document.querySelectorAll('.queue [data-qrow]').length);
// The head's visible buttons, how many rows they take, and their sizes
const head = (page) => page.evaluate(() => {
  const bs = [...document.querySelectorAll('.page-head .actions > *')].filter((e) => e.getClientRects().length);
  const box = (e) => e.getBoundingClientRect();
  return { ids: bs.map((e) => e.id).join(','), rows: new Set(bs.map((e) => Math.round(box(e).top))).size, small: bs.filter((e) => box(e).width < 44 || box(e).height < 44).map((e) => e.id), over: document.documentElement.scrollWidth - innerWidth };
});
const menu = (page) => page.evaluate(() => { const m = document.querySelector('.popover.menu'); return m && !m.hidden ? [...m.children].map((c) => c.classList.contains('pop-title') ? '# ' + c.textContent.trim() : ((c.querySelector(':scope > span') || c).firstChild.textContent || '').trim()).join(' | ') : ''; });

async function zoneQueue(browser, width, opts = {}) {
  const Z = (id, name) => ({ id, name, output: 'upnp:' + id, transport: 'paused', now_playing: { item: items[0], position_ms: 1000, duration_ms: 60000 }, queue: id === 'z1' ? items : [], queue_index: 0, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } });
  const z1 = Z('z1', 'Kitchen'); const z2 = Z('z2', 'Living room');
  const fixtures = { '/zones': { body: [z1, z2] }, '/zones/z1': { body: z1 }, '/zones/z2': { body: z2 }, '/zones/z1/queue': { body: items }, '/zones/z2/queue': { body: [] },
    '/outputs': { body: [{ id: 'upnp:z1', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }, { id: 'upnp:z2', name: 'Living room', kind: 'upnp', caps: {} }] },
    '/playlists/from-queue': { body: { playlist: { summary: { id: 'q1', name: 'Queue · kitchen' } }, skipped: [] } } };
  const run = await fx.create(browser, width, { fixtures, zone: opts.browser ? 'browser' : 'z1', tracks, role: opts.role || 'owner', ...(width < 600 ? { mobile: true } : {}) });
  if (opts.android) await run.context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  if (opts.browser) await run.context.addInitScript((q) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', JSON.stringify({ index: 0, id: 101, pos: 0 })); }, JSON.stringify(items));
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [z1, z2] })); });
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
