// On a phone a library song row (Songs, Favourites, a genre's songs) is one tap target, like the song
// rows of playlists, search and item pages: a tap on its name or its artist plays from there instead of
// opening a page. The song, artist and album pages stay in the row's ⋯ (Go to song page). Desktop and a
// tablet's touch screen keep the names as links.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-phone-song-rows.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/phone-song-rows-shots');
fs.mkdirSync(shots, { recursive: true });
const tracks = Array.from({ length: 4 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, canonical_uri: `${fx.SONG}?n=${i + 1}` }));

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const path0 = (page) => new URL(page.url()).pathname;
// the song the app's player loaded (a fixture's play() is a no-op, so the bar stays at "Starting on…")
const playing = (page, id) => page.waitForFunction((n) => [...document.querySelectorAll('audio, video')].some((a) => a.src.includes(`/library/tracks/${n}/file`)), id, { timeout: 4000 }).then(() => true, () => false);
const tapAt = async (page, loc, touch) => {
  const b = await loc.boundingBox(); const x = b.x + Math.min(b.width / 2, 20); const y = b.y + b.height / 2;
  if (touch) await page.touchscreen.tap(x, y); else await page.mouse.click(x, y);
};
// The page each list is on, and the row's names in it
const PAGES = [
  { at: '/tracks', name: 'Songs', ready: '.tracks[data-list="all"] .track' },
  { at: '/favourites', name: 'Favourites', ready: '.tracks[data-list="fav"] .track', opts: { favs: [fx.SONG] } },
];

async function phoneCase(browser, width, { android = false, touch = true } = {}) {
  const label = `${width}${android ? ' android' : ''}${touch ? '' : ' mouse'}`;
  for (const pg of PAGES) {
    const run = await fx.create(browser, width, { mobile: touch, tracks, ...(pg.opts || {}) });
    const { context, page, state } = run;
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    try {
      await fx.go(page, pg.at);
      await page.waitForFunction(() => document.documentElement.dataset.shell === 'phone');
      const row = page.locator(pg.ready).nth(pg.at === '/favourites' ? 0 : 1);
      await row.waitFor();
      const want = await row.getAttribute('data-tid');
      // the song's name
      await tapAt(page, row.locator('.title [data-act="song"]'), touch);
      await page.waitForTimeout(400);
      check(path0(page) === pg.at, `${label} ${pg.name}: a tap on the song's name stays on the page (${path0(page)})`);
      check(await playing(page, want), `${label} ${pg.name}: …and plays that song (track ${want})`);
      // the artist line
      await fx.go(page, pg.at); await row.waitFor(); await page.waitForTimeout(300);
      await page.evaluate(() => document.querySelectorAll('audio, video').forEach((a) => a.removeAttribute('src'))); // (a reload may load the saved queue)
      await tapAt(page, row.locator('.by [data-artist-credit]').first(), touch);
      await page.waitForTimeout(400);
      check(path0(page) === pg.at, `${label} ${pg.name}: a tap on the artist stays on the page (${path0(page)})`);
      check(await playing(page, want), `${label} ${pg.name}: …and plays that song`);
      // the pages are in ⋯
      await fx.go(page, pg.at); await row.waitFor();
      await row.locator('[data-act="more"]').click();
      const go = page.locator('.popover.menu:not([hidden])').getByText('Go to song page', { exact: true });
      check(await go.waitFor({ timeout: 3000 }).then(() => true, () => false), `${label} ${pg.name}: ⋯ has Go to song page`);
      await go.click();
      await page.waitForFunction(() => location.pathname.startsWith('/song/')).catch(() => {});
      check(path0(page).startsWith('/song/'), `${label} ${pg.name}: Go to song page opens it (${path0(page)})`);
      await fx.go(page, pg.at); await row.waitFor();
      await fx.cleanCopy(page);
      if (pg.at === '/tracks') await page.screenshot({ path: path.join(shots, `songs-${label.replace(/ /g, '-')}.png`) });
      check(state.errors.length === 0, `${label} ${pg.name}: no page errors ${state.errors.join(' | ')}`);
    } catch (e) { check(false, `${label} ${pg.name}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
  }
}

// A tablet's touch screen (desktop layout) and a desktop mouse keep the names as links
async function linkCase(browser, width, touch) {
  const label = `${width}${touch ? ' touch' : ''}`;
  const run = await fx.create(browser, width, { mobile: touch, tracks }); // (mobile: a touch screen with no hover, as a tablet)
  const { context, page, state } = run;
  try {
    await fx.go(page, '/tracks');
    const row = page.locator('.tracks[data-list="all"] .track').nth(1); await row.waitFor();
    await tapAt(page, row.locator('.title [data-act="song"]'), touch);
    await page.waitForFunction(() => location.pathname.startsWith('/song/')).catch(() => {});
    check(path0(page).startsWith('/song/'), `${label} Songs: the song's name still opens its page (${path0(page)})`);
    check(state.errors.length === 0, `${label} Songs: no page errors`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) {
      await phoneCase(browser, width);
      await phoneCase(browser, width, { android: true });
    }
    await phoneCase(browser, 390, { touch: false });
    await linkCase(browser, 1024, true);
    await linkCase(browser, 1280, false);
  } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} FAIL`);
  process.exit(fail ? 1 : 0);
})();
