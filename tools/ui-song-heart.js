// Every song list says which songs you have bookmarked, and bookmarking one takes a single click.
// A song row (`songRows`, the renderer for the album, artist, search, Home, smart/shared playlist
// and collection lists) now carries the heart a row on Library › Songs has: filled and labelled
// "Remove from favourites" for a song you have — shown then without hover, on a touch screen too
// (docs/ux/responsive.md) — an outline that appears when you hover the row with a mouse.
// Before, those lists had only Play and ⋯, so you could not see which songs you had, and saving one
// cost 2 clicks (⋯ → Add to favourites) against 1 on Songs.
// Checked here: the heart saves in place (the list is not redrawn) and the row's ⋯ menu agrees with
// it; a screen reader hears it as a toggle (aria-pressed, like Now Playing's heart) on the album
// rows and on Library › Songs; it never starts the song; from the keyboard Enter saves and the heart keeps focus, so Tab
// carries on down the row; bookmarks that arrive after the page fill the hearts in by themselves;
// while you pick songs on a narrow window the heart stands aside with the row's other controls; a
// collection's fuller row still fits at 700 px; and the phone shell keeps its two-control row (the
// phone lane owns < 600 px).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-song-heart.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/song-heart-shots');
fs.mkdirSync(shots, { recursive: true });

// A library song on an album page, the way the node sends it (crates/pm-api/src/item.rs
// `library_row`): no `page`, a `track_id`, and a page id that is its MusicBrainz recording when it
// is matched. Its favourite key is then that recording's URL — the one PUT /favourites answers with.
const songRow = { no: 1, title: fx.track.title, artist: 'Sefa', artist_id: 'lib-11', album_id: 'lib-7', duration_ms: 240000, track_id: 1, id: `mb-${fx.track.mbid}` };
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: {}, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [songRow], total_ms: 240000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
const fixtures = {
  '/item/album/lib-7': { body: albumPage },
  '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
};

const ROW = '#isongs .srow';
const HEART = `${ROW} [data-sfav]`;
// Playwright counts an element with opacity 0 as visible, so read what the eye gets.
const shown = (page, sel) => page.evaluate((s) => {
  const e = document.querySelector(s);
  if (!e) return { there: false };
  const cs = getComputedStyle(e);
  const r = e.getBoundingClientRect();
  return { there: true, opacity: Number(cs.opacity), display: cs.display, w: r.width, h: r.height };
}, sel);
const heartState = (page, sel = HEART) => page.evaluate((s) => {
  const b = document.querySelector(s);
  return b && { on: b.classList.contains('on'), pressed: b.getAttribute('aria-pressed'), title: b.title, label: b.getAttribute('aria-label'), icon: (b.querySelector('use')?.getAttribute('href') || '').replace('#i-', ''), next: b.nextElementSibling?.dataset.smore != null };
}, sel);
const menuLabels = (page) => page.evaluate(() => [...document.querySelectorAll('.popover.menu:not([hidden]) .pop-item')].map((b) => b.textContent.trim()));
const closeMenu = async (page) => { await page.keyboard.press('Escape'); await page.waitForSelector('.popover.menu:not([hidden])', { state: 'detached', timeout: 2000 }).catch(() => {}); await page.waitForTimeout(120); };
const ok = (m) => console.log('PASS ' + m);

