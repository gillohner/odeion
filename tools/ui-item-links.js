// The top search result's name and an item page's artist, album, "All about this song" and version
// names are real links, like song names in lists: Tab reaches them, Enter or a click opens the page
// inside the app, and Ctrl+click opens it in a new tab while this page stays put.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-item-links.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/item-links-shots');
fs.mkdirSync(shots, { recursive: true });
const WIDTHS = [1280, 1024, 768];

// A library song page with an artist, an album, a link to its page across every source, and a version
const songPage = {
  kind: 'song', id: 'lib-1', title: 'Journey Through Time', source_id: 'library', source: 'Your library',
  uri: fx.SONG, song_uri: fx.SONG, target: { track_id: 1 }, artist: { id: 'lib-11', name: 'Sefa' }, album: { id: 'lib-7', name: 'Journey' }, year: 2020,
  songs: [{ title: 'Journey Through Time', artist: 'Sefa', track_id: 1, uri: 'track:1', duration_ms: 240000 }],
  versions: [{ source: 'library', label: 'Journey Through Time (Live) · Sefa · 2021', page: 'lib-2', id: 2, uri: 'track:2' }],
  albums: [], releases: [], links: [], also_on: [], tags: [], connect: [], open_on: [],
};
// The same song where only the artist's name is known: the name searches for it
const nameOnly = { ...songPage, id: 'lib-3', artist: { name: 'Dr. Peacock' }, album: null, versions: [] };
// The pages the links lead to, so a new tab opened on their address can draw them
const plain = (kind, id, title) => ({ ...songPage, kind, id, title, target: kind === 'album' ? { album_id: 7 } : { artist_id: 11 }, artist: null, album: null, versions: [], song_uri: null });
const fixtures = {
  '/item/song/lib-1': { body: songPage },
  '/item/song/lib-3': { body: nameOnly },
  '/item/album/lib-7': { body: plain('album', 'lib-7', 'Journey') },
  '/item/artist/lib-11': { body: plain('artist', 'lib-11', 'Sefa') },
};
const searchHits = [{ kind: 'artist', id: 'lib-11', title: 'Sefa', sources: ['library'] }];

const path0 = (page) => new URL(page.url()).pathname + new URL(page.url()).search;
// The page is marked before a plain click or Enter; a full reload would lose the mark
const mark = (page) => page.evaluate(() => { window.__stayed = true; });
const inApp = async (page, where) => assert.equal(await page.evaluate(() => window.__stayed === true), true, `${where}: the page opens inside the app, without reloading`);
const focused = (page, sel) => page.locator(sel).evaluate((el) => el === document.activeElement);

// Tab from the element before the link: one press lands on it (a link without an address is skipped)
async function tabOnto(page, sel, where) {
  await page.locator(sel).evaluate((el) => {
    const all = [...document.querySelectorAll('a[href], button, input, [tabindex]')].filter((x) => x.tabIndex >= 0 && x.getClientRects().length && !x.disabled);
    const before = all.filter((x) => x !== el && (x.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING)).pop();
    (before || document.body).focus();
  });
  await page.keyboard.press('Tab');
  assert.equal(await focused(page, sel), true, `${where}: Tab reaches ${sel}`);
}
// Enter on the focused link opens `want` inside the app; Back returns to `from`
async function enterOpens(page, sel, want, from, ready, where) {
  await tabOnto(page, sel, where);
  await mark(page);
  await page.keyboard.press('Enter');
  await page.waitForFunction((w) => location.pathname + location.search === w, want, { timeout: 5000 })
    .catch(() => assert.fail(`${where}: Enter on ${sel} opens ${want} (at ${path0(page)})`));
  await inApp(page, where);
  await page.goBack();
  await page.waitForFunction(([f, r]) => location.pathname + location.search === f && document.querySelector(r), [from, ready]);
}
// Ctrl+click (Cmd on a Mac) opens the address in a new tab; this tab does not navigate
async function newTab(run, sel, want, from, where) {
  const opened = run.context.waitForEvent('page', { timeout: 3000 }).catch(() => null);
  await run.page.locator(sel).click({ modifiers: ['ControlOrMeta'] });
  const tab = await opened;
  assert.ok(tab, `${where}: Ctrl+click on ${sel} opens a new tab`);
  // The app has started in the new tab and drawn that page (an address it can't open falls back to Home)
  await tab.waitForFunction((w) => location.pathname === w && document.querySelector(w.startsWith('/search') ? '#sresults, #stop' : '#iplay'), want, { timeout: 7000 }).catch(() => {});
  assert.equal(new URL(tab.url()).pathname, want, `${where}: the new tab shows the same page (${tab.url()})`);
  assert.equal(await tab.locator(want.startsWith('/search') ? '#sresults, #stop' : '#iplay').count() > 0, true, `${where}: the new tab draws ${want}`);
  await tab.close();
  await run.page.waitForTimeout(150);
  assert.equal(path0(run.page), from, `${where}: Ctrl+click leaves this tab where it was`);
}

