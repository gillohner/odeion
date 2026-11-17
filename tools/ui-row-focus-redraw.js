// The keyboard on a list row stays on it when the list is drawn again in place (a scan, an edit,
// an import: the node's "library changed"). Before, a row drawn again was keyboard-reachable only
// on the next frame (the clickable-row tagger), so putting the focus back did nothing and it fell
// to the page: the next Tab started again at the top. Artists' rows, and Favourites' songs from
// other services (drawn twice: kept as they were, then the new ones once described). The next Tab
// goes on to the next row. Favourites' songs from other services are kept by song, not by place:
// one gone or added above doesn't move the keyboard onto another song. At 1280, 1024 and 768 px.
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-row-focus-redraw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'row-focus-redraw-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (f, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await f()) return true; await sleep(50); } return false; };

const artists = Array.from({ length: 8 }, (_, i) => ({ id: i + 1, name: `Artist ${i + 1}`, albums: 1, tracks: 2 }));
const other = Array.from({ length: 4 }, (_, i) => ({ uri: `https://sefa.bandcamp.com/track/t${i}`, title: `Elsewhere ${i}`, artist: 'Sefa', kind: 'track', site: 'bandcamp' }));
const pages = [
  { name: 'Artists', url: '/artists', row: (k) => `#main [data-artist="${k}"]`, list: '#artistlist' },
  { name: 'Favourites', url: '/favourites', row: (k) => `#main [data-ditem="${k - 1}"]`, list: '#favother .list' },
];

