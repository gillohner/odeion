// A click on a song row of an album or a playlist (`.srow`) plays from that row; the second click of a
// double click doesn't start it again and selects no word of the row, and a drag that selects the row's
// text doesn't start it, as on a library song row (Songs, Favourites). On a touch screen, text selected
// elsewhere on the page doesn't hold a tap on a row, nor does an empty selection inside the row. The row's
// Play button still plays at once. A quick click on the row right after one on its heart plays it, and a
// quick click on a row of another list on the page plays that one, even the same song in the same place.
// The same holds on a playlist of streamed songs (no track id: the click before is keyed on the song's
// address), also when the list is drawn again between the two clicks.
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
// A playlist of songs that play from a service, not the library: their rows have no track id, so the
// click before is keyed on each song's address (`streamed(order)`: the list, and where each plays)
const STREAMED = [101, 102, 103].map((n, i) => ({ n, title: ['Papa', 'Quebec', 'Romeo'][i], uri: `https://tidal.com/browse/track/${n}` }));
const streamed = (order) => {
  const list = order.map((n) => STREAMED.find((s) => s.n === n)); const sum = { ...fx.summary, id: 'c9', name: 'Streamed', items: list.length };
  fixtures['/playlists/c9'] = { body: { summary: sum, playlist: { ...sum, items: list.map((s) => ({ uri: s.uri })) }, tracks: list.map(() => null), entries: list.map((s) => ({ kind: 'track', uri: s.uri })), local_tracks: [] } };
  fixtures['/playlists/c9/play-from'] = { body: { order: { order: ['tidal'], services: [{ id: 'tidal', name: 'TIDAL', connected: true }] }, own_order: false,
    items: list.map((s, i) => ({ index: i, uri: s.uri, title: s.title, artist: 'Sefa', duration_ms: 180000 + i * 1000, track_id: null, stream_uri: `tidal:track:${s.n}`, page: s.uri, plays_from: { service: 'tidal', label: 'TIDAL' }, sources: [], connect: [] })) } };
};
streamed([101, 102, 103]);
const PAGES = [
  { at: '/album/lib-7', name: 'Album', rows: '.srow[data-srow]', nth: 1, want: '2', next: '3' },
  { at: '/playlist/c1', name: 'Collection', rows: '#plsongs .srow[data-srow]:not([data-card])', nth: 0, want: '1' },
  { at: '/playlist/c9', name: 'Streamed', rows: '#plsongs .srow[data-srow]:not([data-card])', nth: 0, want: 's101', next: 's102' },
];

