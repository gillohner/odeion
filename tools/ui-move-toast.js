// A progress message goes when its answer comes, for a move and for Songs' Play all.
//   * Queue › Move to… (a phone: ⋯ › Move to) › another zone says "Moving to Lounge…". With the Play to menu opened while
//     the node moves it, the answer draws the menu again (no "Moved to" toast), and the message
//     stayed its full 3.8 s over the menu, saying it still moved. Now it goes with the answer; a
//     failed move shows its reason in the menu, with no "Moving…" over it either. With the menu
//     closed the answer's "Moved to Lounge" still replaces it.
//   * Songs › Play all on a library bigger than the page loaded says "Loading N songs…": it went
//     after 3.8 s while a big library still loaded (nothing said so any more), and it stayed up to
//     3.8 s over the songs once they played. Now it stays while they load and goes when they play;
//     a quick load (under 0.4 s) shows none.
// Real UI in Chromium against fixtures (no node), at 360/412 px in the browser and the Android app,
// and 1280 px.
//   node tools/ui-move-toast.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/move-toast-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const q = [{ uri: fx.SONG, title: 'Journey Through Time', artist: 'Sefa', album: 'Journey', duration_ms: 240000 }];
const outputs = [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }, { id: 'upnp:l', name: 'Lounge', kind: 'upnp', caps: { hw_volume: 'percent' } }];
const zone = (id, name, output, more) => ({ id, name, output, transport: 'stopped', now_playing: null, queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 }, ...more });
// Kitchen is paused on the song (no wait in the bar after the move); Lounge is empty
const K = zone('z1', 'Kitchen', 'upnp:k', { transport: 'paused', now_playing: { item: q[0], index: 0, position_ms: 30000, duration_ms: 240000 }, queue: q, queue_index: 0 });
const L = zone('z2', 'Lounge', 'upnp:l');
const moved = { ...L, transport: 'paused', now_playing: K.now_playing, queue: q, queue_index: 0 };
const MOVING = /Moving to Lounge/;
// The message showing now ('' when none)
const shown = (page) => page.evaluate(() => { const t = document.querySelector('#toast'); return t.classList.contains('show') ? t.textContent : ''; });

