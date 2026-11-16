// Empty pages whose button depends on whether the library has songs: Favourites (Open Songs when
// there are songs, else the owner's Add music; a listener with no songs gets none) and the queue
// (Shuffle my music only with songs). When the library changes, the page is redrawn at once while
// the node's new song count (GET /info) is still on its way, so it used the old count: right after
// the first songs arrived, Favourites still offered Add music and the queue no Shuffle my music.
// The same came from a first scan: its 2 s check of GET /info saw the songs but drew neither page.
// Now an empty state that shows on screen is drawn again when a new count turns "no songs" into
// "songs" or back (after a library change and during a scan). Without such a change, or with
// favourites listed (no empty state), there is no second redraw (Favourites is fetched once per
// library change, the queue's empty box stays). Favourites is drawn again where it stands (no
// skeleton in between), and the keyboard on its empty-state button stays on the new button (Add
// music → Open Songs), unless they moved elsewhere meanwhile (it stays there). At 1280, 1024 and 768 px. Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-library-count-redraw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'library-count-redraw-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const emptyFavs = { '/favourites': { body: { uris: [], albums: [], artists: [], tracks: [], other: [] } } };

const buttons = (page) => page.evaluate(() => { const b = document.querySelector('.main .empty'); return b ? [...b.querySelectorAll('button')].map((x) => x.textContent.trim()) : null; });
const text = (page) => page.evaluate(() => document.querySelector('.main .empty p')?.textContent || '');