// 1. Desktop with a mouse: the heart waits for the row, one click saves, and nothing is redrawn.
async function mouse(browser) {
  const { page, context, state } = await fx.create(browser, 1280, { fixtures });
  try {
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector(ROW);
    await page.waitForSelector(HEART);
    const before = await shown(page, HEART);
    assert.equal(before.there, true, 'a song row has a heart');
    assert.equal(before.opacity, 0, `not a favourite and not hovered: the heart is out of the way (opacity ${before.opacity})`);
    await page.screenshot({ path: path.join(shots, 'album-1280-nohover.png') });

    await page.locator(ROW).first().hover();
    await page.waitForFunction((s) => Number(getComputedStyle(document.querySelector(s)).opacity) === 1, HEART, { timeout: 3000 });
    const hov = await shown(page, HEART);
    assert.ok(hov.w >= 28 && hov.h >= 28, `the heart is a real target (${hov.w}x${hov.h})`);
    let h = await heartState(page);
    assert.deepEqual([h.on, h.icon, h.title], [false, 'heart', 'Add to favourites'], `an outline heart before saving (${JSON.stringify(h)})`);
    assert.equal(h.label, `Add ${fx.track.title} to favourites`, 'a screen reader hears which song');
    assert.equal(h.pressed, 'false', 'and that the heart is a toggle, off (as Now Playing\'s heart says)');
    assert.equal(h.next, true, 'the heart sits right before the row\'s ⋯, as on Songs');
    await page.screenshot({ path: path.join(shots, 'album-1280-hover.png') });

    // One click bookmarks it (through ⋯ it took two)
    await page.evaluate((s) => { document.querySelector(s).dataset.probe = 'same-node'; }, ROW);
    const quiet = await page.evaluate(() => ({ np: document.querySelector('.player .np-title, #pbtitle')?.textContent || '', cur: document.querySelectorAll('.srow.cur').length }));
    await page.click(HEART);
    await page.waitForFunction((s) => document.querySelector(s).classList.contains('on'), HEART, { timeout: 4000 });
    const puts = state.hits.filter((x) => x.v === '/favourites' && x.m === 'PUT');
    assert.equal(puts.length, 1, 'one click, one save');
    assert.deepEqual(JSON.parse(puts[0].body), { track_id: 1 }, 'saved by its library track');
    h = await heartState(page);
    assert.deepEqual([h.on, h.icon, h.title], [true, 'heart-fill', 'Remove from favourites'], `filled and relabelled (${JSON.stringify(h)})`);
    assert.equal(h.label, `Remove ${fx.track.title} from favourites`);
    assert.equal(h.pressed, 'true', 'a screen reader hears it is on now');
    assert.equal(await page.evaluate((s) => document.querySelector(s).dataset.probe, ROW), 'same-node', 'the list was not redrawn under you');
    assert.equal(await shown(page, HEART).then((x) => x.opacity), 1, 'a favourite keeps its heart');

    // The heart is not the row: pressing it must not start the song (nothing playing, no row marked
    // as the current one, and the queue was never asked to change)
    const after = await page.evaluate(() => ({ np: document.querySelector('.player .np-title, #pbtitle')?.textContent || '', cur: document.querySelectorAll('.srow.cur').length }));
    assert.deepEqual(after, quiet, `the heart does not start the song (${JSON.stringify(quiet)} → ${JSON.stringify(after)})`);
    assert.equal(state.hits.filter((x) => /^\/(zones|queue)\//.test(x.v) && x.m !== 'GET').length, 0, 'and asks nothing of the player');
    await page.screenshot({ path: path.join(shots, 'album-1280-saved.png') });

    // The row's ⋯ menu agrees with the heart
    await page.click(`${ROW} [data-smore]`);
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
    let labels = await menuLabels(page);
    assert.ok(labels.includes('Remove from favourites') && !labels.includes('Add to favourites'), `the ⋯ menu agrees (${labels.join(' · ')})`);
    await closeMenu(page);

    // Clicking it again takes it back out
    await page.locator(ROW).first().hover();
    await page.click(HEART);
    await page.waitForFunction((s) => !document.querySelector(s).classList.contains('on'), HEART, { timeout: 4000 });
    const dels = state.hits.filter((x) => x.v === '/favourites' && x.m === 'DELETE');
    assert.equal(dels.length, 1, 'one click, one removal');
    assert.equal(dels[0].q, '?track_id=1', `removed by the same key (${dels[0].q})`);
    h = await heartState(page);
    assert.deepEqual([h.on, h.pressed, h.title], [false, 'false', 'Add to favourites'], 'an outline again, and off');
    await page.click(`${ROW} [data-smore]`);
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
    labels = await menuLabels(page);
    assert.ok(labels.includes('Add to favourites') && !labels.includes('Remove from favourites'), `the ⋯ menu agrees again (${labels.join(' · ')})`);
    await closeMenu(page);

    // From the keyboard: Enter saves and the heart keeps focus, so Tab carries on down the row
    // (a control disabled while it saves would drop you back to the top of the page)
    await page.evaluate((s) => document.querySelector(s).focus(), HEART);
    assert.equal(await page.evaluate(() => document.activeElement?.dataset.sfav), '0', 'the heart takes keyboard focus');
    await page.keyboard.press('Enter');
    await page.waitForFunction((s) => document.querySelector(s).classList.contains('on'), HEART, { timeout: 4000 });
    assert.equal(await page.evaluate(() => document.activeElement?.dataset.sfav), '0', 'and keeps it while and after it saves');
    assert.equal(await page.evaluate((s) => document.querySelector(s).hasAttribute('disabled'), HEART), false, 'the press is held by aria-busy, never by disabling the button');
    await page.keyboard.press('Tab');
    assert.equal(await page.evaluate(() => document.activeElement?.dataset.smore), '0', 'Tab goes on to the row\'s ⋯, not back to the top of the page');

    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, []);
    ok('1280 px, mouse: a song row\'s heart appears on hover, saves in one click and keeps the list still');
    ok('1280 px, keyboard: Enter saves and the heart keeps focus, so Tab carries on down the row');
  } finally { await context.close(); }
}

// 2. A song you have already: its heart shows without hover, with a mouse and on a touch screen.
// `key` is how the node holds it: by the song's own address (a matched song, the usual case) or by
// the library track (a song nobody has matched yet). Either way the row knows.
async function already(browser, width, touch, key) {
  const { page, context, state } = await fx.create(browser, width, { fixtures, touch, favs: [key] });
  try {
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector(HEART);
    await page.waitForFunction((s) => document.querySelector(s).classList.contains('on'), HEART, { timeout: 4000 });
    const s = await shown(page, HEART);
    assert.notEqual(s.display, 'none', `${width}${touch ? ' touch' : ''}: a favourite's heart is there without hover`);
    assert.equal(s.opacity, 1, `${width}${touch ? ' touch' : ''}: a favourite's heart is shown without hover (opacity ${s.opacity})`);
    if (touch) assert.ok(s.w >= 40 && s.h >= 40, `${width} touch: the heart is a 40 px target (${s.w}x${s.h})`);
    const h = await heartState(page);
    assert.deepEqual([h.on, h.icon, h.title], [true, 'heart-fill', 'Remove from favourites'], `${width}: filled, and it says what pressing it does`);
    assert.equal(h.pressed, 'true', `${width}: drawn on, it says so to a screen reader too`);
    await page.screenshot({ path: path.join(shots, `album-${width}${touch ? '-touch' : ''}-favourite.png`) });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, []);
    ok(`${width} px${touch ? ', touch' : ''}: a song you bookmarked (held as ${key}) shows its filled heart without hover`);
  } finally { await context.close(); }
}

