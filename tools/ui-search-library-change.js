// A search kept for 5 minutes must not keep a playlist's old count once the library changed:
// after a playlist is made or edited in the app, or the node says its library changed, the same
// search typed again asks the node again (the new count); the search on screen is never emptied,
// asked again or cut off by it, even while its answer is still on its way.
// Real UI in Chromium, isolated responses (no node or external services).
//   node tools/ui-search-library-change.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/search-library-change-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(page, predicate, label, arg) {
  const end = Date.now() + 7000;
  while (Date.now() < end) {
    if (await page.evaluate(predicate, arg)) return;
    await sleep(50);
  }
  throw new Error('Timed out: ' + label);
}

async function setup(browser, width) {
  const run = await fx.create(browser, width, { mobile: width < 600 });
  const { context } = run;
  // The playlist's count as the node would say it now (bumped by a write or by the test)
  const lib = { count: 2, asked: [], hold: null };
  await context.route(/\/v1\/search\?/, async (route) => {
    const url = new URL(route.request().url());
    lib.asked.push({ q: url.searchParams.get('q'), scope: url.searchParams.get('in') });
    const n = lib.count;
    if (lib.hold) await lib.hold;
    const hit = { kind: 'playlist', id: 'c1', title: 'Road songs', subtitle: `${n} songs · your playlist`, uri: 'playlist:c1', keys: ['playlist:c1'] };
    const song = { kind: 'song', id: 'lib-1', title: fx.track.title, subtitle: 'Sefa, Dr. Peacock', track_id: 1, uri: 'track:1', keys: ['track:1'] };
    return route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': 'library' }, body: JSON.stringify({ source: 'library', status: 'ok', hits: [hit, song], counts: [] }) + '\n' });
  });
  let socket = null;
  await context.routeWebSocket(/\/v1\/events/, (ws) => {
    socket = ws;
    ws.onMessage((m) => { const j = JSON.parse(m); if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); });
    ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] }));
  });
  // A playlist made on the Playlists page: the node's count for the search goes up
  await context.route(/\/v1\/playlists$/, async (route) => {
    if (route.request().method() !== 'POST') return route.fallback();
    lib.count += 1;
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...fx.detail, summary: { ...fx.summary, id: 'c2', name: 'New one' } }) });
  });
  const changed = () => socket.send(JSON.stringify({ type: 'library_changed' }));
  return { ...run, lib, changed };
}
const shown = (page) => page.evaluate(() => (document.querySelector('#sresults') || {}).innerText || '');
const nav = (page, view) => page.evaluate((v) => document.querySelector(`.nav-item[data-view="${v}"]`).click(), view);
// The same words typed again in the search box (no reload: the app's kept searches stay)
async function typeAgain(page, q) {
  await page.evaluate(() => { const b = document.querySelector('[data-bnav="search"]'); if (b && b.getClientRects().length) b.click(); });
  await page.locator('#search').fill('');
  await page.locator('#search').fill(q);
  await until(page, (w) => document.querySelector('#sresults') && history.state && history.state.v === 'search' && (history.state.arg || {}).q === w, 'search page for the words', q);
}

