// On a phone a library song row (Songs, Favourites, a genre's songs) is one tap target, like the song
// rows of playlists, search and item pages: a tap on its name or its artist plays from there instead of
// opening a page. The song, artist and album pages stay in the row's ⋯ (Go to song page). Desktop and a
// tablet's touch screen keep the names as links. Nor are the names Tab stops on a phone, in any song
// list (tabCase). With two artists credited the ⋯ has one Go to per artist ("Go to Sefa", "Go to
// Dr. Peacock"), as the row's names no longer open them (menuArtists); desktop keeps one Go to artist.
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
// The open ⋯ menu's artist rows, and a press on one of them
const MENU = '.popover.menu:not([hidden])';
const menuArtists = (page) => page.locator(`${MENU} button, ${MENU} [role="menuitem"]`).evaluateAll((els) => els.map((e) => e.textContent.trim()).filter((t) => /^Go to (?!song page|album)|^Search “/.test(t)));
// the song the app's player loaded (a fixture's play() is a no-op, so the bar stays at "Starting on…")
const playing = (page, id) => page.waitForFunction((n) => [...document.querySelectorAll('audio, video')].some((a) => a.src.includes(`/library/tracks/${n}/file`)), id, { timeout: 4000 }).then(() => true, () => false);
const tapAt = async (page, loc, touch) => {
  const b = await loc.boundingBox(); const x = b.x + Math.min(b.width / 2, 20); const y = b.y + b.height / 2;
  if (touch) await page.touchscreen.tap(x, y); else await page.mouse.click(x, y);
};
// The page each list is on, and the row's names in it
const PAGES = [
  { at: '/tracks', name: 'Songs', ready: '.tracks[data-list="all"] .track' },
  { at: '/favourites', name: 'Bookmarks', ready: '.tracks[data-list="fav"] .track', opts: { favs: [fx.SONG] } },
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
      // one Go to per credited artist (the row's names don't open them on a phone)
      await fx.go(page, pg.at); await row.waitFor();
      await row.locator('[data-act="more"]').click();
      await page.locator(MENU).getByText('Go to song page', { exact: true }).waitFor({ timeout: 3000 }).catch(() => {});
      const arts = await menuArtists(page);
      check(arts.join('|') === 'Go to Sefa|Go to Dr. Peacock', `${label} ${pg.name}: ⋯ has one Go to per artist (${arts.join(' | ')})`);
      await page.locator(MENU).getByText('Go to Dr. Peacock', { exact: true }).click().catch(() => {});
      await page.waitForFunction(() => location.pathname.startsWith('/artist/')).catch(() => {});
      check(path0(page) === '/artist/lib-22', `${label} ${pg.name}: Go to Dr. Peacock opens the second artist (${path0(page)})`);
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
    // (the names are links here: ⋯ keeps its one Go to artist)
    await fx.go(page, '/tracks'); await row.waitFor();
    await row.locator('[data-act="more"]').click();
    await page.locator(MENU).getByText('Go to song page', { exact: true }).waitFor({ timeout: 3000 }).catch(() => {});
    const arts = await menuArtists(page);
    check(arts.join('|') === 'Go to artist', `${label} Songs: ⋯ has one Go to artist (${arts.join(' | ')})`);
    check(state.errors.length === 0, `${label} Songs: no page errors`);
  } finally { await context.close(); }
}

