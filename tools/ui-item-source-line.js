// An item page's source line ("From Your library") is left out when it would only say what the
// eyebrow already says: a page with nothing else on the line and no link out ("Album in your
// library" over "From Your library", the cover's badge a third time; "Song on TIDAL" over "From
// TIDAL"), and "All about this song →" stays. It stays
// when it carries more: a song also on a working service ("From Your library · also TIDAL"), or a
// page of another service, whose line has its "Open in" link. At 360 and 412 px (a browser and the
// Android app) and 1280 px.
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-item-source-line.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'item-source-line-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

const A = [{ id: 11, name: 'Sefa' }];
const fmt = { container: 'flac', format: { kind: 'pcm', bits: 16, rate_hz: 44100, channels: 2 } };
const tracks = Array.from({ length: 6 }, (_, i) => ({ id: i + 1, title: `Journey part ${i + 1}`, artists: A, album: { id: 7, title: 'Journey' }, duration_ms: 200000 + i * 1000, track_no: i + 1, file: fmt }));
const songs = tracks.map((t) => ({ no: t.track_no, title: t.title, artist: 'Sefa', track_id: t.id, uri: `track:${t.id}`, duration_ms: t.duration_ms, artist_id: 'lib-11', album: 'Journey', album_id: 'lib-7', id: `lib-t${t.id}` }));
const base = { uri: null, target: {}, year: 2021, image: null, plays_from: null, connect: [], reading: false, pending: false, songs: [], total_ms: 0, albums: [], releases: [], versions: [], also_on: [], open_on: [], links: [], tags: [], opened_as: null, plays_note: null, song_uri: null, link: null, about: [] };
const TIDAL_ALSO = { page: 'tidal-41', source: 'tidal', id: '41', label: 'TIDAL · song', url: 'https://tidal.com/track/41', uri: 'tidal:track:41' };
const fixtures = {
  '/library/albums/7': { body: { id: 7, title: 'Journey', year: 2021, artists: A, tracks } },
  '/item/album/lib-7': { body: { ...base, kind: 'album', id: 'lib-7', title: 'Journey', source: 'Your library', source_id: 'library', section: 'albums', target: { album_id: 7 }, artist: { id: 'lib-11', name: 'Sefa' }, songs } },
  '/item/artist/lib-11': { body: { ...base, kind: 'artist', id: 'lib-11', title: 'Sefa', source: 'Your library', source_id: 'library', section: 'artists', target: { artist_id: 11 }, artist: null, songs } },
  '/item/song/lib-t1': { body: { ...base, kind: 'song', id: 'lib-t1', title: 'Journey part 1', source: 'Your library', source_id: 'library', section: 'tracks', target: { track_id: 1 }, artist: { id: 'lib-11', name: 'Sefa' }, album: { id: 'lib-7', name: 'Journey' }, songs: songs.slice(0, 1), song_uri: 'https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69' } },
  '/item/song/tidal-9': { body: { ...base, kind: 'song', id: 'tidal-9', title: 'Nine', source: 'TIDAL', source_id: 'tidal', section: 'tracks', artist: { id: 'tidal-a1', name: 'Sefa' }, songs: [{ no: 1, title: 'Nine', artist: 'Sefa', duration_ms: 1000, uri: 'tidal:track:9', track_id: null, page: null, id: 'tidal-9' }] } },
  '/item/song/lib-t2': { body: { ...base, kind: 'song', id: 'lib-t2', title: 'Journey part 2', source: 'Your library', source_id: 'library', section: 'tracks', target: { track_id: 2 }, artist: { id: 'lib-11', name: 'Sefa' }, album: { id: 'lib-7', name: 'Journey' }, songs: songs.slice(1, 2), also_on: [TIDAL_ALSO] } },
  '/item/album/tidal-5': { body: { ...base, kind: 'album', id: 'tidal-5', title: 'Journey', source: 'TIDAL', source_id: 'tidal', section: 'albums', artist: { id: 'tidal-a1', name: 'Sefa' }, link: 'https://tidal.com/album/5', songs: [{ no: 1, title: 'One', artist: 'Sefa', duration_ms: 1000, uri: 'tidal:track:1', track_id: null, page: null, id: 'tidal-1' }] } },
  '/sources': { body: [{ id: 'tidal', name: 'TIDAL', enabled: true, healthy: true }] },
  '/play-order': { body: { order: ['library', 'tidal'], services: [{ id: 'library', connected: true }, { id: 'tidal', connected: true }] } },
};
for (const k of Object.keys(fixtures).filter((k) => k.startsWith('/item/'))) fixtures[`${k}/extras`] = { body: { related: [], concerts: [], similar: [] } };