async function ownWrite(browser, width) {
  const { page, context, state, lib } = await setup(browser, width);
  try {
    await fx.go(page, '/search?q=road');
    await until(page, () => /2 songs · your playlist/.test((document.querySelector('#sresults') || {}).innerText || ''), 'first answer drawn');
    assert.equal(lib.asked.length, 1);
    await page.screenshot({ path: path.join(shots, `own-${width}-before.png`) });
    // Kept: the same search again (no write yet) does not ask the node again
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await until(page, () => /2 songs/.test((document.querySelector('#sresults') || {}).innerText || ''), 'kept answer drawn');
    assert.equal(lib.asked.length, 1, 'an unchanged library keeps the search for 5 minutes');
    // A playlist made in the app
    await nav(page, 'playlists');
    await page.locator('#plname').fill('New one');
    await page.locator('#plcreate').click();
    await until(page, () => /Created/.test(document.body.innerText), 'playlist made');
    await typeAgain(page, 'road');
    await until(page, () => /3 songs · your playlist/.test((document.querySelector('#sresults') || {}).innerText || ''), 'new count after the app\'s own playlist write');
    assert.equal(lib.asked.length, 2, 'the search was asked again once after the write');
    assert.doesNotMatch(await shown(page), /2 songs/);
    await page.screenshot({ path: path.join(shots, `own-${width}-after.png`) });
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

async function nodeEvent(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    await fx.go(page, '/search?q=road');
    await until(page, () => /2 songs · your playlist/.test((document.querySelector('#sresults') || {}).innerText || ''), 'first answer drawn');
    // The library changes while its search is on screen: it stays as drawn, nothing asked again
    lib.count = 1;
    changed();
    await sleep(600);
    assert.equal(lib.asked.length, 1, 'the search on screen is not asked again by a library change');
    assert.match(await shown(page), /2 songs · your playlist/, 'the search on screen is not emptied');
    // ... and typed again (from another page) it asks the node: the new count
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await until(page, () => /1 song · your playlist|1 songs · your playlist/.test((document.querySelector('#sresults') || {}).innerText || ''), 'new count after the node\'s library_changed');
    assert.equal(lib.asked.length, 2);
    // A search still on its way when the library changes is not cut off: its answer is drawn
    let release; lib.hold = new Promise((r) => { release = r; });
    lib.count = 4;
    await nav(page, 'home');
    await typeAgain(page, 'roads');
    for (const end = Date.now() + 7000; !lib.asked.some((x) => x.q === 'roads'); await sleep(50)) if (Date.now() > end) throw new Error('Timed out: roads asked');
    changed();
    await sleep(300);
    release(); lib.hold = null;
    await until(page, () => /4 songs · your playlist/.test((document.querySelector('#sresults') || {}).innerText || '') && window.pmLastSearch && window.pmLastSearch.q === 'roads', 'in-flight answer drawn after the change');
    assert.equal(lib.asked.filter((a) => a.q === 'roads').length, 1, 'the search on its way was neither cut off nor asked again');
    await page.screenshot({ path: path.join(shots, `event-${width}-after.png`) });
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// Back to a search after the library changed: its old results are drawn at once (no empty page,
// the scroll kept) while the node is asked again, then the new count takes their place
async function back(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    await fx.go(page, '/search?q=road');
    await until(page, () => /2 songs · your playlist/.test((document.querySelector('#sresults') || {}).innerText || ''), 'first answer drawn');
    await page.locator('#sresults', { hasText: 'Road songs' }).getByText('Road songs').first().click();
    await until(page, () => history.state && history.state.v !== 'search', 'left the search');
    lib.count = 5; changed();
    let release; lib.hold = new Promise((r) => { release = r; });
    await page.goBack();
    await until(page, () => history.state && history.state.v === 'search' && !!document.querySelector('#sresults'), 'back on the search');
    for (const end = Date.now() + 7000; lib.asked.length < 2; await sleep(50)) if (Date.now() > end) throw new Error('Timed out: asked again after Back');
    assert.match(await shown(page), /2 songs · your playlist/, 'while the node is asked again, the old results stay drawn');
    assert.equal(lib.asked[1].scope, 'library', 'only the library is asked again (the services keep their answers)');
    assert.equal(await page.evaluate(() => document.querySelector('#sprog').hidden), false, 'the progress line shows while it is asked again');
    await page.screenshot({ path: path.join(shots, `back-${width}-asking.png`) });
    release(); lib.hold = null;
    await until(page, () => /5 songs · your playlist/.test((document.querySelector('#sresults') || {}).innerText || ''), 'new count after Back');
    assert.equal(lib.asked.length, 2);
    await until(page, () => document.querySelector('#sprog').hidden, 'progress line gone');
    await page.screenshot({ path: path.join(shots, `back-${width}-after.png`) });
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// The search page drawn again after a library change (a song removed from the library from its
// row menu there: onLibraryChanged, then render) keeps the results it shows: not emptied, not asked again
async function redrawn(browser, width) {
  const { page, context, state, lib } = await setup(browser, width);
  try {
    await fx.go(page, '/search?q=road');
    await until(page, () => /2 songs · your playlist/.test((document.querySelector('#sresults') || {}).innerText || ''), 'first answer drawn');
    lib.count = 1;
    const row = page.locator('#sresults .srow', { hasText: fx.track.title }).first();
    await row.locator('[data-smore]').click({ force: true });
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from library' }).click();
    await page.locator('dialog[open] button', { hasText: /^Remove$/ }).click();
    await until(page, () => /Removed from the library/.test(document.body.innerText), 'removed');
    await sleep(400);
    assert.ok(state.hits.some((h) => h.v === '/library/tracks/1' && h.m === 'DELETE'), 'the song was removed');
    assert.equal(lib.asked.length, 1, 'the search page drawn again is not asked again');
    assert.match(await shown(page), /2 songs · your playlist/, 'the search page drawn again keeps its results');
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412, 1280]) {
      await ownWrite(browser, width);
      await nodeEvent(browser, width);
      await redrawn(browser, width);
      await back(browser, width);
      console.log(`ok ${width}px: a playlist write or library change makes the same search ask again; the one on screen stays`);
    }
  } finally { await browser.close(); }
})().catch((e) => { console.error(e); process.exit(1); });