// 3. A touch screen has no hover, so a song that is not a favourite keeps its two-control row.
async function touchPlain(browser) {
  const { page, context, state } = await fx.create(browser, 1024, { fixtures, touch: true });
  try {
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector(ROW);
    await page.waitForSelector(`${ROW} [data-smore]`);
    const s = await shown(page, HEART);
    assert.equal(s.display, 'none', `a touch tablet shows no outline heart to hover (display ${s.display})`);
    assert.equal(await page.locator(`${ROW} [data-smore]`).isVisible(), true, 'the ⋯ is still there, so the song can still be saved from it');
    await page.screenshot({ path: path.join(shots, 'album-1024-touch-plain.png') });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, []);
    ok('1024 px, touch: no hover, so only a favourite\'s heart shows (docs/ux/responsive.md)');
  } finally { await context.close(); }
}

// 4. While you are picking songs on a narrow window, the heart stands aside with the row's other
// controls, so a tap on a row ticks it.
async function selecting(browser) {
  const { page, context, state } = await fx.create(browser, 700, { fixtures, favs: ['track:1'] });
  try {
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector(HEART);
    await page.waitForFunction((s) => document.querySelector(s).classList.contains('on'), HEART, { timeout: 4000 });
    assert.notEqual((await shown(page, HEART)).display, 'none', '700 px: a favourite shows its heart');
    await page.click('#isongs [data-selstart]');
    await page.waitForSelector(`${ROW} .selbox`);
    assert.equal((await shown(page, HEART)).display, 'none', '700 px, picking songs: the heart stands aside');
    assert.equal((await shown(page, `${ROW} [data-smore]`)).display, 'none', '700 px, picking songs: so does ⋯ (as before)');
    await page.screenshot({ path: path.join(shots, 'album-700-selecting.png') });
    assert.deepEqual(state.errors, []);
    ok('700 px: while you pick songs, the heart stands aside with the row\'s other controls');
  } finally { await context.close(); }
}

