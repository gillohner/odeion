// A collection whose songs are all "on this node only" (not identified yet, so not on the
// homeserver) lists them in its "On this node only" section; the Songs card above them must not
// say "This collection is empty" under a head that counts those same songs. A collection with no songs
// at all still says it is empty, and one with both kinds keeps its Songs card.
// Real UI in Chromium against fixtures (no node), 1280, 1024 and 768 px, plus a 412 px phone.
//   node tools/ui-playlist-local-only.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-local-only-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const local = [1, 2, 3].map((k) => ({ ...fx.track, id: 100 + k, title: `Unknown tape ${k}`, mbid: null, canonical_uri: null, duration_ms: 180000 }));
const pfOf = (tracks) => ({ order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false,
  items: tracks.map((t, k) => ({ index: k, uri: t.canonical_uri, title: t.title, artist: 'Sefa', track_id: t.id, plays_from: { service: 'library', label: 'Your library' }, sources: [] })) });
const listOf = (id, tracks, localTracks) => {
  const summary = { ...fx.summary, id, name: `List ${id}`, items: tracks.length + localTracks.length };
  return { summary, fixtures: {
    [`/playlists/${id}`]: { body: { summary, playlist: { ...summary, items: tracks.map((t) => ({ uri: t.canonical_uri })) }, tracks, entries: tracks.map((t) => ({ kind: 'track', uri: t.canonical_uri })), local_tracks: localTracks } },
    [`/playlists/${id}/play-from`]: { body: pfOf(tracks) },
  } };
};
const cases = [
  { id: 'lo', want: 'local', ...listOf('lo', [], local) },
  { id: 'mx', want: 'both', ...listOf('mx', [fx.track], local) },
  { id: 'em', want: 'empty', ...listOf('em', [], []) },
];
const fixtures = Object.assign({ '/playlists': { body: cases.map((c) => c.summary) } }, ...cases.map((c) => c.fixtures));

const look = () => {
  const vis = (el) => !!el && el.getClientRects().length > 0;
  const songs = document.querySelector('#plsongs'); const lo = document.querySelector('#pllocal');
  return {
    sub: (document.querySelector('.hero .sub') || {}).textContent || '',
    songsShown: vis(songs),
    emptyShown: [...document.querySelectorAll('main .empty, main [class*="empty"]')].some((e) => vis(e) && /collection is empty/i.test(e.textContent)),
    emptyAnywhere: /This collection is empty/.test(document.querySelector('main').innerText),
    songRows: songs ? [...songs.querySelectorAll('.srow')].filter(vis).length : 0,
    localShown: vis(lo),
    localRows: lo ? [...lo.querySelectorAll('.srow')].filter(vis).length : 0,
    overflow: document.documentElement.scrollWidth > innerWidth,
  };
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, phone] of [[1280, false], [1024, false], [768, false], [412, true]]) {
      const { context, page } = await fx.create(browser, width, { fixtures, collections: cases.map((c) => c.summary), ...(phone ? { mobile: true, touch: true } : {}) });
      const errors = []; page.on('pageerror', (e) => errors.push(String(e)));
      for (const c of cases) {
        await fx.go(page, `/playlist/${c.id}`);
        await page.waitForFunction((id) => (document.querySelector('main h1') || {}).textContent === `List ${id}`, c.id, { timeout: 7000 }).catch(() => {});
        await page.waitForFunction(() => !document.querySelector('#plsongs .skeleton, #plsongs .chip-sk'), null, { timeout: 4000 }).catch(() => {});
        const tag = `${width} ${c.want}`; const s = await page.evaluate(look);
        if (c.want === 'local') {
          check(/3 songs/.test(s.sub), `${tag}: the head counts the 3 songs ("${s.sub.trim()}")`);
          check(!s.emptyAnywhere, `${tag}: no "This collection is empty" over the songs on this node`);
          check(!s.songsShown, `${tag}: no empty Songs card above "On this node only"`);
          check(s.localShown && s.localRows === 3, `${tag}: "On this node only" lists the 3 songs (${s.localRows})`);
          await page.screenshot({ path: path.join(SHOTS, `${width}-local-only.png`), fullPage: true });
        } else if (c.want === 'both') {
          check(s.songsShown && s.songRows === 1, `${tag}: the Songs card lists the identified song (${s.songRows})`);
          check(s.localShown && s.localRows === 3, `${tag}: "On this node only" lists the other 3 (${s.localRows})`);
          check(!s.emptyAnywhere, `${tag}: nothing says empty`);
        } else {
          check(s.songsShown && s.emptyShown, `${tag}: a collection with no songs still says "This collection is empty"`);
          check(!s.localShown, `${tag}: no "On this node only" section`);
        }
        check(!s.overflow, `${tag}: no sideways scroll`);
      }
      check(!errors.length, `${width}: no page errors${errors.length ? ` (${errors.join('; ')})` : ''}`);
      await context.close();
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
