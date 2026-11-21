// Remove from library on a search result: the song leaves the results at once. Before, the search
// page was drawn again from its kept answer (it isn't asked again while on screen), so the removed
// song stayed listed until the search was started again. The other results stay where they are,
// the page isn't asked again or emptied, and a removal that fails leaves the song listed. A service's
// copy stays without the library's id (also when it has no page), and Remove chosen with the keyboard
// leaves the focus on the next result's ⋯ (the search box after the last one).
// Real UI in Chromium, isolated responses (no node or external services).
//   node tools/ui-search-remove-song.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/search-remove-song-shots');
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
const OTHER = 'Journey Home';
const ISRC = 'USAAA2600001';
const song = (id, title) => ({ kind: 'song', id: `lib-${id}`, title, subtitle: 'Sefa, Dr. Peacock', album: 'Journey', track_id: id, uri: `track:${id}`, keys: [`track:${id}`, ...(id === 1 ? [`isrc:${ISRC}`] : [])] });
// Spotify's copy of song 1: the node marks it with the library's id while the library has it
// (search.rs service(): track_id, id lib-<n>), and not once it's gone
const SPOTIFY = 'https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC';
// (noPage: a copy the node gives no page for)
const spotifyHit = (inLibrary, noPage) => ({ kind: 'song', id: inLibrary ? 'lib-1' : 'spotify-4uLU6hMCjMI75M1A2tKUQC', title: fx.track.title, subtitle: 'Sefa, Dr. Peacock', page: noPage ? '' : SPOTIFY, uri: 'spotify:track:4uLU6hMCjMI75M1A2tKUQC', track_id: inLibrary ? 1 : null, keys: [`isrc:${ISRC}`] });

