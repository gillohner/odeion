// A library list's filter that matches nothing (Albums, Artists, Tracks) says so under the list,
// in a status line that is there from the start; matching again, or Escape, empties it. On Tracks
// (a node search) each keystroke empties it, and only the answer for the text in the box fills it.
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
  // `held`: words whose answer waits until the test lets it go (Tracks' search races)
  const held = new Map();
  await context.route(/\/v1\/library\/search\?/, async (route) => { const q = new URL(route.request().url()).searchParams.get('q'); if (held.has(q)) await held.get(q).promise; words = q; return route.fallback(); });
  const hold = (q) => { let release; const promise = new Promise((r) => { release = r; }); held.set(q, { promise, release }); return () => { held.delete(q); release(); }; };
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
      // A–Z (lists over 40) jumps within the matches: with none, every letter is off
      const letters = () => page.$$eval('[data-az]', (bs) => bs.map((b) => `${b.dataset.az}${b.disabled ? '-' : '+'}${b.classList.contains('on') ? '*' : ''}`).join(' '));
      assert.ok((await letters()).split(' ').every((l) => l.endsWith('-')), `${kind} ${width}: no match, every letter off: ${await letters()}`);
      // Some matches: only their letters stay on, and a letter draws the matches from it on
      // ("band 1" finds Loop B and Other 10–19 by artist; artists by name "other 1": Other 10–19)
      const q = kind === 'albums' ? 'band 1' : 'other 1'; const from = kind === 'albums' ? ['L', 'O'] : ['O'];
      await page.fill('#filter', q);
      await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length === n, [k.rows, kind === 'albums' ? 11 : 10]);
      assert.equal(await letters(), (await page.$$eval('[data-az]', (bs) => bs.map((b) => b.dataset.az))).map((l) => `${l}${from.includes(l) ? '+' : '-'}`).join(' '), `${kind} ${width}: only the matches' letters are on`);
      await page.locator('[data-az="O"]').click();
      await page.waitForFunction(([sel]) => document.querySelector('[data-az="O"]')?.classList.contains('on') && document.querySelectorAll(sel).length > 0, [k.rows]);
      const names = await page.$$eval(`${k.rows}`, (els) => els.map((e) => (e.querySelector('.name, .t')?.textContent || e.textContent).trim()));
      assert.deepEqual(names, Array.from({ length: 10 }, (_, i) => `Other ${10 + i}`), `${kind} ${width}: O jumps within the matches`);
      assert.equal(await page.$eval('.page-head .count', (c) => c.dataset.filtered), `${kind === 'albums' ? 11 : 10} shown`);
      // The chosen letter says so to a screen reader too, and only that one
      assert.deepEqual(await page.$$eval('[data-az]', (bs) => bs.filter((b) => b.getAttribute('aria-pressed') === 'true').map((b) => b.dataset.az)), ['O'], `${kind} ${width}: O is pressed`);
      assert.ok(await page.$$eval('[data-az]', (bs) => bs.every((b) => ['true', 'false'].includes(b.getAttribute('aria-pressed')))), `${kind} ${width}: every letter says whether it is pressed`);
      await noneLine(page, '');
      // Typing again unmarks the letter and draws all the matches
      await page.fill('#filter', q.toUpperCase());
      await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length === n && !document.querySelector('.az.on'), [k.rows, kind === 'albums' ? 11 : 10]);
      // Escape too, and every letter comes back
      await page.locator('[data-az="O"]').click();
      await page.waitForFunction(() => document.querySelector('[data-az="O"]')?.classList.contains('on'));
      await page.locator('#filter').press('Escape');
      await page.waitForFunction(() => !document.querySelector('.az.on') && [...document.querySelectorAll('[data-az]')].every((b) => !b.disabled && b.getAttribute('aria-pressed') === 'false'));
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
    if (kind === 'tracks') {
      // A keystroke empties the line at once, not when its search answers
      const now = await page.evaluate(() => { const f = document.querySelector('#filter'); f.value = 'zzzz'; f.dispatchEvent(new Event('input', { bubbles: true })); return document.querySelector('.filter-none').textContent; });
      assert.equal(now, '', `${kind} ${width}: a keystroke empties the no-match line at once`);
      await noneLine(page, 'No track matches “zzzz”.');
      // An answer for older text that lands after the next keystroke (before that one's search
      // starts) says nothing: "zzz" is sent, "song" is typed, then "zzz" answers
      const letZzz = hold('zzz'); const letSong = hold('song');
      const sent = page.waitForRequest((r) => /\/v1\/library\/search\?/.test(r.url()) && new URL(r.url()).searchParams.get('q') === 'zzz');
      await page.fill('#filter', 'zzz'); await sent;
      await page.fill('#filter', 'song'); letZzz();
      await page.waitForTimeout(700);
      assert.equal(await note(page).textContent(), '', `${kind} ${width}: an outdated answer doesn't say "No track matches"`);
      letSong();
      await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length === n, [k.rows, k.n]);
      await noneLine(page, '');
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
