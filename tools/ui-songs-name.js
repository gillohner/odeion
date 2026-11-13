// One name for songs: the library's song list is "Songs" everywhere it is named (the sidebar, the
// page heading, its filter box, empty state and no-match line, the Back button of a song page),
// as the Library chips, search and collections already say. Favourites and a genre page count and
// head their songs the same way (so do Genres and the empty Queue and Playlists pages), and none of
// these pages says "track" anywhere a person reads. The words the app counts songs with elsewhere
// (a scan done, a backup restored, songs added, Sources, Settings, bulk tags) say songs too.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-songs-name.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/songs-name-shots');
fs.mkdirSync(shots, { recursive: true });
const tracks = Array.from({ length: 20 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
const genre = { genre: { id: 4, name: 'Ambient', albums: 1, tracks: 2 }, albums: [], tracks: tracks.slice(0, 2) };
const songItem = { kind: 'song', id: 'lib-1', title: 'Journey Through Time', section: 'tracks', target: { track_id: 1 }, songs: [], artist: { name: 'Sefa' }, source: 'Your library', source_id: 'library', uri: null, song_uri: null, link: null, albums: [], versions: [], also_on: [], about: [], open_on: [], connect: [], on_pubky: null };
const fixtures = (n) => ({
  '/library/tracks': { body: { tracks: tracks.slice(0, n), next: null } }, '/library/stats': { body: { tracks: n } },
  '/library/search': { body: { tracks: [], albums: [], artists: [] } },
  '/library/genres/4': { body: genre }, '/library/genres': { body: [genre.genre] }, '/item/song/lib-1': { body: songItem },
});
// What a person reads on the page: its text, and the labels, titles and placeholders of what is shown
const words = (page) => page.evaluate(() => {
  const attrs = [...document.querySelectorAll('[aria-label],[title],[placeholder]')].filter((e) => e.getClientRects().length).flatMap((e) => ['aria-label', 'title', 'placeholder'].map((a) => e.getAttribute(a) || ''));
  return document.body.innerText + '\n' + attrs.join('\n') + '\n' + document.title;
});
const noTrack = async (page, where) => {
  const t = await words(page); const m = t.match(/.{0,40}\btracks?\b.{0,40}/i);
  assert.equal(m && m[0], null, `${where}: no "track" in what a person reads`);
};
// The page heading's own words, without its count
const heading = (page) => page.locator('main .page-head h2').first().evaluate((h) => h.firstChild.textContent);

// Counts said where a page can't easily be driven here (toasts after an import or a bulk tag, a
// restored backup), read from the app's code: none counts "tracks"
function countWords() {
  const app = fs.readFileSync(path.join(path.resolve(process.argv[2] || path.join(__dirname, '..')), 'crates/pm-api/static/app.js'), 'utf8');
  const counts = [...app.matchAll(/countOf\([^;]*?,\s*'tracks?'\)|nOf\([^;]*?,\s*'tracks?',/g)].map((m) => m[0]);
  assert.deepEqual(counts, [], 'no count says "tracks"');
  console.log('PASS counts said elsewhere (songs added, a restored backup, bulk tags) say songs');
}

async function run(browser, width) {
  const T = `${width}`;
  // The Songs page, with songs
  {
    const { page, context, state } = await fx.create(browser, width, { fixtures: fixtures(20), tracks, favs: [fx.SONG] });
    try {
      await fx.go(page, '/tracks');
      await page.waitForFunction(() => document.querySelectorAll('#tracklist .track').length >= 12);
      assert.equal((await page.locator('#sidebar [data-view="tracks"]').textContent()).trim(), 'Songs', `${T}: the sidebar says Songs`);
      assert.equal((await heading(page)).trim(), 'Songs', `${T}: the page heading says Songs`);
      assert.equal(await page.locator('#filter').getAttribute('placeholder'), 'Filter songs', `${T}: the filter box`);
      assert.equal(await page.locator('#filter').getAttribute('aria-label'), 'Filter songs', `${T}: the filter box's name`);
      // The node's line under the sidebar counts songs too
      await page.waitForFunction(() => /·/.test(document.querySelector('#conn .node-text')?.textContent || ''));
      assert.match(await page.locator('#conn .node-text').textContent(), /· 20 songs\b/, `${T}: the node line counts songs`);
      await fx.cleanCopy(page);
      await noTrack(page, `${T} Songs`);
      await page.screenshot({ path: path.join(shots, `songs-${width}.png`) });
      await page.fill('#filter', 'zzz');
      await page.waitForFunction(() => (document.querySelector('.filter-none')?.textContent || '') !== '', null, { timeout: 5000 });
      assert.equal(await page.locator('.filter-none').textContent(), 'No song matches “zzz”.', `${T}: the no-match line`);
      // A song opened from the list: Back names the list it came from
      await page.locator('#filter').press('Escape');
      await page.waitForFunction(() => document.querySelectorAll('#tracklist .track').length >= 12);
      await page.locator('#tracklist .track a[data-act="song"]').first().click();
      const back = page.locator('main .pagebar [data-back="tracks"]:visible').first();
      await back.waitFor();
      assert.equal((await back.innerText()).trim(), 'Songs', `${T}: Back from a song opened in Songs says Songs`);
      await noTrack(page, `${T} song page`);
      // Favourites: counted, played and headed as songs
      await fx.go(page, '/favourites');
      await page.locator('#favplay').waitFor();
      assert.match(await page.locator('main .page-head').first().innerText(), /\b1 song\b/, `${T}: Favourites counts songs`);
      assert.equal((await page.locator('#favplay').textContent()).trim(), 'Play songs', `${T}: Favourites' Play button`);
      assert.deepEqual(await page.locator('main .section-head h3').allTextContents(), ['Songs'], `${T}: Favourites' song section`);
      await noTrack(page, `${T} Favourites`);
      await page.screenshot({ path: path.join(shots, `favourites-${width}.png`) });
      // A genre: counted and headed as songs
      await fx.go(page, '/genre/4');
      await page.locator('#gplay').waitFor();
      assert.match(await page.locator('main .page-head').first().innerText(), /\b2 songs\b/, `${T}: a genre counts songs`);
      assert.ok((await page.locator('main .section-head h3').allTextContents()).includes('Songs'), `${T}: a genre's song section`);
      await noTrack(page, `${T} genre`);
      // The Genres list counts each genre's songs
      await fx.go(page, '/genres');
      await page.locator('main [data-genre]').first().waitFor();
      assert.match(await page.locator('main [data-genre]').first().innerText(), /\b2 songs\b/, `${T}: Genres counts songs`);
      await noTrack(page, `${T} Genres`);
      // A scan that ends says how many songs the library has
      state.fixtures['/info'] = { body: { name: 'Fixture node', offline: false, scanning: true, library: { tracks: 20, albums: 1, artists: 2, files: 20, missing_files: 0 } } };
      await fx.go(page, '/tracks');
      await page.waitForFunction(() => document.querySelectorAll('#tracklist .track').length >= 12);
      await page.waitForTimeout(300);
      state.fixtures['/info'] = { body: { name: 'Fixture node', offline: false, scanning: false, library: { tracks: 1234, albums: 1, artists: 2, files: 1234, missing_files: 0 } } };
      await page.waitForFunction(() => /Library up to date/.test(document.querySelector('#toast')?.textContent || ''), null, { timeout: 6000 })
        .catch(() => assert.fail(`${T}: the scan-done toast shows`));
      assert.equal(await page.locator('#toast').textContent(), 'Library up to date: 1,234 songs', `${T}: the scan-done toast counts songs`);
      delete state.fixtures['/info'];
      assert.deepEqual(state.errors, [], 'no page errors');
    } finally { await context.close(); }
  }
  // The Songs page of an empty library
  {
    const { page, context, state } = await fx.create(browser, width, { fixtures: fixtures(0), tracks: [] });
    try {
      await fx.go(page, '/tracks');
      await page.locator('main .empty h3').waitFor();
      assert.equal((await page.locator('main .empty h3').textContent()).trim(), 'No songs yet', `${T}: the empty state`);
      await noTrack(page, `${T} empty Songs`);
      // A song page opened from its address, with no album or artist page to go up to: Back goes to Songs
      await fx.go(page, '/song/lib-1');
      const back = page.locator('main .pagebar [data-back="tracks"]:visible').first();
      await back.waitFor();
      assert.equal((await back.innerText()).trim(), 'Songs', `${T}: a song page's Back says Songs`);
      await noTrack(page, `${T} song page`);
      // The empty Queue and Playlists pages say what to add as songs
      for (const [url, want] of [['/queue', /^Play an album or add songs with the row actions\.$/], ['/playlists', /add songs from any list/]]) {
        await fx.go(page, url);
        await page.locator('main .empty p').first().waitFor();
        assert.match(await page.locator('main .empty p').first().textContent(), want, `${T} ${url}: the empty page`);
        await noTrack(page, `${T} ${url}`);
      }
      assert.deepEqual(state.errors, [], 'no page errors');
    } finally { await context.close(); }
  }
  console.log(`PASS ${T}: the song list, Favourites, Genres, a song page and the empty Queue and Playlists say "songs", never "tracks"`);
}

(async () => {
  countWords();
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) await run(browser, width);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