(async () => {
  const browser = await chromium.launch();
  let fails = 0;
  const check = async (name, fn) => { try { await fn(); console.log('ok  ', name); } catch (e) { fails++; console.log('FAIL', name, '-', e.message.split('\n')[0]); } };
  const cases = [[360, false], [412, false], [360, true], [412, true], [1280, false]];
  for (const [width, android] of cases) {
    const tag = `${width}${android ? ' android' : ''}`;
    const file = `${width}${android ? '-android' : ''}`;
    const open = async (opts) => {
      const o = await fx.create(browser, width, { mobile: width < 600, ...opts });
      if (android) await o.context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
      return o;
    };
    // Kitchen's Queue page, the node's answer to the move after `ms`
    const queueOf = async (transfer, ms) => {
      const fixtures = { '/zones': { body: [K, L] }, '/zones/z1': { body: K }, '/zones/z2': { body: L }, '/outputs': { body: outputs }, '/zones/z1/transfer': transfer, '/zones/z2/queue': { body: q } };
      const o = await open({ fixtures, delay: (v) => (v === '/zones/z1/transfer' ? ms : 0) });
      await o.context.addInitScript(() => localStorage.setItem('pm.zone', 'z1'));
      await o.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [K, L] })); });
      // (Kitchen's queue; the move empties it)
      await o.page.route(/\/v1\/zones\/z1\/queue(\?|$)/, (r) => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o.moved ? [] : q) }));
      await fx.go(o.page, '/queue');
      await o.page.waitForFunction(() => document.querySelector('#zonename').textContent === 'Kitchen');
      // (a phone has Move to under the head's ⋯; the desktop layout has its button)
      await o.page.waitForSelector('#qtransfer:visible, #qmenu:visible');
      return o;
    };
    const moveToLounge = async (page) => {
      await page.click('#qtransfer:visible, #qmenu:visible');
      await page.locator('.popover.menu:not([hidden]) .pop-item[data-mi]', { hasText: 'Lounge' }).first().click();
      await page.waitForFunction(() => /Moving to Lounge/.test(document.querySelector('#toast.show')?.textContent || ''));
    };
    const openPlayTo = (page) => page.$eval('#zonebtn', (b) => b.click()).then(() => page.waitForSelector('#zonepop:not([hidden])'));
    const answered = (state) => state.hits.some((h) => h.v === '/zones/z1/transfer');

    await check(`Move to › Lounge, Play to opened meanwhile: "Moving to…" goes when the node answers (${tag})`, async () => {
      const o = await queueOf({ body: moved }, 1500); const { page, context, state } = o;
      try {
        await moveToLounge(page);
        await openPlayTo(page);
        // (the answer, then the menu drawn again: well inside the message's own 3.8 s)
        await page.waitForFunction(() => document.querySelector('#zonename').textContent === 'Lounge', null, { timeout: 5000 });
        o.moved = true;
        await sleep(400);
        assert.ok(answered(state), 'the node was asked');
        assert.ok(await page.isVisible('#zonepop'), 'the menu is still open');
        await page.screenshot({ path: path.join(shots, `moved-menu-${file}.png`) });
        assert.doesNotMatch(await shown(page), MOVING, 'no "Moving to Lounge…" over the menu after the move');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Move to › Lounge fails, Play to open: the menu says why, no "Moving to…" over it (${tag})`, async () => {
      const { page, context, state } = await queueOf({ status: 400, body: { error: 'Lounge is not reachable' } }, 1500);
      try {
        await moveToLounge(page);
        await openPlayTo(page);
        await page.waitForSelector('#zonepop .pop-err', { timeout: 5000 });
        await sleep(400);
        assert.match(await page.textContent('#zonepop .pop-err'), /not reachable/, 'the reason is in the menu');
        await page.screenshot({ path: path.join(shots, `failed-menu-${file}.png`) });
        assert.doesNotMatch(await shown(page), MOVING, 'no "Moving to Lounge…" over the menu');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Move to › Lounge, menu closed: "Moved to Lounge" replaces "Moving to…" (${tag})`, async () => {
      const o = await queueOf({ body: moved }, 600); const { page, context, state } = o;
      try {
        await moveToLounge(page);
        await page.waitForFunction(() => /Moved to Lounge/.test(document.querySelector('#toast.show')?.textContent || ''), null, { timeout: 5000 });
        o.moved = true;
        await sleep(400);
        assert.match(await shown(page), /Moved to Lounge/, 'the answer stays its time');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Songs › Play all: "Loading N songs…" stays while they load and goes when they play (${tag})`, async () => {
      // (5 songs in the library, 1 on the page: Play all fetches them all, slowly: longer than a
      // message's own 3.8 s)
      let n = 0;
      const { page, context, state } = await open({ fixtures: { '/library/stats': { body: { tracks: 5, albums: 1, artists: 2 } } }, delay: (v) => (v === '/library/tracks' && n++ > 0 ? 6000 : 0) });
      try {
        await fx.go(page, '/tracks');
        await page.waitForSelector('#shuffle');
        await page.click('#shuffle');
        await page.waitForFunction(() => /Loading 5 songs/.test(document.querySelector('#toast.show')?.textContent || ''));
        await sleep(4500);
        assert.match(await shown(page), /Loading 5 songs/, 'still says so after 4.5 s, while the songs load');
        if (width === 360 && !android) await page.screenshot({ path: path.join(shots, 'loading-360.png') });
        await page.waitForFunction(() => !document.querySelector('#shuffle').disabled, null, { timeout: 6000 });
        await sleep(400);
        assert.doesNotMatch(await shown(page), /Loading/, 'gone once they play');
        assert.ok(state.hits.filter((h) => h.v === '/library/tracks').length >= 2, 'Play all fetched the library');
        await page.screenshot({ path: path.join(shots, `played-${file}.png`) });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Songs › Play all, a quicker library: the message goes when the songs play, not 3.8 s later (${tag})`, async () => {
      let n = 0;
      const { page, context, state } = await open({ fixtures: { '/library/stats': { body: { tracks: 5, albums: 1, artists: 2 } } }, delay: (v) => (v === '/library/tracks' && n++ > 0 ? 1200 : 0) });
      try {
        await fx.go(page, '/tracks');
        await page.waitForSelector('#shuffle');
        await page.click('#shuffle');
        await page.waitForFunction(() => /Loading 5 songs/.test(document.querySelector('#toast.show')?.textContent || ''));
        await page.waitForFunction(() => !document.querySelector('#shuffle').disabled, null, { timeout: 6000 });
        await sleep(400);
        assert.doesNotMatch(await shown(page), /Loading/, 'gone once they play');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Songs › Play all, a quick library: no "Loading…" flash (${tag})`, async () => {
      let n = 0;
      const { page, context } = await open({ fixtures: { '/library/stats': { body: { tracks: 5, albums: 1, artists: 2 } } }, delay: (v) => (v === '/library/tracks' && n++ > 0 ? 100 : 0) });
      try {
        await fx.go(page, '/tracks');
        await page.waitForSelector('#shuffle');
        // (every message the page shows, however briefly)
        await page.evaluate(() => { window.__said = []; new MutationObserver(() => { const t = document.querySelector('#toast'); if (t.classList.contains('show')) window.__said.push(t.textContent); }).observe(document.querySelector('#toast'), { attributes: true, childList: true, characterData: true, subtree: true }); });
        await page.click('#shuffle');
        await page.waitForFunction(() => !document.querySelector('#shuffle').disabled, null, { timeout: 6000 });
        await sleep(600);
        const said = await page.evaluate(() => window.__said);
        assert.ok(!said.some((m) => /Loading/.test(m)), `no flash of the loading message: ${JSON.stringify(said)}`);
      } finally { await context.close(); }
    });
  }
  await browser.close();
  console.log(fails ? `${fails} FAIL` : 'all ok');
  process.exit(fails ? 1 : 0);
})();
