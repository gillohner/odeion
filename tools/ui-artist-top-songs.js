// An artist page's Top songs don't repeat the artist under every title: a song credited to the page's
// artist alone shows only its album there (the page's title already says who, as an album page
// says its artist once); a song with other artists too keeps every credit, each its own link.
// Another artist's song on the list keeps its name. Album pages and search results are unchanged.
// At 1280, 1024 and 768 px (and 412 px, a phone), a library artist and a TIDAL artist.
// Real candidate assets, isolated responses: node tools/ui-artist-top-songs.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'artist-top-songs-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let ok = 0;
let fail = 0;
const check = (good, what) => { console.log(`${good ? 'ok  ' : 'FAIL'} ${what}`); if (good) ok++; else fail++; };

const base = { source: 'Your library', source_id: 'library', uri: null, song_uri: null, link: null, plays_from: { service: 'library', label: 'Your library' }, albums: [], versions: [], also_on: [], about: [], open_on: [], connect: [], on_pubky: null };
const album = { album: 'Journey', album_id: 'lib-7' };
// (track 1 is the fixture's library song: "Sefa, Dr. Peacock", both credited)
const libSongs = [
  { id: 'lib-1', track_id: 1, title: 'Journey Through Time', artist: 'Sefa, Dr. Peacock', duration_ms: 240000, ...album },
  { id: 'lib-2', track_id: 2, title: 'Second Song', artist: 'Sefa', duration_ms: 200000, ...album },
  { id: 'lib-3', track_id: 3, title: 'Third Song', artist: 'Sefa', duration_ms: 210000 },
  { id: 'lib-4', track_id: 4, title: 'Fourth Song', artist: 'Sefabeats', duration_ms: 220000, ...album },
  // (rows that carry their credits themselves: Sefa alone, and Sefa with someone)
  { id: 'lib-5', title: 'Fifth Song', artist: 'Sefa', artists: [{ id: 2, name: 'Sefa' }], duration_ms: 230000, ...album },
  { id: 'lib-6', title: 'Sixth Song', artist: 'Sefa, Friend', artists: [{ id: 2, name: 'Sefa' }, { name: 'Friend' }], duration_ms: 230000, ...album },
];
const tidalSongs = [
  { id: 'tidal-t1', uri: 'tidal:track:1', title: 'Tidal One', artist: 'Sefa', duration_ms: 200000, album: 'Tidal Album' },
  { id: 'tidal-t2', uri: 'tidal:track:2', title: 'Tidal Two', artist: 'Sefa & Friend', duration_ms: 200000, album: 'Tidal Album' },
];
const fixtures = {
  '/item/artist/lib-2': { body: { ...base, kind: 'artist', id: 'lib-2', title: 'Sefa', section: 'artists', target: { artist_id: 2 }, songs: libSongs } },
  '/item/artist/tidal-a9': { body: { ...base, source: 'TIDAL', source_id: 'tidal', plays_from: { service: 'tidal', label: 'TIDAL' }, kind: 'artist', id: 'tidal-a9', title: 'Sefa', section: 'artists', target: {}, songs: tidalSongs } },
  '/item/album/lib-7': { body: { ...base, kind: 'album', id: 'lib-7', title: 'Journey', section: 'albums', target: { album_id: 7 }, artist: { id: 'lib-2', name: 'Sefa' }, songs: libSongs.slice(1, 3) } },
  '/item/artist/lib-2/extras': { body: {} }, '/item/artist/tidal-a9/extras': { body: {} }, '/item/album/lib-7/extras': { body: {} },
  '/reviews': { body: { reviews: [] } }, '/tags': { body: { tags: [], uri: null } }, '/comments': { body: { comments: [], indexed: true } },
};
// Each row's second line: its words, and the artist links in it
const subs = (page) => page.$$eval('#isongs .srow', (rows) => rows.map((r) => {
  const sub = r.querySelector('.sub');
  return { title: (r.querySelector('.name') || {}).textContent.trim(), sub: sub ? sub.textContent.replace(/\s+/g, ' ').trim() : '', artists: sub ? [...sub.querySelectorAll('[data-artist-credit]')].map((a) => a.textContent.trim()) : [] };
}));

