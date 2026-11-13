// Sorting Albums or Artists, or choosing a quality chip or Favourites on Albums, keeps the typed
// filter, its matching rows and keyboard focus; an empty Albums view says which choice emptied it.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-library-sort.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/library-sort-shots');
fs.mkdirSync(shots, { recursive: true });
const albums = Array.from({ length: 45 }, (_, i) => ({
  id: i + 1, title: i < 3 ? `Loop ${'ABC'[i]}` : `Other ${i}`,
  album_artist: i < 3 ? `Band ${'CBA'[i]}` : `Band ${i}`,
  year: i < 3 ? [2001, 2003, 2002][i] : 1990, tracks: 1,
}));
const artists = albums.map((a, i) => ({ id: a.id, name: a.title, albums: i < 3 ? [1, 3, 2][i] : 1, tracks: 3 }));
const fixtures = { '/library/albums': { body: albums }, '/library/albums/formats': { body: [] }, '/library/artists': { body: artists } };
const lists = {
  albums: { rows: '#albumgrid .tile', attr: 'data-album', sorts: { year: [2, 3, 1], artist: [3, 2, 1], name: [1, 2, 3] } },
  artists: { rows: '#artistlist [data-artist]', attr: 'data-artist', sorts: { albums: [2, 3, 1], name: [1, 2, 3] } },
};
async function ready(page, sort) {
  await page.locator('[data-lsort]').waitFor();
  assert.equal(await page.locator('[data-lsort]').inputValue(), sort, 'chosen order is shown');
}
async function focusListeners(page) {
  const session = await page.context().newCDPSession(page);
  try {
    const { result } = await session.send('Runtime.evaluate', { expression: 'document' });
    const { listeners } = await session.send('DOMDebugger.getEventListeners', { objectId: result.objectId });
    return listeners.filter((l) => l.type === 'focusin').length;
  } finally { await session.detach(); }
}
async function matches(page, kind, query, ids) {
  const { rows, attr } = lists[kind];
  assert.equal(await page.locator('#filter').inputValue(), query, `${kind}: typed filter survives sorting`);
  assert.deepEqual(await page.locator(rows).evaluateAll((els, attr) => els.map((e) => Number(e.getAttribute(attr))), attr), ids, `${kind}: only matches, in the chosen order`);
  assert.equal(await page.locator('.page-head .count').getAttribute('data-filtered'), `${ids.length} shown`, 'filtered count follows the rows');
}
async function sortMatches(browser, width, theme, kind) {
  const { page, context, state } = await fx.create(browser, width, { fixtures });
  try {
    await page.emulateMedia({ colorScheme: theme });
    await fx.go(page, `/${kind}`);
    await page.locator('#filter').waitFor();
    // A real keyboard change, followed by the other supported sort choices.
    await page.fill('#filter', '  LoOp  ');
    await page.locator('[data-lsort]').focus();
    await page.keyboard.press('End');
    await ready(page, 'added');
    await matches(page, kind, '  LoOp  ', [3, 2, 1]);
    assert.equal(await page.locator('[data-lsort]').evaluate((e) => e === document.activeElement), true, 'keyboard stays on Sort');
    for (const [sort, ids] of Object.entries(lists[kind].sorts)) {
      await page.selectOption('[data-lsort]', sort);
      await ready(page, sort);
      await matches(page, kind, '  LoOp  ', ids);
      assert.equal(await page.locator('[data-lsort]').evaluate((e) => e === document.activeElement), true, 'Sort retains focus');
    }
    await fx.cleanCopy(page);
    await page.screenshot({ path: path.join(shots, `after-${kind}-${width}-${theme}.png`) });
    // Zero matches must remain zero, including characters that cannot be HTML.
    await page.fill('#filter', '<no match>');
    await page.selectOption('[data-lsort]', 'added');
    await ready(page, 'added');
    await matches(page, kind, '<no match>', []);
    await page.locator('#filter').press('Escape');
    assert.equal(await page.locator(lists[kind].rows).count(), albums.length, 'clearing restores the full list');
    await page.selectOption('[data-lsort]', 'name');
    await ready(page, 'name');
    assert.equal(await page.locator('#filter').inputValue(), '');
    assert.equal(await page.locator(lists[kind].rows).count(), albums.length, 'unfiltered sorting still shows all rows');
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log(`PASS ${kind} ${width} ${theme}: keyboard and every sort, exact matches/count, no matches, clear, no overflow`);
  } finally { await context.close(); }
}

