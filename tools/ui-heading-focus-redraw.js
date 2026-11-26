// A page opened from inside the old one puts the keyboard focus on its heading (focusPage). When
// that page is then drawn again where it stands (the library changed: a scan, an import, an edit),
// or its songs are still coming and it asks again 3 s later), the focus stays on the new heading
// instead of falling to the page's body (where a screen reader says nothing of the page and the
// heading's place is lost):
//   * an album opened from its tile with the keyboard while its songs are still coming, drawn
//     again when they have come: the album's new
//     heading has the focus, shows no ring, the next Tab goes into the page, and once the focus has
//     left it the heading can't be focused by a click;
//   * Back to Albums (its heading has the focus), then the library changes: the same on Albums;
//   * a control of the page that had the focus (an album tile) still keeps it, as before.
// At 1280, 1024 and 768 px. Real candidate assets, isolated API fixtures (tools/ui-fixture.js).
//   node tools/ui-heading-focus-redraw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'heading-focus-redraw-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const settled = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(r, 150)))));
const albums = [{ id: 7, title: 'Journey', album_artist: 'Sefa', year: 2001, tracks: 1 }, ...Array.from({ length: 11 }, (_, i) => ({ id: 20 + i, title: `Other ${i}`, album_artist: `Band ${i}`, year: 1990, tracks: 1 }))];
const artists = [{ id: 11, name: 'Sefa', albums: 1, tracks: 1 }, { id: 22, name: 'Dr. Peacock', albums: 1, tracks: 1 }];
const focus = (page) => page.evaluate(() => {
  const a = document.activeElement; const main = document.getElementById('main');
  const cs = a ? getComputedStyle(a) : null;
  return { tag: a ? a.tagName : null, body: a === document.body, inMain: !!a && main.contains(a) && a !== main, heading: !!a && /^H[12]$/.test(a.tagName) && main.contains(a), text: a ? (a.textContent || '').trim().slice(0, 40) : '', ring: cs ? cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0 : false, album: a && a.dataset ? a.dataset.album || null : null };
});
// The library changes, and the page on screen is drawn again: its heading is a new element
const redrawn = async (page, changed) => {
  await page.evaluate(() => { const h = document.querySelector('#main h1, #main .page-head h2'); if (h) h.__old = true; });
  changed();
  const ok = await page.waitForFunction(() => { const h = document.querySelector('#main h1, #main .page-head h2'); return !!h && !h.__old; }, null, { timeout: 5000 }).then(() => true, () => false);
  await settled(page);
  return ok;
};

(async () => {
  const browser = await chromium.launch();
  for (const width of [1280, 1024, 768]) {
    const at = `${width}px`;
    const { context, page } = await fx.create(browser, width, { fixtures: { '/library/albums': { body: albums }, '/library/albums/formats': { body: [] }, '/library/artists': { body: artists } } });
    // (the album's page says its songs are still coming the first time it is read: it asks again 3 s later)
    let asked = 0;
    await page.route(/\/v1\/item\/album\/lib-7(\?|$)/, (r) => { asked++; const pg = fx.itemPage('album', 'lib-7', [fx.track]); return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...pg, pending: asked === 1 }) }); });
    let socket = null;
    await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
    const changed = () => socket.send(JSON.stringify({ type: 'library_changed' }));
    await fx.go(page, '/albums'); await page.waitForSelector('#albumgrid .tile[data-album="7"]');
    for (let i = 0; i < 50 && !socket; i++) await sleep(100);
    check(!!socket, `${at}: the node's events are connected`);
    await settled(page);

    // 1. An album opened from its tile with the keyboard while its songs are coming, then drawn again
    await page.focus('#albumgrid .tile[data-album="7"]'); await page.keyboard.press('Enter');
    await page.waitForFunction(() => /Journey/.test((document.querySelector('#main h1, #main .page-head h2') || {}).textContent || '') && !document.querySelector('#albumgrid')); await settled(page);
    const f1 = await focus(page);
    check(f1.heading && /Journey/.test(f1.text), `${at}: the album's heading has the focus once the album is open (${f1.tag} "${f1.text}")`);
    const d1 = await redrawn(page, () => {});
    const f2 = await focus(page);
    check(d1 && asked === 2, `${at}: the album's page is drawn again when it asks again (${asked} reads)`);
    check(f2.heading && /Journey/.test(f2.text), `${at}: after that redraw the album's new heading still has the focus (${f2.tag} "${f2.text}", on the page's body: ${f2.body})`);
    check(!f2.ring, `${at}: the heading still shows no ring (ring ${f2.ring})`);
    await page.screenshot({ path: path.join(SHOTS, `album-redrawn-${width}.png`) });
    await page.keyboard.press('Tab');
    const f3 = await focus(page);
    check(f3.inMain && !f3.heading, `${at}: the next Tab goes into the album's page, not to the sidebar (${f3.tag} "${f3.text}")`);
    const left = await page.evaluate(() => { const h = document.querySelector('#main h1, #main .page-head h2'); return h ? { tabindex: h.getAttribute('tabindex'), mark: h.hasAttribute('data-navfocus') } : null; });
    check(!!left && left.tabindex == null && !left.mark, `${at}: once the focus has left it, the album's heading can't be focused by a click (${JSON.stringify(left)})`);

    // 2. Back to Albums (its heading has the focus), then the library changes
    await page.evaluate(() => { const b = document.querySelector('#main .pagebar .back, #main [data-back]'); if (b) b.focus(); });
    await page.goBack(); await page.waitForSelector('#albumgrid .tile'); await settled(page);
    const f4 = await focus(page);
    check(f4.heading && /^Albums/.test(f4.text), `${at}: Back puts the focus on Albums' heading (${f4.tag} "${f4.text}")`);
    await sleep(1100);
    const d2 = await redrawn(page, changed);
    const f5 = await focus(page);
    check(d2, `${at}: Albums is drawn again when the library changes`);
    check(f5.heading && /^Albums/.test(f5.text) && !f5.ring, `${at}: after that redraw Albums' new heading still has the focus, with no ring (${f5.tag} "${f5.text}", on the page's body: ${f5.body}, ring ${f5.ring})`);
    await page.screenshot({ path: path.join(SHOTS, `albums-redrawn-${width}.png`) });
    await page.keyboard.press('Tab');
    const left2 = await page.evaluate(() => { const h = document.querySelector('#main h1, #main .page-head h2'); return h ? { tabindex: h.getAttribute('tabindex'), mark: h.hasAttribute('data-navfocus') } : null; });
    check(!!left2 && left2.tabindex == null && !left2.mark, `${at}: once the focus has left it, Albums' heading can't be focused by a click (${JSON.stringify(left2)})`);

    // 3. A tile of the page with the focus keeps it, as before
    await page.focus('#albumgrid .tile[data-album="22"]');
    await sleep(1100);
    const d3 = await redrawn(page, changed);
    const f6 = await focus(page);
    check(d3 && f6.album === '22', `${at}: a focused album tile keeps the focus through the redraw (${f6.tag} album ${f6.album})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
