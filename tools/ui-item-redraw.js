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
    await page.evaluate(() => { window.__old = document.querySelector('#main > *'); });
    const asked = asks(state);
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => window.__old.isConnected && document.querySelector('#ifav').title === 'Remove from favourites', null, { timeout: 3000 })
      .catch(() => assert.fail(`${where}: the heart shows it is on at once`));
    await page.keyboard.press('Enter');
    for (let t = 0; !state.hits.some((h) => h.v === '/favourites' && h.m === 'DELETE') && t < 60; t++) await page.waitForTimeout(50);
    assert.deepEqual(state.hits.filter((h) => h.v === '/favourites' && h.m !== 'GET').map((h) => h.m), ['PUT', 'DELETE'], `${where}: the second press takes the favourite off`);
    assert.ok(asks(state) > asked, `${where}: the heart draws the page again`);
    await page.waitForFunction(() => !window.__old.isConnected, null, { timeout: 6000 });
    await page.waitForTimeout(LATE + 300);
    assert.equal(await page.locator('#ifav').getAttribute('title'), 'Add to favourites', `${where}: the heart is off after both presses`);
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
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
