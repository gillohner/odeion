// Sorting Albums or Artists keeps the typed filter, its matching rows and keyboard focus.
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
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
