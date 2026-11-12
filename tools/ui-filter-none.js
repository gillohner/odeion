// A library list's filter that matches nothing (Albums, Artists, Tracks) says so under the list,
// in a status line that is there from the start; matching again, or Escape, empties it.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-filter-none.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/filter-none-shots');
fs.mkdirSync(shots, { recursive: true });
const albums = Array.from({ length: 45 }, (_, i) => ({ id: i + 1, title: i < 3 ? `Loop ${'ABC'[i]}` : `Other ${i}`, album_artist: `Band ${i}`, year: 1990, tracks: 1 }));
const artists = albums.map((a) => ({ id: a.id, name: a.title, albums: 1, tracks: 3 }));
const tracks = Array.from({ length: 20 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
// The node's library search answers by the words typed: "song" finds three tracks, anything else none
let words = '';
const fixtures = {
  '/library/albums': { body: albums }, '/library/albums/formats': { body: [] }, '/library/artists': { body: artists },
  '/library/tracks': { body: { tracks, next: null } }, '/library/stats': { body: { tracks: tracks.length } },
  '/library/search': { get body() { return { tracks: words === 'song' ? tracks.slice(0, 3) : [], albums: [], artists: [] }; } },
};
const pages = {
  albums: { url: '/albums', rows: '#albumgrid .tile', what: 'album', some: 'loop', n: 3, all: 45 },
  artists: { url: '/artists', rows: '#artistlist [data-artist]', what: 'artist', some: 'loop', n: 3, all: 45 },
  tracks: { url: '/tracks', rows: '#tracklist .track', what: 'track', some: 'song', n: 3, all: 20 },
};
const note = (page) => page.locator('.filter-none');
async function noneLine(page, want) {
  await page.waitForFunction((w) => (document.querySelector('.filter-none')?.textContent || '') === w, want, { timeout: 5000 })
    .catch(async () => assert.equal(await note(page).count() ? await note(page).textContent() : '(no line)', want, 'the no-match line'));
}
async function run(browser, width, kind) {
  const k = pages[kind];
  const { page, context, state } = await fx.create(browser, width, { fixtures });
  await context.route(/\/v1\/library\/search\?/, (route) => { words = new URL(route.request().url()).searchParams.get('q'); return route.fallback(); });
  try {
    await fx.go(page, k.url);
    await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length >= n, [k.rows, Math.min(k.all, 12)]);
    // There from the start, empty and taking no room, so a screen reader hears it when it fills
    assert.equal(await note(page).count(), 1, `${kind} ${width}: one no-match line`);
    assert.equal(await note(page).getAttribute('role'), 'status');
    assert.equal(await note(page).textContent(), '');
    assert.equal((await note(page).boundingBox())?.height ?? 0, 0, 'empty line takes no room');
    await page.fill('#filter', '  zzz ');
    await page.waitForFunction((sel) => document.querySelectorAll(sel).length === 0, k.rows);
    await page.waitForFunction(() => document.querySelector('.page-head .count')?.dataset.filtered === '0 shown');
    await fx.cleanCopy(page);
    await page.screenshot({ path: path.join(shots, `${kind}-none-${width}.png`) });
    await noneLine(page, `No ${k.what} matches “zzz”.`);
    // Seen above the player bar, and nothing scrolls sideways
    const box = await note(page).boundingBox(); const bar = await page.locator('footer.player').boundingBox();
    assert.ok(box && box.height > 0 && bar && box.y + box.height <= bar.y, `${kind} ${width}: the line is visible above the player`);
    const sideways = () => page.evaluate(() => [document.documentElement, document.querySelector('main')].every((el) => el.scrollWidth <= el.clientWidth));
    assert.ok(await sideways(), 'no sideways scroll');
    // A long word with no spaces (a pasted path) wraps
    const long = 'x'.repeat(150);
    await page.fill('#filter', long);
    await noneLine(page, `No ${k.what} matches “${long}”.`);
    assert.ok(await sideways(), `${kind} ${width}: a long query wraps`);
    // Matching again empties it
    await page.fill('#filter', k.some);
    await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length === n, [k.rows, k.n]);
    await noneLine(page, '');
    await page.fill('#filter', 'zzz');
    await noneLine(page, `No ${k.what} matches “zzz”.`);
    if (kind !== 'tracks') {
      // A–Z (lists over 40) draws from that letter on, so the line goes
      await page.locator('[data-az="O"]').click();
      await page.waitForFunction((sel) => document.querySelectorAll(sel).length > 0, k.rows);
      await noneLine(page, '');
      await page.fill('#filter', 'zz'); await page.fill('#filter', 'zzz');
      await noneLine(page, `No ${k.what} matches “zzz”.`);
    }
    if (kind !== 'tracks') {
      // A new order keeps the filter (renderSortedList), and the line with it
      await page.selectOption('[data-lsort]', 'added');
      await page.waitForFunction(() => document.querySelector('[data-lsort]')?.value === 'added' && document.querySelector('#filter')?.value === 'zzz');
      await noneLine(page, `No ${k.what} matches “zzz”.`);
      assert.equal(await note(page).count(), 1, 'still one line after re-drawing');
    }
    await page.locator('#filter').press('Escape');
    await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length >= n, [k.rows, Math.min(k.all, 12)]);
    await noneLine(page, '');
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log(`PASS ${kind} ${width}: a filter with no matches says so; matching or Escape clears it`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 412]) for (const kind of Object.keys(pages)) await run(browser, width, kind);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