async function delayedSort(browser, width, focusChoice, navigateAway = false) {
  const { page, context, state } = await fx.create(browser, width, { fixtures });
  let release; const gate = new Promise((resolve) => { release = resolve; });
  let entered; const pending = new Promise((resolve) => { entered = resolve; });
  let requests = 0;
  await context.route(/\/v1\/library\/artists(?:\?|$)/, async (route) => {
    if (++requests === 2) { entered(); await gate; }
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify(artists) });
  });
  try {
    if (navigateAway === 'history') {
      await fx.go(page, '/albums');
      await page.locator('#sidebar [data-view="artists"]').click();
    } else await fx.go(page, '/artists');
    await page.fill('#filter', 'Loop');
    const beforeListeners = await focusListeners(page);
    await page.locator('[data-lsort]').focus();
    await page.selectOption('[data-lsort]', 'added');
    await pending;
    assert.equal(await page.locator('#main > .skeleton').count(), 1, 'loading is visible while sorting');
    if (navigateAway) {
      if (navigateAway === 'history') await page.goBack();
      else await page.locator('#sidebar [data-view="albums"]').click();
      await page.locator('#albumgrid .tile').first().waitFor();
      assert.equal(await focusListeners(page), beforeListeners, 'leaving a pending sort removes its focus listener, even without moving focus');
    }
    if (focusChoice !== 'sort') {
      await page.keyboard.press('/');
      assert.equal(await page.locator('#search').evaluate((e) => e === document.activeElement), true, 'Search receives keyboard focus during loading');
      if (focusChoice === 'blur') {
        await page.keyboard.press('Escape');
        assert.equal(await page.evaluate(() => document.activeElement === document.body), true, 'Escape deliberately leaves Search');
      }
    }
    release();
    if (navigateAway) {
      await page.waitForTimeout(150);
      assert.equal(new URL(page.url()).pathname, '/albums');
      assert.equal(await page.locator('#filter').inputValue(), '', 'old filter does not cross navigation');
      assert.equal(await page.locator('#albumgrid .tile').count(), albums.length, 'old results cannot replace the new page');
    } else {
      await ready(page, 'added');
      await matches(page, 'artists', 'Loop', [3, 2, 1]);
    }
    const focused = await page.evaluate(() => document.activeElement.matches('[data-lsort]') ? 'sort' : document.activeElement.id || document.activeElement.tagName);
    await page.screenshot({ path: path.join(shots, `delayed-${width}-${focusChoice}${navigateAway ? '-' + navigateAway : ''}.png`) });
    assert.equal(focused, { sort: 'sort', search: 'search', blur: 'BODY' }[focusChoice], 'sorting respects the latest focus choice, including move then blur');
    assert.equal(await focusListeners(page), beforeListeners, 'completed sort removes its focus listener');
    // Cancellation belongs to this render only: a later deliberate sort still restores focus.
    if (!navigateAway) {
      await page.locator('[data-lsort]').focus();
      await page.keyboard.press('Home');
      await ready(page, 'name');
      await matches(page, 'artists', 'Loop', [1, 2, 3]);
      assert.equal(await page.locator('[data-lsort]').evaluate((e) => e === document.activeElement), true, 'a later sort keeps its own focus');
    }
    assert.deepEqual(state.errors, []);
    console.log(`PASS delayed sort ${width} ${focusChoice}: ${navigateAway ? 'navigation wins' : 'filter restored'}, latest focus choice respected`);
  } finally { release(); await context.close(); }
}

