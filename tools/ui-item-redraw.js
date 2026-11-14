// An item page drawn again where it stands keeps its place: a service artist still "Getting the songs
// and albums" asks again every 3 s, a heart redraws the page, and its View all page does the same.
// The page stays on screen meanwhile (no skeleton), at the same scroll (also down in Similar, which
// comes a moment after the page), and the control that had focus has it again. A second press on the
// heart before the redraw undoes the first. Opening a page (here: the same page from a link) still
// starts at the top.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-item-redraw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/item-redraw-shots');
fs.mkdirSync(shots, { recursive: true });
const WIDTHS = [1280, 1024, 768];
const ID = 'tidal-5';
const ITEM = `/item/artist/${ID}`;
const MB = 'https://musicbrainz.org/artist/11111111-1111-1111-1111-111111111111';

// A TIDAL artist whose songs and albums are still coming (pending: the page asks again in 3 s)
const songs = Array.from({ length: 40 }, (_, i) => ({ title: `Song ${i + 1}`, artist: 'Sefa', artist_id: ID, uri: `tidal:track:${i + 1}`, duration_ms: 200000 }));
const albums = Array.from({ length: 60 }, (_, i) => ({ id: `tidal-${700 + i}`, title: `Album ${i + 1}`, year: 2024 - (i % 20), release_type: 'album', page: `https://tidal.com/album/${700 + i}` }));
const artist = (pending) => ({
  kind: 'artist', id: ID, title: 'Sefa', source: 'TIDAL', source_id: 'tidal', section: 'artists', pending, plays_from: { service: 'tidal' },
  uri: fx.SONG, target: { uri: MB }, // (the fixture node's favourites hold fx.SONG once hearted)
  songs, albums, versions: [], also_on: [], tags: [], connect: [], open_on: [], about: [], releases: [], links: [],
});

// Similar, drawn after the page from its extras (a row of artists from the library)
const similar = Array.from({ length: 12 }, (_, i) => ({ id: `lib-${90 + i}`, kind: 'artist', title: `Neighbour ${i + 1}`, subtitle: '', image: null }));
const asks = (state) => state.hits.filter((h) => h.v === ITEM && h.m === 'GET').length;
const active = (page) => page.evaluate(() => { const a = document.activeElement; return a ? (a.id ? `#${a.id}` : a.outerHTML.slice(0, 60)) : 'none'; });
const top = (page) => page.evaluate(() => document.querySelector('#main').scrollTop);
// Records a skeleton drawn into the page from now on
const watchSkeleton = (page) => page.evaluate(() => {
  window.__skel = 0;
  new MutationObserver(() => { if (document.querySelector('#main .sk-grid, #main > .skeleton')) window.__skel++; }).observe(document.querySelector('#main'), { childList: true, subtree: true });
});
const scrollTo = async (page, y) => {
  await page.evaluate((v) => { document.querySelector('#main').scrollTo({ top: v, behavior: 'instant' }); }, y);
  await page.waitForFunction((v) => Math.abs(document.querySelector('#main').scrollTop - v) < 2, y);
};
// The page's next ask (its answer comes LATE ms late) and the page drawn again from it: the old
// page stays on screen while the answer is on its way, and is then replaced (not just left there)
const LATE = 400;
const late = (v) => (v === ITEM ? LATE : v === `${ITEM}/extras` ? 2 * LATE : 0);
async function nextRedraw(page, state, where) {
  await page.evaluate(() => { window.__old = document.querySelector('#main > *'); });
  const n = asks(state);
  const t = Date.now();
  while (asks(state) < n + 1) { if (Date.now() - t > 6000) assert.fail(`${where}: the page asks again`); await page.waitForTimeout(50); }
  assert.equal(await page.evaluate(() => window.__old.isConnected), true, `${where}: the page stays on screen while it is asked again`);
  await page.waitForFunction(() => !window.__old.isConnected && document.querySelector('#iplay, .pager'), null, { timeout: 6000 })
    .catch(() => assert.fail(`${where}: the page is drawn again`));
  await page.waitForTimeout(100);
}

