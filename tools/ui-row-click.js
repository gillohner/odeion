// A click on a library song row (Songs, Favourites) plays from that row with a mouse too, as a
// playlist's or an album's song row does and as a tap on the row does on a touch screen. The row's own
// controls keep their click: the song's name opens its page, the heart and ⋯ are theirs, and none of
// them plays. A double click starts the song once (not again on its second click) and selects no text,
// and a drag that selects the row's text doesn't start it.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-row-click.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/row-click-shots');
fs.mkdirSync(shots, { recursive: true });
const tracks = Array.from({ length: 4 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, canonical_uri: `${fx.SONG}?n=${i + 1}` }));

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const path0 = (page) => new URL(page.url()).pathname;
const PAGES = [
  { at: '/tracks', name: 'Songs', ready: '.tracks[data-list="all"] .track', nth: 1 },
  { at: '/favourites', name: 'Favourites', ready: '.tracks[data-list="fav"] .track', nth: 0, opts: { favs: [fx.SONG] } },
];

// Each time the app's player is given a song's file (a fixture's play() is a no-op), by track id
async function open(browser, width, opts) {
  const run = await fx.create(browser, width, opts);
  await run.context.addInitScript(() => {
    window.__starts = [];
    const d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
    Object.defineProperty(HTMLMediaElement.prototype, 'src', { configurable: true, get() { return d.get.call(this); }, set(v) { const m = /\/library\/tracks\/(\d+)\/file/.exec(v); if (m) window.__starts.push(m[1]); d.set.call(this, v); } });
  });
  return run;
}
const starts = (page) => page.evaluate(() => window.__starts.slice());
const reset = (page) => page.evaluate(() => { window.__starts.length = 0; });
// A point in the row's middle that is on no link or button (the row's own blank space)
const blankPoint = (row) => row.evaluate((r) => {
  const b = r.getBoundingClientRect(); const y = b.top + b.height / 2;
  for (let x = b.left + b.width * 0.3; x < b.right - 20; x += 4) { const el = document.elementFromPoint(x, y); if (el && r.contains(el) && !el.closest('a, button, input, .dur, .fmt')) return { x, y }; }
  return null;
});

