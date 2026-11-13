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
async function withPlayer(browser, width, opts = {}) {
  const run = await fx.create(browser, width, { ...opts, zone: zone.id, fixtures: { '/zones': { body: [zone] }, '/zones/z1/transport': { body: zone } } });
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
    const retry = page.locator('#sfailure button[data-sretry]');
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
    const retry = page.locator('#sfailure button[data-sretry]');
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
    await page.locator('#sfailure button[data-sretry]').waitFor();
    assert.equal(await page.locator('[data-scope-in="tidal"]').count(), 0);
    if (action === 'retry') await page.locator('#sfailure button[data-sretry]').click();
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
      assert.equal(await page.locator('#sfailure button[data-sretry]').count(), 0, 'source discovery does not restore the failed run');
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
    const retry = page.locator('#sfailure button[data-sretry]');
    await retry.focus();
    const original = await retry.elementHandle();
    await page.keyboard.down('Space');
    if (leaveFocus) await page.locator('#search').focus();
    releaseUpdate();
    await page.locator(update === 'tags' ? '#stags [data-stag="late arrival"]' : '[data-scope-in="tidal"]').waitFor();
    assert.match(await page.locator('#sresults').innerText(), /Journey Through Time/, 'late update keeps partial results');
    const sameButton = await original.evaluate((e) => e === document.querySelector('#sfailure button[data-sretry]') && e.isConnected);
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

// One source fails and the rest answer: that source's line has its own Retry. On a phone it is the
// app's small button (it was a 29x17 px link no keyboard could reach): a 44 px reach, Tab gets
// there, Enter or a tap searches again with the same words and scope, and a source that can't be
// helped by trying again (a Premium plan) gets no Retry.
async function sourceRetry(browser, width, android = false) {
  const at = `${width}${android ? ' android' : ''}`;
  const { page, context, state } = await withPlayer(browser, width, { mobile: width < 600 });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  const long = 'Spotify took too long to answer while the network was slow and busy, so some songs from it are missing here';
  const line = (source, error) => JSON.stringify({ source, status: 'error', error, hits: [], counts: [] }) + '\n';
  const requests = [];
  await context.route(/\/v1\/search\?/, (route) => {
    const u = new URL(route.request().url());
    if (u.searchParams.get('in') !== 'everywhere') return route.fulfill(response(answer() + '\n'));
    requests.push({ q: u.searchParams.get('q'), scope: u.searchParams.get('in') });
    return route.fulfill(response(answer() + '\n' + line('spotify', long) + line('tidal', 'TIDAL needs a paid plan for this'), 'library,spotify,tidal'));
  });
  try {
    await fx.go(page, '/search?q=Journey&in=everywhere');
    const retry = page.locator('.snote button[data-sretry="spotify"]');
    await retry.waitFor();
    assert.equal(await page.locator('#sfailure').count(), 0, `${at}: a single source failing is not a failed search`);
    assert.equal(await page.locator('.snote [data-sretry]').count(), 1, `${at}: only the source that may answer next time has a Retry`);
    assert.match(await page.locator('.snote').filter({ hasText: 'TIDAL' }).innerText(), /paid plan/, `${at}: the TIDAL line still says why`);
    const m = await retry.evaluate((b) => {
      const r = b.getBoundingClientRect(), reach = getComputedStyle(b, '::before');
      const grow = reach.content === 'none' ? [0, 0] : [-parseFloat(reach.top) - parseFloat(reach.bottom), -parseFloat(reach.left) - parseFloat(reach.right)];
      const note = b.closest('.snote').getBoundingClientRect();
      return { h: r.height + grow[0], w: r.width + grow[1], inside: r.left >= note.left - 0.5 && r.right <= note.right + 0.5 && r.right <= innerWidth, overflow: document.documentElement.scrollWidth - innerWidth };
    });
    if (width < 600) {
      assert.ok(m.h >= 44 && m.w >= 44, `${at}: Retry reaches 44x44 px (${Math.round(m.w)}x${Math.round(m.h)})`);
      assert.equal(m.inside, true, `${at}: Retry stays on its own line, on screen, beside a long reason`);
    }
    assert.ok(m.overflow <= 0, `${at}: no sideways scroll (${m.overflow} px)`);
    await page.locator('#search').focus();
    let tabs = 0;
    while (tabs < 30 && !(await retry.evaluate((b) => b === document.activeElement))) { await page.keyboard.press('Tab'); tabs++; }
    assert.ok(tabs < 30, `${at}: Tab reaches the source's Retry`);
    await page.screenshot({ path: path.join(shots, `source-retry-${width}${android ? '-android' : ''}.png`) });
    await page.keyboard.press('Enter');
    await until(page, () => document.activeElement?.id === 'search', 'focus returns to search');
    await page.locator('.snote button[data-sretry="spotify"]').waitFor();
    assert.deepEqual(requests, [{ q: 'Journey', scope: 'everywhere' }, { q: 'Journey', scope: 'everywhere' }], `${at}: Enter searches again, same words and scope`);
    if (width < 600) {
      await page.locator('.snote button[data-sretry="spotify"]').tap();
      await until(page, () => document.activeElement?.id === 'search', 'focus returns to search after a tap');
      await page.locator('.snote button[data-sretry="spotify"]').waitFor();
      assert.equal(requests.length, 3, `${at}: a tap searches again`);
    }
    assert.deepEqual(transports(state), [], `${at}: Retry never controls playback`);
    assert.deepEqual(state.errors, []);
    console.log(`PASS ${at} a failed source's Retry: ${Math.round(m.w)}x${Math.round(m.h)} px reach, Tab ${tabs}, Enter${width < 600 ? ' and tap' : ''} search again`);
  } finally { await context.close(); }
}

// Space presses a button on its release. A late reply (here the search's tags) that redraws the
// results while Space is held must keep that source's Retry in place, or the press is lost.
async function heldSourceRetry(browser, width) {
  const { page, context, state } = await withPlayer(browser, width, { mobile: width < 600 });
  let releaseTags;
  const tagsGate = new Promise((resolve) => { releaseTags = resolve; });
  await context.route(/\/v1\/search\/tags\?/, async (route) => { await tagsGate; return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ mine: [], others: ['late arrival'] }) }); });
  const requests = [];
  await context.route(/\/v1\/search\?/, (route) => {
    const u = new URL(route.request().url());
    if (u.searchParams.get('in') !== 'everywhere') return route.fulfill(response(answer() + '\n'));
    requests.push(u.searchParams.get('q'));
    return route.fulfill(response(answer() + '\n' + JSON.stringify({ source: 'spotify', status: 'error', error: 'spotify: operation timed out', hits: [], counts: [] }) + '\n', 'library,spotify'));
  });
  try {
    await fx.go(page, '/search?q=Journey&in=everywhere');
    const retry = page.locator('.snote button[data-sretry="spotify"]');
    await retry.focus();
    const original = await retry.elementHandle();
    await page.keyboard.down('Space');
    releaseTags();
    await page.locator('#stags [data-stag="late arrival"]').waitFor();
    const same = await original.evaluate((e) => e.isConnected && e === document.activeElement);
    assert.equal(same, true, `${width}: the late tags keep the focused Retry in place`);
    await page.keyboard.up('Space');
    await until(page, () => document.activeElement?.id === 'search', 'Space on Retry searches again');
    assert.equal(requests.length, 2, `${width}: Space held across a redraw still searches again`);
    assert.deepEqual(transports(state), [], `${width}: held Space never controls playback`);
    assert.deepEqual(state.errors, []);
    console.log(`PASS ${width} held Space on a source's Retry across late tags: one more search, no playback`);
  } finally { releaseTags(); await context.close(); }
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
    if (process.env.SEARCH_RECOVERY_CASE === 'source') { for (const w of [360, 412]) for (const a of [false, true]) await sourceRetry(browser, w, a); await sourceRetry(browser, 1280); for (const w of [1280, 360]) await heldSourceRetry(browser, w); return; }
    if (process.env.SEARCH_RECOVERY_CASE?.startsWith('held-')) { await heldRetry(browser, 1280, process.env.SEARCH_RECOVERY_CASE.slice(5)); return; }
    for (const [width, theme] of [[1280, 'dark'], [1024, 'light'], [768, 'dark'], [360, 'light'], [412, 'dark']]) {
      await recovery(browser, width, 'http', theme);
      await recovery(browser, width, 'http', theme, 'Space');
    }
    for (const mode of ['network', 'malformed', 'partial', 'incomplete']) await recovery(browser, 1280, mode);
    for (const width of [360, 412]) for (const android of [false, true]) await sourceRetry(browser, width, android);
    await sourceRetry(browser, 1280);
    for (const width of [1280, 360]) await heldSourceRetry(browser, width);
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
