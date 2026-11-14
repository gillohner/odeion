// After a pick in Add to playlist…, the toast names the playlist the songs went to (as a new one's
// "Created “…”" does), so with many playlists you can tell which row the tap hit. Checked from a
// Songs row's ⋯ (one song), an album page's song row ⋯ (one song) and the album's own ⋯ (all its
// songs), each against a second playlist so the name can't be a default. Also: nothing else is
// said in the toast's place (no "Added to playlist"), and the "stays on this node" note counts
// only the songs not matched yet, from an album page too (its songs carry their page id, lib-<n>
// until matched). The same note follows "Created “…”" when the songs go into a new playlist
// (Add to playlist… › New playlist), from a Songs row and an album's ⋯.
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
// (as the node sends them: a matched song's id is its MusicBrainz page, item.rs track_page_id)
const song = (t, no) => { const id = t.mbid ? `mb-${t.mbid}` : `lib-${t.id}`; return { no, title: t.title, artist: 'Sefa', artist_id: 'lib-11', album_id: 'lib-7', duration_ms: 240000, uri: null, track_id: t.id, id, page: null }; };
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

// Add to playlist… › New playlist (its default name), then the toast and what was sent
const createNew = async (page, state, open, what, width) => {
  state.collections = [quiet, road];
  const before = state.hits.length;
  await open();
  await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to playlist…' }).click();
  await page.waitForSelector('dialog #pplist [data-ppnew]');
  await page.evaluate(() => { const t = document.querySelector('#toast'); t.classList.remove('show'); t.textContent = ''; });
  await page.click('dialog #pplist [data-ppnew]');
  await page.waitForFunction(() => document.querySelector('#toast').classList.contains('show') && /^Created/.test(document.querySelector('#toast').textContent.trim()));
  const said = (await page.locator('#toast').textContent()).trim();
  const sent = state.hits.slice(before).filter((h) => h.m === 'POST' && h.v === '/playlists');
  assert.equal(sent.length, 1, `${width} ${what}: one new playlist made`);
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

    // 2. An album page: a song row's ⋯ (itemMenu, one song), the matched one, then the other
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector('#isongs [data-smore="1"]');
    const b = await toastAfterPick(page, state, () => page.click('#isongs [data-smore="0"]', { force: true }), 'album song row (matched)', width);
    assert.equal(b.said, 'Added to “Quiet evenings”', `${width} album song row, matched: the toast names the playlist and adds no note (${b.said})`);
    assert.deepEqual(b.body, { track_ids: [1] }, `${width} album song row, matched: the song sent (${JSON.stringify(b.body)})`);
    const b2 = await toastAfterPick(page, state, () => page.click('#isongs [data-smore="1"]', { force: true }), 'album song row (not matched)', width);
    assert.equal(b2.said, 'Added to “Quiet evenings”. It stays on this node until you pick which song it is (the ?).', `${width} album song row, not matched: the note is said (${b2.said})`);

    // 3. The album's own ⋯: both songs
    const c = await toastAfterPick(page, state, () => page.click('#imore'), 'album ⋯', width);
    await page.screenshot({ path: path.join(shots, `after-album-${width}.png`) });
    assert.equal(c.said, '2 songs added to “Quiet evenings”. 1 stays on this node until you pick which song it is (the ?).', `${width} album ⋯: how many, where, and only the unmatched one in the note (${c.said})`);
    assert.deepEqual(c.body, { track_ids: [1, 2] }, `${width} album ⋯: both songs sent (${JSON.stringify(c.body)})`);
    assert.ok(![a.said, b.said, b2.said, c.said].some((s) => /added to playlist/i.test(s)), `${width}: no toast says just "playlist"`);

    // 4. New playlist: "Created “…”" and the same note. (The fixture's new playlist is "Music
    // and notes", the name a Songs row's toast takes from the answer.)
    const made = await createNew(page, state, async () => { await fx.go(page, '/tracks'); await page.waitForSelector('#tracklist .track'); const row = page.locator('#tracklist .track', { hasText: 'Second Song' }); await row.hover(); await row.locator('[data-act="more"]').click({ force: true }); }, 'Songs row new (not matched)', width);
    assert.equal(made.said, 'Created “Music and notes”. It stays on this node until you pick which song it is (the ?).', `${width} Songs row, new playlist, not matched: the note follows Created (${made.said})`);
    assert.deepEqual(made.body.track_ids, [2], `${width} Songs row, new playlist: the song sent (${JSON.stringify(made.body)})`);
    const made1 = await createNew(page, state, async () => { const row = page.locator('#tracklist .track', { hasText: fx.track.title }).first(); await row.hover(); await row.locator('[data-act="more"]').click({ force: true }); }, 'Songs row new (matched)', width);
    assert.equal(made1.said, 'Created “Music and notes”', `${width} Songs row, new playlist, matched: no note (${made1.said})`);
    const madeAlbum = await createNew(page, state, async () => { await fx.go(page, '/album/lib-7'); await page.waitForSelector('#imore'); await page.click('#imore'); }, 'album ⋯ new', width);
    await page.screenshot({ path: path.join(shots, `after-album-new-${width}.png`) });
    assert.equal(madeAlbum.said, 'Created “Journey”. 1 stays on this node until you pick which song it is (the ?).', `${width} album ⋯, new playlist: only the unmatched one in the note (${madeAlbum.said})`);
    assert.deepEqual(madeAlbum.body, { name: 'Journey', track_ids: [1, 2] }, `${width} album ⋯, new playlist: both songs sent (${JSON.stringify(madeAlbum.body)})`);

    assert.deepEqual(state.errors, [], `${width}: no page errors`);
    console.log(`ok   ${width}: ${a.said} | ${b.said} | ${b2.said} | ${c.said} | ${made.said} | ${made1.said} | ${madeAlbum.said}`);
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