const song = (n) => ({ uri: `https://sefa.bandcamp.com/track/t${n}`, title: `Elsewhere ${n}`, artist: 'Sefa', kind: 'track', site: 'bandcamp' });
async function bySong(browser, width) {
  const favs = (list) => ({ '/favourites': { body: { uris: list.map((o) => o.uri), tracks: [], albums: [], artists: [], other: list } }, '/discover/describe': { body: list } });
  const start = [0, 1, 2, 3].map(song);
  const { context, page, state } = await fx.create(browser, width, { fixtures: favs(start) });
  let socket = null;
  await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
  // Where the keyboard is: the song of the row it is in (read from the row's own title), and the control
  const on = () => page.evaluate(() => { const a = document.activeElement; if (!a || a === document.body) return 'the page'; const row = a.closest('#favother .list-row'); if (!row) return a.id || a.tagName; const t = row.querySelector('.name').firstChild.textContent.trim(); return a === row ? t : `${t} ${a.getAttribute('title')}`; });
  const cases = [
    { name: 'a favourite above it gone, its row', focus: 2, part: '', now: [1, 2, 3], want: 'Elsewhere 2' },
    { name: 'a favourite above it gone, its Play', focus: 2, part: ' [data-dplay]', now: [1, 2, 3], want: 'Elsewhere 2 Play' },
    { name: 'a favourite added above it, its row', focus: 2, part: '', now: [9, 0, 1, 2, 3], want: 'Elsewhere 2' },
    { name: 'that song itself gone, its row', focus: 1, part: '', now: [0, 2, 3], want: 'Elsewhere 2' },
    { name: 'that song, the last, gone, its Play', focus: 3, part: ' [data-dplay]', now: [0, 1, 2], want: 'Elsewhere 2 Play' },
    { name: 'the songs in another order, its More', focus: 1, part: ' [data-dmore]', now: [0, 2, 3, 1], want: 'Elsewhere 1 More' },
  ];
  try {
    for (const c of cases) {
      const at = `${width} Favourites by song, ${c.name}`;
      Object.assign(state.fixtures, favs(start));
      socket = null; // (the page's own socket, not the last load's)
      await fx.go(page, '/favourites');
      const drawn = await until(() => page.evaluate(() => [...document.querySelectorAll('#favother .list-row .name')].some((n) => n.textContent.includes('Elsewhere 3'))), 6000);
      check(drawn, `${at}: the songs from other services are drawn`);
      if (!drawn) continue;
      await sleep(150);
      const sel = `#favother .list-row:nth-child(${c.focus + 1})${c.part}`;
      await page.evaluate((s) => { const el = document.querySelector(s); el.tabIndex = el.tabIndex < 0 ? 0 : el.tabIndex; el.focus(); }, sel);
      const was = await on();
      check(was.startsWith(`Elsewhere ${c.focus}`), `${at}: the keyboard is on Elsewhere ${c.focus} (${was})`);
      check(await until(() => !!socket), `${at}: the app's event socket is open`);
      if (!socket) continue;
      const now = c.now.map(song);
      Object.assign(state.fixtures, favs(now));
      await page.evaluate(() => { document.querySelector('#favother .list').dataset.old = '1'; });
      socket.send(JSON.stringify({ type: 'library_changed' }));
      // (drawn twice: the old rows kept, then the new ones once described)
      const redrawn = await until(() => page.evaluate((n) => { const b = document.querySelector('#favother'); return !!b && !b.querySelector('.list[data-old]') && document.querySelectorAll('#favother .list-row').length === n; }, now.length));
      await sleep(400);
      check(redrawn, `${at}: the list was drawn again with ${now.length} songs`);
      const kept = await on();
      check(kept === c.want, `${at}: the keyboard is on ${c.want} (${kept})`);
      if (width === 1280) await page.screenshot({ path: path.join(SHOTS, `favourites-by-song-${c.focus}-${c.now.length}${c.part ? '-play' : ''}-${width}.png`) });
    }
    check(!state.errors.length, `${width} Favourites by song: no page errors ${state.errors.join(' | ')}`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch(process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {});
  try {
    for (const width of [1280, 1024, 768]) {
      const { context, page, state } = await fx.create(browser, width, { fixtures: { '/library/artists': { body: artists }, '/favourites': { body: { uris: other.map((o) => o.uri), tracks: [], albums: [], artists: [], other } } } });
      let socket = null;
      await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
      const on = () => page.evaluate(() => { const a = document.activeElement; return !a || a === document.body ? 'the page' : a.dataset.artist ? `artist ${a.dataset.artist}` : a.dataset.ditem ? `other ${Number(a.dataset.ditem) + 1}` : a.id || a.tagName; });
      for (const p of pages) {
        const at = `${width} ${p.name}`;
        await fx.go(page, p.url);
        const drawn = await page.waitForSelector(p.row(3), { timeout: 6000 }).then(() => true, () => false);
        check(drawn, `${at}: the rows are drawn`);
        if (!drawn) continue;
        await sleep(150);
        await page.focus(p.row(3));
        const want = p.name === 'Artists' ? 'artist 3' : 'other 3';
        check(await on() === want, `${at}: the third row takes the keyboard (${await on()})`);
        await page.evaluate((s) => { document.querySelector(s).dataset.old = '1'; }, p.list);
        check(await until(() => !!socket), `${at}: the app's event socket is open`);
        if (!socket) continue;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        const redrawn = await until(() => page.evaluate((s) => { const l = document.querySelector(s); return !!l && !l.dataset.old; }, p.list));
        // (Favourites' other rows come twice: watched until they are the new ones, and a while after)
        await sleep(400);
        check(redrawn, `${at}: the list was drawn again`);
        const kept = await on();
        check(kept === want, `${at}: the keyboard stays on the row it was on when the list is drawn again (${kept})`);
        const isNew = await page.evaluate((s) => !document.activeElement.closest('[data-old]') && document.activeElement.matches(s), p.row(3));
        check(isNew, `${at}: the focus is on the new row, not one taken off the page`);
        await page.screenshot({ path: path.join(SHOTS, `${p.name.toLowerCase()}-kept-${width}.png`) });
        await page.keyboard.press('Tab');
        const next = await on(); const wantNext = p.name === 'Artists' ? 'artist 4' : 'other 3';
        // (an other-service row's next Tab stop is its own Play button)
        check(p.name === 'Artists' ? next === wantNext : /^BUTTON$/.test(next) && await page.evaluate(() => !!document.activeElement.closest('[data-ditem="2"]')), `${at}: the next Tab goes on from that row (${next})`);
      }
      check(!state.errors.length, `${width}: no page errors ${state.errors.join(' | ')}`);
      await context.close();
    }
    // Favourites' songs from other services are kept by song, not by place: a favourite gone or
    // added above the focused one leaves the keyboard on that same song (its row, or its Play),
    // and when the focused song itself went, on the one that took its place, or the last one (not the page).
    for (const width of [1280, 1024, 768]) await bySong(browser, width);
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
