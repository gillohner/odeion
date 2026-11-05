// An item page's Similar › "Tagged alike on Pubky" tiles open and play what they show
// (TASK-similar-tiles-click-1105; owner, 2026-11-05: the tiles did nothing on click). Real candidate
// assets on the fixture origin (tools/ui-fixture.js), the page's extras listing a song, an album, an
// artist and a podcast tagged alike, between a coin, a paper and a place:
//   * only the music shows, in order (by the item's own kind, not its tags);
//   * each tile opens its own item (the song's page for a track, the podcast's site), ▶ plays its own;
//   * the page's own "In your library" tiles still open theirs;
// on an album, an artist and a song page, at 390 (phone) and 1280 px.
//   node tools/ui-similar-tiles.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'similar-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const MB = 'https://musicbrainz.org';
const U = { song: '11111111-1111-4111-8111-111111111111', album: '22222222-2222-4222-8222-222222222222', artist: '33333333-3333-4333-8333-333333333333' };
const related = [
  { kind: 'other', title: 'Bitcoin', uri: 'https://bitcoin.org/', page: 'https://bitcoin.org/' },
  { kind: 'track', title: 'Tagged Song', artist: 'Someone', uri: `${MB}/recording/${U.song}`, page: `${MB}/recording/${U.song}` },
  { kind: 'link', title: 'A science paper', uri: 'https://arxiv.org/abs/2401.00001', page: 'https://arxiv.org/abs/2401.00001' },
  { kind: 'album', title: 'Tagged Album', artist: 'Someone', uri: `${MB}/release-group/${U.album}`, page: `${MB}/release-group/${U.album}` },
  { kind: 'place', title: '47.37, 8.54', uri: 'geo:47.37,8.54', page: 'geo:47.37,8.54' },
  { kind: 'artist', title: 'Tagged Artist', uri: `${MB}/artist/${U.artist}`, page: `${MB}/artist/${U.artist}` },
  { kind: 'podcast', title: 'Tagged Podcast', uri: 'https://example.org/podcast', page: 'https://example.org/podcast' },
];
const SHOWN = ['Tagged Song', 'Tagged Album', 'Tagged Artist', 'Tagged Podcast'];
// Where each shown tile goes: an item path, or the site it opens
const WANT = [`/song/mb-${U.song}`, `/album/mb-${U.album}`, `/artist/mb-${U.artist}`, 'web:https://example.org/podcast'];
const mine = [{ id: 'lib-9', kind: 'album', title: 'Library Neighbour', subtitle: '2001', image: null }];
const page = (kind) => ({ kind, id: 'lib-7', uri: null, target: kind === 'artist' ? { artist_id: 11 } : kind === 'song' ? { track_id: 101 } : { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [{ no: 1, title: 'Opening Song', artist: 'Sefa', artist_id: 'lib-11', duration_ms: 200000, uri: null, track_id: 101, id: 'lib-101', page: 'lib-101' }], total_ms: 200000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' });
const fixtures = {};
for (const k of ['album', 'artist', 'song']) {
  fixtures[`/item/${k}/lib-7`] = { body: page(k) };
  fixtures[`/item/${k}/lib-7/extras`] = { body: { collections: [], related, concerts: [], similar: mine } };
}
fixtures['/library/artists/11/similar'] = { body: { artists: [] } };
const open = async (browser, w, kind) => {
  const run = await fx.create(browser, w, { fixtures, touch: w < 600 });
  await run.page.setViewportSize({ width: w, height: 900 });
  await run.page.addInitScript(() => { window.__opened = []; window.open = (u) => { window.__opened.push(String(u)); return null; }; });
  await fx.go(run.page, `/${kind}/lib-7`); await run.page.waitForSelector('#irelated [data-tile]', { timeout: 15000 }).catch(() => {});
  await sleep(300);
  return run;
};
const where = (p) => p.evaluate(() => (window.__opened.length ? `web:${window.__opened[window.__opened.length - 1].replace(/\/$/, '')}` : location.pathname));

(async () => {
  const browser = await chromium.launch();
  for (const w of [390, 1280]) {
    for (const kind of ['album', 'artist', 'song']) {
      const at = `${kind} page at ${w}`;
      let run = await open(browser, w, kind);
      const titles = await run.page.$$eval('#irelated [data-tile] .t', (ts) => ts.map((t) => t.textContent.trim()));
      check(JSON.stringify(titles) === JSON.stringify(SHOWN), `${at}: only the music tagged alike shows, in order (${titles.join(', ')})`);
      await run.page.locator('#irelated').scrollIntoViewIfNeeded().catch(() => {});
      await run.page.screenshot({ path: path.join(SHOTS, `${kind}-${w}.png`), fullPage: false });
      // Each tile opens its own item
      for (let k = 0; k < SHOWN.length; k++) {
        if (k) { await run.context.close(); run = await open(browser, w, kind); }
        await run.page.locator(`#irelated [data-tile="${k}"]`).click({ position: { x: 10, y: 10 } }); await sleep(500);
        const got = await where(run.page);
        check(got === WANT[k], `${at}: "${SHOWN[k]}" opens ${WANT[k]} (${got})`);
      }
      // ▶ plays its own item (asks the node how to play that one)
      await run.context.close(); run = await open(browser, w, kind);
      const n0 = run.state.hits.length;
      await run.page.locator('#irelated [data-tileplay="1"]').click({ force: true }); await sleep(600);
      const asked = run.state.hits.slice(n0).filter((h) => h.v === '/discover/play').map((h) => new URLSearchParams(h.q).get('uri'));
      check(asked.length === 1 && asked[0] === related[3].page, `${at}: ▶ on "${SHOWN[1]}" plays that album (${asked.join(', ') || 'nothing asked'})`);
      check(await run.page.evaluate(() => location.pathname) === `/${kind}/lib-7`, `${at}: ▶ stays on the page`);
      // The page's own library tiles still open theirs (a song page lists songs there instead)
      if (kind !== 'song') {
        await run.context.close(); run = await open(browser, w, kind);
        await run.page.locator('#isimlib [data-tile="0"]').click({ position: { x: 10, y: 10 } }); await sleep(500);
        const got = await where(run.page);
        check(got === `/${kind}/lib-9`, `${at}: "In your library" still opens its own tile (${got})`);
      }
      check(run.state.errors.length === 0, `${at}: no page errors${run.state.errors.length ? ': ' + run.state.errors.join(' | ') : ''}`);
      await run.context.close();
    }
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
