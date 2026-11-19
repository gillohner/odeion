// A click on a song row that only closes its ⋯ menu doesn't count as the first click of a double
// click. Before, it was remembered as a click on the row, so the next quick click on the row (the
// browser's double click second) was held as one and did nothing: open ⋯, click the row to close it,
// click the row again to play it, and nothing played. Now that click plays the song, on a library song
// row (Songs) and on an album's song row; the menu's closing click still plays nothing, and a real
// double click on a row still starts its song once.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-menu-close-click.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/menu-close-click-shots');
fs.mkdirSync(shots, { recursive: true });
const tracks = Array.from({ length: 4 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, canonical_uri: `${fx.SONG}?n=${i + 1}` }));
const songs = tracks.map((t, i) => ({ no: i + 1, title: t.title, artist: 'Sefa', artist_id: 'lib-11', duration_ms: 240000, uri: null, track_id: t.id, id: `lib-${t.id}`, page: `lib-${t.id}` }));
const album = { uri: null, source: 'library', source_id: 'library', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [],
  kind: 'album', id: 'lib-7', target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, songs, about: [], about_text: null, image_credit: null };
const fixtures = { '/item/album/lib-7': { body: album }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } };
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const PAGES = [
  { at: '/tracks', name: 'Songs', row: '.tracks[data-list="all"] .track', more: '[data-act="more"]' },
  { at: '/album/lib-7', name: 'Album', row: '#main [data-srow]', more: '[data-smore]' },
];

// Each time the app's player is given a song's file (a fixture's play() is a no-op), by track id
async function open(browser, width) {
  const run = await fx.create(browser, width, { tracks, fixtures });
  await run.context.addInitScript(() => {
    window.__starts = [];
    const d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
    Object.defineProperty(HTMLMediaElement.prototype, 'src', { configurable: true, get() { return d.get.call(this); }, set(v) { const m = /\/library\/tracks\/(\d+)\/file/.exec(v); if (m) window.__starts.push(m[1]); d.set.call(this, v); } });
  });
  return run;
}
const starts = (page) => page.evaluate(() => window.__starts.slice());
const reset = (page) => page.evaluate(() => { window.__starts.length = 0; });
const menuOpen = (page) => page.evaluate(() => { const m = document.querySelector('.popover.menu'); return !!m && !m.hidden; });
// A point in the row's middle that is on no link or button (the row's own blank space)
const blankPoint = (row) => row.evaluate((r) => {
  const b = r.getBoundingClientRect(); const y = b.top + b.height / 2;
  for (let x = b.left + b.width * 0.3; x < b.right - 20; x += 4) { const el = document.elementFromPoint(x, y); if (el && r.contains(el) && !el.closest('a, button, input, .dur, .fmt')) return { x, y }; }
  return null;
});
const second = async (page, p) => { await page.mouse.move(p.x, p.y); await page.mouse.down({ clickCount: 2 }); await page.mouse.up({ clickCount: 2 }); };

async function run(browser, width) {
  for (const pg of PAGES) {
    const { context, page, state } = await open(browser, width);
    const label = `${width} ${pg.name}`;
    try {
      await fx.go(page, pg.at);
      const row = page.locator(pg.row).nth(1); await row.waitFor(); await page.waitForTimeout(500);
      const want = String(tracks[1].id);
      const p = await blankPoint(row);
      check(!!p, `${label}: the row has a blank spot to click`);
      if (!p) continue;
      // ⋯, then a click on the row closes the menu (and plays nothing), then a quick second click plays it
      await reset(page);
      await row.hover(); await row.locator(pg.more).click(); await page.waitForTimeout(300);
      check(await menuOpen(page), `${label}: ⋯ opens the row's menu`);
      await page.mouse.click(p.x, p.y); await page.waitForTimeout(150);
      check(!(await menuOpen(page)), `${label}: a click on the row closes the menu`);
      check((await starts(page)).length === 0, `${label}: the click that closes the menu plays nothing (${JSON.stringify(await starts(page))})`);
      await page.screenshot({ path: path.join(shots, `${pg.name.toLowerCase()}-closed-${width}.png`) });
      await second(page, p); await page.waitForTimeout(600);
      check(JSON.stringify(await starts(page)) === JSON.stringify([want]), `${label}: the next quick click on the row (a double click's second) plays it (${JSON.stringify(await starts(page))}, want [${want}])`);
      await page.screenshot({ path: path.join(shots, `${pg.name.toLowerCase()}-played-${width}.png`) });
      // A real double click on the row, with no menu, still starts its song once
      await page.waitForTimeout(700); await reset(page);
      const q = await blankPoint(page.locator(pg.row).nth(2));
      check(!!q, `${label}: the next row has a blank spot to click`);
      if (!q) continue;
      await page.mouse.click(q.x, q.y); await second(page, q); await page.waitForTimeout(600);
      check(JSON.stringify(await starts(page)) === JSON.stringify([String(tracks[2].id)]), `${label}: a double click on a row starts its song once (${JSON.stringify(await starts(page))})`);
      check(state.errors.length === 0, `${label}: no page errors (${state.errors.join('; ')})`);
    } finally { await context.close(); }
  }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768]) await run(browser, w); } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})();
