// A song's ⋯ menu reads the same wherever it opens: a row on Songs (trackMenu) and a song row on an
// album page (itemMenu) list the shared actions in the same order (Play next, Add to queue, Add to
// playlist…, Download, Add to favourites, Start radio, Go to song page, Go to artist, Go to album,
// the owner's Edit tags… / Remove from library, Copy link), the song page is "Go to song page" in
// both (before, "Song page" on Songs), and Copy link carries the copy icon (before, the compass, the
// same as the song page). A song not matched yet: "Which song is this?…" among the owner's fixes,
// above Copy link; a matched one: Wrong match?… there.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-song-menus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/song-menus-shots');
fs.mkdirSync(shots, { recursive: true });

// A matched song (it has a canonical id: Wrong match?… on offer) and one not matched yet
const matched = { ...fx.track, canonical: [{ kind: 'recording', mbid: fx.track.mbid }] };
const unmatched = { ...fx.track, id: 2, title: 'Not matched yet', mbid: null, canonical_uri: null, canonical: [] };
const tracks = [matched, unmatched];
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: {}, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [{ no: 1, title: fx.track.title, artist: 'Sefa', artist_id: 'lib-11', album_id: 'lib-7', duration_ms: 240000, uri: 'track:1', track_id: 1, id: 'lib-1', page: 'lib-1' }], total_ms: 240000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
const fixtures = {
  '/library/tracks': { body: { tracks, next: null } },
  '/library/tracks/1': { body: matched },
  '/library/tracks/2': { body: unmatched },
  '/item/album/lib-7': { body: albumPage },
  '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
};

const menu = (page) => page.evaluate(() => [...document.querySelectorAll('.popover.menu:not([hidden]) .pop-item')].map((b) => ({ label: b.textContent.trim(), icon: (b.querySelector('use')?.getAttribute('href') || '').replace('#i-', '') })));
const close = async (page) => { await page.keyboard.press('Escape'); await page.waitForSelector('.popover.menu:not([hidden])', { state: 'detached', timeout: 2000 }).catch(() => {}); await page.waitForTimeout(150); };
// The shared actions, in the order every song menu lists them
const ORDER = ['Play next', 'Add to queue', 'Add to playlist…', 'Download', 'Add to favourites', 'Start radio', 'Go to song page', 'Go to artist', 'Go to album', 'Edit tags…', 'Remove from library', 'Which song is this?…', 'Wrong match?…', 'Copy link'];
const inOrder = (labels) => { const known = labels.filter((l) => ORDER.includes(l)); return known.every((l, i) => i === 0 || ORDER.indexOf(known[i - 1]) < ORDER.indexOf(l)); };

async function run(browser, width) {
  const { page, state } = await fx.create(browser, width, { fixtures });
  try {
    // 1. Songs: the row's ⋯
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    await page.locator('#tracklist .track').first().hover();
    await page.click('#tracklist .track:first-child [data-act="more"]', { force: true });
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
    const songs = await menu(page);
    await page.screenshot({ path: path.join(shots, `songs-row-${width}.png`) });
    const sl = songs.map((m) => m.label);
    assert.ok(sl.includes('Go to song page') && !sl.includes('Song page'), `${width}: Songs' menu says "Go to song page" (${sl.join(' · ')})`);
    assert.equal(songs.find((m) => m.label === 'Copy link')?.icon, 'copy', `${width}: Songs' Copy link has the copy icon`);
    assert.ok(inOrder(sl), `${width}: Songs' menu in the shared order (${sl.join(' · ')})`);
    assert.ok(sl.indexOf('Add to playlist…') === 2, `${width}: Add to playlist… third, after Add to queue (${sl.join(' · ')})`);
    assert.ok(sl.includes('Wrong match?…') && !sl.includes('Which song is this?…') && sl.indexOf('Wrong match?…') > sl.indexOf('Remove from library') && sl.indexOf('Wrong match?…') < sl.indexOf('Copy link'), `${width}: a matched song: Wrong match?… after Remove, before Copy link (${sl.join(' · ')})`);
    // Go to song page opens it
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Go to song page' }).click();
    await page.waitForFunction(() => location.pathname.startsWith('/song/'));
    await page.goBack(); await page.waitForSelector('#tracklist .track');

    // 2. A song not matched yet (owner): Which song is this?… with the fixes, above Copy link
    await page.locator('#tracklist .track').nth(1).hover();
    await page.click('#tracklist .track:nth-child(2) [data-act="more"]', { force: true });
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
    const um = (await menu(page)).map((m) => m.label);
    assert.ok(um.includes('Which song is this?…') && !um.includes('Wrong match?…') && inOrder(um), `${width}: unmatched song's menu in the shared order (${um.join(' · ')})`);
    assert.ok(um.indexOf('Which song is this?…') > um.indexOf('Remove from library') && um.indexOf('Which song is this?…') < um.indexOf('Copy link'), `${width}: Which song is this?… after Remove, before Copy link`);
    await close(page);

    // 3. An album page: a song row's ⋯ (itemMenu) in the same order and words
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector('#isongs [data-smore="0"]');
    await page.click('#isongs [data-smore="0"]', { force: true });
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
    const item = await menu(page);
    await page.screenshot({ path: path.join(shots, `album-row-${width}.png`) });
    const il = item.map((m) => m.label);
    assert.ok(il.includes('Go to song page') && inOrder(il), `${width}: the album row's menu in the shared order (${il.join(' · ')})`);
    assert.equal(item.find((m) => m.label === 'Copy link')?.icon, 'copy', `${width}: the album row's Copy link has the copy icon`);
    // The actions both menus offer come in the same order
    const both = sl.filter((l) => il.includes(l));
    assert.deepEqual(both, il.filter((l) => sl.includes(l)), `${width}: shared actions in the same order (${both.join(' · ')})`);
    assert.ok(both.length >= 8, `${width}: the two menus share most actions (${both.length})`);
    await close(page);

    // 4. The album's own ⋯: Copy link has the copy icon too
    await page.click('#imore');
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
    assert.equal((await menu(page)).find((m) => m.label === 'Copy link')?.icon, 'copy', `${width}: the album's Copy link has the copy icon`);
    await close(page);

    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
    console.log(`ok   ${width}: ${sl.join(' · ')}`);
  } finally {
    await page.context().close();
  }
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  try {
    for (const w of [1280, 1024, 768]) {
      try { await run(browser, w); } catch (e) { failed++; console.log(`FAIL ${w}: ${e.message}`); }
    }
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} width(s) failed`); process.exit(1); }
  console.log('song menus: all ok');
})();
