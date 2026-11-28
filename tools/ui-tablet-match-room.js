// A touch screen always shows an unmatched song's Match beside its title. On an Android tablet with a
// large font (the app's WebView applies it as text zoom) in a narrow window, the number, format and
// length columns grow with the text, and Match (85 px at 200 %) left the title a few letters: 41 px of a
// 143 px cell at 721 px, 88 px at 768. When the title's cell has room for fewer than 16 characters
// (with Match the title would keep under ~10), Match isn't shown there now, as on a phone (the row's ⋯ has "Which song is this?…"); where there is
// room (100 % at every tablet width, 200 % from ~900 px) it shows as before. A mouse is unchanged.
// Real candidate assets, isolated responses: node tools/ui-tablet-match-room.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'tablet-match-room-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Row 1: unmatched, long title; row 2: the same title matched (no Match); row 3: unmatched, short title
const LONG = 'Journey Through Time and the Long Road Back Home Again (Extended Live Version)';
const tracks = [
  { ...fx.track, id: 1, title: LONG },
  { ...fx.track, id: 2, title: LONG, canonical: [{ kind: 'recording', mbid: fx.track.mbid }] },
  { ...fx.track, id: 3, title: 'Short one' },
];
// Android's font size as the app's WebView applies it (text zoom; see tools/ui-phone-library-head.js)
const ZOOM_ARGS = ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'];
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });

// Each row's title cell, link and Match, and the title's character width (ch of its font), in px
const measure = (page) => page.evaluate(() => [...document.querySelectorAll('#tracklist .track')].map((row) => {
  const t = row.querySelector('.title'); const a = t.querySelector(':scope > .lnk'); const p = t.querySelector(':scope > .idmark');
  const probe = document.createElement('span'); probe.style.cssText = 'position:absolute;visibility:hidden;width:10ch'; t.append(probe);
  const ch = probe.getBoundingClientRect().width / 10; probe.remove();
  const cs = getComputedStyle(t); const cell = t.getBoundingClientRect().width - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const shown = !!p && p.getClientRects().length > 0 && getComputedStyle(p).display !== 'none' && Number(getComputedStyle(p).opacity) > 0 && p.getBoundingClientRect().width > 0;
  return { cell, ch, link: a.getBoundingClientRect().width, full: a.scrollWidth, has: !!p, shown, text: a.textContent };
}));

async function touch(browser, width, k) {
  const at = `touch ${width}${k !== 1 ? ` at ${Math.round(k * 100)} %` : ''}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true, tracks });
  try {
    await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track .title > .lnk');
    if (k !== 1) await textZoom(page, k);
    await page.waitForTimeout(300);
    const env = await page.evaluate(() => ({ shell: document.documentElement.dataset.shell, hover: matchMedia('(hover: hover)').matches, rowH: document.querySelector('#tracklist .track').getBoundingClientRect().height }));
    check(env.shell === 'desktop' && !env.hover, `${at}: the tablet layout on a touch screen (${env.shell}, hover ${env.hover})`);
    // (the zoom took effect: a zoomed row is taller than the 54 px of 100 %)
    check(k === 1 ? Math.abs(env.rowH - 54) < 0.5 : env.rowH > 56, `${at}: the rows are ${k === 1 ? '54 px' : 'taller than at 100 %'} (${env.rowH.toFixed(1)} px)`);
    const [long, matched, short] = await measure(page);
    check(long.has && short.has && !matched.has, `${at}: the unmatched songs have a Match, the matched one not`);
    const roomy = long.cell >= 16 * long.ch;
    // The unmatched long title keeps at least 9.5 characters, or its whole cell where that is less
    // (Match left it under 3 at 721 px and 200 %)
    check(long.link >= Math.min(9.5 * long.ch, long.cell) - 0.5, `${at}: the unmatched title keeps ${(long.link / long.ch).toFixed(1)} characters (${Math.round(long.link)} of ${Math.round(long.cell)} px; Match ${long.shown ? 'shown' : 'not shown'})`);
    // Match shows where the cell has room for 16 characters, and not where it hasn't; both unmatched rows alike
    check(long.shown === roomy && short.shown === roomy, `${at}: Match ${roomy ? 'shows' : 'is not shown'} (cell ${(long.cell / long.ch).toFixed(1)} characters; long ${long.shown}, short ${short.shown})`);
    if (!long.shown) check(Math.abs(long.link - matched.link) <= 1, `${at}: without Match the unmatched title is cut where the matched one is (${Math.round(long.link)} vs ${Math.round(matched.link)} px)`);
    if (k === 1) check(long.shown, `${at}: at 100 % Match shows, as before`);
    if (!long.shown && width === 768 && k === 2) {
      // The row's ⋯ still offers the match
      await page.locator('#tracklist .track').first().locator('[data-act="more"]').tap();
      await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor();
      const items = (await page.locator('.popover.menu:not([hidden]) .pop-item').allInnerTexts()).map((s) => s.trim());
      check(items.includes('Which song is this?…'), `${at}: the row's ⋯ has "Which song is this?…" (${items.join(' · ')})`);
      await page.keyboard.press('Escape');
    }
    const fits = await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);
    check(fits, `${at}: no sideways overflow`);
    await page.screenshot({ path: path.join(SHOTS, `songs-${at}.png`.replace(/ /g, '-')) });
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

// A mouse at 768 px and 200 %: Match still comes after a moment on the row (the hover rules, unchanged)
async function mouse(browser) {
  const at = 'mouse 768 at 200 %';
  const { page, context, state } = await fx.create(browser, 768, { tracks });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track .title > .lnk');
    await textZoom(page, 2);
    await page.waitForTimeout(300);
    const box = await page.locator('#tracklist .track').first().locator('.dur').boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.waitForTimeout(700);
    const [long] = await measure(page);
    check(long.shown, `${at}: Match shows on the row under the mouse, as before (${Math.round(long.link)} of ${Math.round(long.cell)} px)`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  const zoomed = await chromium.launch({ args: ZOOM_ARGS });
  try {
    for (const w of [721, 768, 1024]) await touch(browser, w, 1);
    for (const [w, k] of [[721, 1.5], [721, 2], [740, 1.75], [768, 2], [780, 1.5], [800, 2], [900, 2], [1024, 2]]) await touch(zoomed, w, k);
    await mouse(zoomed);
  } finally { await browser.close(); await zoomed.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