// 5. A collection's rows have their own controls (where it plays from, move up, move down, remove):
// the heart still sits right before ⋯, outside them, and the row still fits a narrow window.
async function collection(browser, width) {
  const { page, context, state } = await fx.create(browser, width, { favs: ['track:1'] });
  try {
    await fx.go(page, '/playlist/c1');
    await page.waitForSelector('#plsongs .srow');
    await page.waitForSelector('#plsongs .srow [data-sfav]');
    const shape = await page.evaluate(() => {
      const r = document.querySelector('#plsongs .srow');
      const b = r.querySelector('[data-sfav]');
      return { on: b.classList.contains('on'), inActions: !!b.closest('.q-actions'), beforeMore: b.nextElementSibling?.dataset.smore != null, last: r.lastElementChild?.dataset.smore != null };
    });
    assert.equal(shape.inActions, false, `${width}: the heart is its own control, not one of the row's move buttons`);
    assert.equal(shape.beforeMore, true, `${width}: the heart sits right before ⋯ here too`);
    assert.equal(shape.last, true, `${width}: the row still ends with ⋯`);
    assert.equal(shape.on, true, `${width}: a collection shows which of its songs you have bookmarked`);
    // The fullest row there is: number, play, cover, two lines, length, where it plays from, three
    // move buttons, the new heart and ⋯ — all of it on a 700 px window, with the move buttons out
    await page.locator('#plsongs .srow').first().hover();
    await page.waitForTimeout(150);
    const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    assert.ok(over <= 0, `${width}: a collection's row with every control out fits (${over} px over)`);
    await page.screenshot({ path: path.join(shots, `collection-${width}.png`) });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, []);
    ok(`${width} px: a collection's song rows keep their move buttons, gain the heart before ⋯ and still fit`);
  } finally { await context.close(); }
}

// 6. Bookmarks slow to arrive (a listener's go to their homeserver): the page draws first and the
// hearts fill in by themselves, without the list being drawn again under you.
async function late(browser) {
  const { page, context, state } = await fx.create(browser, 1280, { fixtures, favs: [fx.SONG], delay: (v) => (v === '/favourites' ? 1500 : 0) });
  try {
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector(HEART);
    await page.evaluate((s) => { document.querySelector(s).dataset.probe = 'same-node'; }, ROW);
    assert.equal((await heartState(page)).on, false, 'the page does not wait for the bookmarks');
    await page.screenshot({ path: path.join(shots, 'album-1280-late-waiting.png') });
    await page.waitForFunction((s) => document.querySelector(s).classList.contains('on'), HEART, { timeout: 6000 });
    const h = await heartState(page);
    assert.deepEqual([h.on, h.icon, h.title], [true, 'heart-fill', 'Remove from favourites'], 'the heart fills in once they are in');
    assert.equal(h.label, `Remove ${fx.track.title} from favourites`, 'and a screen reader hears the new state');
    assert.equal(await page.evaluate((s) => document.querySelector(s).dataset.probe, ROW), 'same-node', 'only the hearts were painted again, not the list');
    await page.screenshot({ path: path.join(shots, 'album-1280-late-filled.png') });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, []);
    ok('1280 px: bookmarks that arrive after the page fill the hearts in, without redrawing the list');
  } finally { await context.close(); }
}

// 7. The phone shell is unchanged: the same two controls, and nothing wider than the screen.
async function phone(browser, width) {
  const { page, context, state } = await fx.create(browser, width, { fixtures, mobile: true, favs: ['track:1'] });
  try {
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector(ROW);
    await page.waitForSelector(`${ROW} [data-smore]`);
    assert.equal(await page.evaluate(() => document.documentElement.dataset.shell), 'phone', `${width}: the phone shell`);
    const s = await shown(page, HEART);
    assert.equal(s.display, 'none', `${width}: the phone's song row is unchanged (display ${s.display})`);
    const over = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    assert.ok(over <= 0, `${width}: no horizontal overflow (${over})`);
    await page.screenshot({ path: path.join(shots, `album-${width}-phone.png`) });
    assert.deepEqual(state.errors, []);
    ok(`${width} px: the phone shell keeps its two-control song row`);
  } finally { await context.close(); }
}