async function sparseLists(browser) {
  for (const kind of ['albums', 'artists']) {
    const { page, context, state } = await fx.create(browser, 1280, {
      fixtures: { ...fixtures, [`/library/${kind}`]: { body: kind === 'albums' ? albums.slice(0, 3) : artists.slice(0, 3) } },
    });
    try {
      await fx.go(page, `/${kind}`);
      await page.locator('[data-lsort]').waitFor();
      assert.equal(await page.locator('#filter').count(), 0, 'small lists have no filter');
      await page.locator('[data-lsort]').focus();
      await page.keyboard.press('End');
      await ready(page, 'added');
      assert.equal(await page.locator(lists[kind].rows).count(), 3);
      assert.equal(await page.locator('[data-lsort]').evaluate((e) => e === document.activeElement), true);
      assert.deepEqual(state.errors, []);
      console.log(`PASS ${kind}: sorting a small list without a filter`);
    } finally { await context.close(); }
  }
  const { page, context, state } = await fx.create(browser, 1280, { fixtures });
  try {
    await context.addInitScript(() => localStorage.setItem('pm.list.albums', JSON.stringify({ fav: true })));
    await fx.go(page, '/albums');
    await page.locator('#filter').waitFor();
    assert.equal(await page.locator('#albumgrid').count(), 0, 'no bookmarked albums');
    await page.locator('[data-lsort]').focus();
    await page.keyboard.press('End');
    await ready(page, 'added');
    assert.deepEqual(state.errors, [], 'a filter without matching albums has no input handler');
    assert.equal(await page.locator('#albumgrid').count(), 0);
    console.log('PASS sorting an empty bookmarked-albums view');
  } finally { await context.close(); }
}

// Albums' quality chips and Favourites narrow the typed filter's matches instead of clearing it,
// and keyboard focus stays on the chip that was chosen.
async function albumChips(browser, width) {
  const mbid = (id) => `00000000-0000-4000-8000-${String(id).padStart(12, '0')}`;
  const tagged = albums.map((a) => ({ ...a, release_mbid: mbid(a.id) }));
  const formats = tagged.map((a) => ({ album_id: a.id, lossless: [1, 2, 4, 5].includes(a.id), hi_res: false, dsd: false }));
  const { page, context, state } = await fx.create(browser, width, {
    fixtures: { ...fixtures, '/library/albums': { body: tagged }, '/library/albums/formats': { body: formats } },
    favs: [2, 5].map((id) => `https://musicbrainz.org/release/${mbid(id)}`),
  });
  const chosen = async (selector, ids, how) => {
    if (how === 'keyboard') { await page.locator(selector).focus(); await page.keyboard.press('Enter'); } else await page.locator(selector).click();
    await page.waitForFunction((n) => document.querySelectorAll('#albumgrid .tile').length === n && document.querySelector('.page-head .count')?.dataset.filtered === `${n} shown`, ids.length);
    await matches(page, 'albums', 'loop', ids);
    assert.equal(await page.locator(selector).evaluate((e) => e === document.activeElement), true, `${selector}: focus stays on the chosen chip`);
  };
  try {
    await fx.go(page, '/albums');
    await page.locator('#albumgrid .tile').first().waitFor();
    await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length > 3);
    await page.fill('#filter', 'loop');
    await matches(page, 'albums', 'loop', [1, 2, 3]);
    await chosen('[data-focus="lossless"]', [1, 2], 'pointer');
    assert.equal(await page.locator('[data-focus="lossless"]').evaluate((e) => e.classList.contains('on')), true, 'Lossless is the chosen quality');
    await chosen('[data-lfav]', [2], 'keyboard');
    assert.equal(await page.locator('[data-lfav]').getAttribute('aria-pressed'), 'true');
    if (width === 1280) { await fx.cleanCopy(page); await page.screenshot({ path: path.join(shots, `after-album-chips-${width}.png`) }); }
    await chosen('[data-lfav]', [1, 2], 'keyboard');
    await chosen('[data-focus=""]', [1, 2, 3], 'keyboard');
    await page.locator('#filter').press('Escape');
    assert.equal(await page.locator('#albumgrid .tile').count(), albums.length, 'clearing restores every album');
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log(`PASS album chips ${width}: Lossless and Favourites keep the typed filter and keyboard place`);
  } finally { await context.close(); }
}

