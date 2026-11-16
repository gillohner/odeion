// Home's album rows (Recently played, Recently added) give a library album the same line under its
// title as the Albums page: "Sefa · 2020", "2019" with no album artist, "Dr. Peacock" with no year.
// Home said only the artist ("Sefa"), and the year only when there was no artist, so the same album
// read two ways on two pages. A tile with no line at all ("Neither") keeps the line's height, so its
// cover lines up with its row's (a tile is a button, which centres its content: its cover sat 8-9 px lower).
// Home and Albums at phone widths (and the Android app) and desktop.
// Real candidate assets, isolated responses: node tools/ui-home-album-line.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'home-album-line-shots'));
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
const fixtures = {
  '/library/albums': { body: ALBUMS },
  '/library/home': { body: { recently_played: ALBUMS, recently_added: [...ALBUMS].reverse(), forgotten: [] } },
  '/listenbrainz': { body: { connected: false } },
  '/me/first-steps': { body: { show: false, hidden: true, steps: [] } },
  '/library/folders/suggest': { body: [] },
  ...Object.fromEntries(['following', 'tags', 'concerts', 'podcasts', 'yours', 'new', 'trending-songs', 'trending-albums', 'trending-artists'].map((n) => [`/home/${n}`, { body: { section: n, items: [], tags: [] } }])),
};
// Each tile's line under its title, by album title (the Albums page's format badge left out)
const lines = (page, sel) => page.evaluate((s) => Object.fromEntries([...document.querySelectorAll(s)].map((t) => {
  const a = t.querySelector('.a'); const c = a.cloneNode(true); c.querySelectorAll('.fmt').forEach((f) => f.remove());
  return [t.querySelector('.t').textContent.trim(), c.textContent.trim()];
})), sel);

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, false], [412, true], [1280, false]]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures });
      if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
      try {
        await fx.go(page, '/');
        // Home can draw itself again just after its first answer (the zones' snapshot): wait until it settles
        const filled = () => page.waitForFunction(() => ['played', 'added'].every((k) => document.querySelectorAll(`#h-${k} .hbody .tile[data-tile]`).length === 4 && !document.querySelector(`#h-${k} .skeleton`)), null, { timeout: 5000 }).catch(() => {});
        await filled(); await page.waitForTimeout(600); await filled();
        for (const k of ['played', 'added']) {
          const got = await lines(page, `#h-${k} .hbody .tile`);
          check(Object.keys(got).length === ALBUMS.length, `${at}: Home ${k}: ${ALBUMS.length} album tiles (${Object.keys(got).length})`);
          for (const a of ALBUMS) check(got[a.title] === WANT[a.id], `${at}: Home ${k}: "${a.title}" reads "${WANT[a.id]}" ("${got[a.title]}")`);
        }
        // The line stays on one line inside its tile (ellipsis), at every width
        const fit = await page.evaluate(() => [...document.querySelectorAll('#h-played .hbody .tile .a')].every((a) => a.getBoundingClientRect().height < 24 && a.getBoundingClientRect().right <= a.closest('.tile').getBoundingClientRect().right + 0.5));
        check(fit, `${at}: Home tile lines stay one line inside their tiles`);
        // A tile with no line under its title is as tall as the others: its cover lines up with its row's
        const tops = await page.evaluate(() => ['played', 'added'].map((k) => {
          const rows = {}; for (const t of document.querySelectorAll(`#h-${k} .hbody .tile`)) { const r = t.getBoundingClientRect(); (rows[Math.round(r.top)] ||= []).push(Math.round(t.querySelector('.art').getBoundingClientRect().top)); }
          return Object.values(rows).map((r) => Math.max(...r) - Math.min(...r));
        }).flat());
        check(tops.length > 0 && tops.every((d) => d <= 1), `${at}: Home covers line up in each row, "Neither" too (off by ${tops.join(', ')} px)`);
        await page.locator('#h-played').scrollIntoViewIfNeeded().catch(() => {});
        await page.screenshot({ path: path.join(SHOTS, `home-${at}.png`) });
        await fx.cleanCopy(page).then(() => check(true, `${at}: Home clean copy, no horizontal overflow`), (e) => check(false, `${at}: Home ${e.message}`));

        // The same album reads the same on the Albums page
        await fx.go(page, '/albums');
        await page.waitForSelector('#main .tile[data-album]', { timeout: 5000 }).catch(() => {});
        const albums = await lines(page, '#main .tile[data-album]');
        for (const a of ALBUMS) check(albums[a.title] === WANT[a.id], `${at}: Albums: "${a.title}" reads "${WANT[a.id]}" too ("${albums[a.title]}")`);
        const atops = await page.evaluate(() => { const rows = {}; for (const t of document.querySelectorAll('#main .tile[data-album]')) { const r = t.getBoundingClientRect(); (rows[Math.round(r.top)] ||= []).push(Math.round(t.querySelector('.art').getBoundingClientRect().top)); } return Object.values(rows).map((r) => Math.max(...r) - Math.min(...r)); });
        check(atops.length > 0 && atops.every((d) => d <= 1), `${at}: Albums covers line up in each row (off by ${atops.join(', ')} px)`);
        await page.screenshot({ path: path.join(SHOTS, `albums-${at}.png`) });
        check(state.errors.length === 0, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
