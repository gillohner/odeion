// A ⋯ menu works from the keyboard (desktop and tablet, 768-1280 px): a ⋯ opened with Enter puts the
// focus on the menu's first item; ↓ ↑ (round the ends), Home and End move through the items, and ← →
// there skip no song. Enter on an item runs it, and the focus goes back to the ⋯ that opened it: on
// the Queue, after Add to queue redraws the page, it is on the same song's ⋯. Esc closes the menu with
// the focus back on its ⋯; Tab closes it and goes on from the ⋯ to the next control. Before, the
// focus stayed on the ⋯ (no item could be reached without a mouse) and, after an item, fell to the
// page. A mouse leaves the focus as it was: a click on a ⋯ doesn't move it into the menu, and after
// a clicked item Space still plays or pauses (it doesn't open the ⋯ again), also after Now playing's
// ⋯ › Sleep timer (a menu that opens another: by mouse its first item doesn't take the focus, so
// Space doesn't set a timer; by keyboard it does). Songs' and an album's song ⋯ behave the same.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-menu-keys.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/menu-keys-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, album: { id: 7, title: 'Journey' }, canonical_uri: `${fx.SONG}?n=${id}` }));
const songs = tracks.map((t, i) => ({ no: i + 1, title: t.title, artist: 'Sefa', artist_id: 'lib-11', duration_ms: 240000, uri: null, track_id: t.id, id: `lib-${t.id}`, page: `lib-${t.id}` }));
const album = { uri: null, source: 'library', source_id: 'library', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [],
  kind: 'album', id: 'lib-7', target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, songs, about: [], about_text: null, image_credit: null };
const fixtures = { '/item/album/lib-7': { body: album }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } };
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };
const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));

const MENU = '.popover.menu:not([hidden])';
const menuOpen = (page) => page.locator(MENU).count().then((n) => n > 0);
// Where the focus is: 'body', 'menu:<item>', or the focused control's data key and its row's name
const where = (page) => page.evaluate(() => {
  const a = document.activeElement; if (!a || a === document.body) return 'body';
  if (a.closest('.popover.menu')) return `menu:${a.textContent.trim()}`;
  const row = a.closest('.list-row, .track, [data-srow]'); const name = row && row.querySelector('.name, .title, .t');
  return `${Object.keys(a.dataset).join(',') || a.id || a.tagName}@${name ? name.textContent.trim() : '-'}`;
});
const items = (page) => page.locator(`${MENU} .pop-item:not([disabled])`).evaluateAll((bs) => bs.map((b) => b.textContent.trim()));
// (a menu shown: true; none within 3 s: false, so a missing menu is a failed check, not a crash)
const shown = (page) => page.locator(`${MENU} .pop-item`).first().waitFor({ timeout: 3000 }).then(() => true, () => false);
// (the player's play() and pause() calls: Space played or paused)
const presses = (page) => page.evaluate(() => window.__presses);
const spyPlayer = (context) => context.addInitScript(() => {
  window.__presses = 0;
  for (const k of ['play', 'pause']) { const f = HTMLMediaElement.prototype[k]; HTMLMediaElement.prototype[k] = function (...a) { window.__presses++; return f.apply(this, a); }; }
});
const playingRow = (page) => page.evaluate(() => { const r = document.querySelector('.list.queue .list-row.cur .name'); return r ? r.textContent.trim() : null; });