async function setup(browser, width, opts = {}) {
  const run = await fx.create(browser, width, { mobile: width < 600 });
  const { context } = run;
  if (opts.android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  // The node's library answer: both songs until one is removed
  const lib = { asked: [], removed: new Set(), deletes: [] };
  await context.route(/\/v1\/search\?/, async (route) => {
    const url = new URL(route.request().url());
    lib.asked.push({ q: url.searchParams.get('q'), scope: url.searchParams.get('in') });
    const hits = [song(1, fx.track.title), song(2, OTHER)].filter((h) => !lib.removed.has(h.track_id));
    const lines = [{ source: 'library', status: 'ok', hits, counts: [] }];
    if (opts.service) lines.push({ source: 'spotify', status: 'ok', hits: [spotifyHit(!lib.removed.has(1), opts.noPage)], counts: [] });
    return route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': opts.service ? 'library,spotify' : 'library' }, body: lines.map((l) => JSON.stringify(l)).join('\n') + '\n' });
  });
  await context.route(/\/v1\/library\/tracks\/\d+$/, async (route) => {
    if (route.request().method() !== 'DELETE') return route.fallback();
    const id = Number(route.request().url().split('/').pop());
    lib.deletes.push(id);
    if (opts.fail) return route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'The node could not remove it' }) });
    lib.removed.add(id);
    return route.fulfill({ contentType: 'application/json', body: '{}' });
  });
  return { ...run, lib };
}
const titles = (page) => page.$$eval('#sresults [data-skind="song"] .srow', (rows) => rows.filter((r) => r.getClientRects().length).map((r) => r.innerText));
const listed = async (page, title) => (await titles(page)).some((t) => t.includes(title));
// Home, then the same words typed again (no reload: the app's kept searches stay)
async function again(page) {
  await page.evaluate(() => document.querySelector('.nav-item[data-view="home"]').click());
  await page.evaluate(() => { const b = document.querySelector('[data-bnav="search"]'); if (b && b.getClientRects().length) b.click(); });
  await page.locator('#search').fill('');
  await page.locator('#search').fill('journey');
  await until(page, (t) => [...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'results drawn again', OTHER);
  await sleep(300);
}
async function remove(page, title) {
  const row = page.locator('#sresults .srow', { hasText: title }).first();
  await row.locator('[data-smore]').click({ force: true });
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from library' }).click();
  await page.locator('dialog[open] button', { hasText: /^Remove$/ }).click();
}

async function removed(browser, width, android) {
  const { page, context, state, lib } = await setup(browser, width, { android });
  const tag = `${width}${android ? '-android' : ''}`;
  try {
    await fx.go(page, '/search?q=journey');
    await until(page, (t) => [...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'results drawn', OTHER);
    assert.ok(await listed(page, fx.track.title), `${tag}: the song is in the results`);
    await page.screenshot({ path: path.join(shots, `remove-${tag}-before.png`) });
    await remove(page, fx.track.title);
    await until(page, () => /Removed from the library/.test(document.body.innerText), 'removed');
    await sleep(300);
    assert.deepEqual(lib.deletes, [1], `${tag}: the song was removed on the node`);
    await page.screenshot({ path: path.join(shots, `remove-${tag}-after.png`) });
    assert.ok(!(await listed(page, fx.track.title)), `${tag}: the removed song is no longer listed (${(await titles(page)).join(' | ')})`);
    assert.ok(await listed(page, OTHER), `${tag}: the other song stays listed`);
    assert.match(await page.locator('#sresults [data-skind="song"] .section-head').innerText(), /\b1\b/, `${tag}: the Songs count says 1`);
    assert.equal(lib.asked.length, 1, `${tag}: the search on screen is not asked again`);
    await fx.cleanCopy(page);
    // Started again (Home, then the same words): still not listed, the node's own answer drawn
    await again(page);
    assert.equal(lib.asked.length, 2, `${tag}: started again after the change, the search asks the node again`);
    assert.ok(!(await listed(page, fx.track.title)), `${tag}: started again, the removed song is not listed`);
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// An Everywhere search where Spotify has the same song: the Spotify copy stays listed, but no
// longer as the library's song (its ⋯ has no Remove or Edit tags, its page isn't lib-1)
async function menuItems(page) {
  await until(page, () => document.querySelectorAll('.popover.menu:not([hidden]) .pop-item').length > 0, 'menu open');
  return page.$$eval('.popover.menu:not([hidden]) .pop-item', (els) => els.map((e) => e.innerText.trim()));
}
async function service(browser, width, noPage) {
  const { page, context, state, lib } = await setup(browser, width, { service: true, noPage });
  try {
    await fx.go(page, '/search?q=journey');
    await until(page, (t) => [...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'results drawn', OTHER);
    const row = () => page.locator('#sresults [data-skind="song"] .srow', { hasText: fx.track.title }).first();
    await row().locator('[data-smore]').click({ force: true });
    assert.ok((await menuItems(page)).includes('Remove from library'), `${width}: the library's song offers Remove (${(await menuItems(page)).join(' · ')} / ${(await titles(page)).join(' | ')})`);
    await page.keyboard.press('Escape');
    await until(page, () => !document.querySelector('.popover.menu:not([hidden])'), 'menu closed');
    await remove(page, fx.track.title);
    await until(page, () => /Removed from the library/.test(document.body.innerText), 'removed');
    await sleep(300);
    assert.deepEqual(lib.deletes, [1]);
    assert.ok(await listed(page, fx.track.title), `${width}: Spotify's copy stays listed`);
    assert.equal(lib.asked.length, 1, `${width}: not asked again`);
    const hrefs = await row().evaluate((r) => [...r.querySelectorAll('a[href]')].map((a) => a.getAttribute('href')));
    assert.ok(!hrefs.some((h) => /lib-1\b/.test(h)), `${width}: Spotify's copy doesn't link to the removed song (${hrefs.join(' ')})`);
    // (with no page of its own, nor to a song page made up from its spotify: uri)
    if (noPage) assert.ok(!hrefs.some((h) => /^\/song\//.test(h)), `${width}: a copy with no page has no song link (${hrefs.join(' ')})`);
    await row().locator('[data-smore]').click({ force: true });
    const items = await menuItems(page);
    assert.ok(items.length && !items.includes('Remove from library') && !items.includes('Edit tags…'), `${width}: Spotify's copy no longer offers Remove or Edit tags (${items.join(' · ')})`);
    await page.keyboard.press('Escape');
    await page.screenshot({ path: path.join(shots, `service-${width}${noPage ? '-nopage' : ''}-after.png`) });
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

// Remove chosen with the keyboard: once the song leaves the list, the focus is on the next result's
// ⋯ (not dropped to the page's top); removing the last result puts it in the search box
const focusOn = (page) => page.evaluate(() => { const a = document.activeElement; if (!a || a === document.body) return 'body'; if (a.id === 'search') return 'search'; const r = a.closest('.srow'); return a.matches('[data-smore]') && r ? 'more:' + r.innerText.replace(/\s+/g, ' ') : a.tagName; });
async function removeByKeys(page, title) {
  const more = page.locator('#sresults .srow', { hasText: title }).first().locator('[data-smore]');
  await more.focus();
  await page.keyboard.press('Enter');
  await until(page, () => !!document.querySelector('.popover.menu:not([hidden]) .pop-item'), 'menu open');
  for (let k = 0; k < 30; k += 1) {
    if (await page.evaluate(() => /Remove from library/.test((document.activeElement || {}).innerText || ''))) break;
    await page.keyboard.press('ArrowDown');
  }
  assert.match(await page.evaluate(() => (document.activeElement || {}).innerText || ''), /Remove from library/, 'Remove reached with the arrow keys');
  await page.keyboard.press('Enter');
  await until(page, () => !!document.querySelector('dialog[open]'), 'dialog open');
  await until(page, () => /^Remove$/.test(((document.activeElement || {}).innerText || '').trim()), 'Remove in focus');
  await page.keyboard.press('Enter');
}
async function keyboard(browser, width) {
  const { page, context, state, lib } = await setup(browser, width);
  try {
    await fx.go(page, '/search?q=journey');
    await until(page, (t) => [...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'results drawn', OTHER);
    await removeByKeys(page, fx.track.title);
    await until(page, () => /Removed from the library/.test(document.body.innerText), 'removed');
    await until(page, (t) => ![...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'song gone', fx.track.title);
    await sleep(300);
    assert.deepEqual(lib.deletes, [1]);
    const next = await focusOn(page);
    assert.ok(next.startsWith('more:') && next.includes(OTHER), `${width}: the focus is on the next result's ⋯ (${next})`);
    await page.screenshot({ path: path.join(shots, `keyboard-${width}-after.png`) });
    await removeByKeys(page, OTHER);
    await until(page, () => !document.querySelector('#sresults [data-skind="song"] .srow'), 'last song gone');
    assert.deepEqual(lib.deletes, [1, 2]);
    await sleep(300);
    assert.equal(await focusOn(page), 'search', `${width}: the last result removed, the focus is in the search box`);
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

async function failed(browser, width) {
  const { page, context, state, lib } = await setup(browser, width, { fail: true });
  try {
    await fx.go(page, '/search?q=journey');
    await until(page, (t) => [...document.querySelectorAll('#sresults .srow')].some((r) => r.innerText.includes(t)), 'results drawn', OTHER);
    await remove(page, fx.track.title);
    await until(page, () => /could not remove/.test(document.body.innerText), 'error shown');
    await sleep(300);
    assert.ok(await listed(page, fx.track.title), `${width}: a removal that failed leaves the song listed`);
    assert.ok(await listed(page, OTHER), `${width}: the other song stays listed`);
    assert.equal(lib.asked.length, 1, `${width}: not asked again`);
    // The same search again, drawn from its kept answer (the library didn't change): still listed
    await again(page);
    assert.equal(lib.asked.length, 1, `${width}: the kept search is drawn again, not asked again`);
    assert.ok(await listed(page, fx.track.title), `${width}: started again after a failed removal, the song is still listed`);
    assert.deepEqual(state.errors, []);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
      await removed(browser, width, android);
      await failed(browser, width);
      if (!android) await service(browser, width);
      if (!android) await service(browser, width, true);
      if (!android) await keyboard(browser, width);
      console.log(`ok ${width}px${android ? ' (Android app)' : ''}: Remove from library on a search result takes the song out of the results (a Spotify copy stays, not as the library's, with or without a page); the keyboard focus goes on to the next result; a failed one leaves it`);
    }
  } finally { await browser.close(); }
})().catch((e) => { console.error(e); process.exit(1); });
