// Song and album names in song lists are real links, like the artist names beside them: Tab reaches
// them, Enter opens the page, and Ctrl+click opens it in a new tab while this page stays put;
// Alt+click is a plain click (not the browser's download).
// Covers the library's track rows (Songs) and the song rows of collections, search and item pages.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-song-links.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/song-links-shots');
fs.mkdirSync(shots, { recursive: true });
const WIDTHS = [1280, 1024, 768];
const tracks = Array.from({ length: 4 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));

// Tab from `start` until `target` has focus (at most 12 presses): the link is in the keyboard order
async function tabTo(page, start, target, where) {
  await page.locator(start).focus();
  for (let n = 0; n < 12; n++) {
    await page.keyboard.press('Tab');
    if (await page.locator(target).evaluate((el) => el === document.activeElement)) return;
  }
  assert.fail(`${where}: Tab never reaches ${target} (a link without an address is skipped)`);
}
const path0 = (page) => new URL(page.url()).pathname;
// The page is marked before a plain click or Enter; a full reload would lose the mark
const mark = (page) => page.evaluate(() => { window.__stayed = true; });
const inApp = async (page, where) => assert.equal(await page.evaluate(() => window.__stayed === true), true, `${where}: the page opens inside the app, without reloading`);
// Right-click (a long press on touch screens) on a name is still the row's menu, as before it was a link
async function rowMenu(page, link, where) {
  await page.locator(link).click({ button: 'right' });
  await page.locator('.popover.menu:not([hidden])').waitFor({ timeout: 3000 }).catch(() => assert.fail(`${where}: right-click on ${link} opens the row's menu`));
  await page.keyboard.press('Escape');
  await page.locator('.popover.menu').waitFor({ state: 'hidden' });
}

// Ctrl+click (Cmd on a Mac) opens the address in a new tab; this tab does not navigate
async function newTab(run, link, want, from, where) {
  const opened = run.context.waitForEvent('page', { timeout: 3000 }).catch(() => null);
  await run.page.locator(link).click({ modifiers: ['ControlOrMeta'] });
  const tab = await opened;
  assert.ok(tab, `${where}: Ctrl+click opens a new tab`);
  await tab.waitForURL((u) => u.pathname === want, { timeout: 5000 }).catch(() => {});
  assert.equal(new URL(tab.url()).pathname, want, `${where}: the new tab shows the same page (${tab.url()})`);
  await tab.close();
  await run.page.waitForTimeout(150);
  assert.equal(path0(run.page), from, `${where}: Ctrl+click leaves this tab where it was`);
}

async function trackRows(browser, width) {
  const run = await fx.create(browser, width, { tracks });
  const { page, state } = run;
  try {
    await fx.go(page, '/tracks');
    const link = '.track[data-i="1"] [data-act="song"]';
    await page.locator(link).waitFor();
    const href = await page.locator(link).getAttribute('href');
    assert.match(href || '', /^\/song\/[^/]+$/, `tracks ${width}: the song name has the song page's address`);
    await tabTo(page, '.track[data-i="0"] [data-act="more"]', link, `tracks ${width}`);
    if (width === 1280) await page.screenshot({ path: path.join(shots, `tracks-focus-${width}.png`) });
    await mark(page);
    await page.keyboard.press('Enter');
    await page.waitForFunction((h) => location.pathname === h, href);
    await inApp(page, `tracks ${width} Enter`);
    await page.goBack();
    await page.waitForFunction(() => location.pathname === '/tracks' && document.querySelector('.track[data-i="1"] [data-act="song"]'));
    // A plain click is still one click to the page, inside the app
    await mark(page);
    await page.locator(link).click();
    await page.waitForFunction((h) => location.pathname === h, href);
    await inApp(page, `tracks ${width} click`);
    await page.goBack();
    await page.locator(link).waitFor();
    await newTab(run, link, href, '/tracks', `tracks ${width}`);
    // Alt+click is a plain click here (Chrome and Edge would download the page instead)
    await mark(page);
    await page.locator(link).click({ modifiers: ['Alt'] });
    await page.waitForFunction((h) => location.pathname === h, href, { timeout: 3000 }).catch(() => {});
    assert.equal(path0(page), href, `tracks ${width}: Alt+click opens the song page`);
    await inApp(page, `tracks ${width} Alt+click`);
    await page.goBack();
    await page.locator(link).waitFor();
    await rowMenu(page, link, `tracks ${width}`);
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `tracks ${width}: no page errors`);
  } finally { await run.context.close(); }
}

async function collectionRows(browser, width) {
  const run = await fx.create(browser, width);
  const { page, state } = run;
  try {
    await fx.go(page, '/playlist/c1');
    const song = '[data-ssong="0"]'; const album = '[data-salbum="0"]';
    await page.locator(song).waitFor();
    assert.equal(await page.locator(song).getAttribute('href'), '/song/lib-1', `collection ${width}: the song name has the song page's address`);
    assert.equal(await page.locator(album).getAttribute('href'), '/album/lib-7', `collection ${width}: the album name has the album page's address`);
    await tabTo(page, '[data-splay="0"]', song, `collection ${width}`);
    if (width === 1280) await page.screenshot({ path: path.join(shots, `collection-focus-${width}.png`) });
    await mark(page);
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => location.pathname === '/song/lib-1');
    await inApp(page, `collection ${width} song`);
    await page.goBack();
    await page.waitForFunction(() => location.pathname === '/playlist/c1' && document.querySelector('[data-salbum="0"]'));
    await tabTo(page, song, album, `collection ${width}`);
    await mark(page);
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => location.pathname === '/album/lib-7');
    await inApp(page, `collection ${width} album`);
    await page.goBack();
    await page.waitForFunction(() => location.pathname === '/playlist/c1' && document.querySelector('[data-ssong="0"]'));
    await newTab(run, song, '/song/lib-1', '/playlist/c1', `collection ${width}`);
    await rowMenu(page, album, `collection ${width}`);
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `collection ${width}: no page errors`);
  } finally { await run.context.close(); }
}

// On phones a collection row is one button (a tap plays it; its names take no taps), so its song and
// album links are not extra Tab stops; widening the window to a web width makes them Tab stops again
async function phoneRows(browser) {
  const run = await fx.create(browser, 390);
  const { page, state } = run;
  const tabStops = () => page.locator('[data-ssong="0"], [data-salbum="0"]').evaluateAll((as) => as.map((a) => a.tabIndex));
  const shell = (s) => page.waitForFunction((want) => document.documentElement.dataset.shell === want, s);
  try {
    await fx.go(page, '/playlist/c1');
    await page.locator('[data-ssong="0"]').waitFor({ state: 'attached' });
    await shell('phone');
    assert.deepEqual(await tabStops(), [-1, -1], 'phone 390: the row is the Tab stop, not its names');
    await page.setViewportSize({ width: 1024, height: 900 });
    await page.waitForFunction(() => document.documentElement.dataset.shell !== 'phone');
    assert.deepEqual(await tabStops(), [0, 0], 'phone 390 → 1024: the names become Tab stops');
    await page.setViewportSize({ width: 390, height: 900 });
    await shell('phone');
    assert.deepEqual(await tabStops(), [-1, -1], '1024 → 390: back to one Tab stop per row');
    assert.deepEqual(state.errors, [], 'phone 390: no page errors');
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
      await run(`${width}: track rows`, () => trackRows(browser, width));
      await run(`${width}: collection rows`, () => collectionRows(browser, width));
    }
    await run('390: phone collection rows', () => phoneRows(browser));
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
