// Big counts read with separators, as the Node library's top line already does (12,840, not
// 12840): the count beside a page's title (Songs 12,840, Albums 1,200, Artists 1,500), Albums'
// "0 of 1,200" with Favourites on, a filter's "· 1,500 shown", and the empty Genres page's
// "The 12,840 songs in the library have no genre in their tags". Small counts stay as they are.
// The Play all toast's "Loading 2,500 songs… 1,000 so far" is in ui-play-all-progress.js.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-head-count-separators.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/head-count-separators-shots');
fs.mkdirSync(shots, { recursive: true });

let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

const lib = { tracks: 12840, albums: 1200, artists: 1500, files: 12900, missing_files: 0 };
const albums = Array.from({ length: 1200 }, (_, i) => ({ id: i + 1, title: `Album ${i + 1}`, album_artist: 'Sefa', year: 2020, tracks: 10 }));
const artists = Array.from({ length: 1500 }, (_, i) => ({ id: i + 1, name: `Artist ${i + 1}`, albums: 1, tracks: 8 }));
const fixtures = {
  '/info': { body: { name: 'Fixture node', library: lib, scanning: false } },
  '/library/stats': { body: lib },
  '/library/artists': { body: artists },
  '/library/genres': { body: [] },
};
// The node answers albums in parts of up to `limit` after an id (pageAll)
const pagedAlbums = (context) => context.route(/\/v1\/library\/albums(\?|$)/, (r) => {
  const u = new URL(r.request().url()); const after = Number(u.searchParams.get('after') || 0); const limit = Number(u.searchParams.get('limit') || 1000);
  return r.fulfill({ contentType: 'application/json', body: JSON.stringify(albums.filter((a) => a.id > after).slice(0, limit)) });
});
const head = (page) => page.evaluate(() => {
  const c = document.querySelector('#main .page-head h2 .count');
  return c ? { text: c.textContent, filtered: c.dataset.filtered || '', after: getComputedStyle(c, '::after').content } : null;
});

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [w, android] of [[360, true], [412, false], [1280, false]]) {
      const at = `${android ? 'android' : 'browser'}-${w}`;
      const open = async (p, opts = {}) => {
        const made = await fx.create(browser, w, { mobile: w < 600, fixtures, ...opts });
        if (android) await made.context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
        if (opts.fav) await made.context.addInitScript(() => localStorage.setItem('pm.list.albums', JSON.stringify({ fav: true })));
        await pagedAlbums(made.context);
        await fx.go(made.page, p);
        return made;
      };
      const PAGES = [
        ['/tracks', 'Songs', '12,840'],
        ['/albums', 'Albums', '1,200'],
        ['/artists', 'Artists', '1,500'],
        ['/albums', 'Albums (Bookmarks on)', '0 of 1,200', { fav: true }],
      ];
      for (const [p, title, want, opts] of PAGES) {
        const what = `${at} ${title}`;
        const { page, context, state } = await open(p, opts);
        try {
          const ok = await page.waitForFunction(() => document.querySelector('#main .page-head h2 .count'), null, { timeout: 8000 }).then(() => true, () => false);
          const got = ok ? await head(page) : null;
          check(got && got.text === want, `${what}: the title's count reads ${want} (${JSON.stringify(got && got.text)})`);
          if (title === 'Artists') {
            // Every artist matches "Artist": the filter's note says how many with separators too
            await page.fill('#filter', 'Artist');
            await page.waitForFunction(() => (document.querySelector('#main .page-head .count') || {}).dataset?.filtered, null, { timeout: 4000 }).catch(() => {});
            const f = await head(page);
            check(f && f.filtered === '1,500 shown' && /1,500 shown/.test(f.after), `${what}: the filter's note reads "1,500 shown" (${JSON.stringify(f && f.filtered)})`);
          }
          await fx.cleanCopy(page);
          check(!state.errors.length, `${what}: no page errors ${state.errors.join('; ')}`);
          await page.screenshot({ path: path.join(shots, `${p.slice(1)}${opts && opts.fav ? '-fav' : ''}-${at}.png`) });
        } catch (e) { failed++; console.log(`FAIL ${what}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
      }
      // Genres with none in the files: the empty state counts the library's songs
      {
        const what = `${at} Genres (none in the files)`;
        const { page, context, state } = await open('/genres');
        try {
          const ok = await page.waitForSelector('#main .empty', { timeout: 8000 }).then(() => true, () => false);
          const text = ok ? await page.locator('#main .empty').innerText() : '';
          check(/The 12,840 songs in the library have no genre/.test(text) && !/12840/.test(text), `${what}: the empty state reads "The 12,840 songs" (${JSON.stringify(text.split('\n').find((l) => /songs/.test(l)) || text)})`);
          check(!state.errors.length, `${what}: no page errors ${state.errors.join('; ')}`);
          await page.screenshot({ path: path.join(shots, `genres-${at}.png`) });
        } catch (e) { failed++; console.log(`FAIL ${what}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
      }
      // A small count is as before
      {
        const what = `${at} Collections`;
        const { page, context, state } = await open('/playlists', { collections: [fx.summary] });
        try {
          const ok = await page.waitForFunction(() => document.querySelector('#main .page-head h2 .count'), null, { timeout: 8000 }).then(() => true, () => false);
          const got = ok ? await head(page) : null;
          check(got && got.text === '1', `${what}: a small count reads 1 (${JSON.stringify(got && got.text)})`);
          check(!state.errors.length, `${what}: no page errors ${state.errors.join('; ')}`);
        } catch (e) { failed++; console.log(`FAIL ${what}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})();
