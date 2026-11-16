// Songs › Play all on a library bigger than one part of the node's answer (1000 songs).
//   * The message counted nothing while thousands of songs loaded ("Loading 2,500 songs…" for as
//     long as it took). Now it counts up as each thousand comes: "Loading 2,500 songs… 1,000 so far",
//     "… 2,000 so far", and goes when the songs play.
//   * A node that stopped answering mid-library left that message up and Play all disabled for good.
//     Now a part with no answer for 30 s is given up: "The node stopped answering. Try again.", and
//     Play all can be pressed again (and then plays).
//   * A message that replaced the loading one meanwhile is not written over by the count.
// Real UI in Chromium against fixtures (no node), at 360 px in the browser and the Android app,
// 412 px and 1280 px. The 30 s are skipped with Playwright's clock.
//   node tools/ui-play-all-progress.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/play-all-progress-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const TOTAL = 2500;
const all = Array.from({ length: TOTAL }, (_, k) => ({ ...fx.track, id: k + 1, title: `Song ${k + 1}`, mbid: null, canonical_uri: null }));
const shown = (page) => page.evaluate(() => { const t = document.querySelector('#toast'); return t.classList.contains('show') ? t.textContent : ''; });
const toastIs = (page, re, timeout = 8000) => page.waitForFunction((src) => new RegExp(src).test(document.querySelector('#toast.show')?.textContent || ''), re.source, { timeout });

(async () => {
  const browser = await chromium.launch();
  let fails = 0;
  const check = async (name, fn) => { try { await fn(); console.log('ok  ', name); } catch (e) { fails++; console.log('FAIL', name, '-', e.message.split('\n')[0]); } };
  for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
    const tag = `${width}${android ? ' android' : ''}`;
    const file = `${width}${android ? '-android' : ''}`;
    // pages(after): ms before that part's answer (null: never answers)
    const open = async (pages, clock) => {
      const o = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/library/stats': { body: { tracks: TOTAL, albums: 1, artists: 2 } } } });
      if (android) await o.context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
      o.parts = [];
      await o.context.route('**/v1/library/tracks?*', async (r) => {
        const u = new URL(r.request().url()); const limit = Number(u.searchParams.get('limit') || 100); const after = u.searchParams.get('after');
        // (the page's own first part: a screenful, no cursor, so Play all has to fetch the rest)
        if (limit !== 1000) return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ tracks: all.slice(0, Math.min(limit, 100)), next: null }) });
        const from = after == null ? 0 : Number(after); o.parts.push(from);
        const wait = pages(from);
        if (wait === null) return; // the node never answers this part
        if (wait) await sleep(wait);
        try { await r.fulfill({ contentType: 'application/json', body: JSON.stringify({ tracks: all.slice(from, from + 1000), next: null }) }); } catch { /* the page gave up on it */ }
      });
      if (clock) await o.page.clock.install();
      await fx.go(o.page, '/tracks');
      await o.page.waitForSelector('#shuffle');
      return o;
    };
    await check(`Play all counts up as each thousand comes, and goes when they play (${tag})`, async () => {
      const { page, context, state, parts } = await open((from) => (from === 0 ? 0 : 1500));
      try {
        await page.click('#shuffle');
        await toastIs(page, /^Loading 2,500 songs… 1,000 so far$/);
        if (width === 360) await page.screenshot({ path: path.join(shots, `counting-${file}.png`) });
        await toastIs(page, /^Loading 2,500 songs… 2,000 so far$/);
        await page.waitForFunction(() => !document.querySelector('#shuffle').disabled, null, { timeout: 8000 });
        await sleep(400);
        assert.doesNotMatch(await shown(page), /Loading/, 'gone once they play');
        assert.deepEqual(parts, [0, 1000, 2000], 'three parts asked, in order');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Play all, the node stops answering: given up after 30 s, and Play all works again (${tag})`, async () => {
      let stuck = true;
      const { page, context, state, parts } = await open((from) => (from === 1000 && stuck ? null : 0), true);
      try {
        await page.click('#shuffle');
        await toastIs(page, /^Loading 2,500 songs… 1,000 so far$/);
        await page.clock.fastForward(25000);
        await sleep(300);
        assert.match(await shown(page), /^Loading 2,500 songs/, 'still loading after 25 s');
        assert.equal(await page.$eval('#shuffle', (b) => b.disabled), true, 'Play all waits meanwhile');
        await page.clock.fastForward(6000);
        await toastIs(page, /^The node stopped answering\. Try again\.$/, 5000);
        assert.equal(await page.$eval('#toast', (t) => t.classList.contains('error')), true, 'said as an error');
        await page.waitForFunction(() => !document.querySelector('#shuffle').disabled, null, { timeout: 5000 });
        if (width === 360) await page.screenshot({ path: path.join(shots, `stopped-${file}.png`) });
        // pressed again with the node answering: it plays
        stuck = false;
        const before = parts.length;
        await page.click('#shuffle');
        await page.waitForFunction(() => document.querySelector('#shuffle').disabled === false && !/Loading/.test(document.querySelector('#toast.show')?.textContent || ''), null, { timeout: 8000 });
        await page.waitForFunction(() => !!document.querySelector('audio')?.getAttribute('src'), null, { timeout: 8000 });
        assert.deepEqual(parts.slice(before), [0, 1000, 2000], 'the second press loads all three parts');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Play all: a message that replaced the loading one is not written over by the count (${tag})`, async () => {
      // (the first part comes 1.5 s after the press, well after the message shows at 0.4 s)
      const { page, context, state } = await open(() => 1500);
      try {
        await page.click('#shuffle');
        await toastIs(page, /^Loading 2,500 songs…$/);
        await page.evaluate(() => { const t = document.querySelector('#toast'); t.textContent = 'Another message'; });
        await sleep(2200);
        assert.equal(await shown(page), 'Another message', 'the count stays out of it');
        await page.waitForFunction(() => !document.querySelector('#shuffle').disabled, null, { timeout: 8000 });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  }
  await browser.close();
  console.log(fails ? `${fails} FAIL` : 'all ok');
  process.exit(fails ? 1 : 0);
})();