async function artistPage(browser, width) {
  const fixtures = { [ITEM]: { body: artist(true) }, [`${ITEM}/extras`]: { body: { collections: [], related: [], concerts: [], similar } } };
  const run = await fx.create(browser, width, { fixtures, delay: 0 });
  const { page, state } = run;
  const where = `artist ${width}`;
  try {
    await fx.go(page, `/artist/${ID}`);
    await page.locator('[data-splay="12"]').waitFor();
    await page.waitForTimeout(300);
    state.delay = late; // the page's next asks answer late, so a skeleton would show
    await watchSkeleton(page);
    // Scrolled down to the 13th song and its Play from here focused, as Tab leaves it
    await scrollTo(page, 600);
    await page.locator('[data-splay="12"]').focus({ preventScroll: true });
    await nextRedraw(page, state, where);
    assert.equal(await page.evaluate(() => window.__skel), 0, `${where}: no skeleton while the page is asked again`);
    assert.ok(Math.abs(await top(page) - 600) < 2, `${where}: the page stays scrolled where it was (${await top(page)})`);
    assert.equal(await page.locator('[data-splay="12"]').evaluate((el) => el === document.activeElement), true, `${where}: the same song's Play has focus again (${await active(page)})`);
    if (width === 1280) await page.screenshot({ path: path.join(shots, `artist-redraw-focus-${width}.png`) });
    // At the foot of the page, in Similar: Similar comes again after the page, and the place holds
    await page.locator('#isimilar [data-tile], #isimilar a, #isimilar button').first().waitFor();
    const foot = await page.evaluate(() => { const m = document.querySelector('#main'); return m.scrollHeight - m.clientHeight; });
    await scrollTo(page, foot);
    await nextRedraw(page, state, `${where} foot`);
    assert.equal(await page.locator('#isimilar').count(), 0, `${where} foot: drawn again before Similar is back (the case this checks)`);
    assert.ok(Math.abs(await top(page) - foot) < 2, `${where} foot: the page holds its place while Similar is on its way (${await top(page)} of ${foot})`);
    await page.locator('#isimilar').waitFor();
    await page.waitForTimeout(150);
    assert.ok(Math.abs(await top(page) - foot) < 2, `${where} foot: still there once Similar is back (${await top(page)} of ${foot})`);
    assert.equal(await page.evaluate(() => document.querySelector('#main').hasAttribute('data-hold')), false, `${where} foot: the page's own height once Similar is back`);
    // The songs are all in: the last redraw keeps the place too, and the page stops asking
    fixtures[ITEM].body = artist(false);
    await nextRedraw(page, state, `${where} last`);
    assert.equal(await page.locator('[data-splay="12"]').evaluate((el) => el === document.activeElement), true, `${where}: focus kept on the last redraw (${await active(page)})`);
    const settled = asks(state); await page.waitForTimeout(3400);
    assert.equal(asks(state), settled, `${where}: the page stops asking once everything is in`);
    // The heart redraws the page: it keeps focus (one Enter, no Tab back from the top). It shows
    // the new state at once, and a second press before the redraw undoes the first
    await scrollTo(page, 0);
    await page.locator('#ifav').focus();
    assert.equal(await page.locator('#ifav').getAttribute('aria-pressed'), 'false', `${where}: the heart is a toggle, off, to a screen reader too`);
    await page.evaluate(() => { window.__old = document.querySelector('#main > *'); });
    const asked = asks(state);
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => window.__old.isConnected && document.querySelector('#ifav').title === 'Remove from favourites', null, { timeout: 3000 })
      .catch(() => assert.fail(`${where}: the heart shows it is on at once`));
    assert.equal(await page.locator('#ifav').getAttribute('aria-pressed'), 'true', `${where}: and says it is pressed at once`);
    await page.keyboard.press('Enter');
    for (let t = 0; !state.hits.some((h) => h.v === '/favourites' && h.m === 'DELETE') && t < 60; t++) await page.waitForTimeout(50);
    assert.deepEqual(state.hits.filter((h) => h.v === '/favourites' && h.m !== 'GET').map((h) => h.m), ['PUT', 'DELETE'], `${where}: the second press takes the favourite off`);
    assert.ok(asks(state) > asked, `${where}: the heart draws the page again`);
    await page.waitForFunction(() => !window.__old.isConnected, null, { timeout: 6000 });
    await page.waitForTimeout(LATE + 300);
    assert.equal(await page.locator('#ifav').getAttribute('title'), 'Add to favourites', `${where}: the heart is off after both presses`);
    assert.equal(await page.locator('#ifav').getAttribute('aria-pressed'), 'false', `${where}: and not pressed`);
    assert.equal(await page.locator('#ifav').evaluate((el) => el === document.activeElement), true, `${where}: the heart keeps focus after its redraw (${await active(page)})`);
    assert.equal(await page.evaluate(() => window.__skel), 0, `${where}: no skeleton after the heart`);
    // Opening a page is unchanged: the same page from a link starts at the top
    state.delay = 0;
    await scrollTo(page, 600);
    await page.locator('[data-artist-credit]').nth(12).click();
    await page.waitForTimeout(600);
    await page.locator('#iplay').waitFor();
    assert.ok(await top(page) < 2, `${where}: a page opened from a link starts at the top (${await top(page)})`);
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `${where}: no page errors`);
  } finally { await run.context.close(); }
}

