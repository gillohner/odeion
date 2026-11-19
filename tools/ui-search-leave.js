// Leaving Search (Escape in the search box, or emptying it) goes back to the page the search was
// started from, scrolled where it was, instead of to Home. A search opened straight from a link or a
// reload (nothing before it in the app), from another search, or with a menu open over it still goes
// to Home, as does the phone's Search tab. Two Escapes in a row go back one page, not two.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-search-leave.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/search-leave-shots');
fs.mkdirSync(shots, { recursive: true });
const names = ['Alpha', 'Bravo', 'Charlie', 'Delta', 'Echo', 'Foxtrot', 'Golf', 'Hotel'];
const tracks = Array.from({ length: 80 }, (_, i) => ({ ...fx.track, id: i + 1, title: `${names[i % 8]} song ${i + 1}`, canonical_uri: `${fx.SONG}?n=${i + 1}` }));

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 3000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };
const where = (page) => new URL(page.url()).pathname;
const scrollTop = (page) => page.evaluate(() => document.getElementById('main').scrollTop);

// The Songs page, scrolled down, then a search typed after "/"
async function songsThenSearch(page) {
  await fx.go(page, '/tracks');
  await page.locator('.tracks .track').nth(40).waitFor();
  await page.evaluate(() => document.getElementById('main').scrollTo({ top: 900, behavior: 'instant' }));
  await until(async () => (await scrollTop(page)) === 900);
  const was = await scrollTop(page);
  await page.evaluate(() => document.activeElement && document.activeElement.blur());
  await page.keyboard.press('/');
  await page.keyboard.type('Delta');
  await until(() => where(page) === '/search');
  return was;
}