// Albums left empty by Favourites (or Favourites and a quality) say so, not "no file of that
// quality"; the filter is off meanwhile but keeps its text, which applies again on the way back.
async function emptyAlbums(browser, width) {
  const mbid = (id) => `00000000-0000-4000-8000-${String(id).padStart(12, '0')}`;
  const tagged = albums.map((a) => ({ ...a, release_mbid: mbid(a.id) }));
  const formats = tagged.map((a) => ({ album_id: a.id, lossless: [1, 2, 4, 5].includes(a.id), hi_res: false, dsd: false }));
  const run = async (favs, steps, title, text) => {
    const { page, context, state } = await fx.create(browser, width, {
      fixtures: { ...fixtures, '/library/albums': { body: tagged }, '/library/albums/formats': { body: formats } },
      favs: favs.map((id) => `https://musicbrainz.org/release/${mbid(id)}`),
    });
    try {
      await fx.go(page, '/albums');
      await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length > 3);
      await page.fill('#filter', 'loop');
      for (const sel of steps) { await page.locator(sel).click(); await page.waitForFunction((s) => document.querySelector(s)?.getAttribute('aria-pressed') === 'true', sel); }
      await page.locator('.empty h3').waitFor();
      assert.equal(await page.locator('.empty h3').innerText(), title, `${width}: the empty state names the choice`);
      assert.equal(await page.locator('.empty p').innerText(), text);
      assert.equal(await page.locator('#filter').inputValue(), 'loop', 'the typed filter is kept');
      assert.equal(await page.locator('#filter').isDisabled(), true, 'nothing to filter: the box is off');
      if (width === 1280 && steps.length === 1) { await fx.cleanCopy(page); await page.screenshot({ path: path.join(shots, `after-albums-fav-empty-${width}.png`) }); }
      await page.locator('[data-lfav]').click();
      await page.waitForFunction(() => document.querySelector('[data-lfav]')?.getAttribute('aria-pressed') === 'false' && document.querySelector('#albumgrid .tile'));
      assert.equal(await page.locator('#filter').isDisabled(), false, 'the filter works again');
      await page.waitForFunction(() => document.querySelector('.page-head .count')?.dataset.filtered?.endsWith(' shown'));
      assert.equal(await page.locator('#filter').inputValue(), 'loop');
      assert.equal(await page.locator('#albumgrid .tile').count(), steps.length === 1 ? 3 : 2, 'the kept filter applies again');
      assert.deepEqual(state.errors, [], 'no page errors');
    } finally { await context.close(); }
  };
  const none = ['No favourite albums', 'Open an album and press its heart to see it here.'];
  await run([], ['[data-lfav]'], ...none);
  await run([3], ['[data-focus="lossless"]', '[data-lfav]'], 'No favourite albums of this quality', 'None of your favourite albums has a file of this quality.');
  // No favourite at all: Favourites is what empties it, whatever the quality
  await run([], ['[data-focus="lossless"]', '[data-lfav]'], ...none);
  console.log(`PASS empty albums ${width}: Favourites and Favourites + quality say why, the filter keeps its text`);
}

// An album hearted before MusicBrainz knew it is kept on the node as album:<id>; Favourites shows it
async function nodeFavAlbum(browser) {
  const { page, context, state } = await fx.create(browser, 1280, { fixtures, favs: ['album:2'] });
  try {
    await fx.go(page, '/albums');
    await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length > 3);
    await page.locator('[data-lfav]').click();
    await page.waitForFunction(() => document.querySelector('[data-lfav]')?.getAttribute('aria-pressed') === 'true' && document.querySelectorAll('#albumgrid .tile').length === 1);
    assert.equal(await page.locator('#albumgrid .tile').getAttribute('data-album'), '2', 'the node-kept favourite is listed');
    assert.equal(await page.locator('.page-head .count').innerText(), '1 of 45');
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log('PASS Favourites lists an album hearted before MusicBrainz knew it');
  } finally { await context.close(); }
}