async function queue(browser, width) {
  const { context, page, state } = await fx.create(browser, width, { fixtures, tracks, collections: [fx.summary], touch: width < 1025 });
  await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 0, id: saved[0].id, pos: 0 })]);
  await spyPlayer(context);
  const more = (n) => page.locator('.list.queue .list-row').nth(n).locator('[data-qmore]');
  try {
    await fx.go(page, '/queue');
    await page.locator('.list.queue .list-row').nth(2).waitFor();
    // Enter on Song 2's ⋯: the menu has the focus, on its first item
    await more(1).focus(); await page.keyboard.press('Enter');
    await page.locator(`${MENU} .pop-item`).first().waitFor();
    const list = await items(page);
    check(await where(page) === `menu:${list[0]}`, `${width} Queue: Enter on a ⋯ puts the focus on the menu's first item (${await where(page)})`);
    await page.screenshot({ path: path.join(shots, `queue-menu-keys-${width}.png`) });
    await page.keyboard.press('ArrowDown');
    check(await where(page) === `menu:${list[1]}`, `${width} Queue: ↓ goes to the next item (${await where(page)})`);
    await page.keyboard.press('ArrowUp'); await page.keyboard.press('ArrowUp');
    check(await where(page) === `menu:${list[list.length - 1]}`, `${width} Queue: ↑ from the first item goes round to the last (${await where(page)})`);
    await page.keyboard.press('ArrowDown');
    check(await where(page) === `menu:${list[0]}`, `${width} Queue: ↓ from the last goes round to the first (${await where(page)})`);
    await page.keyboard.press('End');
    check(await where(page) === `menu:${list[list.length - 1]}`, `${width} Queue: End goes to the last item`);
    await page.keyboard.press('Home');
    check(await where(page) === `menu:${list[0]}`, `${width} Queue: Home goes to the first item`);
    const was = await playingRow(page);
    await page.keyboard.press('ArrowRight'); await page.keyboard.press('ArrowLeft'); await page.waitForTimeout(300);
    check(await menuOpen(page) && await where(page) === `menu:${list[0]}` && await playingRow(page) === was, `${width} Queue: ← → in the menu skip no song and keep the menu (${was} → ${await playingRow(page)})`);
    // Esc: closed, the focus back on the ⋯
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
    check(!await menuOpen(page) && await where(page) === 'qmore@Song 2', `${width} Queue: Esc closes it, the focus back on Song 2's ⋯ (${await where(page)})`);
    // Tab: closed, and on from the ⋯ to the next control (not into the page's end, not the body)
    await page.keyboard.press('Enter'); await shown(page);
    await page.keyboard.press('Tab'); await page.waitForTimeout(200);
    const next = await where(page);
    check(!await menuOpen(page) && next !== 'body' && !next.startsWith('menu:') && next !== 'qmore@Song 2' && /@Song 3$/.test(next), `${width} Queue: Tab closes it and goes on from the ⋯ to the next row's control (${next})`);
    // /, N and ? from inside the menu still run (Search, Now playing, Help), and the menu closes
    // first: before, it stayed open over or behind them. Back then closes Now playing (it took the
    // menu's history entry over), with the Queue still there under it
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
    const fromMenu = async (key) => { await more(1).focus(); await page.keyboard.press('Enter'); await shown(page); const inMenu = (await where(page)).startsWith('menu:'); await page.keyboard.press(key); await page.waitForTimeout(400); return inMenu; };
    let inMenu = await fromMenu('/');
    check(inMenu && !await menuOpen(page) && await page.evaluate(() => document.activeElement && document.activeElement.id === 'search' && !(history.state && history.state.transient)), `${width} Queue: / in the menu closes it and goes to the search field, no menu entry left in the history (${await where(page)})`);
    await page.screenshot({ path: path.join(shots, `queue-menu-search-${width}.png`) });
    inMenu = await fromMenu('n');
    check(inMenu && !await menuOpen(page) && await page.locator('#nowplaying').isVisible(), `${width} Queue: N in the menu closes it and opens Now playing`);
    await page.goBack(); await page.waitForTimeout(500);
    check(!await page.locator('#nowplaying').isVisible() && !await menuOpen(page) && new URL(page.url()).pathname === '/queue' && await page.locator('.list.queue .list-row').count() === 3, `${width} Queue: then Back closes Now playing and stays on the Queue (${new URL(page.url()).pathname})`);
    const help = () => page.evaluate(() => [...document.querySelectorAll('dialog[open]')].some((d) => /Keyboard shortcuts/.test(d.textContent)));
    inMenu = await fromMenu('?');
    check(inMenu && !await menuOpen(page) && await help(), `${width} Queue: ? in the menu closes it and opens the list of shortcuts`);
    await page.goBack(); await page.waitForTimeout(500);
    check(!await help() && new URL(page.url()).pathname === '/queue' && await page.locator('.list.queue .list-row').count() === 3, `${width} Queue: Back closes it and stays on the Queue (${new URL(page.url()).pathname})`);
    inMenu = await fromMenu('?');
    check(inMenu && !await menuOpen(page) && await help(), `${width} Queue: ? in the menu again: the list of shortcuts`);
    await page.keyboard.press('Escape'); await page.waitForTimeout(400);
    check(await page.locator('dialog[open]').count() === 0 && new URL(page.url()).pathname === '/queue' && await page.evaluate(() => !(history.state && history.state.transient)), `${width} Queue: Esc closes it, the Queue still there, no menu entry left in the history`);
    // Enter on Add to queue: run, the page drawn again, the focus on Song 2's ⋯
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
    await more(1).focus(); await page.keyboard.press('Enter'); await shown(page);
    for (let k = 0; k < 10 && await where(page) !== 'menu:Add to queue'; k++) await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    await page.locator('.list.queue .list-row').nth(3).waitFor({ timeout: 3000 }).catch(() => {});
    await page.waitForTimeout(400);
    const n = await page.locator('.list.queue .list-row').count();
    check(n === 4, `${width} Queue: Add to queue picked with Enter added the song (${n} rows)`);
    // (two Song 2 rows now, the 2nd and the added 4th: the focus is on the exact ⋯ that opened the
    // menu, not the other Song 2's)
    const on = (k) => more(k).evaluate((b) => b === document.activeElement, null, { timeout: 3000 }).catch(() => false);
    check(await on(1) && !await on(3), `${width} Queue: after it the focus is on the same row's ⋯ again (Song 2, 2nd row), not the added Song 2's nor the page (${await where(page)}, 2nd row: ${await on(1)}, 4th: ${await on(3)})`);
    await page.screenshot({ path: path.join(shots, `queue-after-add-${width}.png`) });
    // A mouse: a click on the ⋯ leaves the focus out of the menu; after a clicked item, Space plays
    // or pauses and opens no menu
    await page.locator('#main h2').first().click();
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
    await more(2).click(); await shown(page);
    check(!(await where(page)).startsWith('menu:'), `${width} Queue: a click on a ⋯ doesn't move the focus into the menu (${await where(page)})`);
    const before = state.hits.length;
    await page.locator(`${MENU} .pop-item`, { hasText: 'Add to favourites' }).click(); await page.waitForTimeout(500);
    const after = await where(page);
    check(after !== 'qmore@Song 2' && !after.startsWith('menu:'), `${width} Queue: after a clicked item the focus isn't put on a ⋯ (${after})`);
    check(state.hits.slice(before).some((h) => h.v === '/favourites' || h.v === '/bookmarks'), `${width} Queue: the clicked item ran`);
    let p0 = await presses(page);
    await page.keyboard.press('Space'); await page.waitForTimeout(400);
    check(!await menuOpen(page) && await presses(page) > p0, `${width} Queue: Space after a clicked item plays or pauses and opens no menu (${await presses(page) - p0} presses)`);
    // Now playing's ⋯ › Sleep timer opens a second menu: by mouse, its first item doesn't take the
    // focus (Space plays or pauses, sets no timer); by keyboard it does
    await page.keyboard.press('n'); await page.locator('#npmore').waitFor({ timeout: 3000 }).catch(() => {});
    await page.locator('#npmore').click(); await shown(page);
    await page.locator(`${MENU} .pop-item`, { hasText: 'Sleep' }).click(); await page.waitForTimeout(300);
    check(await menuOpen(page) && !(await where(page)).startsWith('menu:'), `${width} Now playing: Sleep timer clicked: its menu opens, the focus not in it (${await where(page)})`);
    p0 = await presses(page);
    await page.keyboard.press('Space'); await page.waitForTimeout(400);
    check(await presses(page) > p0 && await page.getByText(/Sleeping in/).count() === 0, `${width} Now playing: then Space plays or pauses and sets no timer (${await presses(page) - p0} presses)`);
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
    await page.locator('#npmore').focus(); await page.keyboard.press('Enter'); await shown(page);
    for (let k = 0; k < 10 && !(await where(page)).includes('Sleep'); k++) await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter'); await page.waitForTimeout(300);
    check(await where(page) === 'menu:15 minutes', `${width} Now playing: Sleep timer picked with Enter: its menu has the focus, on its first item (${await where(page)})`);
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
    check(!await menuOpen(page) && await where(page) === 'npmore@-', `${width} Now playing: Esc closes it, the focus back on the ⋯ (${await where(page)})`);
    check(state.errors.length === 0, `${width} Queue: no page errors ${state.errors.join('; ')}`);
  } finally { await context.close(); }
}