const two = [fx.track, { ...fx.track, id: 2 }];
const cases = [
  { name: 'owner-favourites-first-songs', url: '/favourites', opts: { tracks: [] }, before: ['Add music'], after: ['Open Songs'], to: [fx.track] },
  // The keyboard left the button for Search after the change was drawn, while the new count was
  // still coming: the second redraw doesn't take it back
  { name: 'owner-favourites-first-songs-moved', url: '/favourites', opts: { tracks: [] }, before: ['Add music'], after: ['Open Songs'], to: [fx.track], moved: true },
  { name: 'owner-favourites-songs-gone', url: '/favourites', opts: {}, before: ['Open Songs'], after: ['Add music'], to: [] },
  { name: 'listener-favourites-no-songs-left', url: '/favourites', opts: { role: 'listener' }, before: ['Open Songs'], after: [], afterText: 'Once the node’s owner adds music', to: [] },
  { name: 'owner-queue-first-songs', url: '/queue', opts: { tracks: [] }, before: [], after: ['Shuffle my music'], to: [fx.track] },
  { name: 'owner-queue-songs-gone', url: '/queue', opts: {}, before: ['Shuffle my music'], after: [], to: [] },
  { name: 'owner-favourites-more-songs', url: '/favourites', opts: {}, before: ['Open Songs'], after: ['Open Songs'], to: two, once: true },
  { name: 'owner-queue-more-songs', url: '/queue', opts: {}, before: ['Shuffle my music'], after: ['Shuffle my music'], to: two, kept: true },
  // Favourites listed: no empty state, so the new count draws nothing again
  { name: 'listener-favourites-listed-no-songs-left', url: '/favourites', opts: { role: 'listener', favs: [fx.SONG] }, listed: true, to: [], once: true },
  // A first scan: no library change yet, only its 2 s check of GET /info
  { name: 'owner-favourites-scan', url: '/favourites', scan: true, before: ['Add music'], after: ['Open Songs'] },
  { name: 'owner-queue-scan', url: '/queue', scan: true, before: [], after: ['Shuffle my music'] },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      for (const c of cases) {
        // GET /info answers late once the library has changed, as a busy node does
        let slow = false;
        const info = { name: 'Fixture node', offline: false, scanning: true, library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0 } };
        const fixtures = { ...(c.listed ? {} : emptyFavs), ...(c.scan ? { '/info': { body: info } } : {}) };
        const { page, context, state } = await fx.create(browser, width, { fixtures, ...c.opts, delay: (v) => (slow && v === '/info' ? 500 : 0) });
        let socket = null;
        await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
        try {
          await fx.go(page, c.url);
          const shown = await page.waitForSelector(c.listed ? '.main .track' : '.main .empty', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${width} ${c.name}: the page shows ${c.listed ? 'its favourites' : 'its empty state'}`);
          if (!shown) continue;
          for (let i = 0; i < 50 && !socket; i++) await sleep(100);
          check(!!socket, `${width} ${c.name}: events connected`);
          if (!socket) continue;
          await sleep(300);
          const b0 = await buttons(page);
          if (!c.listed) check(JSON.stringify(b0) === JSON.stringify(c.before), `${width} ${c.name}: before the change: ${c.before.join(', ') || 'no button'} (${(b0 || []).join(', ')})`);
          const favHits = () => state.hits.filter((h) => h.v === '/favourites' && h.m === 'GET').length;
          const f0 = favHits();
          if (c.kept) await page.$eval('.main .empty', (b) => { b.dataset.old = '1'; });
          // The keyboard is on the empty state's button (no skeleton is drawn from here on)
          const keyed = !c.listed && c.before.length > 0;
          if (keyed) await page.focus('.main .empty > .btn');
          await page.evaluate(() => { window.__sk = 0; new MutationObserver(() => { if (document.querySelector('.main .skeleton')) window.__sk++; }).observe(document.querySelector('.main'), { childList: true, subtree: true }); });
          if (c.scan) info.library.tracks = 1; // the scan's next check counts it; no library change is sent
          else { state.tracks = c.to; slow = true; socket.send(JSON.stringify({ type: 'library_changed' })); }
          if (c.moved) { for (let i = 0; i < 40 && favHits() === f0; i++) await sleep(25); await sleep(50); await page.focus('#search'); }
          if (c.listed) {
            await sleep(1500);
            check(favHits() - f0 === 1, `${width} ${c.name}: Favourites is fetched once for the change (${favHits() - f0})`);
            check(await page.locator('.main .track').count() > 0 && !(await page.$('.main .empty')), `${width} ${c.name}: the favourites stay listed`);
            check(!state.errors.length, `${width} ${c.name}: no page errors (${state.errors.join(' | ')})`);
            continue;
          }
          const want = JSON.stringify(c.after);
          const got = await page.waitForFunction((w) => { const b = document.querySelector('.main .empty'); return !!b && JSON.stringify([...b.querySelectorAll('button')].map((x) => x.textContent.trim())) === w; }, want, { timeout: c.scan ? 5000 : 4000 }).then(() => true, () => false);
          await sleep(900); // the late /info is in, any redraw it causes is done
          const b1 = await buttons(page);
          check(got && JSON.stringify(b1) === want, `${width} ${c.name}: after the change: ${c.after.join(', ') || 'no button'} (${(b1 || []).join(', ')})`);
          if (c.url === '/favourites') check(await page.evaluate(() => window.__sk) === 0, `${width} ${c.name}: Favourites is drawn again where it stands, no skeleton in between`);
          if (keyed) {
            const at = await page.evaluate(() => { const a = document.activeElement; return a ? (a.id === 'search' ? '#search' : a.matches('.main .empty > .btn') ? a.textContent.trim() : a.tagName.toLowerCase()) : ''; });
            const wantAt = c.moved ? '#search' : c.after.length ? c.after[0] : null;
            if (wantAt) check(at === wantAt, `${width} ${c.name}: the keyboard is ${c.moved ? 'still in Search' : `on ${wantAt}`} (${at})`);
          }
          if (c.afterText) { const t = await text(page); check(t.startsWith(c.afterText), `${width} ${c.name}: it says "${c.afterText}…" (${t})`); }
          if (c.kept) check(await page.$eval('.main .empty', (b) => b.dataset.old === '1').catch(() => false), `${width} ${c.name}: the queue's empty box is not drawn again`);
          if (c.once) check(favHits() - f0 === 1, `${width} ${c.name}: Favourites is fetched once for the change (${favHits() - f0})`);
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${width} ${c.name}: nothing wider than the screen (${o})`);
          await page.screenshot({ path: path.join(SHOTS, `${c.name}-${width}.png`) });
          check(!state.errors.length, `${width} ${c.name}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Favourites from other services, drawn again by a library change: their rows stay on screen
      // (no skeleton, same rows) until the slow new description comes, then the new rows are drawn
      {
        const name = 'favourites-other-services-kept'; const U = 'https://audius.co/sefa/one';
        let n = 0; let slow = false;
        const described = () => [{ uri: U, page: U, title: `Other ${++n}`, site: 'audius.co', also_on: [], kind: 'track' }];
        const fixtures = { '/favourites': { body: { uris: [U], albums: [], artists: [], tracks: [], other: [U] } } };
        const { page, context, state } = await fx.create(browser, width, { fixtures });
        await page.route('**/v1/discover/describe', async (r) => { if (slow) await sleep(900); return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(described()) }); });
        let socket = null;
        await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
        try {
          await fx.go(page, '/favourites');
          const shown = await page.waitForSelector('#favother [data-ditem]', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${width} ${name}: the other services' favourite is listed`);
          for (let i = 0; i < 50 && !socket; i++) await sleep(100);
          if (shown && socket) {
            await sleep(300);
            await page.$eval('#favother [data-ditem]', (r) => { r.dataset.old = '1'; });
            await page.evaluate(() => { window.__osk = 0; new MutationObserver(() => { if (document.querySelector('#favother .skeleton')) window.__osk++; }).observe(document.querySelector('.main'), { childList: true, subtree: true }); });
            slow = true; socket.send(JSON.stringify({ type: 'library_changed' }));
            await sleep(500);
            check(await page.$eval('#favother [data-ditem]', (r) => r.dataset.old === '1').catch(() => false), `${width} ${name}: while the new description comes, the row on screen stays`);
            const fresh = await page.waitForFunction(() => { const r = document.querySelector('#favother [data-ditem]'); return !!r && !r.dataset.old; }, null, { timeout: 4000 }).then(() => true, () => false);
            check(fresh, `${width} ${name}: then the new rows are drawn`);
            check(await page.evaluate(() => window.__osk) === 0, `${width} ${name}: no skeleton in between`);
          }
          check(!state.errors.length, `${width} ${name}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${passed} ok, ${failed} FAIL` : `${passed} ok, all passed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
