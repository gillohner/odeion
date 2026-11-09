// A saved search opens in its selected source, including its first request.
// Covers direct links, reload, changing the source, Back/Forward, a new query,
// the Everywhere default, and pasted links/people still opening their pages.
// Isolated real app assets; fixture API only, with no network egress.
//   node tools/ui-search-routing.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = process.argv[3] && path.resolve(process.argv[3]);
if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
const QUERY = 'Journey & Time';
const sources = [{ id: 'tidal', name: 'TIDAL', enabled: true, status: 'signed in', features: ['search'] }];
const hit = { kind: 'song', id: 'lib-1', title: QUERY, subtitle: 'Sefa', track_id: 1, keys: ['track:1'] };
const song = { kind: 'song', id: 'lib-1', title: QUERY, target: { track_id: 1 },
  source: 'library', section: 'tracks', artist: { name: 'Sefa', id: 'lib-11' },
  plays_from: { service: 'library', label: 'Your library' }, songs: [] };

async function fixture(browser, width) {
  const run = await fx.create(browser, width, { fixtures: {
    '/sources': { body: sources }, '/item/song/lib-1': { body: song },
  } });
  run.requests = [];
  await run.context.route(url => url.pathname === '/v1/search', async route => {
    const url = new URL(route.request().url());
    const q = url.searchParams.get('q'), scope = url.searchParams.get('in');
    run.requests.push({ q, scope });
    const source = scope === 'everywhere' ? 'library' : scope;
    await route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': source },
      body: JSON.stringify({ source, status: 'ok', hits: [hit], counts: [], ms: 1 }) + '\n' });
  });
  return run;
}

async function drawn(page, scope, q = QUERY) {
  await page.locator('#sresults .srow').first().waitFor();
  assert.equal(await page.locator('#scopechips .on').getAttribute('data-scope-in'), scope, 'selected source');
  assert.equal(await page.locator('#search').inputValue(), q, 'query stays in the box');
  assert.equal(new URL(page.url()).searchParams.get('q'), q, 'query in the URL');
  assert.equal(new URL(page.url()).searchParams.get('in') || 'everywhere', scope, 'source in the URL');
  assert.equal(await page.evaluate(() => history.state.arg.in || 'everywhere'), scope, 'history source');
  await fx.cleanCopy(page);
}

(async () => {
  const browser = await chromium.launch();
  try {
    // The shared startup runs on phones too; keep a narrow-screen regression check.
    for (const width of [1280, 1024, 768, 360, 412]) {
      for (const savedScope of ['library', 'pubky', 'tidal', 'everywhere', null]) {
        const scope = savedScope || 'everywhere';
        const run = await fixture(browser, width);
        const { page, context, state, requests } = run;
        try {
          const params = new URLSearchParams({ q: QUERY });
          if (savedScope) params.set('in', savedScope);
          await fx.go(page, '/search?' + params);
          await page.locator('#sresults .srow').first().waitFor();
          if (SHOTS && savedScope === 'library') await page.screenshot({ path: path.join(SHOTS, `${process.env.SHOT_PREFIX || 'after'}-search-routing-${width}.png`) });
          assert.deepEqual(requests, [{ q: QUERY, scope }], 'first request uses only the saved source');
          await drawn(page, scope);

          requests.length = 0;
          await page.reload();
          await drawn(page, scope);
          assert.deepEqual(requests, [{ q: QUERY, scope }], 'reload issues one request to the same source');

          // A source chosen in the page must also survive a reload.
          const nextScope = scope === 'library' ? 'tidal' : 'library';
          await page.locator(`[data-scope-in="${nextScope}"]`).click();
          await drawn(page, nextScope);
          requests.length = 0;
          await page.reload();
          await drawn(page, nextScope);
          assert.deepEqual(requests, [{ q: QUERY, scope: nextScope }], 'reload preserves a newly chosen source');

          // Open a result by keyboard, then return through the browser history.
          await page.focus('#search');
          await page.press('#search', 'ArrowDown');
          await page.press('#search', 'Enter');
          await page.waitForURL('**/song/lib-1');
          await page.locator('#iplay').waitFor();
          await page.goBack();
          await drawn(page, nextScope);
          await page.goForward();
          await page.waitForURL('**/song/lib-1');
          await page.locator('#iplay').waitFor();
          await page.goBack();
          await drawn(page, nextScope);
          assert.equal(requests.length, 1, 'Back/Forward reuses the current search');

          // A fresh query keeps the restored source; the keyboard still submits.
          requests.length = 0;
          await page.fill('#search', 'Another journey');
          await page.press('#search', 'Enter');
          await drawn(page, nextScope, 'Another journey');
          assert.deepEqual(requests, [{ q: 'Another journey', scope: nextScope }]);
          assert.deepEqual(state.errors, []);
          console.log(`PASS ${width} ${savedScope || 'default'}: direct link, reload, source change, Back/Forward, keyboard query, no overflow`);
        } finally { await context.close(); }
      }
    }

    // Back while the item is still reading the account: a late reply must not replace the
    // restored search with the old item's loading screen. Hold the response until Back is done.
    for (const width of [1280, 1024, 768, 360, 412]) {
      const { page, context, state, requests } = await fixture(browser, width);
      let release;
      const gate = new Promise(resolve => { release = resolve; });
      await context.route(url => url.pathname === '/v1/pubky', async route => {
        await gate;
        await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ user: null }) });
      });
      try {
        await fx.go(page, '/search?' + new URLSearchParams({ q: QUERY, in: 'library' }));
        await drawn(page, 'library');
        await page.focus('#search');
        await page.press('#search', 'ArrowDown');
        const reading = page.waitForRequest(url => new URL(url.url()).pathname === '/v1/pubky');
        await page.press('#search', 'Enter');
        await page.waitForURL('**/song/lib-1');
        await reading;
        await page.goBack();
        await drawn(page, 'library');
        const answered = page.waitForResponse(url => new URL(url.url()).pathname === '/v1/pubky');
        release();
        await (await answered).finished();
        // Watch the restored page through the late response's rendering turn.
        await page.waitForTimeout(150);
        if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `${process.env.SHOT_PREFIX || 'after'}-search-back-pending-${width}.png`) });
        assert.equal(await page.locator('#sresults .srow').count(), 1, 'late account reply keeps the restored results');
        await drawn(page, 'library');
        assert.deepEqual(requests, [{ q: QUERY, scope: 'library' }], 'Back needs no replacement search');
        await page.goForward();
        await page.locator('#iplay').waitFor();
        assert.equal(await page.locator('#main .hero h1').innerText(), QUERY, 'the item still opens after the account reply');
        assert.deepEqual(state.errors, []);
        console.log(`PASS ${width}: Back during account loading keeps results after the late reply; Forward opens the song`);
      } finally { release(); await context.close(); }
    }

    // Startup still goes through the usual query handling, not just the renderer.
    const key = 'zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o';
    for (const [q, target] of [['https://tidal.com/album/199574241', '/album/'], [key, '/person/']]) {
      const { page, context, requests, state } = await fixture(browser, 1280);
      try {
        await fx.go(page, '/search?' + new URLSearchParams({ q, in: 'library' }));
        await page.waitForURL(url => url.pathname.startsWith(target));
        assert.deepEqual(requests, [], 'links and people open without a search');
        assert.deepEqual(state.errors, []);
        console.log(`PASS saved query opens ${target} without searching`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