// Favourites chosen on Albums, on a cold start: the list waits for the favourites to arrive instead
// of saying "No favourite albums". A heart on an album's release group (the album page, Discover
// and New releases heart the group) lists it too, like a heart on its release.
async function coldFavourites(browser, width) {
  const mbid = (id) => `00000000-0000-4000-8000-${String(id).padStart(12, '0')}`;
  const group = (id) => `11111111-0000-4000-8000-${String(id).padStart(12, '0')}`;
  const tagged = albums.map((a) => ({ ...a, release_mbid: mbid(a.id), release_group_mbid: group(a.id) }));
  const { page, context, state } = await fx.create(browser, width, {
    fixtures: { ...fixtures, '/library/albums': { body: tagged } },
    favs: [`https://musicbrainz.org/release/${mbid(2)}`, `https://musicbrainz.org/release-group/${group(5)}`],
    delay: (v) => (v === '/favourites' ? 700 : 0),
  });
  await context.addInitScript(() => localStorage.setItem('pm.list.albums', JSON.stringify({ fav: true })));
  try {
    await fx.go(page, '/albums');
    await page.waitForFunction(() => document.querySelector('#albumgrid .tile, .empty h3'));
    assert.equal(await page.locator('.empty').count(), 0, `${width}: no "No favourite albums" while the favourites are on their way`);
    assert.equal(await page.locator('[data-lfav]').getAttribute('aria-pressed'), 'true', 'Favourites is still chosen');
    assert.deepEqual(await page.locator('#albumgrid .tile').evaluateAll((els) => els.map((e) => Number(e.getAttribute('data-album')))), [2, 5], `${width}: the release's and the release group's hearts are both listed`);
    assert.equal(await page.locator('.page-head .count').innerText(), '2 of 45');
    assert.equal(await page.locator('#filter').isDisabled(), false, 'the filter works');
    if (width === 1280) { await fx.cleanCopy(page); await page.screenshot({ path: path.join(shots, `after-albums-fav-cold-${width}.png`) }); }
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log(`PASS cold Favourites ${width}: waits for the favourites, lists release-group hearts`);
  } finally { await context.close(); }
}

// Favourites that take long (a listener's first go to their homeserver): the page doesn't sit on
// its loading placeholder; after 3 s it says the favourites are on their way (not "No favourite
// albums"), and lists them once they are in
async function slowFavourites(browser, width) {
  const mbid = (id) => `00000000-0000-4000-8000-${String(id).padStart(12, '0')}`;
  const tagged = albums.map((a) => ({ ...a, release_mbid: mbid(a.id) }));
  const { page, context, state } = await fx.create(browser, width, {
    fixtures: { ...fixtures, '/library/albums': { body: tagged } },
    favs: [`https://musicbrainz.org/release/${mbid(3)}`],
    delay: (v) => (v === '/favourites' ? 5500 : 0),
  });
  await context.addInitScript(() => localStorage.setItem('pm.list.albums', JSON.stringify({ fav: true })));
  try {
    const t = Date.now();
    await fx.go(page, '/albums');
    await page.waitForFunction(() => document.querySelector('.empty h3'), null, { timeout: 4500 })
      .catch(() => assert.fail(`${width}: the page is drawn after about 3 s, not left loading`));
    assert.ok(Date.now() - t < 5000, `${width}: drawn before the favourites are in`);
    assert.equal(await page.locator('.empty h3').innerText(), 'Getting your favourites…', `${width}: says they are on their way, not "No favourite albums"`);
    // No count beside the note until they are in ("0 of 45" said none were hearted)
    assert.equal(await page.locator('.page-head .count').count(), 0, `${width}: no count while the favourites are on their way, not "0 of 45"`);
    if (width === 1280) { await fx.cleanCopy(page); await page.screenshot({ path: path.join(shots, `after-albums-fav-slow-${width}.png`) }); }
    // Another order chosen meanwhile is drawn at once (it doesn't wait another 3 s)
    const t2 = Date.now();
    await page.selectOption('[data-lsort]', 'year');
    await page.waitForFunction(() => document.querySelector('[data-lsort]')?.value === 'year' && document.querySelector('#albumswait'), null, { timeout: 1500 })
      .catch(() => assert.fail(`${width}: a new order is drawn at once while the favourites are on their way`));
    assert.ok(Date.now() - t2 < 1500, `${width}: no second wait`);
    assert.equal(await page.locator('.page-head .count').count(), 0, `${width}: still no count after another order is chosen`);
    await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length === 1, null, { timeout: 6000 })
      .catch(() => assert.fail(`${width}: listed once the favourites are in`));
    assert.deepEqual(await page.locator('#albumgrid .tile').evaluateAll((els) => els.map((e) => Number(e.getAttribute('data-album')))), [3]);
    assert.equal(await page.locator('.empty').count(), 0);
    assert.equal(await page.locator('.page-head .count').innerText(), '1 of 45', `${width}: the count says how many are hearted once they are in`);
    // Drawn again once, not in a loop: the list stays as it is
    await page.evaluate(() => { window.__grid = document.querySelector('#albumgrid'); });
    await page.waitForTimeout(3500);
    assert.equal(await page.evaluate(() => window.__grid.isConnected), true, `${width}: drawn again once, not in a loop`);
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log(`PASS slow Favourites ${width}: drawn after 3 s with a note, listed when they come`);
  } finally { await context.close(); }
}