async function releasesPage(browser, width) {
  const fixtures = { [ITEM]: { body: artist(true) } };
  const run = await fx.create(browser, width, { fixtures, delay: 0 });
  const { page, state } = run;
  const where = `releases ${width}`;
  try {
    await fx.go(page, `/artist/${ID}/releases/albums`);
    await page.locator('[data-relpage="next"]').waitFor();
    await page.waitForTimeout(300);
    state.delay = late;
    await watchSkeleton(page);
    const bottom = await page.evaluate(() => { const m = document.querySelector('#main'); return m.scrollHeight - m.clientHeight; });
    await scrollTo(page, bottom);
    await page.locator('[data-relpage="next"]').focus({ preventScroll: true });
    await nextRedraw(page, state, where);
    assert.equal(await page.evaluate(() => window.__skel), 0, `${where}: no skeleton while the page is asked again`);
    assert.ok(Math.abs(await top(page) - bottom) < 2, `${where}: the page stays scrolled where it was (${await top(page)} of ${bottom})`);
    assert.equal(await page.locator('[data-relpage="next"]').evaluate((el) => el === document.activeElement), true, `${where}: Next has focus again (${await active(page)})`);
    // Next still pages from the start of the page
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => /Page 2 of/.test((document.querySelector('.pageinfo') || {}).textContent || ''));
    assert.ok(await top(page) < 2, `${where}: the next page starts at the top (${await top(page)})`);
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `${where}: no page errors`);
  } finally { await run.context.close(); }
}

