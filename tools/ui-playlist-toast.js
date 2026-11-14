// After a pick in Add to playlist…, the toast names the playlist the songs went to (as a new one's
// "Created “…”" does), so with many playlists you can tell which row the tap hit. Checked from a
// Songs row's ⋯ (one song), an album page's song row ⋯ (one song) and the album's own ⋯ (all its
// songs), each against a second playlist so the name can't be a default. Also: nothing else is
// said in the toast's place (no "Added to playlist").
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-playlist-toast.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/playlist-toast-shots');
fs.mkdirSync(shots, { recursive: true });

// A matched song (it has a canonical id) and one not matched yet
const one = { ...fx.track, canonical: [{ kind: 'recording', mbid: fx.track.mbid }] };
const two = { ...fx.track, id: 2, title: 'Second Song', mbid: null, canonical_uri: null, canonical: [] };
const song = (t, no) => ({ no, title: t.title, artist: 'Sefa', artist_id: 'lib-11', album_id: 'lib-7', duration_ms: 240000, uri: `track:${t.id}`, track_id: t.id, id: `lib-${t.id}`, page: `lib-${t.id}` });
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' },
  songs: [song(one, 1), song(two, 2)], total_ms: 480000, albums: [], versions: [], also_on: [], about: [], open_in: [] };
// Two playlists: the newer one is listed first, the pick is the second row
const road = { ...fx.summary, id: 'p-road', name: 'Road trip', items: 4, updated_at: 20 };
const quiet = { ...fx.summary, id: 'p-quiet', name: 'Quiet evenings', items: 9, updated_at: 10 };
const fixtures = {
  '/library/tracks': { body: { tracks: [one, two], next: null } },
  '/library/tracks/1': { body: one },
  '/library/tracks/2': { body: two },
  '/item/album/lib-7': { body: albumPage },
  '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
  '/playlists/p-quiet/items': { body: { ok: true } },
};

const toastAfterPick = async (page, state, open, what, width) => {
  const before = state.hits.length;
  await open();
  await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to playlist…' }).click();
  await page.waitForSelector('dialog #pplist [data-ppi="1"]');
  const rows = await page.locator('dialog #pplist [data-ppi] .name').allTextContents();
  assert.deepEqual(rows, ['Road trip', 'Quiet evenings'], `${width} ${what}: the picker lists both playlists (${rows.join(' · ')})`);
  // (the last check's toast away first, so this one is read)
  await page.evaluate(() => { const t = document.querySelector('#toast'); t.classList.remove('show'); t.textContent = ''; });
  await page.click('dialog #pplist [data-ppi="1"]');
  await page.waitForFunction(() => document.querySelector('#toast').classList.contains('show') && /added to/i.test(document.querySelector('#toast').textContent));
  const said = (await page.locator('#toast').textContent()).trim();
  const sent = state.hits.slice(before).filter((h) => h.m === 'POST' && h.v === '/playlists/p-quiet/items');
  assert.equal(sent.length, 1, `${width} ${what}: one add sent to the playlist picked`);
  return { said, body: JSON.parse(sent[0].body || '{}') };
};

async function run(browser, width) {
  const { page, state } = await fx.create(browser, width, { fixtures, collections: [quiet, road] });
  try {
    // 1. Songs: a row's ⋯ (trackMenu)
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    await page.locator('#tracklist .track').first().hover();
    const a = await toastAfterPick(page, state, () => page.click('#tracklist .track:first-child [data-act="more"]', { force: true }), 'Songs row', width);
    await page.screenshot({ path: path.join(shots, `after-songs-row-${width}.png`) });
    assert.equal(a.said, 'Added to “Quiet evenings”', `${width} Songs row: the toast names the playlist (${a.said})`);
    assert.deepEqual(a.body, { track_ids: [1] }, `${width} Songs row: the song sent (${JSON.stringify(a.body)})`);

    // 2. An album page: a song row's ⋯ (itemMenu, one song)
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector('#isongs [data-smore="1"]');
    const b = await toastAfterPick(page, state, () => page.click('#isongs [data-smore="1"]', { force: true }), 'album song row', width);
    // (an album page passes its songs by track id only, so the "stays on this node" note follows
    // the name for any song there, matched or not: as on main, not what is checked here)
    assert.ok(/^Added to “Quiet evenings”(\.|$)/.test(b.said), `${width} album song row: the toast names the playlist (${b.said})`);

    // 3. The album's own ⋯: both songs
    const c = await toastAfterPick(page, state, () => page.click('#imore'), 'album ⋯', width);
    await page.screenshot({ path: path.join(shots, `after-album-${width}.png`) });
    assert.ok(/^2 songs added to “Quiet evenings”(\.|$)/.test(c.said), `${width} album ⋯: the toast says how many and where (${c.said})`);
    assert.ok(![a.said, b.said, c.said].some((s) => /added to playlist/i.test(s)), `${width}: no toast says just "playlist"`);

    assert.deepEqual(state.errors, [], `${width}: no page errors`);
    console.log(`ok   ${width}: ${a.said} | ${b.said} | ${c.said}`);
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
  console.log('playlist toast: all ok');
})();
