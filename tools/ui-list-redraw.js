// Albums and Artists drawn again where they stand (the library changed: a scan, an identify run,
// an edit) keep the page on screen while the new list comes, then keep what is typed in the
// filter (typing meanwhile counts) with its caret and focus, the place a long list was scrolled
// to (and the focused album), and a chosen A–Z letter. Going elsewhere meanwhile isn't undone;
// opening the page again still starts it empty. (A sort still draws from the start: the
// "no Sort while a redraw loads" check in ui-rows-search-sort.js.) Two answers out of order (a
// redraw's list still coming when a sort is chosen) never put the older list or its error back,
// and changes coming faster than a list loads still draw each answer; a filter
// that matches nothing isn't announced again when the list is drawn again; a filter that is off
// (Favourites on, none shown) keeps its text; and Favourites still on when a slow favourites load
// comes in shows the new ones beside those already listed.
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
  // The list's next answer can be held until the test lets it go (`old`: then answered with the
  // list as it was when asked, however the library changed meanwhile)
  let gate = null;
  await context.route(k.list, async (route) => { const g = gate; if (g) { const old = g.old; g.entered(); await g.p; if (old) return route.fulfill({ status: old.status || 200, contentType: 'application/json', body: JSON.stringify(old.status ? { error: 'The library did not answer' } : old) }); } return route.fallback(); });
  const hold = (old) => { const g = { old }; g.p = new Promise((r) => { g.open = r; }); g.in = new Promise((r) => { g.entered = r; }); gate = g; return g; };
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

    // 6. A filter that matches nothing: drawn again, its line is there at once and isn't said again
    // (a screen reader heard it once); a new filter that matches nothing is said
    await page.fill('#filter', 'zzz');
    const line = `No ${kind === 'albums' ? 'album' : 'artist'} matches “zzz”.`;
    await page.waitForFunction((t) => document.querySelector('.filter-none')?.textContent === t, line);
    await page.evaluate(() => { window.__said = 0; new MutationObserver((ms) => { for (const m of ms) { const t = m.target.nodeType === 3 ? m.target.parentElement : m.target; if (t && t.classList && t.classList.contains('filter-none')) window.__said++; } }).observe(document.querySelector('main'), { childList: true, subtree: true, characterData: true }); });
    await markBox(); changed(); await redrawn(); await sleep(600);
    assert.equal(await page.locator('.filter-none').textContent(), line, `${kind} ${width}: the no-match line stays`);
    assert.equal(await page.evaluate(() => window.__said), 0, `${kind} ${width}: the same no-match line isn't said again when drawn again`);
    await page.locator('#filter').press('End'); await page.keyboard.type('q');
    await page.waitForFunction((t) => document.querySelector('.filter-none')?.textContent === t, line.replace('zzz', 'zzzq'));
    assert.ok(await page.evaluate(() => window.__said) > 0, `${kind} ${width}: a new no-match line is said`);
    await page.locator('#filter').press('Escape');

    // 7. Two answers out of order: a redraw's list still on its way when a sort is chosen (the
    // library changed meanwhile) never puts its older list back over the sorted, newer one
    const key = kind === 'albums' ? '/library/albums' : '/library/artists';
    const full = fixtures[key].body;
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length >= 12, [k.rows]);
    const g3 = hold(full); changed(); await g3.in; gate = null;
    fixtures[key].body = full.slice(0, 200);
    await page.selectOption('[data-lsort]', 'added');
    await page.waitForFunction(([sel, a]) => document.querySelector('.page-head .count')?.textContent === '200' && document.querySelector(sel)?.getAttribute(a) === '200', [k.rows, k.attr]);
    g3.open(); await sleep(600);
    assert.equal(await page.locator('.page-head .count').textContent(), '200', `${kind} ${width}: the older answer doesn't replace the newer list`);
    assert.equal(await page.locator(k.rows).first().getAttribute(k.attr), '200', `${kind} ${width}: still sorted, newest first`);
    // Opened again: the newer list too (the older answer didn't replace what the page keeps)
    await open(k.nav);
    await page.waitForFunction((v) => location.pathname === `/${v}` && !document.querySelector('main .sk-grid'), k.nav);
    await open(kind);
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length >= 12, [k.rows]);
    assert.equal(await page.locator('.page-head .count').textContent(), '200', `${kind} ${width}: opened again, the newer list`);
    // An older answer that fails doesn't put its error over the newer list either
    const g4 = hold({ status: 500 }); changed(); await g4.in; gate = null;
    await page.selectOption('[data-lsort]', 'name');
    await page.waitForFunction(([sel, a]) => document.querySelector(sel)?.getAttribute(a) === '1', [k.rows, k.attr]);
    g4.open(); await sleep(600);
    assert.equal(await page.locator('#navretry').count(), 0, `${kind} ${width}: an older answer's error isn't drawn over the newer list`);
    assert.equal(await page.locator(k.rows).first().getAttribute(k.attr), '1', `${kind} ${width}: the newer list stays`);
    fixtures[key].body = full;

    // 9. Changes coming faster than the list loads: each answer is drawn as it comes, the page
    // isn't left on its loading placeholder until they stop
    await open(k.nav);
    await page.waitForFunction((v) => location.pathname === `/${v}` && !document.querySelector('main .sk-grid'), k.nav);
    changed(); await sleep(400); // (the albums kept in memory are let go: the list is asked for)
    const g5 = hold(); await open(kind); await g5.in;
    const g6 = hold(); changed(); await g6.in; gate = null;
    g5.open();
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length >= 12, [k.rows], { timeout: 4000 })
      .catch(() => assert.fail(`${kind} ${width}: the first answer is drawn while a newer one is on its way`));
    g6.open();
    await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length >= 12 && !document.querySelector('main .sk-grid'), [k.rows]);

    // 8. Favourites on with none shown turns the filter off: its text stays through a redraw and
    // filters again once Favourites is off
    if (kind === 'albums') {
      await page.fill('#filter', 'loop');
      await page.waitForFunction(([sel]) => document.querySelectorAll(sel).length === 3, [k.rows]);
      await page.locator('[data-lfav]').click();
      await page.waitForFunction(() => document.querySelector('#filter')?.disabled && document.querySelector('#filter').value === 'loop');
      await page.evaluate(() => { document.querySelector('.page-head').dataset.old = '1'; });
      changed();
      await page.waitForFunction(() => { const h = document.querySelector('.page-head'); return h && !h.dataset.old; });
      await sleep(200);
      assert.equal(await page.locator('#filter').inputValue(), 'loop', `${kind} ${width}: the filter that is off keeps its text when drawn again`);
      await page.locator('[data-lfav]').click();
      await page.waitForFunction(([sel]) => !document.querySelector('#filter')?.disabled && document.querySelectorAll(sel).length === 3, [k.rows]);
      assert.equal(await page.locator('#filter').inputValue(), 'loop', `${kind} ${width}: and filters again once Favourites is off`);
    }
    assert.deepEqual(state.errors, [], 'no page errors');
    console.log(`PASS ${kind} ${width}: drawn again where it stands, the filter, caret, focus and scroll stay`);
  } finally { if (gate) gate.open(); await context.close(); }
}