// Songs and an album's songs: a song row's ⋯ by keyboard
const PAGES = [
  { at: '/tracks', name: 'Songs', row: '.tracks[data-list="all"] .track', more: '[data-act="more"]' },
  { at: '/album/lib-7', name: 'Album', row: '#main [data-srow]', more: '[data-smore]' },
];
async function lists(browser, width) {
  for (const pg of PAGES) {
    const { context, page, state } = await fx.create(browser, width, { tracks, fixtures, touch: width < 1025 });
    const label = `${width} ${pg.name}`;
    try {
      await fx.go(page, pg.at);
      const row = page.locator(pg.row).nth(1); await row.waitFor(); await page.waitForTimeout(400);
      const more = row.locator(pg.more);
      await more.focus(); await page.keyboard.press('Enter');
      await page.locator(`${MENU} .pop-item`).first().waitFor();
      const list = await items(page);
      check(await where(page) === `menu:${list[0]}`, `${label}: Enter on a song's ⋯ puts the focus on its first item (${await where(page)})`);
      await page.keyboard.press('ArrowDown');
      check(await where(page) === `menu:${list[1]}`, `${label}: ↓ goes to the next item`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(200);
      check(!await menuOpen(page) && await more.evaluate((b) => b === document.activeElement), `${label}: Esc closes it with the focus back on the ⋯ (${await where(page)})`);
      // Copy link (stays on the page, draws nothing): the focus goes back to the ⋯
      await page.keyboard.press('Enter'); await shown(page);
      for (let k = 0; k < 20 && await where(page) !== 'menu:Copy link'; k++) await page.keyboard.press('ArrowDown');
      const had = await where(page) === 'menu:Copy link';
      await page.keyboard.press('Enter'); await page.waitForTimeout(400);
      check(had && !await menuOpen(page) && await more.evaluate((b) => b === document.activeElement), `${label}: after an item picked with Enter the focus is back on the ⋯ (${await where(page)})`);
      check(state.errors.length === 0, `${label}: no page errors ${state.errors.join('; ')}`);
    } finally { await context.close(); }
  }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768]) { await queue(browser, w); await lists(browser, w); } } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