async function topResult(browser, width) {
  const run = await fx.create(browser, width, { searchHits, fixtures });
  const { page, state } = run;
  const where = `search ${width}`; const from = '/search?q=sefa'; const sel = '[data-sopen="top"]';
  try {
    await fx.go(page, from);
    await page.locator(sel).waitFor();
    assert.equal(await page.locator(sel).getAttribute('href'), '/artist/lib-11', `${where}: the top result's name has its page's address`);
    await tabOnto(page, sel, where);
    if (width === 1280) await page.screenshot({ path: path.join(shots, `search-top-focus-${width}.png`) });
    await enterOpens(page, sel, '/artist/lib-11', from, sel, where);
    // A plain click is still one click to the page, inside the app
    await mark(page);
    await page.locator(sel).click();
    await page.waitForFunction(() => location.pathname === '/artist/lib-11');
    await inApp(page, `${where} click`);
    await page.goBack();
    await page.locator(sel).waitFor();
    await newTab(run, sel, '/artist/lib-11', from, where);
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `${where}: no page errors`);
  } finally { await run.context.close(); }
}

async function songLinks(browser, width) {
  const run = await fx.create(browser, width, { fixtures });
  const { page, state } = run;
  const where = `song page ${width}`; const from = '/song/lib-1';
  const links = [
    ['#iartist', '/artist/lib-11'],
    ['#ialbum', '/album/lib-7'],
    ['#iglobal', `/song/mb-${fx.SONG.split('/').pop()}`],
    ['[data-vpage="0"]', '/song/lib-2'],
  ];
  try {
    await fx.go(page, from);
    await page.locator('#iartist').waitFor();
    for (const [sel, want] of links) {
      assert.equal(await page.locator(sel).getAttribute('href'), want, `${where}: ${sel} has its page's address`);
    }
    await tabOnto(page, '#iartist', where);
    if (width === 1280) await page.screenshot({ path: path.join(shots, `song-artist-focus-${width}.png`) });
    for (const [sel, want] of links) await enterOpens(page, sel, want, from, sel, `${where} ${sel}`);
    await newTab(run, '#ialbum', '/album/lib-7', from, where);
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `${where}: no page errors`);
  } finally { await run.context.close(); }
}

// Only the artist's name is known: the name is a link to a search for it, and Enter runs that search
async function nameSearch(browser, width) {
  const run = await fx.create(browser, width, { fixtures });
  const { page, state } = run;
  const where = `artist name ${width}`; const from = '/song/lib-3';
  try {
    await fx.go(page, from);
    await page.locator('#iartistq').waitFor();
    assert.equal(await page.locator('#iartistq').getAttribute('href'), '/search?q=Dr.%20Peacock', `${where}: the name has the search's address`);
    await enterOpens(page, '#iartistq', '/search?q=Dr.%20Peacock', from, '#iartistq', where);
    // The link searches everywhere, as its address says, also after a search was narrowed to Library
    // (inside the app: Enter, Library, Back to this page)
    await mark(page);
    await page.locator('#iartistq').focus();
    await page.keyboard.press('Enter');
    await page.locator('[data-scope-in="library"]').click();
    await page.waitForFunction(() => location.search.includes('in=library'));
    await page.goBack();
    await page.waitForFunction((f) => location.pathname === f && document.querySelector('#iartistq'), from);
    await inApp(page, `${where} Library search and back`);
    await enterOpens(page, '#iartistq', '/search?q=Dr.%20Peacock', from, '#iartistq', `${where} after a Library search`);
    await newTab(run, '#iartistq', '/search', from, where);
    assert.deepEqual(state.errors, [], `${where}: no page errors`);
  } finally { await run.context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0; let passed = 0;
  const run = async (label, fn) => {
    try { await fn(); passed++; console.log(`PASS ${label}`); }
    catch (e) { failed++; console.log(`FAIL ${label}: ${e.message.split('\n')[0]}`); }
  };
  try {
    for (const width of WIDTHS) {
      await run(`${width}: top search result`, () => topResult(browser, width));
      await run(`${width}: song page links`, () => songLinks(browser, width));
      await run(`${width}: artist name search`, () => nameSearch(browser, width));
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
