// A list page with nothing in it shows no "0" beside its title. Before, Genres, Albums, Artists,
// Songs and Playlists read "Genres 0" above "No genres yet" (the same thing said twice), while
// Favourites already dropped its count when empty. Now one rule: a count of 0 is not shown, and
// the empty state says it. With something in the list the count is there as before.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-empty-head-count.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/empty-head-count-shots');
fs.mkdirSync(shots, { recursive: true });

let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

const album = { id: 7, title: 'Journey', album_artist: 'Sefa', year: 2020, tracks: 1 };
// [page, title, the empty state's words, what makes it hold something, its count then]
const PAGES = [
  ['/genres', 'Genres', /No genres yet/, { fixtures: { '/library/genres': { body: [{ id: 1, name: 'Rock', albums: 1, tracks: 3 }] } } }, '1'],
  ['/albums', 'Albums', /No albums yet/, { fixtures: { '/library/albums': { body: [album] } } }, '1'],
  ['/artists', 'Artists', /No artists yet/, { fixtures: { '/library/artists': { body: fx.artists.map((a) => ({ ...a, albums: 1, tracks: 1 })) } } }, '2'],
  ['/tracks', 'Songs', /No songs yet/, {}, '1'],
  ['/playlists', 'Collections', /No collections yet/, { collections: [fx.summary] }, '1'],
  // Albums with Favourites kept on (it is remembered on this device): "0 of 0" said it too
  ['/albums', 'Albums', /No albums yet/, { fixtures: { '/library/albums': { body: [album] } } }, '0 of 1', true],
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) {
      for (const [p, title, none, some, want, favOn] of PAGES) {
        for (const empty of [true, false]) {
          const what = `${w} ${title}${favOn ? ' (Bookmarks on)' : ''} ${empty ? 'empty' : 'with something'}`;
          const { page, context, state } = await fx.create(browser, w, empty ? { tracks: [] } : some);
          if (favOn) await context.addInitScript(() => localStorage.setItem('pm.list.albums', JSON.stringify({ fav: true })));
          try {
            await fx.go(page, p);
            await page.waitForFunction((t) => ((document.querySelector('#main .page-head h2') || {}).textContent || '').startsWith(t), title);
            if (empty) await page.waitForSelector('#main .empty');
            else await page.waitForSelector('#main .page-head h2 .count');
            const got = await page.evaluate(() => {
              const h2 = document.querySelector('#main .page-head h2');
              const n = h2.querySelector('.count');
              return { h2: h2.textContent, count: n ? n.textContent : null, empty: (document.querySelector('#main .empty') || {}).textContent || '' };
            });
            if (empty) {
              check(got.count === null && got.h2 === title, `${what}: the title has no count (${JSON.stringify(got.h2)})`);
              check(none.test(got.empty), `${what}: the empty state says there is nothing`);
            } else {
              check(got.count === want, `${what}: the count is there (${JSON.stringify(got.count)}, want ${JSON.stringify(want)})`);
            }
            await fx.cleanCopy(page);
            check(!state.errors.length, `${what}: no page errors ${state.errors.join('; ')}`);
            await page.screenshot({ path: path.join(shots, `${p.slice(1)}${favOn ? '-fav' : ''}-${empty ? 'empty' : 'some'}-${w}.png`) });
          } catch (e) { failed++; console.log(`FAIL ${what}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
        }
      }
    }
  } finally { await browser.close(); }
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})();
