// Favourites' count line names only what you have, in the order of the page's sections.
// Before, it always read "N albums · N songs · N artists": "0 albums · 1 song · 0 artists" for one
// hearted song, in a different order from the sections below it (Albums, Artists, Songs), and the
// "From other services" items were not counted at all, so a page holding only those read
// "0 albums · 0 songs · 0 artists" above them. Now: "1 song"; "1 album · 2 artists · 3 songs";
// "2 from other services"; and an empty page shows no count (its empty state says it).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-favourites-count.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/favourites-count-shots');
fs.mkdirSync(shots, { recursive: true });

let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

const album = { id: 7, title: 'Journey', album_artist: 'Sefa', year: 2020 };
const song = (id) => ({ ...fx.track, id, title: `Song ${id}` });
const OTHER = ['https://audius.co/sefa/one', 'https://archive.org/details/two'];
const fav = (o) => ({ uris: [], albums: [], artists: [], tracks: [], other: [], ...o });
const cases = [
  { name: 'one-song', f: fav({ tracks: [song(1)] }), want: '1 song', heads: ['Songs'] },
  { name: 'all-kinds', f: fav({ albums: [album], artists: [{ id: 11, name: 'Sefa' }, { id: 22, name: 'Dr. Peacock' }], tracks: [song(1), song(2), song(3)] }), want: '1 album · 2 artists · 3 songs', heads: ['Albums', 'Artists', 'Songs'] },
  { name: 'albums-and-songs', f: fav({ albums: [album], tracks: [song(1)] }), want: '1 album · 1 song', heads: ['Albums', 'Songs'] },
  { name: 'other-only', f: fav({ other: OTHER }), want: '2 from other services', heads: ['From other services'] },
  { name: 'songs-and-other', f: fav({ tracks: [song(1)], other: OTHER.slice(0, 1) }), want: '1 song · 1 from other services', heads: ['Songs', 'From other services'] },
  { name: 'empty', f: fav({}), want: null, heads: [] },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) {
      for (const c of cases) {
        const described = c.f.other.map((u, i) => ({ uri: u, page: u, title: `Other ${i + 1}`, site: new URL(u).hostname, also_on: [], kind: 'track' }));
        const { page, context, state } = await fx.create(browser, w, { fixtures: { '/favourites': { body: c.f }, '/discover/describe': { body: described } } });
        try {
          await fx.go(page, '/favourites');
          await page.waitForFunction(() => /Favourites/.test((document.querySelector('#main .page-head h2') || {}).textContent || ''));
          if (c.f.other.length) await page.waitForSelector('#favother .list-row');
          const got = await page.evaluate(() => {
            const n = document.querySelector('#main .page-head h2 .count');
            return { count: n ? n.textContent : null, heads: [...document.querySelectorAll('#main .section-head h3')].map((h) => h.textContent), empty: !!document.querySelector('#main .empty') };
          });
          check(got.count === c.want, `${w} ${c.name}: count ${JSON.stringify(got.count)} (want ${JSON.stringify(c.want)})`);
          check(JSON.stringify(got.heads) === JSON.stringify(c.heads), `${w} ${c.name}: sections ${got.heads.join(', ') || 'none'} in the count's order`);
          check(!/\b0 (albums|songs|artists)\b/.test(got.count || ''), `${w} ${c.name}: no "0 …" in the count`);
          if (c.name === 'empty') check(got.empty, `${w} empty: the empty state says there is nothing yet`);
          await fx.cleanCopy(page);
          check(!state.errors.length, `${w} ${c.name}: no page errors ${state.errors.join('; ')}`);
          await page.screenshot({ path: path.join(shots, `favourites-${c.name}-${w}.png`) });
        } catch (e) { failed++; console.log(`FAIL ${w} ${c.name}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})();
