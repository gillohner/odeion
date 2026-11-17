// The keyboard on a list row stays on it when the list is drawn again in place (a scan, an edit,
// an import: the node's "library changed"). Before, a row drawn again was keyboard-reachable only
// on the next frame (the clickable-row tagger), so putting the focus back did nothing and it fell
// to the page: the next Tab started again at the top. Artists' rows, and Favourites' songs from
// other services (drawn twice: kept as they were, then the new ones once described). The next Tab
// goes on to the next row. At 1280, 1024 and 768 px.
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
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
