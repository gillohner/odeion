// A click on a song row of an album or a playlist (`.srow`) plays from that row; the second click of a
// double click doesn't start it again and selects no word of the row, and a drag that selects the row's
// text doesn't start it, as on a library song row (Songs, Favourites). On a touch screen, text selected
// elsewhere on the page doesn't hold a tap on a row. The row's Play button still plays at once.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-srow-click.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/srow-click-shots');
fs.mkdirSync(shots, { recursive: true });
const tracks = Array.from({ length: 4 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, canonical_uri: `${fx.SONG}?n=${i + 1}` }));

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
// An album of the library with four songs (the item page's answer, as ui-busy.js's)
const songs = tracks.map((t, i) => ({ no: i + 1, title: t.title, artist: 'Sefa', duration_ms: 200000 + i * 1000, uri: `track:${t.id}`, track_id: t.id, id: `lib-${t.id}`, page: `lib-${t.id}` }));
const album = { kind: 'album', id: 'lib-7', uri: null, target: {}, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs, total_ms: 806000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
const fixtures = { '/item/album/lib-7': { body: album }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } }, ...Object.fromEntries(tracks.map((t) => [`/library/tracks/${t.id}`, { body: t }])) };
const PAGES = [
  { at: '/album/lib-7', name: 'Album', rows: '.srow[data-srow]', nth: 1, want: '2', next: '3' },
  { at: '/playlist/c1', name: 'Playlist', rows: '#plsongs .srow[data-srow]:not([data-card])', nth: 0, want: '1' },
];

// Each time the app's player is given a song's file (a fixture's play() is a no-op), by track id
async function open(browser, width, opts) {
  const run = await fx.create(browser, width, { tracks, fixtures, ...opts });
  await run.context.addInitScript(() => {
    window.__starts = []; window.__details = [];
    document.addEventListener('click', (e) => { if (e.target.closest && e.target.closest('[data-srow]')) window.__details.push(e.detail); }, true);
    const d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
    Object.defineProperty(HTMLMediaElement.prototype, 'src', { configurable: true, get() { return d.get.call(this); }, set(v) { const m = /\/library\/tracks\/(\d+)\/file/.exec(v); if (m) window.__starts.push(m[1]); d.set.call(this, v); } });
  });
  return run;
}
const starts = async (page) => JSON.stringify(await page.evaluate(() => window.__starts.slice()));
const reset = (page) => page.evaluate(() => { window.__starts.length = 0; getSelection().removeAllRanges(); });
// The row's length (plain text, no link or button)
const dur = (row) => row.locator(':scope > span.muted.small').last();

async function desktop(browser, width) {
  for (const pg of PAGES) {
    const { context, page, state } = await open(browser, width);
    const label = `${width} ${pg.name}`;
    try {
      await fx.go(page, pg.at);
      const row = page.locator(pg.rows).nth(pg.nth); await row.waitFor(); await page.waitForTimeout(500); await reset(page);
      const want = `["${pg.want}"]`;
      check(await dur(row).innerText() !== '', `${label}: the row shows its length`);
      // one click on the row plays from it
      await dur(row).click(); await page.waitForTimeout(500);
      check(await starts(page) === want, `${label}: a click on the row plays that song (${await starts(page)}, want ${want})`);
      // a double click starts it once and selects no text
      await page.waitForTimeout(800); await reset(page);
      await dur(row).dblclick(); await page.waitForTimeout(600);
      check(await starts(page) === want, `${label}: a double click starts the song once (${await starts(page)})`);
      const sel = await page.evaluate(() => String(getSelection()));
      await page.screenshot({ path: path.join(shots, `${pg.name.toLowerCase()}-dblclick-${width}.png`) });
      check(sel.trim() === '', `${label}: …and selects no text of the row ("${sel.trim()}")`);
      // a drag that selects the row's length text doesn't start it
      await page.waitForTimeout(800); await reset(page);
      const b = await dur(row).boundingBox();
      await page.mouse.move(b.x + 1, b.y + b.height / 2); await page.mouse.down(); await page.mouse.move(b.x + b.width - 1, b.y + b.height / 2, { steps: 8 }); await page.mouse.up(); await page.waitForTimeout(500);
      const dragged = await page.evaluate(() => String(getSelection()));
      check(dragged.trim() !== '' && await starts(page) === '[]', `${label}: a drag that selects the row's text plays nothing ("${dragged.trim()}", ${await starts(page)})`);
      // text selected somewhere else on the page doesn't hold a click on the row
      await reset(page);
      const held = await page.evaluate(() => { const h = document.querySelector('#main h1, #main h2'); if (!h) return ''; const r = document.createRange(); r.selectNodeContents(h); getSelection().addRange(r); return String(getSelection()).trim(); });
      await dur(row).click(); await page.waitForTimeout(500);
      check(held !== '' && await starts(page) === want, `${label}: with the heading selected ("${held}"), a click on the row still plays it (${await starts(page)})`);
      // a click on the row, then one just below the line on the next row that the browser counts as a
      // double click's second, plays the first row and then the second
      if (pg.next) {
        await page.waitForTimeout(800); await reset(page);
        const a = await row.boundingBox(); const n = await page.locator(pg.rows).nth(pg.nth + 1).boundingBox(); const x = b.x + b.width / 2;
        await page.mouse.click(x, a.y + a.height - 2); await page.mouse.move(x, n.y + 2); await page.mouse.down({ clickCount: 2 }); await page.mouse.up({ clickCount: 2 }); await page.waitForTimeout(600);
        const two = `["${pg.want}","${pg.next}"]`;
        check(await page.evaluate(() => window.__details.slice(-1)[0]) === 2 && await starts(page) === two, `${label}: a double click's second on the next row plays that row (${await starts(page)}, want ${two})`);
        // the row clicked a while ago, then a click off the rows, then one on that row again that the
        // browser counts as a double click's second: it plays (the click before it wasn't on this row)
        await page.waitForTimeout(800); await page.mouse.click(x, a.y + a.height / 2); await page.waitForTimeout(800); await reset(page);
        const h = await page.locator('#main h1, #main h2').first().boundingBox();
        await page.mouse.click(h.x + 2, h.y + h.height / 2); await page.mouse.move(x, a.y + a.height / 2); await page.mouse.down({ clickCount: 2 }); await page.mouse.up({ clickCount: 2 }); await page.waitForTimeout(600);
        check(await page.evaluate(() => window.__details.slice(-1)[0]) === 2 && await starts(page) === want, `${label}: after a click off the rows, a double click's second on the row plays it (${await starts(page)}, want ${want})`);
      }
      // the row's Play button plays at once, and a double click on it still plays (its own control)
      await page.waitForTimeout(800); await reset(page); await row.hover();
      await row.locator('[data-splay]').click(); await page.waitForTimeout(500);
      check(await starts(page) === want, `${label}: the row's Play button plays it (${await starts(page)})`);
      check(state.errors.length === 0, `${label}: no page errors (${state.errors.join(' | ')})`);
    } finally { await context.close(); }
  }
}