async function run(browser, width) {
  const tag = `${width}`;
  {
    const { context, page, state } = await fx.create(browser, width, { tracks });
    const was = await songsThenSearch(page);
    check(was > 0, `${tag}: Songs scrolled before searching (${was})`);
    check(where(page) === '/search', `${tag}: typing opens Search`);
    await page.screenshot({ path: path.join(shots, `${width}-search.png`) });
    await page.keyboard.press('Escape');
    check(await until(() => where(page) === '/tracks'), `${tag}: Escape goes back to Songs (at ${where(page)})`);
    if (where(page) === '/tracks') await page.locator('.tracks .track').nth(40).waitFor();
    check(await until(async () => Math.abs((await scrollTop(page)) - was) < 4), `${tag}: Songs is where it was scrolled (${await scrollTop(page)} vs ${was})`);
    check((await page.inputValue('#search')) === '', `${tag}: the search box is empty`);
    await page.screenshot({ path: path.join(shots, `${width}-after-escape.png`) });

    // Emptying the box (select all, Backspace) also goes back, and the box keeps the focus
    await songsThenSearch(page);
    await page.keyboard.press('Control+A'); await page.keyboard.press('Backspace');
    check(await until(() => where(page) === '/tracks'), `${tag}: emptying the box goes back to Songs (at ${where(page)})`);
    check(await page.evaluate(() => document.activeElement && document.activeElement.id === 'search'), `${tag}: the box keeps the focus to type again`);
    await page.keyboard.type('Echo');
    check(await until(() => where(page) === '/search'), `${tag}: typing again opens Search again`);
    await page.keyboard.press('Escape');
    check(await until(() => where(page) === '/tracks'), `${tag}: and Escape goes back to Songs again`);

    // Two Escapes before the first lands: one page back (Albums), not two (Songs)
    await fx.go(page, '/tracks'); await page.locator('.tracks .track').first().waitFor();
    await page.click('.nav-item[data-view="albums"]');
    await until(() => where(page) === '/albums');
    await page.click('#search'); await page.keyboard.type('Golf');
    await until(() => where(page) === '/search');
    await page.evaluate(() => { const s = document.getElementById('search'); for (let i = 0; i < 2; i++) s.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
    await sleep(600);
    check(where(page) === '/albums', `${tag}: two Escapes go back one page (at ${where(page)})`);

    // Back from a result's page comes back to Search; Escape there goes to the page before Search
    await page.click('#search'); await page.keyboard.type('Hotel');
    await until(() => where(page) === '/search');
    await page.click('.nav-item[data-view="artists"]');
    await until(() => where(page) === '/artists');
    await page.goBack();
    await until(() => where(page) === '/search');
    await page.focus('#search'); await page.keyboard.press('Escape');
    check(await until(() => where(page) === '/albums'), `${tag}: after Back to Search, Escape goes to the page before it (at ${where(page)})`);
    check(state.errors.length === 0, `${tag}: no page errors ${JSON.stringify(state.errors)}`);
    await context.close();
  }
  {
    // Opened straight on a search link: nothing before it, so Home
    const { context, page } = await fx.create(browser, width, { tracks });
    await fx.go(page, '/search?q=Delta');
    await page.locator('#search').waitFor();
    await until(async () => (await page.inputValue('#search')) === 'Delta');
    await page.focus('#search'); await page.keyboard.press('Escape');
    check(await until(() => where(page) === '/'), `${tag}: a search opened from a link goes Home (at ${where(page)})`);
    await context.close();
  }
  {
    // A search opened from another search's row menu (Search “Freddie”): leaving it goes Home with an
    // empty box, as before (Back would put "Delta" back in the box being typed in)
    const searchHits = [{ kind: 'song', id: 'tidal-1', title: 'Bohemian song', subtitle: 'Freddie', artist: 'Freddie', uri: 'https://tidal.com/browse/track/1', sources: ['tidal'] }];
    const { context, page, state } = await fx.create(browser, width, { tracks, searchHits });
    const freddie = async () => {
      await fx.go(page, '/tracks'); await page.locator('.tracks .track').first().waitFor();
      await page.click('#search'); await page.keyboard.type('Delta');
      await until(() => where(page) === '/search');
      const row = page.locator('#main [data-srow]').first(); await row.waitFor(); await row.hover();
      await row.locator('[data-smore]').click();
      await page.locator('.popover.menu').getByText('Search “Freddie”').click();
      return until(async () => (await page.inputValue('#search')) === 'Freddie' && new URL(page.url()).searchParams.get('q') === 'Freddie');
    };
    check(await freddie(), `${tag}: the menu's Search “Freddie” opens a second search`);
    await page.click('#search'); await page.keyboard.press('Control+A'); await page.keyboard.press('Backspace');
    check(await until(() => where(page) === '/'), `${tag}: emptying a search opened from a search goes Home (at ${where(page)})`);
    await sleep(300);
    check((await page.inputValue('#search')) === '', `${tag}: and the box stays empty (“${await page.inputValue('#search')}”)`);
    check(await freddie(), `${tag}: the second search again`);
    await page.focus('#search'); await page.keyboard.press('Escape');
    check(await until(() => where(page) === '/'), `${tag}: Escape on a search opened from a search goes Home (at ${where(page)})`);

    // A row's menu open over Search when the box is left: Home and the menu closed, as before
    await fx.go(page, '/tracks'); await page.locator('.tracks .track').first().waitFor();
    await page.click('#search'); await page.keyboard.type('Delta');
    await until(() => where(page) === '/search');
    const row = page.locator('#main [data-srow]').first(); await row.waitFor(); await row.hover();
    await row.locator('[data-smore]').click(); await page.locator('.popover.menu').waitFor();
    await page.evaluate(() => { const s = document.getElementById('search'); s.value = ''; s.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
    check(await until(() => where(page) === '/'), `${tag}: leaving Search with a menu open goes Home (at ${where(page)})`);
    check(await page.locator('.popover.menu').isHidden(), `${tag}: and the menu is closed`);
    check(state.errors.length === 0, `${tag}: no page errors ${JSON.stringify(state.errors)}`);
    await context.close();
  }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) await run(browser, w);
    // The phone's Search tab (its own place in the bottom bar): Escape still goes Home
    const { context, page } = await fx.create(browser, 360, { tracks, mobile: true });
    await fx.go(page, '/tracks');
    await page.locator('.tracks .track').first().waitFor();
    await page.click('[data-bnav="search"]');
    await page.focus('#search'); await page.keyboard.type('Delta');
    await until(() => where(page) === '/search');
    await page.keyboard.press('Escape');
    check(await until(() => where(page) === '/'), `360: the phone's Search tab goes Home on Escape, as before (at ${where(page)})`);
    await context.close();
  } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} FAIL`);
  process.exit(fail ? 1 : 0);
})();
