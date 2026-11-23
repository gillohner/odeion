// Odeion's own words: Bookmarks (never Favourites) and Collections (never Playlists), on every
// page as it is drawn. Real candidate assets on the fixture origin (tools/ui-fixture.js); the text,
// titles, labels and placeholders of each page, of a song's ⋯ menu, the Add to collection picker,
// the account menu, Now playing and every Settings page are read at desktop and phone widths.
// Other products' own words may stay (a service's playlists, an M3U playlist file, Roon's
// favourites): the same list as tools/refactor/wording.js keeps.
//   node tools/ui-wording.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const wording = require('./refactor/wording');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'wording-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// What a person can read or hear on the page now: visible text, and the titles, labels and placeholders
const copy = (page) => page.evaluate(() => {
  const seen = (e) => e.getClientRects().length > 0;
  const attrs = [...document.querySelectorAll('[aria-label],[title],[placeholder],[alt]')].filter(seen).flatMap((e) => ['aria-label', 'title', 'placeholder', 'alt'].map((a) => e.getAttribute(a) || '')).filter(Boolean);
  return [...document.body.innerText.split('\n'), ...attrs].map((s) => s.trim()).filter(Boolean);
});
// The old words in a line of copy, but for what other products call their own
const old = (line) => wording.edits(line, '').map(([a, b]) => line.slice(a, b));
const read = async (page, where, found) => { for (const line of await copy(page)) { const w = old(line); if (w.length) found.push(`${where}: “${line.slice(0, 90)}”`); } };

const tracks = [1, 2, 3].map((n) => ({ ...fx.track, id: n, title: `Song number ${n}` }));
const SETTINGS = ['accounts', 'home', 'quality', 'playfrom', 'privacy', 'appearance', 'apps', 'people', 'playback', 'nodehome', 'discover', 'usage', 'backup', 'about'];
const PAGES = ['/', '/library', '/albums', '/artists', '/genres', '/tracks', '/playlists', '/playlist/c1', '/favourites', '/radio', '/sources', '/queue', '/zones', '/downloads', '/add-music', '/library/node', '/library/import', '/search?q=song', '/album/lib-7', '/song/lib-1', '/artist/lib-11'];

(async () => {
  const browser = await chromium.launch();
  for (const width of [1280, 390]) {
    const at = `${width}px`; const found = [];
    const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, tracks, collections: [fx.summary], favs: [fx.SONG] });
    for (const p of PAGES) { await fx.go(page, p); await sleep(450); await read(page, `${at} ${p}`, found); }
    // every Settings page
    for (const t of SETTINGS) { await fx.go(page, `/settings/${t}`); await sleep(350); await read(page, `${at} /settings/${t}`, found); }
    check(await page.evaluate(() => /About and updates/.test(document.querySelector('#main').innerText)), `${at}: the Settings pages were drawn`);
    // a song's ⋯ menu and the picker it opens
    await fx.go(page, '/tracks'); await page.waitForSelector('#main .track');
    const more = page.locator('#main .track [data-act="more"]').first();
    await more.click(); await sleep(250); await read(page, `${at} a song's ⋯ menu`, found);
    const menu = await page.evaluate(() => [...document.querySelectorAll('.popover.menu .pop-item')].map((b) => b.innerText.trim()));
    check(menu.some((m) => /^Add to collection…$/.test(m)) && menu.some((m) => /^(Add to|Remove from) Bookmarks$/.test(m)), `${at}: a song's ⋯ menu says Add to collection… and Bookmarks (${menu.join(' · ')})`);
    await page.locator('.popover.menu .pop-item', { hasText: 'Add to collection' }).first().click(); await sleep(300); await read(page, `${at} the Add to collection picker`, found);
    check(await page.evaluate(() => { const d = document.querySelector('dialog[open]'); return !!d && /Add to collection/.test(d.innerText) && d.querySelector('input').placeholder === 'Find a collection'; }), `${at}: the picker is "Add to collection", with "Find a collection"`);
    await page.keyboard.press('Escape'); await sleep(150);
    // the account menu
    await page.locator('#me').click(); await sleep(250); await read(page, `${at} the account menu`, found); await page.keyboard.press('Escape');
    // the ways in
    if (width >= 600) {
      const nav = await page.evaluate(() => [...document.querySelectorAll('.sidebar .nav-item')].map((b) => b.innerText.trim()));
      check(nav.includes('Bookmarks') && nav.includes('Collections') && !nav.includes('Favourites') && !nav.includes('Playlists'), `${at}: the side menu says Bookmarks and Collections`);
    } else {
      await fx.go(page, '/albums'); await sleep(300);
      const chips = await page.evaluate(() => [...document.querySelectorAll('#main .libnav .chip, #main .libnav button')].map((b) => b.innerText.trim()));
      check(chips.includes('Bookmarks') && chips.includes('Collections'), `${at}: the library's chips say Bookmarks and Collections (${chips.join(' · ')})`);
    }
    await fx.go(page, '/favourites'); await sleep(300);
    check(await page.evaluate(() => document.querySelector('#main h2, #main h1').innerText.trim().startsWith('Bookmarks')) && /Bookmarks/.test(await page.title()), `${at}: the page and its tab are named Bookmarks (${await page.title()})`);
    await fx.go(page, '/playlists'); await sleep(300);
    check(await page.evaluate(() => document.querySelector('#main h2, #main h1').innerText.trim().startsWith('Collections')) && /Collections/.test(await page.title()), `${at}: the page and its tab are named Collections (${await page.title()})`);
    await page.screenshot({ path: path.join(SHOTS, `collections-${width}.png`) });
    check(found.length === 0, `${at}: no page says Favourites or Playlists (${found.length ? found.slice(0, 12).join(' | ') : 'none'})`);
    check(!state.errors.length, `${at}: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  // the words another product calls its own stay
  check(wording.edits('Loading your TIDAL playlists…', '').length === 0 && wording.edits('An M3U playlist file', '').length === 0 && wording.edits('Roon favourites become Bookmarks', '').length === 0, 'a service\'s playlists, a playlist file and Roon\'s favourites keep their names');
  check(wording.edits('No playlists yet', '').length === 1 && wording.edits('Add to favourites', '').length === 1, 'Odeion\'s own old words are found');
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
