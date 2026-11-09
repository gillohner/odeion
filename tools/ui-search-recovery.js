// Failed searches must be retryable, without claiming that nothing matched.
// Real UI in Chromium, isolated search responses (no node or external services).
//   node tools/ui-search-recovery.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/search-recovery-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(page, predicate, label) {
  const end = Date.now() + 7000;
  while (Date.now() < end) {
    if (await page.evaluate(predicate)) return;
    await sleep(50);
  }
  throw new Error('Timed out: ' + label);
}
const hit = { kind: 'song', id: 'lib-1', title: fx.track.title, subtitle: 'Sefa, Dr. Peacock', track_id: 1, uri: 'track:1', keys: ['track:1'] };
const answer = (hits = [hit]) => JSON.stringify({ source: 'library', status: 'ok', hits, counts: [] });
const response = (body, sources = 'library') => ({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': sources }, body });

async function recovery(browser, width, mode, theme = 'light') {
  const run = await fx.create(browser, width);
  const { page, context, state } = run;
  const scope = mode === 'partial' || mode === 'incomplete' ? 'everywhere' : 'library';
  await context.addInitScript((t) => localStorage.setItem('pm.theme', t), theme);
  const requests = [];
  let releaseRetry;
  const retryReply = new Promise((resolve) => { releaseRetry = resolve; });
  await context.route(/\/v1\/search\?/, async (route) => {
    const url = new URL(route.request().url());
    if (url.searchParams.get('in') !== scope) return route.fulfill(response(answer() + '\n'));
    requests.push({ q: url.searchParams.get('q'), scope: url.searchParams.get('in') });
    if (requests.length === 1) {
      if (mode === 'http') return route.fulfill({ status: 503, body: 'Temporarily unavailable' });
      if (mode === 'network') return route.abort('connectionreset');
      if (mode === 'malformed') return route.fulfill(response('{broken\n'));
      if (mode === 'partial') return route.fulfill(response(answer() + '\n{broken\n', 'library,tidal'));
      if (mode === 'incomplete') return route.fulfill(response(answer() + '\n', 'library,tidal'));
    }
    await retryReply;
    return route.fulfill(response(answer() + '\n'));
  });
  try {
    await fx.go(page, '/search?q=Journey');
    if (scope === 'library') await page.locator('[data-scope-in="library"]').click();
    await until(page, () => window.pmLastSearch?.q === 'Journey', 'search finishes');
    if (scope === 'library') await until(page, () => window.pmLastSearch?.scope === 'library', 'scoped search finishes');
    await page.screenshot({ path: path.join(shots, `${mode}-${width}-${theme}.png`) });
    assert.match(await page.locator('#sresults').innerText(), /Search couldn.t finish/, `${mode}: persistent search failure`);
    assert.doesNotMatch(await page.locator('#sresults').innerText(), /Nothing matched/, 'failure is not an empty result');
    const retry = page.locator('#sresults button[data-sretry]');
    assert.equal(await retry.count(), 1, 'one retry button');
    if (mode === 'partial' || mode === 'incomplete') {
      assert.ok((await page.locator('#sresults').innerText()).includes(hit.title), 'received results stay available');
      if (mode === 'incomplete') {
        await sleep(5100);
        assert.equal(await page.locator('#sresults [data-sretry]').count(), 1, 'a late slow-source timer does not add a second retry');
      }
    }
    await fx.cleanCopy(page);
    assert.equal(await page.locator('#sprog').isVisible(), false, 'failure ends the progress indicator');
    // Keyboard activation must issue a fresh search and retain the user's query and scope.
    await retry.focus();
    await page.keyboard.press('Enter');
    await until(page, () => !document.querySelector('#sprog').hidden, 'retry progress appears');
    assert.equal(await retry.count(), 0, 'retry disappears while searching, preventing repeated presses');
    assert.doesNotMatch(await page.locator('#sresults').innerText(), /Nothing matched/, 'waiting for retry headers is not an empty result');
    assert.equal(await page.locator('#search').inputValue(), 'Journey');
    assert.equal(await page.locator('#search').evaluate((e) => e === document.activeElement), true, 'focus returns to search');
    releaseRetry();
    await until(page, () => document.querySelector('#sprog').hidden && !!document.querySelector('#sresults .srow'), 'retry succeeds');
    assert.deepEqual(requests, [{ q: 'Journey', scope }, { q: 'Journey', scope }], 'one fresh request for the same query and scope');
    assert.doesNotMatch(await page.locator('#sresults').innerText(), /Search couldn.t finish|Nothing matched/);
    assert.deepEqual(state.errors, [], 'no page errors');
    await fx.cleanCopy(page);
    await page.screenshot({ path: path.join(shots, `recovered-${mode}-${width}-${theme}.png`) });
    console.log(`PASS ${width} ${theme} ${mode}: error, retained results, keyboard retry, progress, recovery`);
  } finally {
    releaseRetry();
    await context.close();
  }
}

async function healthyAndCancelled(browser) {
  const { page, context, state } = await fx.create(browser, 1280);
  await context.route(/\/v1\/search\?/, async (route) => {
    const q = new URL(route.request().url()).searchParams.get('q');
    if (q.startsWith('Slow')) await sleep(500);
    // A complete final answer without a newline is still a valid search response.
    return route.fulfill(response(answer(q === 'Empty' ? [] : [hit])));
  });
  try {
    await fx.go(page, '/search?q=Empty&in=library');
    await until(page, () => window.pmLastSearch?.q === 'Empty', 'empty search finishes');
    assert.match(await page.locator('#sresults').innerText(), /Nothing matched/, 'successful empty searches keep their empty state');
    assert.equal(await page.locator('[data-sretry]').count(), 0);
    await page.fill('#search', 'Slow');
    await page.press('#search', 'Enter');
    await page.fill('#search', 'Journey');
    await page.press('#search', 'Enter');
    await until(page, () => window.pmLastSearch?.q === 'Journey', 'newer search finishes');
    await sleep(600);
    assert.match(await page.locator('#sresults').innerText(), /Journey Through Time/);
    assert.doesNotMatch(await page.locator('#sresults').innerText(), /Search couldn.t finish|Nothing matched/);
    assert.deepEqual(state.errors, []);
    await page.fill('#search', 'journey');
    await page.press('#search', 'Enter');
    await page.locator('#scopechips [data-scope-in="everywhere"]').waitFor();
    assert.match(await page.locator('#sresults').innerText(), /Journey Through Time/, 'case-only edits render the cached search');
    await page.fill('#search', 'Slow again');
    await page.press('#search', 'Enter');
    await page.locator('#sidebar [data-view="tracks"]').click();
    await sleep(600);
    assert.equal(await page.locator('#sprog').isVisible(), false, 'leaving search clears its progress');
    console.log('PASS healthy empty results, final line without newline, superseded search, case-only edits');
  } finally { await context.close(); }
}

async function lateReplies(browser, whileFailed = false) {
  const { page, context, state } = await fx.create(browser, 1280);
  let attempts = 0;
  let tags = 0;
  let releaseTags;
  const oldTags = new Promise((resolve) => { releaseTags = resolve; });
  await context.route(/\/v1\/search\/tags\?/, async (route) => {
    if (++tags === 1) await oldTags;
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ mine: [], others: [] }) });
  });
  await context.route(/\/v1\/search\?/, (route) => route.fulfill(response(++attempts === 1 ? '{broken\n' : answer() + '\n')));
  try {
    await fx.go(page, '/search?q=Journey');
    const retry = page.locator('#sresults button[data-sretry]');
    await retry.focus();
    if (whileFailed) {
      // The failed run's own tag reply can add tags without discarding keyboard focus.
      releaseTags();
      await sleep(150);
      assert.equal(await retry.evaluate((e) => e === document.activeElement), true, 'late tags preserve Retry focus');
      await sleep(5100);
      assert.equal(await retry.evaluate((e) => e === document.activeElement), true, 'settled slow-source timer preserves Retry focus');
    }
    await page.keyboard.press('Enter');
    await page.locator('#sresults .srow').first().waitFor();
    if (!whileFailed) {
      releaseTags();
      await sleep(150);
      assert.match(await page.locator('#sresults').innerText(), /Journey Through Time/, 'old tag reply cannot replace recovered results');
      await sleep(5100);
    }
    assert.match(await page.locator('#sresults').innerText(), /Journey Through Time/);
    assert.doesNotMatch(await page.locator('#sresults').innerText(), /Search couldn.t finish/);
    assert.equal(await page.locator('#sprog').isVisible(), false);
    assert.equal(attempts, 2);
    assert.deepEqual(state.errors, []);
    console.log(whileFailed ? 'PASS late tag reply and slow-source timer preserve Retry focus' : 'PASS old tag reply and slow-source timer cannot undo a successful retry');
  } finally { releaseTags(); await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, theme] of [[1280, 'dark'], [1024, 'light'], [768, 'dark'], [360, 'light'], [412, 'dark']]) {
      await recovery(browser, width, 'http', theme);
    }
    for (const mode of ['network', 'malformed', 'partial', 'incomplete']) await recovery(browser, 1280, mode);
    await healthyAndCancelled(browser);
    await lateReplies(browser);
    await lateReplies(browser, true);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
