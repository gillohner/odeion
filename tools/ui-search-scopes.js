// Saved service searches keep a visible, reusable source choice for owners and listeners.
// Real assets with isolated API fixtures; no credentials or network egress.
//   node tools/ui-search-scopes.js [repo] [screenshots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = process.argv[3] && path.resolve(process.argv[3]);
if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
const sources = ['tidal', 'qobuz', 'spotify', 'deezer'].map((id, i) => ({
  id, name: ['TIDAL', 'Qobuz', 'Spotify', 'Deezer'][i], enabled: true, status: 'signed in', features: ['search'],
}));
const wait = () => { let release; const promise = new Promise(resolve => { release = resolve; }); return { promise, release }; };
async function fixture(browser, width, role, delayedSources = false) {
  const services = sources.map(s => ({ ...s }));
  const run = await fx.create(browser, width, { role, token: `fixture-${role}`, mobile: width < 600, fixtures: { '/sources': { body: services } } });
  run.services = services;
  run.requests = [];
  run.tags = wait(); run.sources = wait();
  if (!delayedSources) run.sources.release();
  await run.context.route(url => url.pathname === '/v1/search/tags', async route => {
    await run.tags.promise;
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ mine: [], others: ['fixture-arrived'] }) });
  });
  await run.context.route(url => url.pathname === '/v1/sources', async route => {
    // Let the shared fixture record the request too, including any forbidden listener read.
    await run.sources.promise;
    await route.fallback();
  });
  await run.context.route(url => url.pathname === '/v1/search', async route => {
    const request = route.request(), url = new URL(request.url());
    const q = url.searchParams.get('q'), scope = url.searchParams.get('in');
    run.requests.push({ q, scope, token: request.headers().authorization });
    // The real endpoint routes #tags to Library and Pubky, even with a service in the URL.
    const answers = q.startsWith('#') ? ['library', 'pubky'] : [scope === 'everywhere' ? 'library' : scope];
    await route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': answers.join(',') },
      body: answers.map(source => JSON.stringify({ source, status: 'ok', hits: [{ kind: 'song', id: 'lib-1', title: q, subtitle: 'Fixture artist', keys: ['track:1'], track_id: 1 }], counts: [], ms: 1 })).join('\n') + '\n' });
  });
  return run;
}
async function settled(page, scope) {
  await page.waitForFunction(scope => window.pmLastSearch?.scope === scope, scope);
  await page.locator('#sresults .srow').first().waitFor();
}
async function selected(page, scope) {
  assert.equal(await page.locator(`#scopechips .on[data-scope-in="${scope}"]`).count(), 1, 'the saved service has a selected source control');
  const bounds = await page.locator(`#scopechips [data-scope-in="${scope}"]`).evaluate(button => {
    const row = button.parentElement.getBoundingClientRect(), b = button.getBoundingClientRect();
    return { left: b.left, right: b.right, min: Math.max(0, row.left), max: Math.min(innerWidth, row.right) };
  });
  assert.ok(bounds.left >= bounds.min - 1 && bounds.right <= bounds.max + 1, `selected source is fully visible: ${JSON.stringify(bounds)}`);
  await fx.cleanCopy(page);
}
const requestScopes = run => run.requests.map(r => r.scope);
async function scrollLeftByHand(page) {
  await page.locator('#scopechips').hover();
  await page.evaluate(() => {
    window.scopeWheeled = false;
    document.querySelector('#scopechips').addEventListener('wheel', () => { window.scopeWheeled = true; }, { once: true });
  });
  await page.mouse.wheel(-1000, 0);
  // At a width where the row already fits, scrollLeft is zero before the wheel reaches it.
  // Wait for the real input event before letting delayed source discovery answer.
  await page.waitForFunction(() => window.scopeWheeled && document.querySelector('#scopechips').scrollLeft === 0);
}
const noOwnerReads = run => {
  if (run.state.role === 'listener') assert.deepEqual(run.state.hits.filter(r => ['/sources', '/play-order', '/favourites', '/playlists'].includes(r.v)), [], 'listeners never read owner metadata');
  assert.ok(run.requests.every(r => r.token === `Bearer ${run.state.token}`), 'searches keep the current caller token');
  assert.deepEqual(run.state.errors, []);
};

