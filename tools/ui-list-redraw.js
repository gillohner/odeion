// Albums and Artists drawn again where they stand (the library changed: a scan, an identify run,
// an edit) keep the page on screen while the new list comes, then keep what is typed in the
// filter (typing meanwhile counts) with its caret and focus, the place a long list was scrolled
// to (and the focused album), and a chosen A–Z letter. Going elsewhere meanwhile isn't undone;
// opening the page again still starts it empty. (A sort still draws from the start: the
// "no Sort while a redraw loads" check in ui-rows-search-sort.js.)
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-list-redraw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/list-redraw-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// 300 of each, so a long list grows as it scrolls (120 albums, 150 artists at a time)
const albums = Array.from({ length: 300 }, (_, i) => ({ id: i + 1, title: i < 3 ? `Loop ${i + 1}` : `Other ${String(i).padStart(3, '0')}`, album_artist: `Band ${i}`, year: 1990, tracks: 1 }));
const artists = albums.map((a) => ({ id: a.id, name: a.title, albums: 1, tracks: 3 }));
const fixtures = { '/library/albums': { body: albums }, '/library/albums/formats': { body: [] }, '/library/artists': { body: artists } };
const pages = {
  albums: { url: '/albums', list: /\/v1\/library\/albums(\?|$)/, rows: '#albumgrid .tile', attr: 'data-album', box: '#albumgrid', nav: 'artists' },
  artists: { url: '/artists', list: /\/v1\/library\/artists(\?|$)/, rows: '#artistlist [data-artist]', attr: 'data-artist', box: '#artistlist', nav: 'albums' },
};