// On a touch screen the rows take no text selection, so a selection left elsewhere (a long press on the
// heading, its handle maybe dragged down past the songs) stays: a tap on a row plays it all the same
async function touch(browser, width) {
  for (const pg of PAGES) {
    const { context, page, state } = await open(browser, width, { touch: true });
    const label = `${width} touch ${pg.name}`;
    try {
      await fx.go(page, pg.at);
      const row = page.locator(pg.rows).nth(pg.nth); await row.waitFor(); await page.waitForTimeout(500); await reset(page);
      const want = `["${pg.want}"]`;
      await dur(row).tap(); await page.waitForTimeout(500);
      check(await starts(page) === want, `${label}: a tap on the row plays it (${await starts(page)})`);
      await page.waitForTimeout(800); await reset(page);
      const held = await page.evaluate(() => { const h = document.querySelector('#main h1, #main h2'); if (!h) return ''; const r = document.createRange(); r.selectNodeContents(h); getSelection().addRange(r); return String(getSelection()).trim(); });
      check(held !== '', `${label}: the page's heading is selected ("${held}")`);
      await dur(row).tap(); await page.waitForTimeout(500);
      check(await starts(page) === want, `${label}: with text selected elsewhere, a tap on the row still plays it (${await starts(page)})`);
      // a selection from the heading down past the row (ending in the list's last row) doesn't hold it
      await page.waitForTimeout(800); await reset(page);
      const across = await page.evaluate(([sel, nth]) => { const h = document.querySelector('#main h1, #main h2'); const rows = document.querySelectorAll(sel.replace(':not([data-card])', '')); const last = rows[rows.length - 1]; const at = document.querySelectorAll(sel)[nth]; if (!h || !last || last === at) return false; const r = document.createRange(); r.setStart(h, 0); r.setEnd(last, 0); getSelection().addRange(r); return getSelection().containsNode(at, false); }, [pg.rows, pg.nth]);
      check(across, `${label}: a selection runs from the heading down past the row`);
      await dur(row).tap(); await page.waitForTimeout(500);
      check(await starts(page) === want, `${label}: …and a tap on a row inside it still plays it (${await starts(page)})`);
      // a double tap on the row starts it once
      await page.waitForTimeout(800); await reset(page);
      const d = await dur(row).boundingBox(); const tap = (y) => page.touchscreen.tap(d.x + d.width / 2, y);
      await tap(d.y + d.height / 2); await tap(d.y + d.height / 2); await page.waitForTimeout(600);
      check(await page.evaluate(() => window.__details.slice(-1)[0]) === 2 && await starts(page) === want, `${label}: a double tap on the row starts it once (${await starts(page)})`);
      // two quick taps on two rows, close enough to count as a double tap (a phone counts taps up to
      // 100 dp apart, more than a row), play the first row and then the second
      if (pg.next) {
        await page.waitForTimeout(800); await reset(page);
        const a = await row.boundingBox(); const b = await page.locator(pg.rows).nth(pg.nth + 1).boundingBox();
        await tap(a.y + a.height - 3); await tap(b.y + 3); await page.waitForTimeout(600);
        const two = `["${pg.want}","${pg.next}"]`;
        const detail = await page.evaluate(() => window.__details.slice(-1)[0]);
        check(detail === 2, `${label}: the second tap, ${Math.round(b.y + 3 - (a.y + a.height - 3))} px below the first, comes as a double tap's second (detail ${detail})`);
        check(await starts(page) === two, `${label}: …and still plays the row it hit (${await starts(page)}, want ${two})`);
      }
      check(state.errors.length === 0, `${label}: no page errors (${state.errors.join(' | ')})`);
    } finally { await context.close(); }
  }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) await desktop(browser, w);
    await touch(browser, 768); await touch(browser, 412);
  } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})();
