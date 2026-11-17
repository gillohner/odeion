// A control reached by the keyboard is never hidden under a page's compact header, and its focus
// ring is never cut at the page's edge. On an album or a playlist page the compact header (title and
// Play) stays on top once the big one scrolls away; going back up the songs with Shift+Tab used to
// stop each row at the very top, under that header (a song's More button wholly hidden). A control
// scrolled to at the very top (Songs, going back up) or bottom of the page lost 1-4 px of its ring at
// the page's edge.
// While the compact header shows, the page keeps it and 8 px clear at its top: the header is 52 px high on wider screens and 46 px up to 720 px wide (phone), so a control
// scrolled to sits just below it, not hidden and not further down.
// At 1280, 1024, 768 and 412 px (phone).
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-sticky-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'sticky-focus-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// .main scrolls smoothly: read where a control stopped once the page stands still
const settle = async (page) => { let was = null; for (let i = 0; i < 40; i++) { await sleep(40); const t = await page.evaluate(() => document.getElementById('main').scrollTop); if (t === was) return; was = t; } };

const A = [{ id: 11, name: 'Sefa' }];
const fmt = { container: 'flac', format: { kind: 'pcm', bits: 16, rate_hz: 44100, channels: 2 } };
const tracks = Array.from({ length: 24 }, (_, i) => ({ id: i + 1, title: `Journey part ${i + 1}`, artists: A, album: { id: 7, title: 'Journey' }, duration_ms: 200000 + i * 1000, track_no: i + 1, file: fmt }));
const many = Array.from({ length: 50 }, (_, i) => ({ ...tracks[i % tracks.length], id: 100 + i, title: `Long list song ${i + 1}` }));
const summary = { id: 'c1', name: 'Long mix', items: tracks.length, covers: [], visibility: 'private', synced: false, layout: 'list' };
const fixtures = {
  '/library/albums/7': { body: { id: 7, title: 'Journey', year: 2021, artists: A, tracks } },
  '/item/album/lib-7': { body: { kind: 'album', id: 'lib-7', title: 'Journey', source_id: 'library', source: 'Your library', target: { album_id: 7 }, artist: { id: 'lib-11', name: 'Sefa' }, year: 2021,
    songs: tracks.map((t) => ({ title: t.title, artist: 'Sefa', track_id: t.id, uri: `track:${t.id}`, duration_ms: t.duration_ms })), albums: [], releases: [], links: [], also_on: [], tags: [], connect: [], open_on: [], versions: [] } },
  '/playlists': { body: [summary] },
  '/playlists/c1': { body: { summary, playlist: { ...summary, items: tracks.map((t) => ({ uri: `track:${t.id}` })) }, tracks, entries: tracks.map((t) => ({ kind: 'track', uri: `track:${t.id}` })), local_tracks: [] } },
};

// Where the focused control is against the compact header and the page's edges, ring included
const where = () => {
  const e = document.activeElement; const main = document.getElementById('main');
  if (!e || !main.contains(e)) return null;
  const r = e.getBoundingClientRect(); const m = main.getBoundingClientRect();
  const cs = getComputedStyle(e); const g = cs.outlineStyle === 'none' ? 0 : Math.max(0, (parseFloat(cs.outlineWidth) || 0) + (parseFloat(cs.outlineOffset) || 0)); // (a ring drawn inside sticks out by nothing)
  const bar = main.querySelector(':scope > .stickyhead'); const shown = !!bar && !bar.hidden && bar.getBoundingClientRect().height > 0;
  const b = shown ? bar.getBoundingClientRect() : null;
  const label = (e.getAttribute('aria-label') || e.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 40);
  return { label, top: main.scrollTop, h: r.height, fits: r.height + 2 * g < m.height, barShown: shown,
    // (from the header's bottom to where the page lines a control up: its top edge plus the space it keeps clear)
    clear: b ? m.top + main.clientTop + parseFloat(getComputedStyle(main).scrollPaddingTop) - b.bottom : null,
    under: b ? Math.max(0, Math.min(r.bottom + g, b.bottom) - Math.max(r.top - g, b.top)) : 0,
    cutTop: r.top >= m.top - 0.5 ? Math.max(0, m.top - (r.top - g)) : 0,
    cutBottom: r.bottom <= m.bottom + 0.5 ? Math.max(0, r.bottom + g - m.bottom) : 0 };
};

