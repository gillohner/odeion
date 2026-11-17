// While songs are being selected, a control reached by the keyboard never stops under the selection
// bar. The bar floats over the bottom of the page; going down a playlist's or an album's ticks with
// Tab used to stop each next row at the page's bottom edge, under the bar (the tick and the row's
// buttons hidden, so you couldn't see what you were about to tick). Stop selecting puts the page's
// usual bottom edge back.
// At 1280, 1024 and 768 px, and on a 412 px phone (its bar sits over the page the same way).
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-select-bar-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'select-bar-focus-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// .main scrolls smoothly: read where a control stopped once the page has stood still for 3 readings
const settle = async (page) => { let was = null; let still = 0; for (let i = 0; i < 60; i++) { await sleep(40); const t = await page.evaluate(() => document.getElementById('main').scrollTop); still = t === was ? still + 1 : 0; if (still >= 3) return; was = t; } };

const A = [{ id: 11, name: 'Sefa' }];
const fmt = { container: 'flac', format: { kind: 'pcm', bits: 16, rate_hz: 44100, channels: 2 } };
const tracks = Array.from({ length: 24 }, (_, i) => ({ id: i + 1, title: `Journey part ${i + 1}`, artists: A, album: { id: 7, title: 'Journey' }, duration_ms: 200000 + i * 1000, track_no: i + 1, file: fmt }));
const summary = { id: 'c1', name: 'Long mix', items: tracks.length, covers: [], visibility: 'private', synced: false, layout: 'list' };
const fixtures = {
  '/library/albums/7': { body: { id: 7, title: 'Journey', year: 2021, artists: A, tracks } },
  '/item/album/lib-7': { body: { kind: 'album', id: 'lib-7', title: 'Journey', source_id: 'library', source: 'Your library', target: { album_id: 7 }, artist: { id: 'lib-11', name: 'Sefa' }, year: 2021,
    songs: tracks.map((t) => ({ title: t.title, artist: 'Sefa', track_id: t.id, uri: `track:${t.id}`, duration_ms: t.duration_ms })), albums: [], releases: [], links: [], also_on: [], tags: [], connect: [], open_on: [], versions: [] } },
  '/playlists': { body: [summary] },
  '/playlists/c1': { body: { summary, playlist: { ...summary, items: tracks.map((t) => ({ uri: `track:${t.id}` })) }, tracks, entries: tracks.map((t) => ({ kind: 'track', uri: `track:${t.id}` })), local_tracks: [] } },
};

// The focused control against the selection bar (ring included) and the page's bottom edge
const where = () => {
  const e = document.activeElement; const main = document.getElementById('main');
  if (!e || !main.contains(e)) return null;
  const r = e.getBoundingClientRect(); const m = main.getBoundingClientRect();
  const cs = getComputedStyle(e); const g = cs.outlineStyle === 'none' ? 0 : Math.max(0, (parseFloat(cs.outlineWidth) || 0) + (parseFloat(cs.outlineOffset) || 0));
  const bar = document.querySelector('.selbar'); const b = bar && bar.getBoundingClientRect();
  const row = e.closest('.srow'); const rowNo = row ? [...row.parentElement.children].filter((c) => c.matches('.srow')).indexOf(row) : -1;
  const what = e === row ? 'row' : (e.getAttribute('aria-label') || e.textContent || e.tagName).trim().replace(/\s+/g, ' ').slice(0, 30);
  const label = row ? `song ${rowNo + 1} ${what}` : what;
  // (only a control within the page's height counts: one taller than what the bar leaves can't fit)
  const room = b ? b.top - m.top : m.height;
  return { label, rowNo, top: main.scrollTop, h: r.height, fits: r.height + 2 * g + 16 < room,
    // (below the bar's top edge counts across the page's whole width: the bar's width follows its labels)
    under: b ? Math.max(0, Math.min(r.bottom + g, b.bottom) - Math.max(r.top - g, b.top)) : 0,
    cutBottom: Math.max(0, r.bottom + g - m.bottom) };
};
const pad = (page) => page.evaluate(() => parseFloat(getComputedStyle(document.getElementById('main')).scrollPaddingBottom) || 0);
const barHeight = (page) => page.evaluate(() => document.querySelector('.selbar').offsetHeight);

(async () => {
  const browser = await chromium.launch(process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {});
  try {
    const playlist = { name: 'playlist', url: '/playlist/c1', rows: '#plsongs .srow' }; const album = { name: 'album', url: '/album/lib-7', rows: '#main .srow' };
    const runs = [1280, 1024, 768].flatMap((width) => [{ width, p: playlist }, { width, p: album }]).concat([{ width: 412, p: playlist, mobile: true }]);
    for (const { width, p, mobile } of runs) {
      {
        const at = `${width} ${p.name}`;
        const { context, page, state } = await fx.create(browser, width, { mobile: !!mobile, fixtures, tracks, collections: [summary] });
        try {
          await fx.go(page, p.url);
          const drawn = await page.locator(p.rows).nth(tracks.length - 1).waitFor().then(() => true, () => false);
          check(drawn, `${at}: the songs are drawn`);
          if (!drawn) continue;
          await sleep(300);
          const pad0 = await pad(page);
          const sel = page.locator('[data-selstart]').first();
          await sel.click(); await page.waitForSelector('.selbar'); await sleep(150);
          const [padOn, bar] = [await pad(page), await barHeight(page)];
          check(padOn >= bar + 8, `${at}: while selecting, the page's bottom edge for scrolling is above the bar (${pad0} → ${padOn} px, bar ${bar} px)`);
          // Down the ticks with Tab from Select: no stop is under the bar
          await sel.focus(); await settle(page);
          const under = []; const rows = new Set(); let scrolled = 0; let shot = false; let was = await page.evaluate(() => document.getElementById('main').scrollTop);
          for (let i = 0; i < tracks.length * 12; i++) {
            await page.keyboard.press('Tab'); await settle(page);
            const w = await page.evaluate(where);
            if (!w || w.rowNo < 0) { if (rows.has(tracks.length - 1)) break; if (!w) continue; }
            if (Math.abs(w.top - was) > 0.5) scrolled++; was = w.top;
            if (w.rowNo >= 0) rows.add(w.rowNo);
            if (w.fits && (w.under > 0.5 || w.cutBottom > 0.5)) {
              under.push(`${w.label} ${w.under > 0.5 ? `${w.under.toFixed(0)}/${w.h.toFixed(0)}px under the bar` : `${w.cutBottom.toFixed(1)}px cut at the bottom`}`);
              if (!shot) { shot = true; await page.screenshot({ path: path.join(SHOTS, `${p.name}-under-${width}.png`) }); }
            }
          }
          check(rows.size === tracks.length && scrolled > 5, `${at}: Tab goes down every song while selecting (${rows.size} of ${tracks.length} songs, ${scrolled} scrolls)`);
          check(under.length === 0, `${at}: Tab down the songs while selecting, nothing stops under the selection bar${under.length ? ` (${under.length}: ${under.slice(0, 3).join('; ')})` : ''}`);
          await page.screenshot({ path: path.join(SHOTS, `${p.name}-down-${width}.png`) });
          // Stop selecting: the page's usual bottom edge is back
          await page.click('.selbar [data-sel="cancel"]'); await sleep(100);
          const pad1 = await pad(page);
          check(!(await page.$('.selbar')) && pad1 === pad0, `${at}: Stop selecting puts the page's bottom edge back (${padOn} → ${pad1} px)`);
          check(state.errors.length === 0, `${at}: no page errors${state.errors.length ? ` (${state.errors[0]})` : ''}`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