(async () => {
  const browser = await chromium.launch();
  let failures = 0;
  const check = async (name, fn) => {
    try { await fn(); console.log('PASS ' + name); }
    catch (error) { failures++; console.error('FAIL ' + name + ': ' + error.message); }
  };
  try {
    for (const width of [1280, 1024, 768, 360, 412]) for (const role of ['owner', 'listener']) for (const colorScheme of ['light', 'dark']) {
      await check(`${width} ${role} ${colorScheme}: direct/reload, source reselect, keyboard focus and manual scroll`, async () => {
        const run = await fixture(browser, width, role), { page, context } = run;
        const scope = role === 'owner' ? 'spotify' : 'tidal';
        try {
          await page.emulateMedia({ colorScheme });
          await fx.go(page, `/search?q=Review&in=${scope}`);
          await settled(page, scope);
          if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `${process.env.SHOT_PREFIX || 'after'}-scope-${width}-${role}-${colorScheme}.png`) });
          await selected(page, scope);
          assert.deepEqual(requestScopes(run), [scope], 'the direct link starts only its service');
          assert.equal(await page.evaluate(() => document.activeElement.tagName), 'BODY', 'revealing the source does not take focus');
          await page.reload();
          await settled(page, scope);
          await selected(page, scope);
          assert.deepEqual(requestScopes(run), [scope, scope], 'reload keeps its source');
          await page.locator('[data-scope-in="library"]').click();
          await settled(page, 'library');
          assert.equal(await page.locator(`[data-scope-in="${scope}"]`).count(), 1, 'the restored service remains an option after changing source');
          await page.locator(`[data-scope-in="${scope}"]`).focus();
          await page.keyboard.press('Enter');
          await selected(page, scope);
          assert.equal(await page.evaluate(() => document.activeElement.dataset.scopeIn), scope, 'source activation keeps keyboard focus');
          assert.deepEqual(requestScopes(run), [scope, scope, 'library'], 'reselecting the service reuses its results');

          // A later result redraw must retain the same button, including native activation
          // between keydown and keyup, and must not undo a person's scroll away from it.
          await page.evaluate(() => { window.keptScopeButton = document.activeElement; });
          if (width < 600) await scrollLeftByHand(page);
          const scroll = await page.locator('#scopechips').evaluate(row => row.scrollLeft);
          run.tags.release();
          await page.locator('[data-stag="fixture-arrived"]').waitFor();
          assert.equal(await page.evaluate(() => document.activeElement === window.keptScopeButton && window.keptScopeButton.isConnected), true, 'late results keep the actual focused button');
          assert.equal(await page.locator('#scopechips').evaluate(row => row.scrollLeft), scroll, 'late results preserve manual horizontal scrolling');
          await fx.cleanCopy(page);
          noOwnerReads(run);
        } finally { run.tags.release(); run.sources.release(); await context.close(); }
      });
    }
    for (const width of [360, 412, 768]) for (const manual of [false, true]) {
      await check(`${width}: delayed sources ${manual ? 'respect manual scrolling' : 'keep the selected source visible'} without taking search focus`, async () => {
        const run = await fixture(browser, width, 'owner', true), { page, context } = run;
        try {
          await fx.go(page, '/search?q=Delayed&in=spotify');
          await settled(page, 'spotify');
          await selected(page, 'spotify');
          await page.focus('#search');
          if (manual) await scrollLeftByHand(page);
          run.sources.release();
          await page.locator('[data-scope-in="deezer"]').waitFor();
          if (manual) assert.equal(await page.locator('#scopechips').evaluate(row => row.scrollLeft), 0, 'source discovery does not undo manual scrolling');
          else await selected(page, 'spotify');
          assert.equal(await page.evaluate(() => document.activeElement.id), 'search', 'source discovery preserves focus elsewhere');
          noOwnerReads(run);
        } finally { run.tags.release(); run.sources.release(); await context.close(); }
      });
    }
    for (const width of [360, 412, 1280]) {
      await check(`${width}: listener tag searches retain their requested service on direct/reload and reselect`, async () => {
        const run = await fixture(browser, width, 'listener'), { page, context } = run;
        try {
          await fx.go(page, '/search?q=%23jazz&in=tidal');
          await settled(page, 'tidal'); await selected(page, 'tidal');
          await page.reload();
          await settled(page, 'tidal'); await selected(page, 'tidal');
          await page.locator('[data-scope-in="library"]').click();
          await settled(page, 'library');
          await page.locator('[data-scope-in="tidal"]').click();
          await selected(page, 'tidal');
          assert.deepEqual(requestScopes(run), ['tidal', 'tidal', 'library']);
          noOwnerReads(run);
        } finally { run.tags.release(); run.sources.release(); await context.close(); }
      });
      await check(`${width}: source discovery reorders saved choices around the focused button`, async () => {
        const run = await fixture(browser, width, 'owner', true), { page, context } = run;
        try {
          await fx.go(page, '/search?q=Review&in=spotify');
          await settled(page, 'spotify');
          // A restored browser-history entry supplies another saved scope before discovery.
          await page.evaluate(() => {
            const state = { v: 'search', arg: { q: 'Review', in: 'tidal' }, depth: 1 };
            history.pushState(state, '', '/search?q=Review&in=tidal');
            dispatchEvent(new PopStateEvent('popstate', { state }));
          });
          await settled(page, 'tidal');
          await page.locator('[data-scope-in="tidal"]').focus();
          await page.evaluate(() => { window.keptScopeButton = document.activeElement; });
          run.sources.release();
          await page.locator('[data-scope-in="deezer"]').waitFor();
          assert.deepEqual(await page.locator('#scopechips button').evaluateAll(bs => bs.map(b => b.dataset.scopeIn)), ['everywhere', 'library', 'pubky', 'tidal', 'qobuz', 'spotify', 'deezer']);
          assert.equal(await page.evaluate(() => document.activeElement === window.keptScopeButton), true, 'reordering never detaches the focused button');
          noOwnerReads(run);
        } finally { run.tags.release(); run.sources.release(); await context.close(); }
      });
    }
    for (const width of [360, 412]) {
      for (const manual of [false, true]) {
        await check(`${width}: Tab to a restored service ${manual ? 'then scroll away' : 'keeps focus visible'} during source discovery`, async () => {
          const run = await fixture(browser, width, 'owner', true), { page, context } = run;
          try {
            await fx.go(page, '/search?q=Review&in=spotify');
            await settled(page, 'spotify');
            await page.focus('#search');
            for (let tabs = 0; tabs < 12 && await page.evaluate(() => document.activeElement.dataset.scopeIn !== 'spotify'); tabs++) await page.keyboard.press('Tab');
            assert.equal(await page.evaluate(() => document.activeElement.dataset.scopeIn), 'spotify', 'Tab reaches the restored service');
            await selected(page, 'spotify');
            await page.evaluate(() => { window.keptScopeButton = document.activeElement; });
            if (manual) await scrollLeftByHand(page);
            run.sources.release();
            await page.locator('[data-scope-in="deezer"]').waitFor();
            assert.equal(await page.evaluate(() => document.activeElement === window.keptScopeButton && window.keptScopeButton.isConnected), true, 'discovery retains the actual focused button');
            if (manual) assert.equal(await page.locator('#scopechips').evaluate(row => row.scrollLeft), 0, 'discovery respects scrolling away from the focused service');
            else await selected(page, 'spotify');
            noOwnerReads(run);
          } finally { run.tags.release(); run.sources.release(); await context.close(); }
        });
      }
      await check(`${width}: Tab into the source row keeps the focused choice visible when sources arrive`, async () => {
        const run = await fixture(browser, width, 'owner', true), { page, context } = run;
        try {
          await fx.go(page, '/search?q=Review&in=spotify');
          await settled(page, 'spotify');
          await page.focus('#search');
          for (let tabs = 0; tabs < 6 && await page.evaluate(() => document.activeElement.dataset.scopeIn !== 'everywhere'); tabs++) await page.keyboard.press('Tab');
          assert.equal(await page.evaluate(() => document.activeElement.dataset.scopeIn), 'everywhere', 'Tab enters the row at Everywhere');
          run.sources.release();
          await page.locator('[data-scope-in="deezer"]').waitFor();
          const focus = await page.evaluate(() => {
            const b = document.activeElement, r = b.getBoundingClientRect();
            return { scope: b.dataset.scopeIn, visible: r.left >= 0 && r.right <= innerWidth };
          });
          assert.deepEqual(focus, { scope: 'everywhere', visible: true }, 'discovery keeps the focused choice on screen');
          noOwnerReads(run);
        } finally { run.tags.release(); run.sources.release(); await context.close(); }
      });
    }
    await check('switching a service off keeps it out of previously saved search choices', async () => {
      const run = await fixture(browser, 1280, 'owner'), { page, context } = run;
      await context.route(url => url.pathname === '/v1/sources/tidal/enabled', route => {
        run.services[0].enabled = JSON.parse(route.request().postData()).enabled;
        return route.fulfill({ contentType: 'application/json', body: '{}' });
      });
      try {
        await fx.go(page, '/search?q=Review&in=tidal');
        await settled(page, 'tidal'); await selected(page, 'tidal');
        await page.locator('[data-view="sources"]').click();
        await page.locator('[data-src-on="tidal"]').uncheck();
        await page.locator('.toast').filter({ hasText: 'is off' }).waitFor();
        await page.locator('[data-src-on="tidal"]:not(:checked)').waitFor();
        await page.goBack();
        await page.locator('#scopechips').waitFor();
        assert.equal(await page.locator('[data-scope-in="tidal"]').count(), 0, 'a cached scope cannot reintroduce an explicitly disabled service');
        noOwnerReads(run);
      } finally { run.tags.release(); run.sources.release(); await context.close(); }
    });
    assert.equal(failures, 0, `${failures} source-control cases failed`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