// The parts drawn after the page (Similar here) when they are slow or focused: a place at the foot
// holds across two redraws while Similar takes longer than the next ask; focus on a Similar tile
// goes back once Similar is drawn again; an error drawn instead of the page ends the hold; a heart
// saved after another page opened leaves that page's heart alone.
const B = 'lib-90';
const other = { kind: 'artist', id: B, title: 'Neighbour 1', source: 'library', section: 'artists', uri: 'https://musicbrainz.org/artist/22222222-2222-2222-2222-222222222222', target: { uri: 'https://musicbrainz.org/artist/22222222-2222-2222-2222-222222222222' }, songs: [], albums: [], versions: [], also_on: [], tags: [], connect: [], open_on: [], about: [], releases: [], links: [] };
async function laterParts(browser, width) {
  const fixtures = { [ITEM]: { body: artist(true) }, [`${ITEM}/extras`]: { body: { collections: [], related: [], concerts: [], similar } }, [`/item/artist/${B}`]: { body: other }, [`/item/artist/${B}/extras`]: { body: { collections: [], related: [], concerts: [], similar: [] } } };
  const run = await fx.create(browser, width, { fixtures, delay: 0 });
  const { page, state } = run;
  const where = `later parts ${width}`;
  try {
    await fx.go(page, `/artist/${ID}`);
    await page.locator('#isimilar [data-tile="2"]').waitFor();
    await page.waitForTimeout(300);
    // Focus on the third Similar tile: back on it once Similar is drawn again
    state.delay = late;
    await page.locator('#isimilar [data-tile="2"]').focus();
    await nextRedraw(page, state, `${where} focus`);
    await page.locator('#isimilar [data-tile="2"]').waitFor();
    await page.waitForTimeout(150);
    assert.equal(await page.locator('#isimilar [data-tile="2"]').evaluate((el) => el === document.activeElement), true, `${where}: the Similar tile has focus again once Similar is back (${await active(page)})`);
    // The next answer brings singles too (a release section more above Similar): focus still waits
    // for the Similar tile, not the same tile in the new section
    fixtures[ITEM].body = { ...artist(true), albums: [...albums, ...Array.from({ length: 4 }, (_, k) => ({ id: `tidal-${800 + k}`, title: `Single ${k + 1}`, year: 2024, release_type: 'single', page: `https://tidal.com/album/${800 + k}` }))] };
    await page.locator('#isimilar [data-tile="2"]').focus();
    await nextRedraw(page, state, `${where} new section`);
    assert.ok(await page.locator('#ialbums [data-tile="2"]').count() >= 2, `${where}: a release section more (the case this checks)`);
    assert.equal(await page.evaluate(() => !!document.activeElement.closest('#ialbums')), false, `${where}: focus isn't put on a release tile (${await active(page)})`);
    await page.locator('#isimilar [data-tile="2"]').waitFor();
    await page.waitForTimeout(150);
    assert.equal(await page.locator('#isimilar [data-tile="2"]').evaluate((el) => el === document.activeElement), true, `${where}: the Similar tile has focus once Similar is back (${await active(page)})`);
    fixtures[ITEM].body = artist(true);
    await nextRedraw(page, state, `${where} section gone`);
    await page.locator('#isimilar [data-tile="2"]').waitFor();
    // Similar slower than the next ask: the foot of the page holds across both redraws
    state.delay = (v) => (v === ITEM ? LATE : v === `${ITEM}/extras` ? 4500 : 0);
    await page.evaluate(() => document.activeElement && document.activeElement.blur());
    const foot = await page.evaluate(() => { const m = document.querySelector('#main'); return m.scrollHeight - m.clientHeight; });
    await scrollTo(page, foot);
    await nextRedraw(page, state, `${where} slow 1`);
    assert.equal(await page.locator('#isimilar').count(), 0, `${where}: drawn again before Similar is back (the case this checks)`);
    await nextRedraw(page, state, `${where} slow 2`);
    assert.equal(await page.locator('#isimilar').count(), 0, `${where}: Similar still on its way at the second redraw (the case this checks)`);
    assert.ok(Math.abs(await top(page) - foot) < 2, `${where}: the place at the foot holds across a second redraw (${await top(page)} of ${foot})`);
    // Focus on a Similar tile while Similar is slower than the next ask: still wanted after a second
    // redraw, and back on the tile when Similar comes with the last one
    state.delay = 0;
    await fx.go(page, `/artist/${ID}`);
    await page.locator('#isimilar [data-tile="2"]').waitFor();
    await page.waitForTimeout(300);
    state.delay = (v) => (v === ITEM ? LATE : v === `${ITEM}/extras` ? 4500 : 0);
    await page.locator('#isimilar [data-tile="2"]').focus();
    await nextRedraw(page, state, `${where} slow focus 1`);
    await nextRedraw(page, state, `${where} slow focus 2`);
    assert.equal(await page.locator('#isimilar').count(), 0, `${where}: Similar still on its way at the second redraw (the case this checks)`);
    fixtures[ITEM].body = artist(false);
    await nextRedraw(page, state, `${where} slow focus 3`);
    await page.locator('#isimilar [data-tile="2"]').waitFor({ timeout: 6000 });
    await page.waitForTimeout(150);
    assert.equal(await page.locator('#isimilar [data-tile="2"]').evaluate((el) => el === document.activeElement), true, `${where}: the Similar tile has focus again after two slow redraws (${await active(page)})`);
    // A page that keeps asking while Similar never comes back in time: the hold still ends 20 s after
    // it began (each redraw used to start its 6 s again, so blank space could stay ~40 s)
    if (width === WIDTHS[0]) {
      fixtures[ITEM].body = artist(true);
      state.delay = 0;
      await fx.go(page, `/artist/${ID}`);
      await page.locator('#isimilar [data-tile="2"]').waitFor();
      await page.waitForTimeout(300);
      state.delay = (v) => (v === ITEM ? LATE : v === `${ITEM}/extras` ? 4500 : 0);
      await page.evaluate(() => { const m = document.querySelector('#main'); window.__held = 0; new MutationObserver(() => { if (!window.__held && m.hasAttribute('data-hold')) window.__held = Date.now(); }).observe(m, { attributes: true, attributeFilter: ['data-hold'] }); });
      await scrollTo(page, await page.evaluate(() => { const m = document.querySelector('#main'); return m.scrollHeight - m.clientHeight; }));
      await page.waitForFunction(() => window.__held, null, { timeout: 8000 }).catch(() => assert.fail(`${where}: held while Similar is on its way (the case this checks)`));
      const began = await page.evaluate(() => window.__held);
      await page.waitForFunction((b) => Date.now() > b + 20600, began, { timeout: 25000, polling: 200 });
      const late = state.hits.filter((h) => h.v === ITEM && h.m === 'GET' && h.at > began + 15000).length;
      assert.ok(late >= 1, `${where}: the page was asked again past 15 s (the case this checks)`);
      assert.equal(await page.locator('#isimilar').count(), 0, `${where}: Similar still on its way after 20 s (the case this checks)`);
      assert.equal(await page.evaluate(() => document.querySelector('#main').hasAttribute('data-hold')), false, `${where}: the hold ends 20 s after it began, however often the page is asked again`);
      state.delay = 0;
    }
    // An error drawn instead of the page ends the hold: no blank space held under the notice
    fixtures[ITEM].body = artist(true);
    state.delay = 0;
    await fx.go(page, `/artist/${ID}`);
    await page.locator('#isimilar [data-tile="2"]').waitFor();
    await page.waitForTimeout(300);
    state.delay = (v) => (v === ITEM ? LATE : v === `${ITEM}/extras` ? 4500 : 0);
    await scrollTo(page, await page.evaluate(() => { const m = document.querySelector('#main'); return m.scrollHeight - m.clientHeight; }));
    await nextRedraw(page, state, `${where} before the error`);
    assert.equal(await page.evaluate(() => document.querySelector('#main').hasAttribute('data-hold')), true, `${where}: held while Similar is on its way (the case this checks)`);
    fixtures[ITEM] = { status: 502, body: { error: 'The service did not answer' } };
    state.delay = 0;
    await page.waitForFunction(() => document.querySelector('#main .notice') && !document.querySelector('#iplay'), null, { timeout: 6000 })
      .catch(() => assert.fail(`${where}: the error is drawn`));
    assert.equal(await page.evaluate(() => document.querySelector('#main').hasAttribute('data-hold')), false, `${where}: the error notice ends the hold`);
    // A heart saved after another page opened: that page keeps its own heart and isn't drawn again
    fixtures[ITEM] = { body: artist(false) };
    await fx.go(page, `/artist/${ID}`);
    await page.locator('#isimilar [data-tile="0"]').waitFor();
    state.delay = (v, m) => (v === '/favourites' && m === 'PUT' ? 1500 : 0);
    await page.locator('#ifav').click();
    await page.waitForTimeout(100);
    await page.locator('#isimilar [data-tile="0"]').click();
    await page.waitForFunction((t) => document.querySelector('.item-hero h1')?.textContent.includes(t), other.title, { timeout: 5000 });
    const bAsks = state.hits.filter((h) => h.v === `/item/artist/${B}` && h.m === 'GET').length;
    for (let t = 0; !state.hits.some((h) => h.v === '/favourites' && h.m === 'PUT') && t < 60; t++) await page.waitForTimeout(50);
    await page.waitForTimeout(1500 + 600);
    assert.equal(await page.locator('#ifav').getAttribute('title'), 'Add to favourites', `${where}: the other page's heart stays its own`);
    assert.equal(state.hits.filter((h) => h.v === `/item/artist/${B}` && h.m === 'GET').length, bAsks, `${where}: the other page isn't drawn again by the earlier heart`);
    await fx.cleanCopy(page);
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
      await run(`${width}: artist page asked again keeps scroll and focus; the heart keeps focus`, () => artistPage(browser, width));
      await run(`${width}: View all asked again keeps scroll and focus`, () => releasesPage(browser, width));
      await run(`${width}: slow or focused Similar, an error and a late heart`, () => laterParts(browser, width));
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