async function run(browser, width, kind) {
  const k = pages[kind];
  const { page, context, state } = await fx.create(browser, width, { fixtures });
  // The list's next answer can be held until the test lets it go
  let gate = null;
  await context.route(k.list, async (route) => { const g = gate; if (g) { g.entered(); await g.p; } return route.fallback(); });
  const hold = () => { const g = {}; g.p = new Promise((r) => { g.open = r; }); g.in = new Promise((r) => { g.entered = r; }); gate = g; return g; };
  let socket = null;
  await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
  const changed = () => socket.send(JSON.stringify({ type: 'library_changed' }));
  const rows = () => page.locator(k.rows).count();
  // A new list element: the redraw is done
  const markBox = () => page.$eval(k.box, (b) => { b.dataset.old = '1'; });
  const redrawn = () => page.waitForFunction((sel) => { const b = document.querySelector(sel); return b && !b.dataset.old; }, k.box);
  const skeletonSeen = () => page.evaluate(() => window.__skeleton === true);
  try {
    await fx.go(page, k.url);
    await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length >= n, [k.rows, 12]);
    for (let i = 0; i < 50 && !socket; i++) await sleep(100);
    assert.ok(socket, 'events connected');
    // Any skeleton drawn into the page from now on is noted
    await page.evaluate(() => { window.__skeleton = false; new MutationObserver(() => { if (document.querySelector('main .sk-grid')) window.__skeleton = true; }).observe(document.querySelector('main'), { childList: true, subtree: true }); });

    // 1. A typed filter, its caret and focus; typing while the new list comes counts too
    await page.fill('#filter', 'loop');
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length === 3, [k.rows]);
    await markBox();
    const g = hold(); changed(); await g.in;
    assert.equal(await page.locator('#filter').count(), 1, `${kind} ${width}: the page stays while the new list comes`);
    await page.locator('#filter').press('End'); await page.keyboard.type(' 2'); await page.keyboard.press('ArrowLeft'); await page.keyboard.press('ArrowLeft');
    await page.screenshot({ path: path.join(shots, `${kind}-${width}-redraw-waiting.png`) });
    g.open(); gate = null;
    await redrawn();
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length === 1, [k.rows]);
    assert.equal(await skeletonSeen(), false, `${kind} ${width}: no skeleton while drawn again`);
    assert.equal(await page.locator('#filter').inputValue(), 'loop 2', `${kind} ${width}: the filter keeps what was typed, also while it loaded`);
    assert.deepEqual(await page.locator(k.rows).evaluateAll((els, a) => els.map((e) => e.getAttribute(a)), k.attr), ['2'], `${kind} ${width}: only the match`);
    assert.equal(await page.locator('.page-head .count').getAttribute('data-filtered'), '1 shown', `${kind} ${width}: the count follows`);
    assert.deepEqual(await page.$eval('#filter', (f) => [document.activeElement === f, f.selectionStart, f.selectionEnd]), [true, 4, 4], `${kind} ${width}: focus and caret stay in the filter`);
    // Typing goes on where the caret was
    await page.keyboard.type('x');
    assert.equal(await page.locator('#filter').inputValue(), 'loopx 2');
    await page.screenshot({ path: path.join(shots, `${kind}-${width}-filter-kept.png`) });

    // 2. A long list scrolled far down stays there, with the focused album
    await page.locator('#filter').press('Escape');
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length >= 120, [k.rows]);
    for (let i = 0; i < 20 && await rows() < 300; i++) { await page.evaluate(() => { const m = document.querySelector('main'); m.scrollTo({ top: m.scrollHeight, behavior: 'instant' }); }); await sleep(150); }
    assert.equal(await rows(), 300, `${kind} ${width}: the list grew as it scrolled`);
    const before = await page.evaluate(([sel, a]) => {
      const m = document.querySelector('main'); m.scrollTo({ top: Math.round(m.scrollHeight * 0.7), behavior: 'instant' });
      const r = m.getBoundingClientRect(); const els = [...document.querySelectorAll(sel)];
      const mid = els.find((e) => { const b = e.getBoundingClientRect(); return b.top > r.top + 120 && b.bottom < r.bottom - 120; });
      if (mid && mid.tagName === 'BUTTON') mid.focus({ preventScroll: true });
      return { top: m.scrollTop, id: mid ? mid.getAttribute(a) : null, focused: document.activeElement === mid };
    }, [k.rows, k.attr]);
    assert.ok(before.top > 3000 && before.id, `${kind} ${width}: scrolled far down (${before.top})`);
    await markBox(); changed(); await redrawn(); await sleep(400);
    const after = await page.evaluate(([sel, a]) => { const m = document.querySelector('main'); const r = m.getBoundingClientRect(); const e = document.querySelector(`${sel.split(' ')[0]} [${a}="${CSS.escape(window.__want)}"]`); return { top: m.scrollTop, focus: document.activeElement?.getAttribute(a) || document.activeElement?.tagName, seen: !!e && e.getBoundingClientRect().top >= r.top && e.getBoundingClientRect().bottom <= r.bottom }; }, [k.rows, k.attr, await page.evaluate((id) => { window.__want = id; }, before.id)]);
    assert.ok(Math.abs(after.top - before.top) <= 2, `${kind} ${width}: the scroll stays (${before.top} → ${after.top})`);
    assert.ok(after.seen, `${kind} ${width}: the same row is still on screen`);
    if (before.focused) assert.equal(after.focus, before.id, `${kind} ${width}: the focused album keeps focus`);
    assert.equal(await skeletonSeen(), false, `${kind} ${width}: no skeleton`);
    await page.screenshot({ path: path.join(shots, `${kind}-${width}-scroll-kept.png`) });

    // 3. A chosen A–Z letter is chosen again: the list still starts at that letter
    await page.evaluate(() => document.querySelector('main').scrollTo({ top: 0, behavior: 'instant' }));
    await page.locator('[data-az="O"]').click();
    const first = () => page.locator(k.rows).first().getAttribute(k.attr);
    await page.waitForFunction(([sel]) => document.querySelector('[data-az="O"]')?.classList.contains('on') && document.querySelector(sel), [k.rows]);
    const fromO = await first();
    assert.equal(fromO, '4', `${kind} ${width}: O starts at Other 003`);
    for (let i = 0; i < 20 && await rows() < 297; i++) { await page.evaluate(() => { const m = document.querySelector('main'); m.scrollTo({ top: m.scrollHeight, behavior: 'instant' }); }); await sleep(150); }
    assert.equal(await rows(), 297, `${kind} ${width}: from O to the end`);
    // 100 gone from the library (at the end of the list): the list is shorter, with no empty
    // space held below it
    fixtures[kind === 'albums' ? '/library/albums' : '/library/artists'].body = (kind === 'albums' ? albums : artists).slice(0, 200);
    await markBox(); changed(); await redrawn();
    assert.equal(await first(), fromO, `${kind} ${width}: the list still starts at O`);
    assert.equal(await page.locator('[data-az="O"]').getAttribute('aria-pressed'), 'true', `${kind} ${width}: O is still chosen`);
    assert.equal(await rows(), 197, `${kind} ${width}: the shorter list`);
    assert.equal(await page.evaluate(() => 'hold' in document.querySelector('main').dataset), false, `${kind} ${width}: no spacer held`);
    fixtures['/library/albums'].body = albums; fixtures['/library/artists'].body = artists;

    // 4. Going to another page while the new list comes: that page stays
    // (the sidebar, or the Library chips where there is none)
    const open = async (v) => { const side = page.locator(`.nav-item[data-view="${v}"]`); await (await side.isVisible() ? side : page.locator(`[data-libnav="${v}"]`)).click(); };
    const g2 = hold(); changed(); await g2.in;
    await open(k.nav);
    await page.waitForFunction((v) => location.pathname === `/${v}` && document.querySelector('.page-head h2')?.textContent.toLowerCase().startsWith(v), k.nav);
    g2.open(); gate = null; await sleep(400);
    assert.ok((await page.locator('.page-head h2').textContent()).toLowerCase().startsWith(k.nav), `${kind} ${width}: the page gone to stays`);

    // 5. Opening the page again starts it from the top with an empty filter
    await open(kind);
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length >= 12, [k.rows]);
    await page.fill('#filter', 'loop');
    await open(k.nav);
    await page.waitForFunction((v) => location.pathname === `/${v}` && !document.querySelector('main .sk-grid') && document.querySelector('.page-head h2')?.textContent.toLowerCase().startsWith(v), k.nav);
    await open(kind);
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length >= 12, [k.rows]);
    assert.equal(await page.locator('#filter').inputValue(), '', `${kind} ${width}: opened again, the filter is empty`);
    assert.equal(await page.evaluate(() => document.querySelector('main').scrollTop), 0, `${kind} ${width}: opened again, at the top`);
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log(`PASS ${kind} ${width}: drawn again where it stands, the filter, caret, focus and scroll stay`);
  } finally { if (gate) gate.open(); await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) for (const kind of Object.keys(pages)) await run(browser, width, kind);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
