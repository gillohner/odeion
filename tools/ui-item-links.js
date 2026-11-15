// The top search result's name and an item page's artist, album, "All about this song" and version
// names are real links, like song names in lists: Tab reaches them, Enter or a click opens the page
// inside the app, and Ctrl+click opens it in a new tab while this page stays put. So is "Connect
// TIDAL" for something none of your services has (an album page, a Favourites row), to TIDAL's tab.
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
// An album that none of the connected services has: its songs section offers Connect TIDAL
const NOWHERE = 'https://musicbrainz.org/release-group/5b11f4ce-a62d-471e-81fc-a69a8278c7da';
fixtures['/item/album/mb-nowhere'] = { body: { ...plain('album', 'mb-nowhere', 'Nowhere Yet'), source_id: 'musicbrainz', source: 'MusicBrainz', uri: NOWHERE, target: { uri: NOWHERE }, plays_from: null, songs: [], connect: ['tidal'] } };
// Favourites › From other services: a song no connected service has, with the same Connect link in its row
const OTHER = 'https://tidal.com/browse/track/77';
fixtures['/favourites'] = { body: { uris: [OTHER], tracks: [], albums: [], artists: [], other: [OTHER] } };
fixtures['/discover/describe'] = { body: [{ uri: OTHER, page: OTHER, title: 'Far Away Song', artist: 'Someone Else', kind: 'track', site: null, also_on: [], connect: ['tidal'] }] };
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
const readyFor = (w) => (w.startsWith('/search') ? '#sresults, #stop' : w.startsWith('/sources') ? '.set-tab[aria-current="page"]' : '#iplay');
async function newTab(run, sel, want, from, where) {
  const opened = run.context.waitForEvent('page', { timeout: 3000 }).catch(() => null);
  await run.page.locator(sel).click({ modifiers: ['ControlOrMeta'] });
  const tab = await opened;
  assert.ok(tab, `${where}: Ctrl+click on ${sel} opens a new tab`);
  // The app has started in the new tab and drawn that page (an address it can't open falls back to Home)
  await tab.waitForFunction(([w, r]) => location.pathname === w && document.querySelector(r), [want, readyFor(want)], { timeout: 7000 }).catch(() => {});
  assert.equal(new URL(tab.url()).pathname, want, `${where}: the new tab shows the same page (${tab.url()})`);
  assert.equal(await tab.locator(readyFor(want)).count() > 0, true, `${where}: the new tab draws ${want}`);
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

// "Connect TIDAL" for something none of your services has is a real link to Sources: Tab reaches it,
// Enter or a click opens it inside the app, Ctrl+click opens it in a new tab. With one service
// named it opens that service's tab (/sources/tidal), with more ("TIDAL or Spotify") the Sources
// page. In a Favourites row a click on it opens Sources only, not also the row's page; the row's
// Play menu offers the same Connect TIDAL, also to TIDAL's tab.
const EITHER = 'https://musicbrainz.org/release-group/0c4a7a3c-5d44-4b8c-9c6e-3a3c1a1d2e01';
const connectFixtures = {
  ...fixtures,
  '/item/album/mb-either': { body: { ...plain('album', 'mb-either', 'Either Way'), source_id: 'musicbrainz', source: 'MusicBrainz', uri: EITHER, target: { uri: EITHER }, plays_from: null, songs: [], connect: ['tidal', 'spotify'] } },
  '/sources': { body: [
    { id: 'tidal', name: 'TIDAL', description: '', status: 'ready', enabled: true },
    { id: 'spotify', name: 'Spotify', description: '', status: 'ready', enabled: true },
  ] },
  '/discover/play': { body: { uri: OTHER, plays_from: null, items: [], track_ids: [], connect: ['tidal'], reason: 'Not on your connected services' } },
};
// On Sources, the tab that is open (TIDAL's, or another one when no single service was named)
const openTab = (page) => page.locator('.set-tab[aria-current="page"]').getAttribute('id');
async function connectLinks(browser, width) {
  const run = await fx.create(browser, width, { fixtures: connectFixtures });
  const { page, state } = run;
  try {
    for (const [from, box, label, want, where] of [
      ['/album/mb-nowhere', '#isongs', 'Connect TIDAL', '/sources/tidal', `album page Connect ${width}`],
      ['/favourites', '#favother', 'Connect TIDAL', '/sources/tidal', `Favourites row Connect ${width}`],
      ['/album/mb-either', '#isongs', 'Connect TIDAL or Spotify', '/sources', `album page Connect two ${width}`],
    ]) {
      const sel = `${box} a[href="${want}"]`;
      await fx.go(page, from);
      await page.locator(box).getByText(label).waitFor();
      const link = page.locator(box).getByText(label);
      assert.equal(await link.count(), 1, `${where}: the ${label} link is shown`);
      assert.equal(await link.getAttribute('href'), want, `${where}: ${label} has the address ${want}`);
      await tabOnto(page, sel, where);
      if (width === 1280) await page.screenshot({ path: path.join(shots, `connect-focus-${from.split('/').pop()}-${width}.png`) });
      await enterOpens(page, sel, want, from, sel, where);
      await mark(page);
      await page.locator(sel).click();
      await page.waitForFunction((w) => location.pathname === w && document.querySelector('.set-tab[aria-current="page"]'), want);
      await inApp(page, `${where} click`);
      if (want === '/sources/tidal') {
        assert.equal(await openTab(page), 'srctab-tidal', `${where}: Sources opens on TIDAL's tab`);
        if (width === 1280 && box === '#isongs') await page.screenshot({ path: path.join(shots, `connect-opens-tidal-${width}.png`) });
      }
      await page.goBack();
      await page.waitForFunction((f) => location.pathname === f, from);
      await page.locator(sel).waitFor();
      await newTab(run, sel, want, from, where);
    }
    // The Favourites row's Play, for a song no connected service has: its menu's Connect TIDAL opens TIDAL's tab
    await fx.go(page, '/favourites');
    await page.locator('#favother [data-dplay]').first().click();
    const item = page.locator('.pop-item', { hasText: 'Connect TIDAL' });
    await item.waitFor();
    await mark(page);
    await item.click();
    await page.waitForFunction(() => location.pathname === '/sources/tidal' && document.querySelector('.set-tab[aria-current="page"]'))
      .catch(() => assert.fail(`Play menu Connect ${width}: opens /sources/tidal (at ${path0(page)})`));
    await inApp(page, `Play menu Connect ${width}`);
    assert.equal(await openTab(page), 'srctab-tidal', `Play menu Connect ${width}: Sources opens on TIDAL's tab`);
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `Connect ${width}: no page errors`);
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
      await run(`${width}: Connect a service`, () => connectLinks(browser, width));
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
