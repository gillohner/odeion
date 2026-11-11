// Three keyboard and redraw cases, real candidate assets against isolated fixtures (no node):
//   * phone song rows: Tab reaches each row (a button: the row is its Play, the play triangle is
//     hidden on phones), Enter or Space on a row plays from that song and doesn't toggle playback;
//   * Search with no words while a search still runs (the phone's Search tab, the box emptied): the
//     progress line goes, the search box is no longer a combobox over results, and the old search
//     ending later changes nothing;
//   * Artists Sort while the library changes: the redraw that comes during the sort (its list in first
//     or last) keeps the typed filter, its matching rows and count, and Sort's keyboard focus. (Albums
//     sort from the list in memory: no answer to wait for, so no redraw comes in between.)
//   [ONLY=rows|search|sort] node tools/ui-rows-search-sort.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'rows-search-sort-shots'));
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(50); } };

// ---- phone song rows -------------------------------------------------------
const songs = [1, 2, 3, 4].map((i) => ({ no: i, title: `Song ${i}`, artist: 'Eric Clapton', duration_ms: 200000 + i * 1000, uri: `track:${i}`, track_id: i, id: 'lib-' + i, page: 'lib-' + i }));
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: { album_id: 7 }, title: 'Story', artist: { name: 'Eric Clapton', id: 'lib-31' }, album: null, year: 1990, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, songs, total_ms: 810000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
const libTrack = (s) => ({ ...fx.track, id: s.track_id, title: s.title, artists: [{ id: 31, name: 'Eric Clapton' }], album: { id: 7, title: 'Story' }, duration_ms: s.duration_ms });
const rowFixtures = {
  ...Object.fromEntries(songs.map((s) => [`/library/tracks/${s.track_id}`, { body: libTrack(s) }])),
  '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
};
// A zone on the node (stopped, nothing queued): what a row plays is what the page sends it
const zone = { id: 'linn', name: 'Linn', output: 'upnp:linn', transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 3, muted: false }, now_playing: null };
rowFixtures['/zones'] = { body: [zone] };
rowFixtures['/outputs'] = { body: [{ id: 'upnp:linn', name: 'Linn', kind: 'upnp', caps: { hw_volume: 'percent' } }] };
rowFixtures['/zones/linn/queue'] = { body: [] };

async function phoneRows(browser, width) {
  const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures: rowFixtures, zone: 'linn' });
  await page.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  try {
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector('#isongs .srow');
    await sleep(300);
    assert.equal(await page.evaluate(() => document.documentElement.dataset.shell), 'phone');
    const rows = await page.locator('#isongs .srow').evaluateAll((els) => els.map((r) => ({ tab: r.tabIndex, role: r.getAttribute('role'), play: !!r.querySelector('[data-splay]') && getComputedStyle(r.querySelector('[data-splay]')).display })));
    assert.equal(rows.length, songs.length);
    for (const r of rows) { assert.equal(r.tab, 0, 'a row is in the Tab order'); assert.equal(r.role, 'button', 'a row is a button'); assert.equal(r.play, 'none', 'no play triangle on phones'); }
    // Tab from the first row: row, its ⋯, the next row, …
    await page.locator('#isongs .srow').first().focus();
    const order = ['row0'];
    for (let i = 0; i < 5; i++) { await page.keyboard.press('Tab'); order.push(await page.evaluate(() => { const a = document.activeElement; return a.matches('.srow') ? 'row' + a.dataset.srow : a.dataset.smore != null ? 'more' + a.dataset.smore : a.tagName; })); }
    assert.deepEqual(order, ['row0', 'more0', 'row1', 'more1', 'row2', 'more2'], 'Tab walks rows and their ⋯ in order');
    // Enter on the second row plays from Song 2; Space on the fourth plays from Song 4 (and doesn't
    // press play/pause)
    const sent = () => state.hits.filter((h) => h.m !== 'GET' && h.v.startsWith('/zones/linn/'));
    const queued = async (n) => { await until(async () => sent().length >= n); return sent().slice(n - 1, n).map((h) => [h.v, JSON.parse(h.body || '{}')])[0]; };
    await page.locator('#isongs .srow').nth(1).focus();
    await page.keyboard.press('Enter');
    const first = await queued(1);
    assert.equal(first && first[0], '/zones/linn/queue', 'Enter sends the queue');
    assert.deepEqual(first[1].track_ids, [2, 3, 4], 'Enter plays from the row');
    assert.equal(first[1].replace && first[1].play, true);
    await page.locator('#isongs .srow').nth(3).focus();
    await page.keyboard.press('Space');
    const second = await queued(2);
    assert.equal(second && second[0], '/zones/linn/queue', 'Space sends the queue');
    assert.deepEqual(second[1].track_ids, [4], 'Space plays from the row');
    await sleep(400);
    assert.equal(sent().length, 2, 'one send per key, no play/pause');
    assert.equal(await page.evaluate(() => document.activeElement.matches('#isongs .srow') && document.activeElement.dataset.srow), '3', 'focus stays on the row');
    await page.screenshot({ path: path.join(shots, `phone-rows-${width}.png`) });
    assert.deepEqual(state.errors, []);
    console.log(`PASS phone rows ${width}: Tab order, Enter and Space play the row`);
  } finally { await context.close(); }
}

