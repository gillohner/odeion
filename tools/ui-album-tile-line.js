// An album tile's line under its title says who and when, with " · " only between two things:
// an album with no album artist (untagged files, or the field cleared in Edit tags) reads "2020",
// not "· 2020", and one with an artist but no year reads just the artist.
// Albums page at phone widths (and the Android app) and desktop.
// Real candidate assets, isolated responses: node tools/ui-album-tile-line.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'album-tile-line-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const ALBUMS = [
  { id: 7, title: 'Both', album_artist: 'Sefa', year: 2020, tracks: 1 },
  { id: 8, title: 'No artist', album_artist: null, year: 2019, tracks: 1 },
  { id: 9, title: 'No year', album_artist: 'Dr. Peacock', year: null, tracks: 1 },
  { id: 10, title: 'Neither', album_artist: null, year: null, tracks: 1 },
];
const WANT = { 7: 'Sefa · 2020', 8: '2019', 9: 'Dr. Peacock', 10: '' };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, false], [412, true], [1280, false]]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/library/albums': { body: ALBUMS } } });
      if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
      try {
        await fx.go(page, '/albums');
        await page.waitForSelector('#main .tile[data-album]', { timeout: 5000 }).catch(() => {});
        const lines = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll('#main .tile[data-album]')].map((t) => {
          const a = t.querySelector('.a'); const c = a.cloneNode(true); c.querySelectorAll('.fmt').forEach((f) => f.remove());
          return [t.dataset.album, c.textContent.trim()];
        })));
        check(Object.keys(lines).length === ALBUMS.length, `${at}: ${ALBUMS.length} album tiles (${Object.keys(lines).length})`);
        for (const a of ALBUMS) check(lines[a.id] === WANT[a.id], `${at}: "${a.title}" tile line reads "${WANT[a.id]}" ("${lines[a.id]}")`);
        check(Object.values(lines).every((l) => !/^\s*·|·\s*$/.test(l)), `${at}: no tile line starts or ends with " · "`);
        check(state.errors.length === 0, `${at}: no page errors (${state.errors.join('; ')})`);
        await page.screenshot({ path: path.join(SHOTS, `albums-${at}.png`) });
        await fx.cleanCopy(page).then(() => check(true, `${at}: clean copy, no horizontal overflow`), (e) => check(false, `${at}: ${e.message}`));
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
