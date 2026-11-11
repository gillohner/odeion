// Phone Library navigation: the current section (or focused tab on resize) stays in view, with 44 px targets.
// Real app assets, isolated API fixtures; no live accounts or devices touched.
// NODE_PATH=/path/to/node_modules node tools/ui-phone-library-nav.js [repo root] [shots dir]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = process.argv[3];
if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, label) => { console.log(`${ok ? 'ok' : 'FAIL'} ${label}`); if (!ok) failed++; };
const settle = (page) => page.waitForTimeout(250);
const geometry = (page, section) => page.evaluate((section) => {
  const row = document.querySelector('.libnav');
  const chip = row?.querySelector(section ? `[data-libnav="${section}"]` : '[aria-current="page"]');
  if (!row || !chip || !row.clientWidth) return null;
  const r = row.getBoundingClientRect(), c = chip.getBoundingClientRect();
  // The right-hand fade is not a readable part of the row.
  const edge = r.right - (row.classList.contains('more-right') ? 28 : 0);
  return { visible: c.left >= r.left - 1 && c.right <= edge + 1, x: c.x + c.width / 2, y: c.y + c.height / 2,
    left: row.scrollLeft, current: row.querySelector('[aria-current="page"]')?.dataset.libnav,
    targets: [...row.children].every((e) => e.getBoundingClientRect().height >= 44 && e.getBoundingClientRect().width >= 44) };
}, section);
const tap = async (page, section) => {
  const g = await geometry(page, section);
  check(g?.visible, `${section} can be tapped without scrolling the navigation`);
  if (!g?.visible) return false;
  // A coordinate tap cannot silently scroll the target into view like locator.click().
  await page.touchscreen.tap(g.x, g.y);
  await page.waitForURL(`**/${section}`);
  await settle(page);
  return true;
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) {
      const label = `${width} ${android ? 'Android' : 'browser'}`;
      const { context, page, state } = await fx.create(browser, width, { mobile: true, collections: [fx.summary], favs: [fx.SONG] });
      await context.addInitScript((android) => {
        if (android) window.pmDesktop = { platform: 'android', version: 'fixture' };
        localStorage.setItem('pm.theme', android ? 'dark' : 'light');
      }, android);
      try {
        await fx.go(page, '/favourites');
        await page.waitForSelector('.libnav [aria-current="page"]');
        await settle(page);
        let g = await geometry(page);
        check(g?.visible && g.current === 'favourites', `${label}: deep link shows the selected Bookmarks tab`);
        check(g?.targets, `${label}: every Library tab is at least 44 by 44 px`);
        await fx.cleanCopy(page);
        if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `bookmarks-${width}-${android ? 'android' : 'browser'}.png`) });

        await page.reload();
        await page.waitForSelector('.libnav [aria-current="page"]');
        await settle(page);
        check((await geometry(page))?.visible, `${label}: reload keeps the current tab visible`);

        if (await tap(page, 'playlists')) {
          g = await geometry(page);
          check(g?.visible && g.current === 'playlists', `${label}: Collections stays selected and visible`);
          if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `collections-${width}-${android ? 'android' : 'browser'}.png`) });
          if (await tap(page, 'favourites')) {
            await page.goBack();
            await page.waitForURL('**/playlists');
            await settle(page);
            check((await geometry(page))?.current === 'playlists' && (await geometry(page))?.visible, `${label}: Back restores the visible current tab`);
          }
        }

        await fx.go(page, '/sources');
        await page.waitForSelector('.libnav [aria-current="page"]');
        await settle(page);
        check((await geometry(page))?.visible, `${label}: the last Library section is visible`);
        // A redraw elsewhere must not pull the navigation away from a person's swipe,
        // or scroll the page vertically to bring the row into view.
        await page.evaluate(() => {
          document.querySelector('.libnav').scrollLeft = 0;
          const spacer = document.createElement('div'); spacer.style.height = '2000px';
          document.querySelector('#main').appendChild(spacer);
        });
        await settle(page);
        await page.evaluate(() => { document.querySelector('#main').scrollTop = 200; document.querySelector('#main h2').append(' '); });
        await settle(page);
        check((await geometry(page))?.left === 0, `${label}: unrelated updates preserve a manual horizontal scroll`);
        check(await page.$eval('#main', (e) => e.scrollTop) === 200, `${label}: unrelated updates preserve the page's vertical scroll`);

        await page.setViewportSize({ width: width === 360 ? 412 : 360, height: 900 });
        await settle(page);
        check((await geometry(page))?.visible, `${label}: resizing reveals the current tab again`);
        check(await page.$eval('#main', (e) => e.scrollTop) === 200, `${label}: revealing the tab does not scroll the page`);
        await fx.cleanCopy(page);

        await page.setViewportSize({ width, height: 900 });
        await fx.go(page, '/sources');
        await page.waitForSelector('.libnav [aria-current="page"]');
        await settle(page);
        // Use the keyboard, not locator.focus/click (which can scroll for us).
        const focusedLibrary = () => page.evaluate(() => document.activeElement?.dataset.libnav === 'library');
        for (let i = 0; i < 30 && !await focusedLibrary(); i++) await page.keyboard.press('Tab');
        check(await focusedLibrary() && (await geometry(page, 'library'))?.visible, `${label}: Tab reaches a visible Your library button`);
        await page.setViewportSize({ width: width === 360 ? 412 : 360, height: 900 });
        await settle(page);
        check(await focusedLibrary(), `${label}: resizing retains keyboard focus on Your library`);
        check((await geometry(page, 'library'))?.visible, `${label}: resizing keeps the entire focused button visible`);
        check((await geometry(page))?.current === 'sources', `${label}: resizing does not change the selected section`);
        if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `focused-resize-${width}-${android ? 'android' : 'browser'}.png`) });
        await fx.cleanCopy(page);

        // Keeping focus visible on resize must not turn into recentering on every redraw.
        const manualLeft = await page.$eval('.libnav', (e) => { e.scrollLeft = e.scrollWidth; return e.scrollLeft; });
        await page.$eval('#main h2', (e) => e.append(' '));
        await settle(page);
        check((await geometry(page))?.left === manualLeft && await focusedLibrary(), `${label}: unrelated updates preserve manual scrolling even with a focused tab`);
        await page.setViewportSize({ width, height: 900 });
        await settle(page);
        check(await focusedLibrary() && (await geometry(page, 'library'))?.visible, `${label}: resizing back reveals the focused button again`);
        await page.keyboard.press('Enter');
        await page.waitForURL('**/library');
        await settle(page);
        check((await geometry(page))?.current === 'library' && (await geometry(page))?.visible, `${label}: Enter opens the visible focused section`);

        await fx.go(page, '/library');
        await page.waitForSelector('.libnav [aria-current="page"]');
        await settle(page);
        check((await geometry(page))?.visible && (await geometry(page))?.left === 0, `${label}: the first Library section starts at the beginning`);
        check(state.errors.length === 0, `${label}: no page errors (${state.errors.join(', ')})`);
      } finally { await context.close(); }
    }

    const { context, page } = await fx.create(browser, 800, { touch: true });
    try {
      await fx.go(page, '/favourites'); await page.waitForSelector('.libnav', { state: 'attached' }); await settle(page);
      check(!await page.locator('.libnav').isVisible(), 'desktop keeps its sidebar navigation');
      await page.setViewportSize({ width: 360, height: 800 }); await settle(page);
      check((await geometry(page))?.visible, 'switching from desktop to phone reveals the current section');
      const fontSize = await page.$eval('.libnav .chip', (e) => parseFloat(getComputedStyle(e).fontSize));
      await page.evaluate((size) => { document.documentElement.style.setProperty('--fs-sm', `${size * 1.3}px`); }, fontSize); await settle(page);
      check(await page.$eval('.libnav .chip', (e) => parseFloat(getComputedStyle(e).fontSize)) >= fontSize * 1.29, 'the larger-text check actually enlarges Library labels');
      check((await geometry(page))?.visible && (await geometry(page))?.targets, 'larger text keeps the current section visible and touchable');
      await fx.cleanCopy(page);
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} failed` : 'all passed');
  process.exitCode = failed ? 1 : 0;
})().catch((e) => { console.error(e); process.exitCode = 1; });