// A tile's Play sits on its own cover and plays that album (it used to be placed against the page,
// all tiles' Plays stacked in one spot at the window's corner, and a click there played the last
// album). On a touch screen, with no hover, the Play shows at a 44 px target; with a mouse it shows on
// hover only. A real click or tap (not a forced one) is what proves where it is.
async function tilePlay(browser, width, mobile) {
  const songsOf = Object.fromEntries(albums.map((a) => [`/library/albums/${a.id}/tracks`, { body: { tracks: [{ ...fx.track, id: 1000 + a.id, album: { id: a.id, title: a.title } }] } }]));
  const { page, context, state } = await fx.create(browser, width, { mobile, fixtures: { ...fixtures, ...songsOf } });
  const at = `${width}${mobile ? ' touch' : ''}`;
  try {
    await fx.go(page, '/albums');
    await page.waitForFunction((n) => document.querySelectorAll('#albumgrid .tile').length === n, albums.length);
    const tiles = await page.locator('#albumgrid .tile').evaluateAll((els) => els.slice(0, 6).map((t) => {
      const plays = t.querySelectorAll('.tile-play'), p = plays[0], art = t.querySelector('.art');
      const r = p.getBoundingClientRect(), a = art.getBoundingClientRect();
      return { id: Number(t.dataset.album), plays: plays.length, onCover: p.parentElement === art && r.left >= a.left && r.right <= a.right && r.top >= a.top && r.bottom <= a.bottom, w: r.width, h: r.height, opacity: Number(getComputedStyle(p).opacity) };
    }));
    for (const t of tiles) assert.ok(t.plays === 1 && t.onCover, `${at}: album ${t.id}'s Play sits on its own cover (${JSON.stringify(t)})`);
    if (mobile) assert.ok(tiles.every((t) => t.opacity === 1 && t.w >= 44 && t.h >= 44), `${at}: Play shown at 44 px without hover (${JSON.stringify(tiles[0])})`);
    else assert.ok(tiles.every((t) => t.opacity === 0), `${at}: Play hidden until hover`);
    const tile = page.locator('#albumgrid .tile').nth(3), want = tiles[3].id;
    await tile.scrollIntoViewIfNeeded();
    if (!mobile) await tile.hover();
    const box = await tile.locator('.tile-play').boundingBox();
    const x = box.x + box.width / 2, y = box.y + box.height / 2;
    assert.equal(await page.evaluate(([x, y]) => document.elementFromPoint(x, y)?.closest('.tile')?.dataset.album, [x, y]), String(want), `${at}: nothing covers the 4th album's Play`);
    const n0 = state.hits.length;
    if (mobile) await page.touchscreen.tap(x, y); else await page.mouse.click(x, y);
    const asked = () => state.hits.slice(n0).map((h) => h.v).filter((v) => /^\/library\/albums\/\d+\/tracks$/.test(v));
    for (let i = 0; i < 30 && !asked().length; i++) await page.waitForTimeout(100);
    await page.waitForTimeout(300);
    assert.deepEqual(asked(), [`/library/albums/${want}/tracks`], `${at}: the Play on the 4th tile plays that album`);
    assert.equal(new URL(page.url()).pathname, '/albums', `${at}: playing from a tile stays on Albums`);
    if (width === 360 || width === 1280) await page.screenshot({ path: path.join(shots, `after-albums-tile-play-${width}.png`) });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log(`PASS tile Play ${at}: on its own cover, plays its album`);
  } finally { await context.close(); }
}