async function desktop(browser, width) {
  for (const pg of PAGES) {
    const { context, page, state } = await open(browser, width, { tracks, ...(pg.opts || {}) });
    const label = `${width} ${pg.name}`;
    try {
      await fx.go(page, pg.at);
      const row = page.locator(pg.ready).nth(pg.nth); await row.waitFor(); await page.waitForTimeout(400); await reset(page);
      const want = await row.getAttribute('data-tid');
      check(await row.evaluate((r) => getComputedStyle(r).cursor) === 'pointer', `${label}: the row shows a pointer, as a playlist's song row does`);
      // one click on the row's length plays from that row, and stays on the page
      await row.locator('.dur').click(); await page.waitForTimeout(500);
      check(JSON.stringify(await starts(page)) === JSON.stringify([want]), `${label}: a click on the row plays that song (${JSON.stringify(await starts(page))}, want [${want}])`);
      check(path0(page) === pg.at, `${label}: …and stays on the page (${path0(page)})`);
      // one click on the row's blank middle too
      await reset(page); const pt = await blankPoint(row);
      check(!!pt, `${label}: the row has blank space to click`);
      if (pt) { await page.mouse.click(pt.x, pt.y); await page.waitForTimeout(500); }
      check(JSON.stringify(await starts(page)) === JSON.stringify([want]), `${label}: a click on the row's blank middle plays it too (${JSON.stringify(await starts(page))})`);
      // a double click starts it once and selects no text
      await reset(page); await page.evaluate(() => getSelection().removeAllRanges());
      await row.locator('.dur').dblclick(); await page.waitForTimeout(500);
      check(JSON.stringify(await starts(page)) === JSON.stringify([want]), `${label}: a double click starts the song once (${JSON.stringify(await starts(page))})`);
      check(await page.evaluate(() => String(getSelection()).trim() === ''), `${label}: …and selects no text of the row ("${await page.evaluate(() => String(getSelection()))}")`);
      await page.screenshot({ path: path.join(shots, `after-${pg.name.toLowerCase()}-${width}.png`) });
      // a drag that selects the row's text doesn't start it
      await page.waitForTimeout(1000); await reset(page); const d = await row.locator('.dur').boundingBox(); const f = await row.locator('.fmt').boundingBox();
      await page.mouse.move(f.x + 3, f.y + f.height / 2); await page.mouse.down(); await page.mouse.move(d.x + d.width * 0.5, d.y + d.height / 2, { steps: 8 }); await page.mouse.up(); await page.waitForTimeout(500);
      const sel = await page.evaluate(() => String(getSelection()));
      check(sel.trim() !== '' && (await starts(page)).length === 0, `${label}: a drag that selects the row's text plays nothing ("${sel.trim()}", ${JSON.stringify(await starts(page))})`);
      await page.evaluate(() => getSelection().removeAllRanges());
      // the row's controls keep their own click and play nothing: ⋯ (its menu) and the heart
      await reset(page); await row.hover();
      await row.locator('[data-act="more"]').click(); await page.waitForTimeout(300);
      check(await page.locator('.popover.menu:not([hidden])').count() > 0, `${label}: ⋯ still opens the row's menu`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(200);
      await row.hover(); await row.locator('[data-act="more"]').dblclick(); await page.waitForTimeout(300); await page.keyboard.press('Escape');
      await row.hover(); await row.locator('[data-act="fav"]').click(); await page.waitForTimeout(600);
      check((await starts(page)).length === 0, `${label}: a click or double click on ⋯ or the heart plays nothing (${JSON.stringify(await starts(page))})`);
      // the song's name keeps its link: it opens the song page and plays nothing
      if (pg.at === '/tracks') {
        await fx.go(page, pg.at); await row.waitFor(); await page.waitForTimeout(400); await reset(page);
        await row.locator('.title [data-act="song"]').click(); await page.waitForTimeout(600);
        check(path0(page).startsWith('/song/'), `${label}: a click on the song's name opens its page (${path0(page)})`);
        check((await starts(page)).length === 0, `${label}: …and plays nothing (${JSON.stringify(await starts(page))})`);
      }
      check(state.errors.length === 0, `${label}: no page errors (${state.errors.join(' | ')})`);
    } finally { await context.close(); }
  }
}

// On a phone a tap on the row already played; a double click there now starts it once too, and on a
// touch screen (rows take no text selection there) text selected elsewhere doesn't hold the tap
async function phone(browser, width, touch) {
  const { context, page, state } = await open(browser, width, { tracks, touch });
  try {
    await fx.go(page, '/tracks');
    if (width < 600) await page.waitForFunction(() => document.documentElement.dataset.shell === 'phone');
    const row = page.locator('.tracks[data-list="all"] .track').nth(1); await row.waitFor(); await page.waitForTimeout(400); await reset(page);
    await row.locator('.dur').click(); await page.waitForTimeout(500);
    check(JSON.stringify(await starts(page)) === '["2"]', `${width} ${touch ? 'touch' : 'phone'}: a tap on the row plays that song (${JSON.stringify(await starts(page))})`);
    await reset(page); await row.locator('.dur').dblclick(); await page.waitForTimeout(500);
    check(JSON.stringify(await starts(page)) === '["2"]', `${width} ${touch ? 'touch' : 'phone'}: a double click starts it once (${JSON.stringify(await starts(page))})`);
    if (touch) {
      // the page's heading selected (a long press elsewhere), then a tap on a row: it plays
      await reset(page);
      const held = await page.evaluate(() => { const h = document.querySelector('#main h1, #main h2'); if (!h) return ''; const r = document.createRange(); r.selectNodeContents(h); getSelection().removeAllRanges(); getSelection().addRange(r); return String(getSelection()).trim(); });
      check(held !== '', `${width} touch: the page's heading is selected ("${held}")`);
      await row.locator('.dur').tap(); await page.waitForTimeout(500);
      check(JSON.stringify(await starts(page)) === '["2"]', `${width} touch: with text selected elsewhere, a tap on the row still plays it (${JSON.stringify(await starts(page))})`);
    }
    check(state.errors.length === 0, `${width} ${touch ? 'touch' : 'phone'}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) await desktop(browser, w);
    await phone(browser, 412);
    await phone(browser, 412, true); await phone(browser, 768, true);
  } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})();