(async () => {
  const browser = await chromium.launch(process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {});
  try {
    for (const width of [1280, 1024, 768, 412]) {
      for (const p of [{ name: 'album', url: '/album/lib-7', rows: '#main .srow' }, { name: 'playlist', url: '/playlist/c1', rows: '#plsongs .srow' }]) {
        const at = `${width} ${p.name}`;
        const { context, page, state } = await fx.create(browser, width, { fixtures, tracks });
        await fx.go(page, p.url);
        const drawn = await page.locator(p.rows).nth(tracks.length - 1).waitFor().then(() => true, () => false);
        check(drawn, `${at}: the songs are drawn`);
        if (!drawn) { await context.close(); continue; }
        await sleep(300);
        // Down the page with Tab from its top: each control stops with its ring inside the page
        await page.evaluate(() => { const m = document.getElementById('main'); m.tabIndex = -1; m.focus(); m.removeAttribute('tabindex'); });
        // (a row that already sits at the edge as the page opens isn't scrolled to: only stops that scroll count)
        const downCut = []; let scrolled = 0; let was = await page.evaluate(() => document.getElementById('main').scrollTop);
        for (let i = 0; i < 200; i++) {
          await page.keyboard.press('Tab'); await settle(page);
          const w = await page.evaluate(where);
          if (!w) { if (i > 5) break; continue; }
          const moved = Math.abs(w.top - was) > 0.5; was = w.top;
          if (!moved) continue;
          scrolled++;
          if (w.fits && w.cutBottom > 0.5) downCut.push(`${w.label} ${w.cutBottom.toFixed(1)}px`);
        }
        check(scrolled > 5 && downCut.length === 0, `${at}: Tab down the page, no control it scrolls to has its ring cut at the page's bottom edge (${scrolled} scrolls)${downCut.length ? ` (${downCut.slice(0, 3).join('; ')})` : ''}`);
        // From the last song's More back up the page with Shift+Tab: the compact header shows, and
        // no control stops under it
        await page.locator(p.rows).last().locator('[data-smore], .icon-btn').last().focus();
        await settle(page);
        const under = []; let barSeen = false; let rowsSeen = 0; let shot = false; const clear = new Set();
        for (let i = 0; i < tracks.length * 4 + 4; i++) {
          await page.keyboard.press('Shift+Tab'); await settle(page);
          const w = await page.evaluate(where);
          if (!w) continue;
          if (w.barShown) { barSeen = true; clear.add(w.clear.toFixed(1)); }
          if (w.barShown && /^Journey part|part \d+$/.test(w.label)) rowsSeen++;
          if (w.under > 0.5 || (w.fits && w.cutTop > 0.5)) {
            under.push(`${w.label} ${w.under > 0.5 ? `${w.under.toFixed(0)}/${w.h.toFixed(0)}px under the header` : `${w.cutTop.toFixed(1)}px cut at the top`}`);
            if (!shot) { shot = true; await page.screenshot({ path: path.join(SHOTS, `${p.name}-under-${width}.png`) }); }
          }
        }
        check(barSeen && rowsSeen > 10, `${at}: Shift+Tab goes back up the songs below the compact header (${rowsSeen} stops with it shown)`);
        check(under.length === 0, `${at}: Shift+Tab up the songs, nothing stops under the compact header or past the top edge${under.length ? ` (${under.length}: ${under.slice(0, 3).join('; ')})` : ''}`);
        check(clear.size === 1 && Math.abs(Number([...clear][0]) - 8) <= 1, `${at}: while the compact header shows, the page keeps it and 8 px clear at its top (${[...clear].join(', ')} px past the header)`);
        await page.screenshot({ path: path.join(SHOTS, `${p.name}-up-${width}.png`) });
        check(state.errors.length === 0, `${at}: no page errors${state.errors.length ? ` (${state.errors[0]})` : ''}`);
        await context.close();
      }
      // Songs, back up from the last song with Shift+Tab: each control stops a little below the top
      // edge, so its focus ring shows whole (it stopped flush, the ring's top 2-4 px cut off)
      {
        const { context, page, state } = await fx.create(browser, width, { fixtures, tracks: many });
        await fx.go(page, '/tracks');
        const drawn = await page.locator('#main .track').nth(many.length - 1).waitFor().then(() => true, () => false);
        check(drawn, `${width} songs: the songs are drawn`);
        if (drawn) {
          await sleep(300);
          await page.locator('#main .track').last().locator('button').last().focus(); await settle(page);
          const cut = []; let scrolled = 0; let was = await page.evaluate(() => document.getElementById('main').scrollTop);
          for (let i = 0; i < many.length * 5; i++) {
            await page.keyboard.press('Shift+Tab'); await settle(page);
            const w = await page.evaluate(where);
            if (!w) continue;
            const moved = Math.abs(w.top - was) > 0.5; was = w.top;
            if (!moved) continue;
            scrolled++;
            if (w.fits && w.cutTop > 0.5) { cut.push(`${w.label} ${w.cutTop.toFixed(1)}px`); if (cut.length === 1) await page.screenshot({ path: path.join(SHOTS, `songs-cut-${width}.png`) }); }
          }
          check(scrolled >= 3 && cut.length === 0, `${width} songs: Shift+Tab up the songs, no control it scrolls to has its ring cut at the page's top edge (${scrolled} scrolls)${cut.length ? ` (${cut.length}: ${cut.slice(0, 3).join('; ')})` : ''}`);
          await page.screenshot({ path: path.join(SHOTS, `songs-up-${width}.png`) });
          check(state.errors.length === 0, `${width} songs: no page errors${state.errors.length ? ` (${state.errors[0]})` : ''}`);
        }
        await context.close();
      }
      // Settings at the bottom: its last control's ring is whole
      const { context, page } = await fx.create(browser, width, { fixtures, tracks });
      await fx.go(page, '/settings'); await sleep(1200);
      await page.evaluate(() => { const m = document.getElementById('main'); m.tabIndex = -1; m.focus(); m.removeAttribute('tabindex'); });
      const cut = []; let scrolled = 0; let was = 0;
      for (let i = 0; i < 120; i++) {
        await page.keyboard.press('Tab'); await settle(page);
        const w = await page.evaluate(where);
        if (!w) continue;
        const moved = Math.abs(w.top - was) > 0.5; was = w.top;
        if (!moved) continue;
        scrolled++;
        if (w.fits && (w.cutBottom > 0.5 || w.cutTop > 0.5)) cut.push(`${w.label} ${Math.max(w.cutBottom, w.cutTop).toFixed(1)}px`);
      }
      check((width > 768 || scrolled > 0) && cut.length === 0, `${width} settings: Tab through Settings, no control it scrolls to has its ring cut at the page's edge (${scrolled} scrolls)${cut.length ? ` (${cut.slice(0, 3).join('; ')})` : ''}`);
      await context.close();
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})();
