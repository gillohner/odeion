// On a phone every way back and the top bar's Help and Install buttons are full 44 px targets,
// and the taller Back buttons do not push the page down.
// Real candidate assets, isolated responses: node tools/ui-phone-back-targets.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-back-targets-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// One library album page, so the item page's Back (in its bar with the breadcrumbs) is shown.
const album = { uri: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library', image: null, tags: [],
  kind: 'album', id: 'lib-7', target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020,
  songs: [{ no: 1, title: fx.track.title, artist: 'Sefa', artist_id: 'lib-11', duration_ms: 240000, uri: null, track_id: 1, id: 'lib-1', page: 'lib-1' }], about: [], about_text: null, image_credit: null };
const fixtures = {
  '/item/album/lib-7': { body: album },
  '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
};

const box = (page, selector) => page.locator(selector).first().evaluate((el) => {
  const b = el.getBoundingClientRect();
  return { left: b.left, right: b.right, top: b.top, bottom: b.bottom, width: b.width, height: b.height };
});
const big = (b) => b.width >= 44 && b.height >= 44;
const size = (b) => `${b.width.toFixed(1)}×${b.height.toFixed(1)}`;

// A real touch near the bottom edge verifies the whole target, not just its label.
async function tapEdge(page, selector) {
  const b = await page.locator(selector).first().boundingBox();
  await page.touchscreen.tap(b.x + b.width / 2, b.y + b.height - 2);
}

// The content below a Back button starts where it did with the old button (most: the old distance
// from the top of the button's container, measured on the previous layout), not lower.
async function noPush(page, selector, at, most) {
  const r = await page.locator(selector).first().evaluate((el) => {
    const main = document.querySelector('#main');
    const box = el.parentElement;
    const top = box.getBoundingClientRect().top + parseFloat(getComputedStyle(box).paddingTop);
    const b = el.getBoundingClientRect();
    let next = el.nextElementSibling;
    while (next && !next.getClientRects().length) next = next.nextElementSibling;
    return { below: next ? next.getBoundingClientRect().top - top : null, top: b.top, mainTop: main.getBoundingClientRect().top };
  });
  check(r.below !== null && r.below <= most, `${at}: the page below starts ${r.below.toFixed(1)} px down (at most ${most})`);
  check(r.top >= r.mainTop, `${at}: the button stays inside the page`);
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures });
      state.collections = [fx.summary];
      try {
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });

        // The top bar: Help (and Install in a browser) match the 44 px account button.
        await fx.go(page, '/library');
        await page.waitForSelector('.topbar .fb-help');
        const top = await page.evaluate(() => [...document.querySelectorAll('.topbar .fb-help, .topbar #installbtn, .topbar #me')]
          .filter((el) => el.getClientRects().length)
          .map((el) => { const b = el.getBoundingClientRect(); return { label: el.getAttribute('aria-label'), width: b.width, height: b.height, left: b.left, right: b.right }; }));
        const want = android ? 2 : 3;
        check(top.length === want, `${at}: ${want} top bar buttons shown (${top.map((b) => b.label).join(', ')})`);
        check(top.every(big), `${at}: top bar buttons at least 44 px (${top.map((b) => `${b.label}: ${size(b)}`).join(', ')})`);
        check(top.every((b) => b.left >= 0 && b.right <= width), `${at}: top bar buttons inside the screen`);
        const search = await box(page, '.topbar .search');
        check(search.width >= 120, `${at}: the search field keeps room to type (${search.width.toFixed(0)} px)`);
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at}: no sideways overflow`);
        await page.screenshot({ path: path.join(SHOTS, `topbar-${at}.png`), clip: { x: 0, y: 0, width, height: 80 } });
        await tapEdge(page, '.topbar .fb-help');
        await page.waitForSelector('.topbar .fb-help[aria-expanded="true"]');
        check(true, `${at}: one edge tap opens Help`);
        await page.keyboard.press('Escape');

        // A playlist page's Back.
        await fx.go(page, '/playlist/c1');
        await page.waitForSelector('#main > .back');
        const pl = await box(page, '#main > .back');
        check(big(pl), `${at} playlist: Back at least 44 px (${size(pl)})`);
        await noPush(page, '#main > .back', `${at} playlist Back`, 51);
        await page.screenshot({ path: path.join(SHOTS, `playlist-${at}.png`) });
        await tapEdge(page, '#main > .back');
        await page.waitForFunction(() => !location.pathname.startsWith('/playlist/'));
        check(true, `${at} playlist: one edge tap goes back (${await page.evaluate(() => location.pathname)})`);

        // An item page's Back, in the bar with its breadcrumbs.
        await fx.go(page, '/album/lib-7');
        await page.waitForSelector('#main .pagebar > .back');
        const item = await box(page, '#main .pagebar > .back');
        check(big(item), `${at} album: Back at least 44 px (${size(item)})`);
        await noPush(page, '#main .pagebar', `${at} album bar`, 43);
        await page.screenshot({ path: path.join(SHOTS, `album-${at}.png`) });
        await tapEdge(page, '#main .pagebar > .back');
        await page.waitForFunction(() => !location.pathname.startsWith('/album/'));
        check(true, `${at} album: one edge tap goes back (${await page.evaluate(() => location.pathname)})`);

        // A Settings panel's Back.
        await fx.go(page, '/settings/appearance');
        await page.waitForSelector('#set-appearance .set-back');
        const set = await box(page, '#set-appearance .set-back');
        check(big(set), `${at} settings: Back at least 44 px (${size(set)})`);
        await noPush(page, '#set-appearance .set-back', `${at} settings Back`, 47);
        await page.screenshot({ path: path.join(SHOTS, `settings-${at}.png`) });
        await tapEdge(page, '#set-appearance .set-back');
        await page.waitForFunction(() => location.pathname !== '/settings/appearance');
        check(true, `${at} settings: one edge tap goes back (${await page.evaluate(() => location.pathname)})`);

        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // These rules belong to the phone layout: the desktop layout keeps its compact Back and top bar.
    const { page, context } = await fx.create(browser, 1024, { fixtures });
    try {
      await fx.go(page, '/playlist/c1');
      await page.waitForSelector('#main > .back');
      check((await box(page, '#main > .back')).height < 44, '1024 px desktop keeps its compact Back');
      check((await box(page, '.topbar .fb-help')).height < 44, '1024 px desktop keeps its compact Help');
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
