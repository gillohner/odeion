// A playlist's size reads "N songs" only when every item in it is a song (the node's tracks_only);
// a playlist that also holds links, albums, artists or posts reads "N items". The same words on the Playlists
// page, in a song's ⋯ › Add to playlist… picker, and on someone's shared playlists (their profile and
// Shared by …; phone and desktop).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-playlist-count.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/playlist-count-shots');
fs.mkdirSync(shots, { recursive: true });
const KEY = 'o1234567890abcdefghijklmnopqrstuvwxyz1234567890abcde'.slice(0, 52);
assert.match(KEY, /^[a-z0-9]{52}$/);
const pl = (id, name, items, tracksOnly, t) => ({ id, name, items, covers: [], visibility: 'private', synced: false, layout: 'list', tracks_only: tracksOnly, created_at: 1, updated_at: t });
const own = [pl('m1', 'Music and notes', 2, false, 30), pl('s3', 'Three songs', 3, true, 20), pl('s1', 'One song', 1, true, 10), pl('e0', 'Empty', 0, true, 5), pl('n1', 'One link', 1, false, 1)];
const want = { 'Music and notes': '2 items', 'Three songs': '3 songs', 'One song': '1 song', Empty: '0 songs', 'One link': '1 item' };
const shared = [pl('01ABCDEFGHJKM', 'Shared mix', 4, false, 2), pl('01ABCDEFGHJKN', 'Shared songs', 5, true, 1)];
const wantShared = { 'Shared mix': '4 items', 'Shared songs': '5 songs' };
let ok = 0;
const check = (got, exp, what) => { assert.deepEqual(got, exp, what); ok++; };
// name -> sub line of each row in a list
const rows = (page, sel) => page.$$eval(sel, (rs) => Object.fromEntries(rs.map((r) => [r.querySelector('.name')?.textContent.trim(), r.querySelector('.sub')?.textContent.trim()])));
(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, mobile] of [[360, true], [412, true], [1280, false]]) {
      const { page, context, state } = await fx.create(browser, width, { mobile, collections: own, fixtures: { [`/shared/${KEY}/playlists`]: { body: shared }, [`/profiles/${KEY}`]: { body: { name: 'Fixture friend', links: [], tagged: [] } } } });
      const tag = `${width}`;
      try {
        // The Playlists page
        await fx.go(page, '/playlists');
        await page.waitForSelector('[data-pl="m1"]');
        check(await rows(page, '[data-pl]'), want, `${tag} Playlists page counts`);
        await page.screenshot({ path: path.join(shots, `${tag}-playlists.png`) });
        // A song's ⋯ › Add to playlist…
        await fx.go(page, '/tracks');
        await page.locator('button[aria-label^="More for"]').first().click();
        await page.getByText('Add to playlist…').click();
        await page.waitForSelector('#pplist [data-ppi]');
        const picked = await rows(page, '#pplist [data-ppi]');
        check(picked, want, `${tag} Add to playlist picker counts`);
        await page.screenshot({ path: path.join(shots, `${tag}-picker.png`) });
        await page.locator('[data-ppclose]').click();
        // Someone's shared playlists
        await fx.go(page, `/shared/${KEY}`);
        await page.waitForSelector('[data-shpl]');
        check(await rows(page, '[data-shpl]'), wantShared, `${tag} shared playlists counts`);
        await page.screenshot({ path: path.join(shots, `${tag}-shared.png`) });
        // Their profile's Playlists they share
        await fx.go(page, `/person/${KEY}`);
        await page.waitForSelector('[data-pshared]');
        check(await rows(page, '[data-pshared]'), wantShared, `${tag} profile's shared playlists counts`);
        await page.screenshot({ path: path.join(shots, `${tag}-profile.png`) });
        await fx.cleanCopy(page);
        check(state.errors, [], `${tag} no page errors`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`ui-playlist-count: ${ok} ok`);
})().catch((e) => { console.error('FAIL', e.message); process.exit(1); });