// Each time the app's player is given a song's file (a fixture's play() is a no-op), by track id, or
// s<id> for a song a service streams
async function open(browser, width, opts) {
  const run = await fx.create(browser, width, { tracks, fixtures, ...opts });
  await run.context.addInitScript(() => {
    window.__starts = []; window.__details = [];
    document.addEventListener('click', (e) => { if (e.target.closest && e.target.closest('[data-srow]')) window.__details.push(e.detail); }, true);
    const d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
    Object.defineProperty(HTMLMediaElement.prototype, 'src', { configurable: true, get() { return d.get.call(this); }, set(v) { const m = /\/library\/tracks\/(\d+)\/file/.exec(v) || /\/sources\/\w+\/stream\/(\w+)/.exec(v); if (m) window.__starts.push(/\/stream\//.test(v) ? `s${m[1]}` : m[1]); d.set.call(this, v); } });
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
      // a click on the row's heart, then a quick click on the row that comes as a double click's second
      // (a quick tap there does on a phone, which counts taps up to 100 dp apart): the row plays (the
      // click before was the heart's own); after its play button it doesn't start again
      for (const [act, name] of [['sfav', 'the heart'], ['splay', 'its play button']]) {
        await page.waitForTimeout(800); await reset(page); await row.hover();
        await row.locator(`[data-${act}]`).click(); await page.waitForTimeout(50);
        const p2 = await dur(row).boundingBox(); await page.mouse.move(p2.x + p2.width / 2, p2.y + p2.height / 2); await page.mouse.down({ clickCount: 2 }); await page.mouse.up({ clickCount: 2 }); await page.waitForTimeout(600);
        check(await page.evaluate(() => window.__details.slice(-1)[0]) === 2 && await starts(page) === want, `${label}: a click on ${name}, then a double click's second on the row, plays it${act === 'splay' ? ' once' : ''} (${await starts(page)}, want ${want})`);
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
      // an empty selection inside the row (the rows take no selection here, so it has no words): a tap
      // on the row still plays it
      await page.waitForTimeout(800); await reset(page);
      const inRow = await dur(row).evaluate((el) => { const g = document.createRange(); g.selectNodeContents(el); getSelection().removeAllRanges(); getSelection().addRange(g); const s = getSelection(); return !s.isCollapsed && el.contains(s.anchorNode) && String(s) === ''; });
      check(inRow, `${label}: an empty selection sits inside the row`);
      await dur(row).tap(); await page.waitForTimeout(500);
      check(await starts(page) === want, `${label}: …and a tap on the row still plays it (${await starts(page)})`);
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

// Two lists on one page with the same song in the same place (a playlist's first song, and the first of
// its songs on this node only): a click on the first list's row, then a quick click on the other's
// that comes as a double click's second, plays both (they are two rows)
async function twoLists(browser, width) {
  const local = { ...fx.detail, local_tracks: [{ ...tracks[0] }] };
  const { context, page, state } = await open(browser, width, { fixtures: { ...fixtures, '/playlists/c1': { body: local } } });
  const label = `${width} two lists`;
  try {
    await fx.go(page, '/playlist/c1');
    const a = page.locator('#plsongs .srow[data-srow]:not([data-card])').first(); const b = page.locator('#pllocal .srow[data-srow]').first();
    await a.waitFor(); await b.waitFor(); await page.waitForTimeout(500);
    check(await a.getAttribute('data-tid') === await b.getAttribute('data-tid') && await a.getAttribute('data-srow') === await b.getAttribute('data-srow'), `${label}: both lists have the same song in the same place`);
    await reset(page);
    await dur(a).click(); await page.waitForTimeout(50);
    const p2 = await dur(b).boundingBox(); await page.mouse.move(p2.x + p2.width / 2, p2.y + p2.height / 2); await page.mouse.down({ clickCount: 2 }); await page.mouse.up({ clickCount: 2 }); await page.waitForTimeout(600);
    await page.screenshot({ path: path.join(shots, `two-lists-${width}.png`) });
    check(await page.evaluate(() => window.__details.slice(-1)[0]) === 2 && await starts(page) === '["1","1"]', `${label}: a double click's second on the other list's row plays it too (${await starts(page)}, want ["1","1"])`);
    check(state.errors.length === 0, `${label}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
}

// A streamed playlist drawn again between two quick clicks on its first row (another device moved its
// songs, or nothing changed): a second click on a row that now holds another song plays that song;
// one on the same song in the same place is still a double click's second and doesn't start it again
async function streamedRedraw(browser, width) {
  streamed([101, 102, 103]);
  const { context, page, state } = await open(browser, width);
  let socket = null;
  await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
  const label = `${width} streamed redraw`; const rows = '#plsongs .srow[data-srow]:not([data-card])';
  const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await page.waitForTimeout(50); } return false; };
  // (drawn again, and done: a moved list draws first from the earlier answer, its chips still waiting)
  const redraw = async (order) => {
    await page.evaluate((sel) => { document.querySelector(sel).dataset.old = '1'; }, rows);
    streamed(order); socket.send(JSON.stringify({ type: 'library_changed' }));
    return until(() => page.evaluate(([sel, uri]) => { const r = document.querySelector(sel); return !!r && !r.dataset.old && r.dataset.suri === uri && !document.querySelector('#plsongs .chip-sk'); }, [rows, `tidal:track:${order[0]}`]));
  };
  // a click on the row, then (after the redraw) a double click's second on the same place
  const second = async () => { const b = await dur(page.locator(rows).first()).boundingBox(); await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2); await page.mouse.down({ clickCount: 2 }); await page.mouse.up({ clickCount: 2 }); await page.waitForTimeout(600); };
  try {
    await fx.go(page, '/playlist/c9');
    const row = page.locator(rows).first(); await row.waitFor(); await page.waitForTimeout(500);
    check(await row.getAttribute('data-tid') === null && await row.getAttribute('data-suri') === 'tidal:track:101', `${label}: the streamed row has no track id and is known by its address (${await row.getAttribute('data-suri')})`);
    await reset(page);
    await dur(row).click(); await page.waitForTimeout(300);
    check(await redraw([102, 101, 103]), `${label}: the list was drawn again with Quebec first`);
    await second();
    await page.screenshot({ path: path.join(shots, `streamed-redraw-${width}.png`) });
    check(await page.evaluate(() => window.__details.slice(-1)[0]) === 2 && await starts(page) === '["s101","s102"]', `${label}: a double click's second on the song that moved into the row plays it (${await starts(page)}, want ["s101","s102"])`);
    await page.waitForTimeout(800); await reset(page);
    await dur(page.locator(rows).first()).click(); await page.waitForTimeout(300);
    check(await redraw([102, 101, 103]), `${label}: the list was drawn again unchanged`);
    await second();
    check(await page.evaluate(() => window.__details.slice(-1)[0]) === 2 && await starts(page) === '["s102"]', `${label}: …a double click's second on the same song in the same place doesn't start it again (${await starts(page)}, want ["s102"])`);
    check(state.errors.length === 0, `${label}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); streamed([101, 102, 103]); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) await desktop(browser, w);
    await twoLists(browser, 1280);
    await streamedRedraw(browser, 1280);
    await touch(browser, 768); await touch(browser, 412);
  } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})();