// Favourites on, some listed already: a heart saved while the favourites are slow to come (over
// the 3 s Albums waits) shows its album once they are in, drawn where it stands
async function lateFavourites(browser, width) {
  const tracks = [{ ...fx.track, id: 1, title: 'Song 1' }];
  const favs = (uris) => ({ body: { uris, tracks: [], albums: [], artists: [], other: [] } });
  const fix = { ...fixtures, '/library/tracks': { body: { tracks, next: null } }, '/favourites': favs(['album:1']) };
  const { page, context, state } = await fx.create(browser, width, { fixtures: fix, tracks });
  await context.addInitScript(() => localStorage.setItem('pm.list.albums', JSON.stringify({ fav: true })));
  const tiles = () => page.locator('#albumgrid .tile').evaluateAll((els) => els.map((e) => e.getAttribute('data-album')));
  try {
    await fx.go(page, '/albums');
    await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length === 1);
    await fx.go(page, '/tracks');
    await page.locator('#tracklist [data-act="fav"]').first().waitFor();
    // Hearted elsewhere meanwhile: album 2 comes with the next favourites, 6 s late
    fix['/favourites'] = favs(['album:1', 'album:2']);
    state.delay = (v, m) => (v === '/favourites' && m === 'GET' ? 6000 : 0);
    await page.locator('#tracklist [data-act="fav"]').first().click();
    const open = page.locator('.nav-item[data-view="albums"]');
    await (await open.isVisible() ? open : page.locator('[data-libnav="albums"]')).click();
    await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length >= 1, null, { timeout: 6000 });
    assert.deepEqual(await tiles(), ['1'], `late favourites ${width}: the favourites listed already show first (the case this checks)`);
    await page.locator('#albumgrid .tile').first().focus();
    await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length === 2, null, { timeout: 8000 })
      .catch(() => assert.fail(`late favourites ${width}: the new favourite shows once they are in`));
    assert.deepEqual((await tiles()).sort(), ['1', '2'], `late favourites ${width}: both favourites`);
    assert.equal(await page.locator('.page-head .count').textContent(), '2 of 300', `late favourites ${width}: the count follows`);
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('data-album')), '1', `late favourites ${width}: the focused album keeps focus`);
    assert.deepEqual(state.errors, [], 'no page errors');
    await page.screenshot({ path: path.join(shots, `albums-${width}-late-favourite.png`) });
    console.log(`PASS late favourites ${width}: a favourite that comes late joins those listed`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) { for (const kind of Object.keys(pages)) await run(browser, width, kind); await lateFavourites(browser, width); }
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
