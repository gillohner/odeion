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
const zone = { id: 'z1', name: 'Living room', output: 'upnp:fixture', transport: 'paused', queue: [{ id: 1, uri: 'track:1', title: hit.title }], queue_index: 0, queue_version: 1, repeat: 'off', radio: false, sync: 'single', volume: { percent: 20, muted: false }, now_playing: { item: { id: 1, uri: 'track:1', title: hit.title, artist: 'Sefa', duration_ms: 240000 }, index: 0, position_ms: 1000, duration_ms: 240000 } };
async function withPlayer(browser, width) {
  const run = await fx.create(browser, width, { zone: zone.id, fixtures: { '/zones': { body: [zone] }, '/zones/z1/transport': { body: zone } } });
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => {
    ws.onMessage((m) => { const j = JSON.parse(m); if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); });
    ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] }));
  });
  return run;
}
const transports = (state) => state.hits.filter((h) => h.v === '/zones/z1/transport' && h.m === 'POST').map((h) => JSON.parse(h.body).action);

async function recovery(browser, width, mode, theme = 'light', key = 'Enter') {
  const run = await withPlayer(browser, width);
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
    await page.keyboard.press(key);
    assert.deepEqual(transports(state), [], `${key} on Retry never controls playback`);
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
    console.log(`PASS ${width} ${theme} ${mode} ${key}: error, retained results, keyboard retry, progress, recovery`);
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

async function delayedSources(browser, action) {
  const { page, context, state } = await fx.create(browser, 1280);
  let releaseSources;
  const sources = new Promise((resolve) => { releaseSources = resolve; });
  let calls = 0;
  await context.route(/\/v1\/sources$/, async (route) => {
    await sources;
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify([{ id: 'tidal', name: 'TIDAL', status: 'ready', enabled: true, features: ['search'] }]) });
  });
  await context.route(/\/v1\/search\?/, (route) => route.fulfill(response(++calls === 1 ? '{broken\n' : answer() + '\n')));
  try {
    await fx.go(page, '/search?q=Journey');
    await page.locator('button[data-sretry]').waitFor();
    assert.equal(await page.locator('[data-scope-in="tidal"]').count(), 0);
    if (action === 'retry') await page.locator('button[data-sretry]').click();
    else if (action === 'query') { await page.fill('#search', 'New query'); await page.press('#search', 'Enter'); }
    else { await page.locator('#sidebar [data-view="tracks"]').click(); await page.locator('#tracklist').waitFor(); }
    if (action !== 'leave') await page.locator('#sresults .srow').first().waitFor();
    releaseSources();
    if (action === 'leave') {
      await sleep(200);
      assert.equal(new URL(page.url()).pathname, '/tracks');
      assert.equal(await page.locator('#scopechips').count(), 0, 'source discovery never reopens Search');
    } else {
      await page.locator('[data-scope-in="tidal"]').waitFor();
      assert.match(await page.locator('#sresults').innerText(), /Journey Through Time/);
      assert.equal(await page.locator('button[data-sretry]').count(), 0, 'source discovery does not restore the failed run');
      assert.equal(await page.locator('#search').inputValue(), action === 'query' ? 'New query' : 'Journey');
      await page.locator('[data-scope-in="tidal"]').click();
      await until(page, () => window.pmLastSearch?.scope === 'tidal', 'discovered source is usable');
    }
    assert.deepEqual(state.errors, []);
    console.log(`PASS delayed nonempty sources after ${action}`);
  } finally { releaseSources(); await context.close(); }
}

async function keyboardControls(browser) {
  const { page, context, state } = await withPlayer(browser, 1280);
  try {
    await fx.go(page, '/library/import');
    await page.locator('[data-libtab="import"].on').waitFor();
    await page.locator('[data-libtab="import"]').focus();
    await page.keyboard.press('ArrowLeft');
    await page.locator('[data-libtab="node"].on').waitFor();
    assert.deepEqual(transports(state), [], 'handled tab arrows do not skip songs');
    await page.locator('#libadd').focus();
    await page.keyboard.press('Space');
    await page.locator('dialog[open]').waitFor();
    assert.deepEqual(transports(state), [], 'Space opens Add music without playing');
    await page.keyboard.press('Escape');
    await page.locator('dialog[open]').waitFor({ state: 'hidden' });
    // The global shortcuts still work when the page itself has focus.
    await page.evaluate(() => document.activeElement.blur());
    await page.keyboard.press('Space');
    await sleep(150);
    await page.keyboard.press('ArrowRight');
    await sleep(150);
    await page.keyboard.press('ArrowLeft');
    await sleep(150);
    assert.deepEqual(transports(state), ['play', 'next', 'previous']);
    // A component can consume a shortcut even without a native interactive tag.
    await page.evaluate(() => document.body.addEventListener('keydown', (e) => e.preventDefault(), { once: true }));
    await page.keyboard.press('Space');
    await sleep(150);
    assert.deepEqual(transports(state), ['play', 'next', 'previous'], 'defaultPrevented is respected');
    assert.deepEqual(state.errors, []);
    console.log('PASS native button Space, tab arrows, consumed events and page playback shortcuts');
  } finally { await context.close(); }
}

