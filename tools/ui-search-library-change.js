// A search kept for 5 minutes must not keep a playlist's old count once the library changed:
// after a playlist is made or edited in the app, or the node says its library changed, the same
// search typed again asks the node again (the new count); the search on screen is never emptied,
// asked again or cut off by it, even while its answer is still on its way. The library changing
// again while it is asked again asks once more; a hung ask is given up (old results kept, still
// stale); an unchanged answer leaves the results as drawn; a search that ended in an error is asked
// for its library answer only (its error goes when that answer was the one missing or every source
// had answered), unless it failed before the node named any source (a full search); a song removed
// during that ask is not listed again by an answer read before the removal, unless Bring back (Your
// library › Node library) put it back before that answer landed.
// Real UI in Chromium, isolated responses (no node or external services).
//   node tools/ui-search-library-change.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/search-library-change-shots');
fs.mkdirSync(shots, { recursive: true });
const SONG2 = 'Journey Home';
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(page, predicate, label, arg) {
  const end = Date.now() + 7000;
  while (Date.now() < end) {
    if (await page.evaluate(predicate, arg)) return;
    await sleep(50);
  }
  throw new Error('Timed out: ' + label);
}

async function setup(browser, width, opts = {}) {
  const run = await fx.create(browser, width, { mobile: width < 600 });
  const { context, page } = run;
  if (opts.clock) await page.clock.install(); // (the hung case moves the clock on)
  // The playlist's count as the node would say it now (bumped by a write or by the test);
  // sources: the sources the node names (one that never answers ends the search in an error)
  // (songGone: the song is out of the library, read when the ask comes in, before any hold)
  // (song2: a second song of yours listed too, until song2Gone)
  const lib = { count: 2, asked: [], hold: null, sources: 'library', noLibrary: 0, breakAfter: 0, offline: 0, songGone: false, song2: false, song2Gone: false };
  await context.route(/\/v1\/search\?/, async (route) => {
    const url = new URL(route.request().url());
    lib.asked.push({ q: url.searchParams.get('q'), scope: url.searchParams.get('in') });
    const n = lib.count; const withSong = !lib.songGone; const withSong2 = lib.song2 && !lib.song2Gone;
    if (lib.hold) await lib.hold;
    // (offline: that many asks never reach the node, before it names any source)
    if (lib.offline > 0) { lib.offline -= 1; return route.abort('internetdisconnected').catch(() => {}); }
    // (noLibrary: that many asks end before the library answers)
    if (lib.noLibrary > 0) { lib.noLibrary -= 1; return route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': lib.sources }, body: '' }).catch(() => {}); }
    const hit = { kind: 'playlist', id: 'c1', title: 'Road songs', subtitle: `${n} songs · your collection`, uri: 'playlist:c1', keys: ['playlist:c1'] };
    const song = { kind: 'song', id: 'lib-1', title: fx.track.title, subtitle: 'Sefa, Dr. Peacock', track_id: 1, uri: 'track:1', keys: ['track:1'] };
    const song2 = { kind: 'song', id: 'lib-2', title: SONG2, subtitle: 'Sefa, Dr. Peacock', track_id: 2, uri: 'track:2', keys: ['track:2'] };
    // (breakAfter: that many asks' streams break after every source answered: a cut-off line)
    let broken = ''; if (lib.breakAfter > 0) { lib.breakAfter -= 1; broken = '{"source":"lib'; }
    // (.catch: an ask the app gave up on is gone by then)
    return route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': lib.sources }, body: JSON.stringify({ source: 'library', status: 'ok', hits: [hit, ...(withSong ? [song] : []), ...(withSong2 ? [song2] : [])], counts: [] }) + '\n' + broken }).catch(() => {});
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
    await until(page, () => /2 songs · your collection/.test((document.querySelector('#sresults') || {}).innerText || ''), 'first answer drawn');
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
    await until(page, () => /Created/.test(document.body.innerText), 'collection made');
    await typeAgain(page, 'road');
    await until(page, () => /3 songs · your collection/.test((document.querySelector('#sresults') || {}).innerText || ''), 'new count after the app\'s own collection write');
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
    await until(page, () => /2 songs · your collection/.test((document.querySelector('#sresults') || {}).innerText || ''), 'first answer drawn');
    // The library changes while its search is on screen: it stays as drawn, nothing asked again
    lib.count = 1;
    changed();
    await sleep(600);
    assert.equal(lib.asked.length, 1, 'the search on screen is not asked again by a library change');
    assert.match(await shown(page), /2 songs · your collection/, 'the search on screen is not emptied');
    // ... and typed again (from another page) it asks the node: the new count
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await until(page, () => /1 song · your collection|1 songs · your collection/.test((document.querySelector('#sresults') || {}).innerText || ''), 'new count after the node\'s library_changed');
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
    await until(page, () => /4 songs · your collection/.test((document.querySelector('#sresults') || {}).innerText || '') && window.pmLastSearch && window.pmLastSearch.q === 'roads', 'in-flight answer drawn after the change');
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
    await until(page, () => /2 songs · your collection/.test((document.querySelector('#sresults') || {}).innerText || ''), 'first answer drawn');
    await page.locator('#sresults', { hasText: 'Road songs' }).getByText('Road songs').first().click();
    await until(page, () => history.state && history.state.v !== 'search', 'left the search');
    lib.count = 5; changed();
    let release; lib.hold = new Promise((r) => { release = r; });
    await page.goBack();
    await until(page, () => history.state && history.state.v === 'search' && !!document.querySelector('#sresults'), 'back on the search');
    for (const end = Date.now() + 7000; lib.asked.length < 2; await sleep(50)) if (Date.now() > end) throw new Error('Timed out: asked again after Back');
    assert.match(await shown(page), /2 songs · your collection/, 'while the node is asked again, the old results stay drawn');
    assert.equal(lib.asked[1].scope, 'library', 'only the library is asked again (the services keep their answers)');
    assert.equal(await page.evaluate(() => document.querySelector('#sprog').hidden), false, 'the progress line shows while it is asked again');
    await page.screenshot({ path: path.join(shots, `back-${width}-asking.png`) });
    release(); lib.hold = null;
    await until(page, () => /5 songs · your collection/.test((document.querySelector('#sresults') || {}).innerText || ''), 'new count after Back');
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
    await until(page, () => /2 songs · your collection/.test((document.querySelector('#sresults') || {}).innerText || ''), 'first answer drawn');
    lib.count = 1;
    const row = page.locator('#sresults .srow', { hasText: fx.track.title }).first();
    await row.locator('[data-smore]').click({ force: true });
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from library' }).click();
    await page.locator('dialog[open] button', { hasText: /^Remove$/ }).click();
    await until(page, () => /Removed from the library/.test(document.body.innerText), 'removed');
    await sleep(400);
    assert.ok(state.hits.some((h) => h.v === '/library/tracks/1' && h.m === 'DELETE'), 'the song was removed');
    assert.equal(lib.asked.length, 1, 'the search page drawn again is not asked again');
    assert.match(await shown(page), /2 songs · your collection/, 'the search page drawn again keeps its results');
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

const countShown = (page, n) => until(page, (k) => new RegExp(`${k} songs · your collection`).test((document.querySelector('#sresults') || {}).innerText || ''), `${n} songs drawn`, n);
// Open the search, leave it for a result, change the library, and come Back while the node's
// library answer is held: { release } lets that answer through
async function backWhileHeld(page, lib, changed, count) {
  await fx.go(page, '/search?q=road');
  await countShown(page, 2);
  await page.locator('#sresults', { hasText: 'Road songs' }).getByText('Road songs').first().click();
  await until(page, () => history.state && history.state.v !== 'search', 'left the search');
  lib.count = count; changed();
  let release; lib.hold = new Promise((r) => { release = r; });
  await page.goBack();
  await until(page, () => history.state && history.state.v === 'search' && !!document.querySelector('#sresults'), 'back on the search');
  for (const end = Date.now() + 7000; lib.asked.length < 2; await sleep(50)) if (Date.now() > end) throw new Error('Timed out: asked again after Back');
  return { release: () => { release(); lib.hold = null; } };
}

// The library changes again while its search is asked again, and the search is opened again before
// that answer is in: the answer that lands may be older than the change, so it is asked once more
// (on screen: right away) and the newest count is drawn, not the one in between. Only once: a change
// during that ask too (a long scan) leaves it stale until it is opened again
async function changedWhileAsked(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    const held = await backWhileHeld(page, lib, changed, 5); // (that ask says 5)
    lib.count = 6; changed();
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await sleep(300);
    assert.equal(lib.asked.length, 2, 'not asked twice at once');
    held.release();
    let release2; lib.hold = new Promise((r) => { release2 = r; });
    for (const end = Date.now() + 7000; lib.asked.length < 3; await sleep(50)) if (Date.now() > end) throw new Error('Timed out: asked once more');
    assert.equal(lib.asked[2].scope, 'library');
    lib.count = 9; changed();
    release2(); lib.hold = null;
    await countShown(page, 6);
    await until(page, () => document.querySelector('#sprog').hidden, 'progress line gone');
    await sleep(400);
    assert.equal(lib.asked.length, 3, 'asked once more after the change during the ask, not again and again');
    // ... still stale: opened again it asks (the newest count), and no more after that
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await countShown(page, 9);
    assert.equal(lib.asked.length, 4);
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await countShown(page, 9);
    await sleep(300);
    assert.equal(lib.asked.length, 4, 'the newest answer is kept');
    await page.screenshot({ path: path.join(shots, `again-${width}-after.png`) });
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// The node never answers the ask again: after 15 s it is given up, the old results stay (no
// progress line left on), and the search, still stale, asks again when opened again
async function hung(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width, { clock: true });
  try {
    const held = await backWhileHeld(page, lib, changed, 7);
    assert.equal(await page.evaluate(() => document.querySelector('#sprog').hidden), false, 'progress line while asked');
    await page.clock.fastForward(16000);
    await until(page, () => document.querySelector('#sprog').hidden, 'progress line gone after the hung ask is given up');
    assert.match(await shown(page), /2 songs · your collection/, 'the old results stay');
    await page.screenshot({ path: path.join(shots, `hung-${width}-given-up.png`) });
    held.release();
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await countShown(page, 7);
    assert.equal(lib.asked.length, 3, 'still stale: asked again when opened again');
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// The library answer asked again is the same as before: the results are not drawn again
async function unchanged(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    const held = await backWhileHeld(page, lib, changed, 2);
    await page.evaluate(() => { document.querySelector('#sresults .srow').pmMark = 1; });
    held.release();
    await until(page, () => document.querySelector('#sprog').hidden, 'progress line gone');
    assert.equal(lib.asked.length, 2);
    assert.equal(await page.evaluate(() => (document.querySelector('#sresults .srow') || {}).pmMark), 1, 'the same answer leaves the rows as drawn');
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// A search that ended in an error ("Search couldn't finish", a service never answered) and went
// stale is asked for its library answer only when opened again (no automatic full search); its
// Retry stays
async function errored(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    lib.sources = 'library,tidal';
    await fx.go(page, '/search?q=road');
    await countShown(page, 2);
    await until(page, () => !!document.querySelector('#sfailure [data-sretry]'), 'search could not finish');
    lib.count = 8; changed();
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await countShown(page, 8);
    assert.equal(lib.asked.length, 2);
    assert.equal(lib.asked[1].scope, 'library', 'only the library is asked again');
    assert.ok(await page.evaluate(() => !!document.querySelector('#sfailure [data-sretry]')), 'Retry stays');
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// A search that could not finish because the library answer was the one missing: the library
// answer asked again fills it in, and "Search couldn't finish" goes
async function erroredLibrary(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    lib.noLibrary = 1;
    await fx.go(page, '/search?q=road');
    await until(page, () => !!document.querySelector('#sfailure [data-sretry]'), 'search could not finish');
    lib.count = 3; changed();
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await countShown(page, 3);
    assert.equal(lib.asked.length, 2);
    assert.equal(lib.asked[1].scope, 'library');
    await until(page, () => !document.querySelector('#sfailure'), '"Search couldn\'t finish" gone');
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// The stream broke after every source had answered ("Search couldn't finish"), and the library
// answer asked again is the same as before: nothing to redraw in the list, but the error banner goes
async function erroredAfterAll(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    lib.breakAfter = 1;
    await fx.go(page, '/search?q=road');
    await countShown(page, 2);
    await until(page, () => !!document.querySelector('#sfailure [data-sretry]'), 'search could not finish');
    changed(); // (the count stays 2: the same answer comes back)
    await nav(page, 'home');
    await typeAgain(page, 'road');
    for (const end = Date.now() + 7000; lib.asked.length < 2; await sleep(50)) if (Date.now() > end) throw new Error('Timed out: asked again');
    assert.equal(lib.asked[1].scope, 'library', 'only the library is asked again');
    await until(page, () => document.querySelector('#sprog').hidden, 'progress line gone');
    await until(page, () => !document.querySelector('#sfailure'), '"Search couldn\'t finish" gone after the same answer');
    await countShown(page, 2);
    assert.equal(lib.asked.length, 2);
    await page.screenshot({ path: path.join(shots, `broken-${width}-after.png`) });
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// The node was offline: the search failed before the node named any source. After the library
// changes, opening it again is a full search (not the error and Retry from before)
async function offlineFirst(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    lib.offline = 1;
    await fx.go(page, '/search?q=road');
    await until(page, () => !!document.querySelector('#sfailure [data-sretry]'), 'search could not finish');
    changed();
    await nav(page, 'home');
    await typeAgain(page, 'road');
    await countShown(page, 2);
    assert.equal(lib.asked.length, 2);
    assert.equal(lib.asked[1].scope, 'everywhere', 'a full search, not the library alone');
    await until(page, () => !document.querySelector('#sfailure'), '"Search couldn\'t finish" gone');
    await page.screenshot({ path: path.join(shots, `offline-${width}-after.png`) });
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// A song removed from its row while the search's library answer is asked again (read before the
// removal): that answer, landing after it, doesn't list the song again
const songListed = (page, title = fx.track.title) => page.evaluate((t) => [...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), title);
async function removeSong(page, lib, title = fx.track.title) {
  const row = page.locator('#sresults .srow', { hasText: title }).first();
  await row.locator('[data-smore]').click({ force: true });
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from library' }).click();
  await page.locator('dialog[open] button', { hasText: /^Remove$/ }).click();
  await until(page, () => /Removed from the library/.test(document.body.innerText), 'removed');
  if (title === fx.track.title) lib.songGone = true; else lib.song2Gone = true;
}
async function removedWhileAsked(browser, width) {
  const { page, context, state, lib, changed } = await setup(browser, width);
  try {
    const held = await backWhileHeld(page, lib, changed, 4); // (that ask still has the song)
    assert.ok(await songListed(page), 'the song is listed');
    await removeSong(page, lib);
    await until(page, (t) => ![...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'the song leaves the results', fx.track.title);
    // The answer read before the removal lands; the one asked after it (the library changed during
    // the ask) is held, so only the older answer is drawn meanwhile
    held.release();
    let release2; lib.hold = new Promise((r) => { release2 = r; });
    await countShown(page, 4);
    for (const end = Date.now() + 7000; lib.asked.length < 3; await sleep(50)) if (Date.now() > end) throw new Error('Timed out: asked once more');
    await sleep(200);
    assert.ok(!(await songListed(page)), 'the answer read before the removal does not list the removed song again');
    await page.screenshot({ path: path.join(shots, `removed-${width}-landed.png`) });
    release2(); lib.hold = null;
    await until(page, () => document.querySelector('#sprog').hidden, 'progress line gone');
    assert.ok(!(await songListed(page)), 'nor the one after it');
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// The same, but the song is brought back (Your library › Node library › Bring back: the same song,
// the same id) before that answer lands: the answer is not cut, so Back on the search lists the song
// at once, while the next ask is still on its way (before, it stayed out until that ask was in).
// Another song removed then and not brought back stays out (Bring back names a file: only that
// file's song is back), also when the app never learnt its file (other: 'failed', its lookup
// answered 500; 'pending', its lookup still on its way)
const FILES = { 1: '/music/Sefa/Journey/01 Journey Through Time.flac', 2: '/music/Sefa/Journey/02 Journey Home.flac' };
async function broughtBack(browser, width, other = 'known') {
  const { page, context, state, lib, changed } = await setup(browser, width);
  lib.song2 = true;
  const restored = []; const hiddenFiles = new Set();
  let releaseLookup = () => {}; const lookupHeld = new Promise((r) => { releaseLookup = r; });
  // A song's file as the node says it (none once it is out: its file is hidden, so 404), and Node
  // library's list of files taken out
  await context.route(/\/v1\/library\/tracks\/\d+$/, (route) => {
    const id = Number(route.request().url().split('/').pop()); const f = FILES[id];
    if (route.request().method() === 'DELETE') { hiddenFiles.add(f); return route.fulfill({ status: 204, body: '' }); }
    if (id === 2 && other === 'failed') return route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'database busy' }) });
    if (id === 2 && other === 'pending') return lookupHeld.then(() => route.fulfill({ status: 404, contentType: 'application/json', body: '{}' })).catch(() => {});
    if (hiddenFiles.has(f)) return route.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ error: `no track ${id}` }) });
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...fx.track, id, file: { ...fx.track.file, path: f } }) });
  });
  await context.route(/\/v1\/library\/hidden$/, (route) => route.fulfill({ contentType: 'application/json', body: JSON.stringify([...hiddenFiles].reverse().map((p) => ({ path: p, hidden_at: Math.floor(Date.now() / 1000) - 5 }))) }));
  await context.route(/\/v1\/library\/hidden\/restore$/, (route) => { const p = JSON.parse(route.request().postData()).path; restored.push(p); hiddenFiles.delete(p); if (p === FILES[1]) lib.songGone = false; return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ restored: true, scan_started: false, scan_queued: false }) }); });
  try {
    const held = await backWhileHeld(page, lib, changed, 4); // (that ask still has both songs)
    assert.ok(await songListed(page) && await songListed(page, SONG2), 'both songs are listed');
    await removeSong(page, lib);
    await until(page, (t) => ![...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'the song leaves the results', fx.track.title);
    await removeSong(page, lib, SONG2);
    await until(page, (t) => ![...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'the other song leaves the results', SONG2);
    await nav(page, 'library');
    await until(page, () => !!document.querySelector('[data-libtab="node"]'), 'Your library');
    await page.locator('[data-libtab="node"]').click();
    await until(page, () => document.querySelectorAll('[data-unhide]').length === 2, 'both files listed as taken out');
    await page.locator('.set-row', { hasText: '01 Journey Through Time.flac' }).locator('[data-unhide]').click();
    await until(page, () => /reading it again|next scan/.test(document.body.innerText), 'brought back');
    assert.deepEqual(restored, [FILES[1]], 'the first song\'s file was brought back on the node');
    // The answer read before the removal lands while Your library is open (no new ask: the search
    // isn't on screen); the ask Back starts is held
    held.release();
    await sleep(300);
    const asked = lib.asked.length;
    let release2; lib.hold = new Promise((r) => { release2 = r; });
    for (let k = 0; k < 4 && !(await page.evaluate(() => history.state && history.state.v === 'search')); k += 1) await page.goBack();
    await until(page, () => history.state && history.state.v === 'search' && !!document.querySelector('#sresults'), 'back on the search');
    for (const end = Date.now() + 7000; lib.asked.length <= asked; await sleep(50)) if (Date.now() > end) throw new Error('Timed out: asked again after Back');
    await countShown(page, 4);
    await sleep(200);
    await page.screenshot({ path: path.join(shots, `brought-back-${other}-${width}.png`) });
    assert.ok(await songListed(page), `${width} (${other}): brought back before the answer landed, the song is listed again at once (${(await shown(page)).replace(/\s+/g, ' ')})`);
    assert.ok(!(await songListed(page, SONG2)), `${width} (${other}): the other removed song, not brought back, stays out (${(await shown(page)).replace(/\s+/g, ' ')})`);
    release2(); lib.hold = null;
    await until(page, () => document.querySelector('#sprog').hidden, 'progress line gone');
    assert.ok(await songListed(page) && !(await songListed(page, SONG2)), `${width} (${other}): and after the next answer`);
    assert.deepEqual(state.errors, []);
  } finally { releaseLookup(); await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412, 1280]) {
      await ownWrite(browser, width);
      await nodeEvent(browser, width);
      await redrawn(browser, width);
      await back(browser, width);
      await changedWhileAsked(browser, width);
      await unchanged(browser, width);
      await errored(browser, width);
      await erroredLibrary(browser, width);
      await erroredAfterAll(browser, width);
      await offlineFirst(browser, width);
      await removedWhileAsked(browser, width);
      await broughtBack(browser, width);
      await broughtBack(browser, width, 'failed');
      await broughtBack(browser, width, 'pending');
      if (width === 360) await hung(browser, width);
      console.log(`ok ${width}px: a collection write or library change makes the same search ask again; the one on screen stays; a change during the ask asks once more; a hung ask is given up; same answer not redrawn; errored asks the library only (and its error goes when that was the one missing, or when the stream broke after every answer); an offline one starts afresh; a song removed during the ask stays out, unless brought back meanwhile (only the song whose file was brought back, not one whose file lookup failed or is still on its way)`);
    }
  } finally { await browser.close(); }
})().catch((e) => { console.error(e); process.exit(1); });