const cases = [
  { url: '/album/lib-7', name: 'library album', eyebrow: /^Album in your library/, line: null },
  { url: '/artist/lib-11', name: 'library artist', eyebrow: /^Artist in your library/, line: null },
  { url: '/song/lib-t1', name: 'library song, nowhere else', eyebrow: /^Song in your library/, line: null, global: true },
  { url: '/song/lib-t2', name: 'library song also on TIDAL', eyebrow: /^Song in your library/, line: /^From\s*Your library\s*· also\s*TIDAL/ },
  { url: '/album/tidal-5', name: 'TIDAL album', eyebrow: /^Album on TIDAL/, line: /^From\s*TIDAL/, link: 'https://tidal.com/album/5' },
  { url: '/song/tidal-9', name: 'TIDAL song with no link out', eyebrow: /^Song on TIDAL/, line: null },
];

const read = (page) => page.evaluate(() => {
  const hero = document.querySelector('#main .item-hero'); const line = document.querySelector('#ialso');
  const eb = hero && hero.querySelector('.eyebrow');
  const out = line ? line.querySelector('a.chiplink[target="_blank"]') : null;
  return {
    eyebrow: eb ? eb.textContent.trim() : '',
    line: line ? line.textContent.replace(/\s+/g, ' ').trim() : null, link: out ? out.getAttribute('href') : null,
    global: (document.querySelector('#main .item-hero #iglobal') || {}).textContent || '',
    ov: document.scrollingElement.scrollWidth - innerWidth,
  };
});

(async () => {
  const browser = await chromium.launch(process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {});
  try {
    for (const [width, android] of [[360, false], [412, false], [360, true], [1280, false]]) {
      const shell = `${width}${android ? ' Android' : ''}`;
      const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures: JSON.parse(JSON.stringify(fixtures)) });
      if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
      try {
        for (const c of cases) {
          const at = `${shell} ${c.name}`;
          await fx.go(page, c.url);
          const drawn = await page.waitForFunction(() => { const h = document.querySelector('#main .item-hero h1'); return !!h && h.textContent.trim().length > 0; }, null, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${at}: the page is drawn`);
          if (!drawn) continue;
          await page.waitForTimeout(300);
          const r = await read(page);
          check(c.eyebrow.test(r.eyebrow), `${at}: the eyebrow says where it is from ("${r.eyebrow}")`);
          if (c.line) check(r.line !== null && c.line.test(r.line), `${at}: the source line stays, it says more than the eyebrow ("${r.line}")`);
          else check(r.line === null, `${at}: no source line repeating the eyebrow (${r.line === null ? 'none' : `"${r.line}"`})`);
          // (the link to the song's page with every source stays when the line goes)
          if (c.global) check(/^All about this song/.test(r.global), `${at}: "All about this song →" stays ("${r.global}")`);
          if (c.link) check(r.link === c.link, `${at}: the line keeps its link out (${r.link})`);
          check(r.ov <= 0, `${at}: nothing wider than the screen (${r.ov})`);
          if (c.url === '/album/lib-7' && !android) await page.screenshot({ path: path.join(SHOTS, `album-library-${width}.png`) });
          if (c.url === '/song/lib-t2' && !android) await page.screenshot({ path: path.join(SHOTS, `song-also-${width}.png`) });
        }
        check(!state.errors.length, `${shell}: no page errors ${state.errors.join(' | ')}`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