// A click leaves focus on its button (Chromium), but the playback keys stay the page's: Space plays
// or pauses (it doesn't press the clicked button again) and the arrows skip.
async function clickedControlKeys(browser, width) {
  const { page, context, state } = await withPlayer(browser, width);
  try {
    await fx.go(page, '/library/import');
    await page.locator('#sidebar [data-view="tracks"]').click();
    await page.locator('#tracklist').waitFor();
    assert.equal(await page.locator('#sidebar [data-view="tracks"]').evaluate((e) => e === document.activeElement), true, 'the clicked button keeps focus');
    await page.keyboard.press('Space');
    await sleep(150);
    await page.keyboard.press('ArrowRight');
    await sleep(150);
    await page.keyboard.press('ArrowLeft');
    await sleep(150);
    assert.deepEqual(transports(state), ['play', 'next', 'previous'], 'playback keys after a click');
    assert.equal(await page.locator('#tracklist').count(), 1, 'Space did not press the clicked button again');
    assert.deepEqual(state.errors, []);
    console.log(`PASS ${width} Space and arrows control playback after a click on a button`);
  } finally { await context.close(); }
}

async function navigationShortcut(browser, width, key) {
  const { page, context, state } = await withPlayer(browser, width);
  try {
    await fx.go(page, '/library/import');
    await page.locator('#sidebar [data-view="tracks"]').click();
    await page.locator('#tracklist').waitFor();
    assert.equal(await page.locator('#sidebar [data-view="tracks"]').evaluate((e) => e === document.activeElement), true, 'navigation leaves its button focused');
    await page.keyboard.press(key);
    await page.screenshot({ path: path.join(shots, `navigation-${key === '/' ? 'search' : 'help'}-${width}.png`) });
    if (key === '/') assert.equal(await page.locator('#search').evaluate((e) => e === document.activeElement), true, '/ still opens Search from navigation');
    else {
      assert.equal(await page.locator('dialog[open]').count(), 1, '? still opens Help from navigation');
      assert.match(await page.locator('dialog[open]').innerText(), /Keyboard shortcuts/);
    }
    assert.deepEqual(transports(state), [], 'navigation shortcuts do not control playback');
    assert.deepEqual(state.errors, []);
    console.log(`PASS ${width} ${key}: shortcut works after clicking navigation`);
  } finally { await context.close(); }
}