// The names in a song row are not Tab stops on a phone either (a tap plays the row; ⋯ has the pages),
// in the library's lists (.track) as in the others (.srow), artists read in later too; Tab goes from
// the row's play button to its ⋯. The desktop layout keeps them as Tab stops, and a change of layout
// (a window resized across 600 px) redoes them.
const TRACK_NAMES = '.track .main-col .lnk'; const SROW_NAMES = '.srow:not(.card-row) :is(.name, .sub) .lnk:not(.vmark)'; // (as app.css names them)
const tabs = (page, sel) => page.locator(sel).evaluateAll((els) => els.map((e) => e.tabIndex));
async function tabCase(browser, width, { touch = true } = {}) {
  const phone = width < 600; const want = phone ? -1 : 0; const label = `${width}${touch ? '' : ' mouse'} keys`;
  const run = await fx.create(browser, width, { mobile: touch, tracks });
  const { context, page, state } = run;
  try {
    await fx.go(page, '/tracks');
    const row = page.locator('.tracks[data-list="all"] .track').nth(1); await row.waitFor();
    const songs = await tabs(page, `.tracks[data-list="all"] ${TRACK_NAMES}`);
    check(songs.length >= 12 && songs.every((n) => n === want), `${label} Songs: ${songs.length} names (song, artists, album) have tabIndex ${want} (${[...new Set(songs)]})`);
    if (phone) {
      await row.locator('[data-act="play"]').focus(); await page.keyboard.press('Tab');
      check(await row.locator('[data-act="more"]').evaluate((e) => e === document.activeElement), `${label} Songs: Tab goes from the row's play button to its ⋯`);
      // a change of layout redoes them, and back
      await page.setViewportSize({ width: 1280, height: 900 }); await page.waitForFunction(() => document.documentElement.dataset.shell !== 'phone');
      const wide = await tabs(page, `.tracks[data-list="all"] ${TRACK_NAMES}`);
      check(wide.length === songs.length && wide.every((n) => n === 0), `${label} Songs: widened to the desktop layout, the names are Tab stops again (${[...new Set(wide)]})`);
      await page.setViewportSize({ width, height: 900 }); await page.waitForFunction(() => document.documentElement.dataset.shell === 'phone');
      const back = await tabs(page, `.tracks[data-list="all"] ${TRACK_NAMES}`);
      check(back.length === songs.length && back.every((n) => n === -1), `${label} Songs: narrowed again, they are not (${[...new Set(back)]})`);
    }
    // a playlist whose page has only the flattened artist line: the artists are read in afterwards
    state.fixtures = { '/playlists/c1': { body: { ...fx.detail, tracks: [null, null] } } };
    await fx.go(page, '/playlist/c1');
    const credits = page.locator('#plsongs .srow').first().locator('[data-artist-credit]');
    const split = await page.waitForFunction(() => document.querySelectorAll('#plsongs .srow [data-artist-credit]').length === 2, null, { timeout: 4000 }).then(() => true, () => false);
    check(split, `${label} collection: the flattened artist line is read in as two artists`);
    const pl = await tabs(page, `#plsongs ${SROW_NAMES}`);
    check(split && pl.length >= 3 && pl.every((n) => n === want), `${label} collection: its row's ${pl.length} names, read-in artists too, have tabIndex ${want} (${pl})`);
    check(await credits.count() === 2, `${label} collection: two artist links`);
    // its ⋯: one Go to per artist on a phone; desktop keeps its one row (the names are links there)
    await page.locator('#plsongs .srow').first().locator('[data-smore]').click();
    await page.locator(MENU).getByText('Add to queue', { exact: true }).waitFor({ timeout: 3000 }).catch(() => {});
    const arts = await menuArtists(page);
    check(phone ? arts.join('|') === 'Go to Sefa|Go to Dr. Peacock' : arts.length === 1, `${label} collection: ⋯ has ${phone ? 'one Go to per artist' : 'one artist row'} (${arts.join(' | ')})`);
    if (phone) {
      await page.locator(MENU).getByText('Go to Dr. Peacock', { exact: true }).click().catch(() => {});
      await page.waitForFunction(() => location.pathname.startsWith('/artist/')).catch(() => {});
      check(path0(page) === '/artist/lib-22', `${label} collection: Go to Dr. Peacock opens the second artist (${path0(page)})`);
      await fx.go(page, '/playlist/c1');
      await page.waitForFunction(() => document.querySelectorAll('#plsongs .srow [data-artist-credit]').length === 2, null, { timeout: 4000 }).catch(() => {});
    } else await page.keyboard.press('Escape');
    if (phone) { // (the artists too follow a change of layout)
      await page.setViewportSize({ width: 1280, height: 900 }); await page.waitForFunction(() => document.documentElement.dataset.shell !== 'phone');
      const wide = await tabs(page, `#plsongs ${SROW_NAMES}`);
      check(wide.length === pl.length && wide.every((n) => n === 0), `${label} collection: widened, its names are Tab stops again (${wide})`);
      await page.setViewportSize({ width, height: 900 }); await page.waitForFunction(() => document.documentElement.dataset.shell === 'phone');
      const back = await tabs(page, `#plsongs ${SROW_NAMES}`);
      check(back.length === pl.length && back.every((n) => n === -1), `${label} collection: narrowed again, they are not (${back})`);
    }
    check(state.errors.length === 0, `${label}: no page errors ${state.errors.join(' | ')}`);
  } catch (e) { check(false, `${label}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
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
    for (const width of [360, 412]) await tabCase(browser, width);
    await tabCase(browser, 390, { touch: false });
    await tabCase(browser, 1280, { touch: false });
  } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} FAIL`);
  process.exit(fail ? 1 : 0);
})();
