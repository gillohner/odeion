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
// /, N and ? from a ⋯ (a song's, Now playing's and its Sleep timer's) close it first; after Back no
// menu's history entry is left and the page is the one the person was on. N then Back (or Esc) puts
// the focus back on the ⋯ it had, not on the mini player; Help opened over Now playing closes alone
// on Back, Now playing stays. Phones (412/360 px, the ⋯ sheet) the same. So does Help opened over Now
// playing with the zone picker, the Sound panel, the volume or the copy's popup open there: the panel
// closes first, and Back closes Help alone (before, Help, the panel and Now playing went together).
// The copy's popup closes on Esc, /, N and ? as the others do. A page opened from a Now playing link by
// keyboard leaves the focus on the mini player, not nowhere (it was on the row the page replaced).
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
const transient = (page) => page.evaluate(() => !!(history.state && history.state.transient));
const helpOpen = (page) => page.evaluate(() => [...document.querySelectorAll('dialog[open]')].some((d) => /Keyboard shortcuts/.test(d.textContent)));
const npShown = (page) => page.locator('#nowplaying').isVisible();
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
    check(!await page.locator('#nowplaying').isVisible() && !await menuOpen(page) && new URL(page.url()).pathname === '/queue' && await page.locator('.list.queue .list-row').count() === 3 && !await transient(page), `${width} Queue: then Back closes Now playing and stays on the Queue, no entry left in the history (${new URL(page.url()).pathname})`);
    check(await where(page) === 'qmore@Song 2', `${width} Queue: and the focus is back on Song 2's ⋯, not the mini player (${await where(page)})`);
    const help = () => helpOpen(page);
    inMenu = await fromMenu('?');
    check(inMenu && !await menuOpen(page) && await help(), `${width} Queue: ? in the menu closes it and opens the list of shortcuts`);
    await page.goBack(); await page.waitForTimeout(500);
    check(!await help() && new URL(page.url()).pathname === '/queue' && await page.locator('.list.queue .list-row').count() === 3 && !await transient(page), `${width} Queue: Back closes it and stays on the Queue, no entry left in the history (${new URL(page.url()).pathname})`);
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