// Other tile grids (Home's See all, tag and item pages) keep their own Play: the library-album Play
// binding that runs after every page used to take theirs over and ask for a library album by the
// tile's place in the list. Now that the Play shows on a touch screen this matters on a phone.
async function seeAllTilePlay(browser, width, mobile) {
  const items = [0, 1, 2].map((i) => ({ kind: 'album', uri: `https://musicbrainz.org/release-group/0000000${i}-0000-4000-8000-000000000000`, title: `Trending ${i}`, artist: 'Sefa' }));
  const { page, context, state } = await fx.create(browser, width, { mobile, fixtures: { ...fixtures, '/home/trending-albums': { body: { section: 'trending-albums', items, tags: [] } } } });
  const at = `${width}${mobile ? ' touch' : ''}`;
  try {
    await fx.go(page, '/home/trending-albums');
    const tile = page.locator('#hsfull .tile').nth(1);
    await tile.waitFor();
    await tile.scrollIntoViewIfNeeded();
    if (!mobile) await tile.hover();
    const box = await tile.locator('.tile-play').boundingBox();
    const n0 = state.hits.length;
    if (mobile) await page.touchscreen.tap(box.x + box.width / 2, box.y + box.height / 2); else await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    const asked = () => state.hits.slice(n0).filter((h) => h.v === '/discover/play' || /^\/library\/albums\/\d+\/tracks$/.test(h.v));
    for (let i = 0; i < 30 && !asked().length; i++) await page.waitForTimeout(100);
    await page.waitForTimeout(300);
    const got = asked().map((h) => h.v + (h.v === '/discover/play' ? decodeURIComponent(h.q) : ''));
    assert.ok(got.length && got.every((v) => v.startsWith('/discover/play') && v.includes(items[1].uri)), `${at}: See all's 2nd Play plays that album, not a library album (${JSON.stringify(got)})`);
    console.log(`PASS See all tile Play ${at}: plays its own item`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 360, 412]) for (const theme of ['light', 'dark']) {
      for (const kind of ['albums', 'artists']) await sortMatches(browser, width, theme, kind);
    }
    for (const width of [1280, 1024, 768]) {
      for (const focusChoice of ['blur', 'search', 'sort']) await delayedSort(browser, width, focusChoice);
    }
    await delayedSort(browser, 1280, 'search', 'pointer');
    await delayedSort(browser, 1280, 'search', 'history');
    await sparseLists(browser);
    for (const width of [1280, 1024, 768, 412]) await albumChips(browser, width);
    for (const width of [1280, 1024, 768, 412]) await emptyAlbums(browser, width);
    await nodeFavAlbum(browser);
    for (const width of [1280, 1024, 768]) await coldFavourites(browser, width);
    for (const width of [1280, 768]) await slowFavourites(browser, width);
    for (const width of [1280, 1024, 768]) await tilePlay(browser, width, false);
    for (const width of [360, 412]) await tilePlay(browser, width, true);
    await seeAllTilePlay(browser, 1280, false);
    await seeAllTilePlay(browser, 360, true);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