// 8. One song in two rows (an album and its bonus disc, a search and a collection: the same song
// is often listed twice). Saving it from one row turns the other row's heart over too, at once,
// and a slow first favourites answer (a cold start: asked for before the save, answered after it)
// does not put the old list back and turn the hearts out again.
// The node's favourites, answered from what they were when each GET was asked (a slow answer is
// the old list); `hold`: the first GET waits for release().
async function favRoute(page, { start = [], hold = false } = {}) {
  const favs = new Set(start); const asks = []; let release = () => {}; let sent = () => {};
  const held = new Promise((ok) => { release = ok; }); const answered = new Promise((ok) => { sent = ok; });
  await page.route('**/v1/favourites*', async (route) => {
    const m = route.request().method(); asks.push(m);
    const body = (b) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(b) });
    if (m === 'PUT') { favs.add('track:1'); favs.add(fx.SONG); return body({ uri: fx.SONG }); }
    if (m === 'DELETE') { favs.clear(); return body({}); }
    const uris = [...favs];
    const first = hold && asks.filter((x) => x === 'GET').length === 1;
    if (first) await held;
    await body({ uris, tracks: [], albums: [], artists: [], other: [] });
    if (first) sent();
  });
  // answered: the held answer has been sent
  return { release, asks, answered };
}
const twice = { '/item/album/lib-7': { body: { ...albumPage, songs: [songRow, { ...songRow, no: 2 }], total_ms: 480000 } }, '/item/album/lib-7/extras': fixtures['/item/album/lib-7/extras'] };
async function elsewhere(browser) {
  const { page, context, state } = await fx.create(browser, 1280, { fixtures: twice });
  try {
    const node = await favRoute(page, { hold: true });
    await fx.go(page, '/album/lib-7');
    await page.waitForSelector(`${ROW}:nth-child(2) [data-sfav]`);
    const H1 = `${ROW}:nth-child(1) [data-sfav]`; const H2 = `${ROW}:nth-child(2) [data-sfav]`;
    assert.equal(node.asks.filter((m) => m === 'GET').length, 1, 'the first favourites answer is still on its way');
    await page.evaluate((s) => { document.querySelector(s).dataset.probe = 'same-node'; }, ROW);
    await page.locator(ROW).first().hover();
    await page.click(H1);
    await page.waitForFunction((s) => document.querySelector(s).classList.contains('on'), H1, { timeout: 4000 });
    await page.waitForFunction((s) => document.querySelector(s).classList.contains('on'), H2, { timeout: 2000 }).catch(() => {});
    const two = await heartState(page, H2);
    assert.deepEqual([two.on, two.icon, two.label], [true, 'heart-fill', `Remove ${fx.track.title} from favourites`], `the same song's other row turns over at once (${JSON.stringify(two)})`);
    await page.screenshot({ path: path.join(shots, 'album-1280-two-rows-saved.png') });
    // The old answer (no bookmarks) lands now, after the save
    node.release();
    await node.answered;
    await page.waitForTimeout(300);
    const after = [await heartState(page, H1), await heartState(page, H2)];
    assert.deepEqual(after.map((h) => h.on), [true, true], `a slow first answer does not turn the hearts out again (${JSON.stringify(after)})`);
    assert.equal(node.asks.filter((m) => m === 'PUT').length, 1, 'one click, one save');
    await page.click(`${ROW}:nth-child(2) [data-smore]`);
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
    const labels = await menuLabels(page);
    assert.ok(labels.includes('Remove from favourites'), `the other row's ⋯ menu agrees (${labels.join(' · ')})`);
    await closeMenu(page);
    assert.equal(await page.evaluate((s) => document.querySelector(s).dataset.probe, ROW), 'same-node', 'only the hearts were painted, not the list');
    // Taken out from the second row: the first turns back too
    await page.locator(`${ROW}:nth-child(2)`).hover();
    await page.click(H2);
    await page.waitForFunction((s) => !document.querySelector(s).classList.contains('on'), H1, { timeout: 4000 });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, []);
    ok('1280 px: a song in two rows turns over in both, and a slow first favourites answer does not undo the save');
  } finally { await context.close(); }
}