// Now playing's ⋯ and its Sleep timer menu: /, N and ? from inside them; N then Back or Esc gives the
// focus back to the ⋯ it had. Also at phone widths (412/360: the ⋯ is a sheet there)
async function nowPlaying(browser, width) {
  const phone = width < 600;
  const { context, page, state } = await fx.create(browser, width, { fixtures, tracks, collections: [fx.summary], touch: width < 1025 });
  await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 0, id: saved[0].id, pos: 0 })]);
  const more = (n) => page.locator('.list.queue .list-row').nth(n).locator('[data-qmore]');
  const at = async () => `${new URL(page.url()).pathname}${await transient(page) ? ' +entry' : ''}`;
  const onQueue = async () => new URL(page.url()).pathname === '/queue' && !await transient(page) && await page.locator('.list.queue .list-row').count() === 3;
  const label = `${width} ${phone ? 'Phone' : 'Desktop'}`;
  // (after a failed check: back on the Queue with nothing open, so the next checks still run)
  const reset = async () => {
    if (await helpOpen(page)) { await page.keyboard.press('Escape'); await page.waitForTimeout(400); }
    if (await npShown(page)) { await page.keyboard.press('Escape'); await page.waitForTimeout(400); }
    if (new URL(page.url()).pathname !== '/queue' || await transient(page)) { await fx.go(page, '/queue'); await page.locator('.list.queue .list-row').nth(2).waitFor(); await page.waitForTimeout(300); }
  };
  try {
    await fx.go(page, '/queue');
    await page.locator('.list.queue .list-row').nth(2).waitFor();
    await page.waitForTimeout(300);
    if (phone) {
      // (a phone's song ⋯: /, N and ? from the sheet as on a desktop)
      const fromRow = async (key) => { await more(1).focus(); await page.keyboard.press('Enter'); const s = await shown(page); await page.keyboard.press(key); await page.waitForTimeout(500); return s; };
      let s = await fromRow('n');
      check(s && !await menuOpen(page) && await npShown(page), `${label} Queue: N in a song's ⋯ sheet closes it and opens Now playing`);
      await page.screenshot({ path: path.join(shots, `phone-np-from-sheet-${width}.png`) });
      await page.goBack(); await page.waitForTimeout(500);
      check(!await npShown(page) && await onQueue(), `${label} Queue: Back closes Now playing, on the Queue, no entry left (${await at()})`);
      check(await where(page) === 'qmore@Song 2', `${label} Queue: the focus back on Song 2's ⋯ (${await where(page)})`);
      await reset();
      s = await fromRow('?');
      check(s && !await menuOpen(page) && await helpOpen(page), `${label} Queue: ? in the ⋯ sheet closes it and opens the list of shortcuts`);
      await page.screenshot({ path: path.join(shots, `phone-help-from-sheet-${width}.png`) });
      await page.goBack(); await page.waitForTimeout(500);
      check(!await helpOpen(page) && await onQueue(), `${label} Queue: Back closes it, on the Queue, no entry left (${await at()})`);
      await reset();
      s = await fromRow('/');
      check(s && !await menuOpen(page) && !await transient(page) && await page.evaluate(() => document.activeElement && document.activeElement.id === 'search'), `${label} Queue: / in the ⋯ sheet closes it and goes to the search field, no entry left (${await where(page)})`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(400);
      await reset();
    }
    // N on a ⋯ (no menu) then Back, then Esc: the focus back on that ⋯
    await more(2).focus(); await page.keyboard.press('n'); await page.waitForTimeout(400);
    check(await npShown(page), `${label}: N on Song 3's ⋯ opens Now playing`);
    await page.goBack(); await page.waitForTimeout(500);
    check(!await npShown(page) && await onQueue() && await where(page) === 'qmore@Song 3', `${label}: Back closes it, the focus back on Song 3's ⋯ (${await where(page)}, ${await at()})`);
    await reset(); await more(2).focus(); await page.keyboard.press('n'); await page.waitForTimeout(400);
    await page.keyboard.press('Escape'); await page.waitForTimeout(500);
    check(!await npShown(page) && await onQueue() && await where(page) === 'qmore@Song 3', `${label}: Esc closes it, the focus back on Song 3's ⋯ (${await where(page)}, ${await at()})`);
    // Now playing's ⋯ (and Sleep timer's menu): /, N, ?
    const npMenu = async (sleep) => {
      if (!await npShown(page)) { await page.keyboard.press('n'); await page.locator('#npmore').waitFor({ timeout: 3000 }).catch(() => {}); await page.waitForTimeout(300); }
      await page.locator('#npmore').focus(); await page.keyboard.press('Enter'); let s = await shown(page);
      if (sleep) {
        for (let k = 0; k < 10 && !(await where(page)).includes('Sleep'); k++) await page.keyboard.press('ArrowDown');
        await page.keyboard.press('Enter'); await page.waitForTimeout(300);
        s = s && await where(page) === 'menu:15 minutes';
      }
      return s && (await where(page)).startsWith('menu:');
    };
    for (const sleep of [false, true]) {
      const m = sleep ? 'Now playing ⋯ › Sleep timer' : 'Now playing ⋯';
      let s = await npMenu(sleep);
      await page.keyboard.press('?'); await page.waitForTimeout(400);
      check(s && !await menuOpen(page) && await helpOpen(page) && await npShown(page), `${label} ${m}: ? closes the menu and opens the list of shortcuts over Now playing`);
      if (!sleep) await page.screenshot({ path: path.join(shots, `np-help-${width}.png`) });
      await page.goBack(); await page.waitForTimeout(500);
      check(!await helpOpen(page) && await npShown(page) && new URL(page.url()).pathname === '/queue' && await transient(page), `${label} ${m}: Back closes only the list of shortcuts, Now playing stays (${await npShown(page)}, ${await at()})`);
      await page.goBack(); await page.waitForTimeout(500);
      check(!await npShown(page) && await onQueue(), `${label} ${m}: Back again closes Now playing, on the Queue, no entry left (${await at()})`);
      await reset();
      s = await npMenu(sleep);
      await page.keyboard.press('?'); await page.waitForTimeout(400);
      await page.keyboard.press('Escape'); await page.waitForTimeout(500);
      check(s && !await helpOpen(page) && await npShown(page) && await transient(page), `${label} ${m}: Esc closes only the list of shortcuts, Now playing stays (${await at()})`);
      await page.goBack(); await page.waitForTimeout(500);
      check(!await npShown(page) && await onQueue(), `${label} ${m}: then Back closes Now playing, on the Queue, no entry left (${await at()})`);
      await reset();
      // (N pressed with the list of shortcuts open over Now playing closes Now playing under it: Esc
      // then leaves no entry in the history)
      s = await npMenu(sleep);
      await page.keyboard.press('?'); await page.waitForTimeout(400);
      await page.keyboard.press('n'); await page.waitForTimeout(400);
      await page.keyboard.press('Escape'); await page.waitForTimeout(500);
      check(s && !await helpOpen(page) && !await npShown(page) && await onQueue(), `${label} ${m}: ?, N, then Esc: all closed, on the Queue, no entry left (${await at()})`);
      await reset();
      s = await npMenu(sleep);
      await page.keyboard.press('n'); await page.waitForTimeout(500);
      check(s && !await menuOpen(page) && !await npShown(page) && await onQueue(), `${label} ${m}: N closes the menu and Now playing, on the Queue, no entry left (${await at()})`);
      await reset();
      s = await npMenu(sleep);
      await page.keyboard.press('/'); await page.waitForTimeout(500);
      check(s && !await menuOpen(page) && !await npShown(page) && !await transient(page) && await page.evaluate(() => document.activeElement && document.activeElement.id === 'search'), `${label} ${m}: / closes the menu and Now playing, the search field has the focus, no entry left (${await where(page)}, ${await at()})`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(500);
      await reset();
      check(await page.getByText(/Sleeping in/).count() === 0, `${label} ${m}: no sleep timer set`);
    }
    check(state.errors.length === 0, `${label}: no page errors ${state.errors.join('; ')}`);
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

// Desktop and tablet: Help over Now playing with a panel open there; the copy's popup on Esc, /, N
// and ?; a Now playing link opened by keyboard
async function npPanels(browser, width) {
  const job = { current: { id: 'j1', kind: 'copy', state: 'running', total: 25, done: 3, failed: 0, skipped: 0, bytes_total: 750e6, bytes_done: 90e6, errors: [], errors_count: 0, current_folder: '/music', speed_bps: 2.4e6, eta_s: 600 }, queued: [], last: null };
  const { context, page, state } = await fx.create(browser, width, { fixtures: { ...fixtures, '/pubky/files/jobs': { body: job } }, tracks, collections: [fx.summary], touch: width < 1025 });
  await context.addInitScript(([q, a]) => { try { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); } catch { /* about:blank, past the first page */ } }, [JSON.stringify(saved), JSON.stringify({ index: 0, id: saved[0].id, pos: 0 })]);
  const label = `${width} ${width < 1024 ? 'Tablet' : 'Desktop'}`;
  const vis = (sel) => page.locator(sel).isVisible();
  const at = async () => `${new URL(page.url()).pathname}${await transient(page) ? ' +entry' : ''}`;
  const onQueue = async () => new URL(page.url()).pathname === '/queue' && !await transient(page);
  const scrim = () => vis('#scrim');
  // (after a failed check: back on the Queue with nothing open)
  const reset = async () => {
    for (let k = 0; k < 4 && (await helpOpen(page) || await scrim() || await npShown(page)); k++) { await page.keyboard.press('Escape'); await page.waitForTimeout(300); }
    if (!await onQueue()) { await fx.go(page, '/queue'); await page.locator('.list.queue .list-row').nth(2).waitFor(); await page.waitForTimeout(300); }
  };
  const openNp = async () => { if (!await npShown(page)) { await page.keyboard.press('n'); await page.locator('#nowplaying').waitFor({ timeout: 3000 }).catch(() => {}); await page.waitForTimeout(300); } return npShown(page); };
  // (opened by a click, the key pressed on the button that opened it: the volume's slider owns the keys)
  const openPanel = async (btn, pop) => { await page.locator(btn).click(); await page.locator(pop).waitFor({ timeout: 3000 }).catch(() => {}); await page.locator(btn).focus(); return vis(pop); };
  try {
    await fx.go(page, '/queue');
    await page.locator('.list.queue .list-row').nth(2).waitFor();
    await page.locator('#jobchip').waitFor({ timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(300);
    check(await vis('#jobchip'), `${label}: the copy's chip shows the running copy`);
    const PANELS = [['zone picker', '#zonebtn', '#zonepop'], ['Sound panel', '#quality', '#qpanel'], ['volume', '#volbtn', '#volpop'], ["copy's popup", '#jobchip', '#jobpop']];
    for (const [name, btn, pop] of PANELS) {
      await reset();
      const np = await openNp();
      // ? over Now playing: the panel closes, Help is a layer of its own; Back closes Help, then Now playing
      const was = np && await vis(btn) && await openPanel(btn, pop);
      await page.keyboard.press('?'); await page.waitForTimeout(500);
      check(was && !await vis(pop) && !await scrim() && await helpOpen(page) && await npShown(page), `${label} ${name} over Now playing: ? closes it and opens the list of shortcuts over Now playing`);
      if (btn === '#jobchip') await page.screenshot({ path: path.join(shots, `np-job-help-${width}.png`) });
      await page.goBack(); await page.waitForTimeout(600);
      check(!await helpOpen(page) && await npShown(page) && !await vis(pop) && new URL(page.url()).pathname === '/queue' && await transient(page), `${label} ${name} over Now playing: Back closes only the list of shortcuts, Now playing stays (${await npShown(page)}, ${await at()})`);
      await page.goBack(); await page.waitForTimeout(600);
      check(!await npShown(page) && await onQueue(), `${label} ${name} over Now playing: Back again closes Now playing, on the Queue, no entry left (${await at()})`);
    }
    // Esc with the copy's popup over Now playing closes only the popup, the focus back on its chip
    await reset();
    { const was = await openNp() && await openPanel('#jobchip', '#jobpop');
      await page.locator('#jobpop button').first().focus();
      await page.keyboard.press('Escape'); await page.waitForTimeout(500);
      const f = await page.evaluate(() => document.activeElement && document.activeElement.id);
      check(was && !await vis('#jobpop') && !await scrim() && await npShown(page) && f === 'jobchip', `${label} copy's popup over Now playing: Esc closes only it, Now playing stays, the focus on its chip (${f}, ${await at()})`); }
    // The copy's popup over the Queue: Esc closes it alone; /, N and ? close it first
    for (const key of ['Escape', '/', 'n', '?']) {
      await reset();
      const was = await openPanel('#jobchip', '#jobpop');
      await page.keyboard.press(key); await page.waitForTimeout(500);
      const then = key === 'Escape' ? !await npShown(page) && !await helpOpen(page) : key === '/' ? await page.evaluate(() => document.activeElement && document.activeElement.id === 'search') : key === 'n' ? await npShown(page) : await helpOpen(page);
      const f = await page.evaluate(() => document.activeElement && document.activeElement.id);
      check(was && !await vis('#jobpop') && !await scrim() && then && (key === 'n' || key === '?' ? await transient(page) : !await transient(page)) && (key !== 'Escape' || f === 'jobchip'), `${label} copy's popup: ${key} closes it${key === 'Escape' ? ', the focus on its chip' : ' and the shortcut runs'} (${f}, ${await at()})`);
      if (key === 'n' || key === '?') { await page.goBack(); await page.waitForTimeout(600); check(!await npShown(page) && !await helpOpen(page) && await onQueue(), `${label} copy's popup: ${key}, then Back closes it, on the Queue, no entry left (${await at()})`); }
      if (key === '/') { await page.keyboard.press('Escape'); await page.waitForTimeout(300); }
    }
    // A page opened from a Now playing link by keyboard: the focus on the mini player, not on nothing
    await reset();
    await page.locator('.list.queue .list-row').nth(2).locator('[data-qmore]').focus();
    await openNp();
    const link = page.locator('#nowplaying [data-np-album]').first();
    const had = await link.count();
    if (had) { await link.focus(); await page.keyboard.press('Enter'); }
    await page.waitForFunction(() => /^\/album\//.test(location.pathname), null, { timeout: 3000 }).catch(() => {}); await page.waitForTimeout(500);
    const focus = await page.evaluate(() => { const a = document.activeElement; return !a || a === document.body ? 'body' : a.closest('#np') ? 'mini player' : a.id || a.tagName; });
    check(had && !await npShown(page) && /^\/album\//.test(new URL(page.url()).pathname) && focus === 'mini player', `${label}: the album opened from Now playing by keyboard, the focus on the mini player (${focus}, ${await at()})`);
    // (the same from Now playing's ⋯ › Go to song page: the page opens through setView, Now playing open)
    await reset();
    await page.locator('.list.queue .list-row').nth(2).locator('[data-qmore]').focus();
    await openNp();
    await page.locator('#npmore').focus(); await page.keyboard.press('Enter'); const menu = await shown(page);
    for (let k = 0; k < 12 && !(await where(page)).includes('Go to song page'); k++) await page.keyboard.press('ArrowDown');
    const item = (await where(page)).includes('Go to song page');
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => /^\/song\//.test(location.pathname), null, { timeout: 3000 }).catch(() => {}); await page.waitForTimeout(500);
    const focus2 = await page.evaluate(() => { const a = document.activeElement; return !a || a === document.body ? 'body' : a.closest('#np') ? 'mini player' : a.id || a.tagName; });
    check(menu && item && !await npShown(page) && /^\/song\//.test(new URL(page.url()).pathname) && focus2 === 'mini player', `${label}: Now playing ⋯ › Go to song page by keyboard, the focus on the mini player (${focus2}, ${await at()})`);
    check(state.errors.length === 0, `${label}: no page errors ${state.errors.join('; ')}`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) { await queue(browser, w); await lists(browser, w); await nowPlaying(browser, w); await npPanels(browser, w); }
    for (const w of [412, 360]) await nowPlaying(browser, w);
  } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