async function open(browser, width, route) {
  const { page, context, state } = await fx.create(browser, width, width < 600 ? { mobile: true, fixtures: { ...fixtures } } : { fixtures: { ...fixtures } });
  await fx.go(page, route);
  await page.waitForSelector('#isongs .srow');
  // (track 1's credits are read after the page draws: wait for its two links)
  if (route === '/artist/lib-2') await page.waitForFunction(() => [...document.querySelectorAll('#isongs .srow')].some((r) => r.querySelectorAll('[data-artist-credit]').length === 2), null, { timeout: 5000 }).catch(() => {});
  return { page, context, state };
}

async function run(browser, width) {
  const tag = `${width}`;
  {
    const { page, context, state } = await open(browser, width, '/artist/lib-2');
    const rows = await subs(page);
    const by = (t) => rows.find((r) => r.title === t) || { sub: '(missing)', artists: [] };
    check(by('Second Song').sub === 'Journey', `${tag}: library artist: a song by Sefa alone shows only its album (${JSON.stringify(by('Second Song').sub)})`);
    check(by('Third Song').sub === '', `${tag}: library artist: a song by Sefa alone with no album shows nothing under it (${JSON.stringify(by('Third Song').sub)})`);
    check(by('Journey Through Time').artists.join(',') === 'Sefa,Dr. Peacock' && /Journey$/.test(by('Journey Through Time').sub), `${tag}: library artist: a song with a featured artist keeps both credits and its album (${JSON.stringify(by('Journey Through Time'))})`);
    check(by('Fourth Song').artists.join(',') === 'Sefabeats', `${tag}: library artist: another artist's song keeps its name (${JSON.stringify(by('Fourth Song'))})`);
    check(by('Fifth Song').sub === 'Journey', `${tag}: library artist: a row credited [Sefa] shows only its album (${JSON.stringify(by('Fifth Song').sub)})`);
    check(by('Sixth Song').artists.join(',') === 'Sefa,Friend', `${tag}: library artist: a row credited [Sefa, Friend] keeps both (${JSON.stringify(by('Sixth Song'))})`);
    await page.screenshot({ path: path.join(SHOTS, `after-artist-top-songs-${width}.png`) });
    try { await fx.cleanCopy(page); check(true, `${tag}: library artist: clean text, nothing wider than the screen`); } catch (e) { check(false, `${tag}: library artist: ${e.message}`); }
    check(state.errors.length === 0, `${tag}: library artist: no page errors ${state.errors.join('; ')}`);
    await context.close();
  }
  {
    const { page, context, state } = await open(browser, width, '/artist/tidal-a9');
    const rows = await subs(page);
    const by = (t) => rows.find((r) => r.title === t) || { sub: '(missing)', artists: [] };
    check(by('Tidal One').sub === 'Tidal Album', `${tag}: TIDAL artist: a song by Sefa alone shows only its album (${JSON.stringify(by('Tidal One').sub)})`);
    check(by('Tidal Two').artists.join(',') === 'Sefa & Friend', `${tag}: TIDAL artist: "Sefa & Friend" keeps its credit (${JSON.stringify(by('Tidal Two'))})`);
    check(state.errors.length === 0, `${tag}: TIDAL artist: no page errors ${state.errors.join('; ')}`);
    await context.close();
  }
  {
    const { page, context, state } = await open(browser, width, '/album/lib-7');
    const rows = await subs(page);
    check(rows.length === 2 && rows.every((r) => r.artists.length === 0), `${tag}: album page: its songs, all by its artist, still don't name the artist on every row (${JSON.stringify(rows)})`);
    check(state.errors.length === 0, `${tag}: album page: no page errors ${state.errors.join('; ')}`);
    await context.close();
  }
  {
    // Search's song results are drawn by the same rows: there a song by Sefa alone still names Sefa
    const searchHits = [{ kind: 'song', id: 'lib-2', track_id: 2, title: 'Second Song', subtitle: 'Sefa', album: 'Journey', sources: ['library'] }];
    const { page, context, state } = await fx.create(browser, width, width < 600 ? { mobile: true, searchHits, fixtures: { ...fixtures } } : { searchHits, fixtures: { ...fixtures } });
    await fx.go(page, '/search?q=sefa');
    await page.waitForSelector('.srow', { timeout: 5000 }).catch(() => {});
    const names = await page.$$eval('.srow [data-artist-credit]', (els) => els.map((e) => e.textContent.trim()));
    check(names.includes('Sefa'), `${tag}: search results: a song by Sefa alone still names Sefa (${names.join(',')})`);
    check(state.errors.length === 0, `${tag}: search results: no page errors ${state.errors.join('; ')}`);
    await context.close();
  }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768, 412]) await run(browser, w); } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