// 9. Library › Songs' own heart (`.track`): it says which song it is for, and bookmarks that
// arrive after the list is drawn (a cold start) fill it in without drawing the row again.
async function songsRow(browser) {
  const { page, context, state } = await fx.create(browser, 1280, { fixtures });
  try {
    const node = await favRoute(page, { start: ['track:1', fx.SONG], hold: true });
    await fx.go(page, '/tracks');
    const T = '.track [data-act="fav"]';
    await page.waitForSelector(T, { state: 'attached' });
    await page.waitForTimeout(300);
    let h = await heartState(page, T);
    assert.deepEqual([h.on, h.pressed, h.label], [false, 'false', `Add ${fx.track.title} to favourites`], `the Songs heart says which song, and that it is off (${JSON.stringify(h)})`);
    await page.evaluate((s) => { document.querySelector(s).closest('.track').dataset.probe = 'same-node'; }, T);
    node.release();
    await page.waitForFunction((s) => document.querySelector(s).classList.contains('on'), T, { timeout: 4000 }).catch(() => {});
    h = await heartState(page, T);
    assert.deepEqual([h.on, h.pressed, h.icon, h.label], [true, 'true', 'heart-fill', `Remove ${fx.track.title} from favourites`], `it fills in, pressed, once they are in (${JSON.stringify(h)})`);
    assert.equal(await page.evaluate((s) => document.querySelector(s).closest('.track').dataset.probe, T), 'same-node', 'the row was not drawn again');
    await page.locator('.track').first().hover();
    await page.click(T);
    await page.waitForFunction((s) => !document.querySelector(s).classList.contains('on'), T, { timeout: 4000 });
    assert.deepEqual(node.asks.filter((m) => m !== 'GET'), ['DELETE'], 'one click, one removal');
    h = await heartState(page, T);
    assert.deepEqual([h.pressed, h.label], ['false', `Add ${fx.track.title} to favourites`], 'off again after its own press');
    await page.screenshot({ path: path.join(shots, 'songs-1280-late.png') });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, []);
    ok('1280 px: the Songs row heart says which song it is for and fills in when the bookmarks arrive late');
  } finally { await context.close(); }
}

// 10. Now Playing's heart: bookmarks that arrive after it is drawn fill it in, and its own press
// still turns it over once per press.
const SAVED = JSON.stringify([{ id: 1, uri: 'track:1', track_id: 1, title: fx.track.title, artist: 'Sefa', album: 'Journey', duration_ms: 240000 }]);
async function nowPlaying(browser) {
  const { page, context, state } = await fx.create(browser, 1280, { fixtures });
  try {
    await context.addInitScript((saved) => { localStorage.setItem('pm.zone', 'browser'); localStorage.setItem('pm.browserQueue', saved); localStorage.setItem('pm.browserAt', JSON.stringify({ index: 0, pos: 12 })); }, SAVED);
    const node = await favRoute(page, { start: ['track:1', fx.SONG], hold: true });
    await fx.go(page, '/library');
    await page.waitForSelector('#npart');
    await page.click('#npart');
    await page.waitForSelector('#nowplaying:not([hidden]) #npfav');
    await page.waitForTimeout(500);
    const np = () => page.evaluate(() => { const b = document.querySelector('#npfav'); return b && { on: b.classList.contains('on'), pressed: b.getAttribute('aria-pressed'), label: b.getAttribute('aria-label') }; });
    assert.equal((await np()).on, false, 'the bookmarks are still on their way');
    node.release();
    await page.waitForFunction(() => document.querySelector('#npfav')?.classList.contains('on'), null, { timeout: 4000 }).catch(() => {});
    let h = await np();
    assert.deepEqual([h.on, h.pressed, h.label], [true, 'true', 'In favourites'], `Now Playing's heart fills in once they are in (${JSON.stringify(h)})`);
    // Its own press: out, then in again, one request each
    await page.click('#npfav');
    await page.waitForFunction(() => !document.querySelector('#npfav')?.classList.contains('on'), null, { timeout: 4000 });
    await page.click('#npfav');
    await page.waitForFunction(() => document.querySelector('#npfav')?.classList.contains('on'), null, { timeout: 4000 });
    await page.waitForTimeout(300);
    h = await np();
    assert.equal(h.on, true, 'pressed twice: in favourites again');
    assert.deepEqual(node.asks.filter((m) => m !== 'GET'), ['DELETE', 'PUT'], 'one request per press');
    await page.screenshot({ path: path.join(shots, 'nowplaying-1280-late.png') });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, []);
    ok('1280 px: Now Playing\'s heart fills in when the bookmarks arrive late, and its press still turns it over');
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    await mouse(browser);
    await already(browser, 1280, false, fx.SONG);
    await already(browser, 1024, true, 'track:1');
    await touchPlain(browser);
    await selecting(browser);
    await collection(browser, 1280);
    await collection(browser, 700);
    await late(browser);
    await phone(browser, 360);
    await phone(browser, 412);
    await elsewhere(browser);
    await songsRow(browser);
    await nowPlaying(browser);
    console.log('song heart: all ok');
  } finally { await browser.close(); }
})().catch((e) => { console.error(e); process.exit(1); });