async function heldRetry(browser, width, update, theme = 'light', leaveFocus = false) {
  const { page, context, state } = await withPlayer(browser, width);
  await context.addInitScript((t) => localStorage.setItem('pm.theme', t), theme);
  let releaseUpdate;
  const updateGate = new Promise((resolve) => { releaseUpdate = resolve; });
  await context.route(update === 'tags' ? /\/v1\/search\/tags\?/ : /\/v1\/sources$/, async (route) => {
    await updateGate;
    const body = update === 'tags' ? { mine: [], others: ['late arrival'] }
      : [{ id: 'tidal', name: 'TIDAL', status: 'ready', enabled: true, features: ['search'] }];
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify(body) });
  });
  const requests = [];
  let releaseRetry;
  const retryGate = new Promise((resolve) => { releaseRetry = resolve; });
  await context.route(/\/v1\/search\?/, async (route) => {
    const u = new URL(route.request().url());
    requests.push({ q: u.searchParams.get('q'), scope: u.searchParams.get('in') });
    if (requests.length === 1) return route.fulfill(response(answer() + '\n{broken\n'));
    await retryGate;
    return route.fulfill(response(answer() + '\n'));
  });
  try {
    await fx.go(page, '/search?q=Journey');
    const retry = page.locator('#sresults button[data-sretry]');
    await retry.focus();
    const original = await retry.elementHandle();
    await page.keyboard.down('Space');
    if (leaveFocus) await page.locator('#search').focus();
    releaseUpdate();
    await page.locator(update === 'tags' ? '#stags [data-stag="late arrival"]' : '[data-scope-in="tidal"]').waitFor();
    assert.match(await page.locator('#sresults').innerText(), /Journey Through Time/, 'late update keeps partial results');
    const sameButton = await original.evaluate((e) => e === document.querySelector('#sresults button[data-sretry]') && e.isConnected);
    await page.screenshot({ path: path.join(shots, `held-${update}-${width}-${theme}${leaveFocus ? '-search' : ''}.png`) });
    await page.keyboard.up('Space');
    await sleep(150);
    assert.equal(requests.length, leaveFocus ? 1 : 2, leaveFocus ? 'Space release after moving focus does not activate Retry' : `${update} arriving during Space must not swallow Retry activation`);
    assert.equal(sameButton, true, 'asynchronous updates keep the actual Retry node connected');
    assert.deepEqual(transports(state), [], 'held Space never controls playback');
    if (leaveFocus) {
      assert.equal(await page.locator('#search').evaluate((e) => e === document.activeElement), true, 'late update does not reclaim focus');
    } else {
      assert.deepEqual(requests, [{ q: 'Journey', scope: 'everywhere' }, { q: 'Journey', scope: 'everywhere' }], 'one retry of the same query and source');
      assert.equal(await page.locator('#sprog').isVisible(), true, 'Retry shows immediate progress');
      assert.equal(await retry.count(), 0, 'Retry is removed while the new request runs');
      releaseRetry();
      await until(page, () => document.querySelector('#sprog').hidden && !!document.querySelector('#sresults .srow'), 'held-Space retry succeeds');
      assert.doesNotMatch(await page.locator('#sresults').innerText(), /Search couldn.t finish/);
    }
    assert.deepEqual(state.errors, []);
    await fx.cleanCopy(page);
    console.log(`PASS ${width} ${theme} held Space + ${update}${leaveFocus ? ' + moved focus' : ''}: native activation, retained Retry node, no playback`);
  } finally { releaseUpdate(); releaseRetry(); await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    // A focused mode also lets the unchanged UI act as each regression's negative control.
    if (process.env.SEARCH_RECOVERY_CASE === 'sources') { await delayedSources(browser, 'retry'); return; }
    if (process.env.SEARCH_RECOVERY_CASE === 'keyboard') { await recovery(browser, 1280, 'http', 'light', 'Space'); return; }
    if (process.env.SEARCH_RECOVERY_CASE === 'controls') { await keyboardControls(browser); return; }
    if (process.env.SEARCH_RECOVERY_CASE === 'clicked') { await clickedControlKeys(browser, 1280); return; }
    if (process.env.SEARCH_RECOVERY_CASE === 'shortcut-search') { await navigationShortcut(browser, 1280, '/'); return; }
    if (process.env.SEARCH_RECOVERY_CASE === 'shortcut-help') { await navigationShortcut(browser, 1280, '?'); return; }
    if (process.env.SEARCH_RECOVERY_CASE?.startsWith('held-')) { await heldRetry(browser, 1280, process.env.SEARCH_RECOVERY_CASE.slice(5)); return; }
    for (const [width, theme] of [[1280, 'dark'], [1024, 'light'], [768, 'dark'], [360, 'light'], [412, 'dark']]) {
      await recovery(browser, width, 'http', theme);
      await recovery(browser, width, 'http', theme, 'Space');
    }
    for (const mode of ['network', 'malformed', 'partial', 'incomplete']) await recovery(browser, 1280, mode);
    await healthyAndCancelled(browser);
    await lateReplies(browser);
    await lateReplies(browser, true);
    for (const action of ['retry', 'query', 'leave']) await delayedSources(browser, action);
    await keyboardControls(browser);
    for (const width of [1280, 768]) await clickedControlKeys(browser, width);
    for (const width of [1280, 1024, 768]) for (const key of ['/', '?']) await navigationShortcut(browser, width, key);
    for (const [width, theme] of [[1280, 'light'], [1024, 'dark'], [768, 'light'], [360, 'dark'], [412, 'light']]) {
      for (const update of ['tags', 'sources']) await heldRetry(browser, width, update, theme);
    }
    for (const update of ['tags', 'sources']) await heldRetry(browser, 1280, update, 'light', true);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