// ---- Search with no words while a search still runs -------------------------
// The phone's Search tab with the box emptied (its words deleted before the 250 ms a new search
// waits for), while the last search is still on its way
async function emptySearchWhileRunning(browser, width) {
  const SLOW = 2500;
  const { page, context, state } = await fx.create(browser, width, { mobile: true, delay: (v) => (v === '/search' ? SLOW : 0) });
  try {
    await fx.go(page, '/');
    await page.locator('#search').fill('blues');
    await page.locator('#search').press('Enter');
    await page.waitForFunction(() => location.search.includes('q=blues'));
    assert.ok(await until(() => page.evaluate(() => !document.getElementById('sprog').hidden)), 'progress shows while the search runs');
    assert.equal(await page.locator('#search').getAttribute('aria-expanded'), 'true');
    const asked = Date.now();
    await page.evaluate(() => { document.getElementById('search').value = ''; });
    await page.locator('[data-bnav="search"]').click();
    await page.waitForFunction(() => location.pathname === '/search' && !location.search.includes('q='));
    const check = async (when) => {
      const st = await page.evaluate(() => { const s = document.getElementById('search'); return { prog: document.getElementById('sprog').hidden, exp: s.getAttribute('aria-expanded'), ctl: s.getAttribute('aria-controls'), act: s.getAttribute('aria-activedescendant'), results: !!document.getElementById('sresults'), empty: !!document.querySelector('#main .empty') }; });
      assert.equal(st.prog, true, `${when}: no progress line on Search with no words`);
      assert.equal(st.exp, 'false', `${when}: the box is not an open combobox`);
      assert.equal(st.ctl, null, `${when}: the box controls no results`);
      assert.equal(st.act, null, `${when}: no active result`);
      assert.equal(st.results, false, `${when}: no results drawn`);
    };
    await check('at once');
    // The old search ends now: nothing of it comes back
    await sleep(Math.max(0, SLOW + 600 - (Date.now() - asked)));
    assert.ok(state.hits.some((h) => h.v === '/search'), 'the search was asked');
    await check('after the old search ended');
    await page.screenshot({ path: path.join(shots, `empty-search-${width}.png`) });
    assert.deepEqual(state.errors, []);
    console.log(`PASS empty search while one runs ${width}: progress and combobox cleared, its late end ignored`);
  } finally { await context.close(); }
}

// ---- Sort while the library changes ----------------------------------------
const artists = Array.from({ length: 45 }, (_, i) => ({ id: i + 1, name: i < 3 ? `Loop ${'ABC'[i]}` : `Other ${i}`, albums: i < 3 ? [1, 3, 2][i] : 1, tracks: 3 }));
const lists = { artists: { url: /\/v1\/library\/artists(?:\?|$)/, body: artists, rows: '#artistlist [data-artist]', attr: 'data-artist' } };
// `order`: which answer comes last, the sort's own ('sort') or the library redraw's ('library')
async function sortDuringRedraw(browser, width, kind, order) {
  const { page, context, state } = await fx.create(browser, width, { mobile: width < 600 });
  const L = lists[kind];
  const gates = {}; const held = {};
  const gate = (n) => { gates[n] = {}; gates[n].p = new Promise((r) => { gates[n].open = r; }); held[n] = new Promise((r) => { gates[n].entered = r; }); };
  gate(2); gate(3);
  let n = 0;
  await context.route(L.url, async (route) => {
    const k = ++n;
    if (gates[k]) { gates[k].entered(); await gates[k].p; }
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify(L.body) });
  });
  let socket = null;
  await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
  try {
    await fx.go(page, `/${kind}`);
    await page.locator('#filter').waitFor();
    assert.ok(await until(() => !!socket), 'events connected');
    await page.fill('#filter', 'Loop');
    await page.locator('[data-lsort]').focus();
    await page.selectOption('[data-lsort]', 'added');
    await held[2];
    // The library changes while the sort's list is on its way: the page redraws
    socket.send(JSON.stringify({ type: 'library_changed' }));
    await held[3];
    if (order === 'sort') { gates[3].open(); await sleep(300); gates[2].open(); } else { gates[2].open(); await sleep(300); gates[3].open(); }
    assert.ok(await until(async () => (await page.locator(L.rows).count()) === 3), `${kind}: only the matches once both lists are in (rows: ${await page.locator(L.rows).count()})`);
    await sleep(200);
    assert.equal(await page.locator('#filter').inputValue(), 'Loop', 'the typed filter survives the redraw');
    assert.deepEqual(await page.locator(L.rows).evaluateAll((els, attr) => els.map((e) => Number(e.getAttribute(attr))), L.attr), [3, 2, 1], 'matches in the chosen order');
    assert.equal(await page.locator('.page-head .count').getAttribute('data-filtered'), '3 shown', 'the count follows the rows');
    assert.equal(await page.locator('[data-lsort]').inputValue(), 'added');
    assert.equal(await page.locator('[data-lsort]').evaluate((e) => e === document.activeElement), true, 'Sort keeps the keyboard focus');
    await page.screenshot({ path: path.join(shots, `sort-redraw-${kind}-${width}-${order}.png`) });
    assert.deepEqual(state.errors, []);
    console.log(`PASS sort during a library redraw ${kind} ${width} (${order} answers last): filter, rows, count and focus kept`);
  } finally { Object.values(gates).forEach((g) => g.open()); await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    // ONLY=rows|search|sort runs one part
    const run = (part) => !process.env.ONLY || process.env.ONLY === part;
    if (run('rows')) for (const w of [360, 390, 412]) await phoneRows(browser, w);
    if (run('search')) for (const w of [360, 390, 412]) await emptySearchWhileRunning(browser, w);
    if (run('sort')) for (const w of [1280, 390]) for (const order of ['sort', 'library']) await sortDuringRedraw(browser, w, 'artists', order);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
