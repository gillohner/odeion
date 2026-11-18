// A control reached by the keyboard on a phone Library page is never under the Library chips. On a
// phone the chips (Your library, Albums, Songs, ...) stay pinned at the top of the page while it
// scrolls; going back up Songs with Shift+Tab stopped each row at the very top, under the chips (a
// song's More button and its play button wholly or half hidden, about 2 stops in 5). Now a control
// the page scrolls to stops 8 px below them. Open Identify still lands with the IDENTIFY heading
// right below the chips (it used to make that room itself; the page does now, so it isn't doubled).
// Larger text makes the chips taller; the room kept above a row grows with them, also when only
// the chips change size (the phone's text size changed while the page is open: nothing is added
// to the page). At 360 and 412 px; Open Identify at 1280 px too (the heading 8 px from the top).
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-phone-libnav-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'phone-libnav-focus-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// .main scrolls smoothly: read where a control stopped once the page stands still
// (three readings alike, so a slow frame mid-scroll doesn't count as stopped)
const settle = async (page) => { let was = null; let same = 0; for (let i = 0; i < 60; i++) { await sleep(40); const t = await page.evaluate(() => document.getElementById('main').scrollTop); same = t === was ? same + 1 : 0; if (same >= 2) return; was = t; } };

const A = [{ id: 11, name: 'Sefa' }];
const fmt = { container: 'flac', format: { kind: 'pcm', bits: 16, rate_hz: 44100, channels: 2 } };
const tracks = Array.from({ length: 40 }, (_, i) => ({ id: i + 1, title: `Journey part ${i + 1}`, artists: A, album: { id: 7, title: 'Journey' }, duration_ms: 200000 + i * 1000, track_no: i + 1, file: fmt }));
const collections = Array.from({ length: 30 }, (_, i) => ({ id: `p${i + 1}`, name: `Mix number ${i + 1}`, items: 3, covers: [], visibility: 'private', synced: false, layout: 'list' }));
const stats = { tracks: 4, albums: 0, artists: 0, files: 4, missing_files: 0, total_duration_ms: 0 };
const idSt = { phase: 'new', progress: { running: false, total: 0, done: 0, identified: 0, suggested: 0, unknown: 0, errors: 0 }, counts: { total: 4, identified: 0, suggested: 0, suggested_missing: 0, unknown: 0, unchecked: 4, skipped: 0, outdated: 0, by: { tags: 0, album: 0, fingerprint: 0, you: 0, title_artist: 0, auto: 0 } }, acoustid: false, acoustid_own: false, acoustid_state: 'missing', write_ids: false, auto_match: true };
const nodeLib = {
  '/library/suggestions/albums': { body: { albums: [], songs: 0, total_albums: 0, offset: 0, next: null } },
  '/library/identify': { body: idSt },
  '/library/folders': { body: [['/music', true]] },
  '/library/webdav': { body: [] },
  '/library/drives': { body: { providers: [], roots: [] } },
  '/library/hidden': { body: [] },
  '/library/stats': { body: stats },
};

// Where the focused control is against the pinned chips, its ring included
const where = () => {
  const e = document.activeElement; const main = document.getElementById('main');
  if (!e || !main.contains(e) || e.closest('.libnav')) return null;
  const nav = main.querySelector(':scope > .libnav'); if (!nav) return { noNav: true };
  const r = e.getBoundingClientRect(); const b = nav.getBoundingClientRect(); const m = main.getBoundingClientRect();
  const cs = getComputedStyle(e); const g = cs.outlineStyle === 'none' ? 0 : Math.max(0, (parseFloat(cs.outlineWidth) || 0) + (parseFloat(cs.outlineOffset) || 0));
  const label = (e.getAttribute('aria-label') || e.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 40);
  return { label, top: main.scrollTop, pinned: getComputedStyle(nav).position === 'sticky' && b.top <= m.top + 0.5, fits: r.height + 2 * g < m.height - b.height,
    under: Math.max(0, Math.min(r.bottom + g, b.bottom) - Math.max(r.top - g, b.top)), gap: Math.round(r.top - b.bottom) };
};

(async () => {
  const browser = await chromium.launch(process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {});
  try {
    for (const width of [360, 412]) {
      for (const p of [{ name: 'Songs', url: '/tracks', rows: '#main .track', n: tracks.length }, { name: 'Playlists', url: '/playlists', rows: '#main .list-row', n: collections.length }]) {
        const at = `${width} ${p.name}`;
        const { context, page, state } = await fx.create(browser, width, { tracks, collections });
        try {
          await fx.go(page, p.url);
          const drawn = await page.locator(p.rows).nth(p.n - 1).waitFor().then(() => true, () => false);
          check(drawn, `${at}: the list is drawn`);
          if (!drawn) continue;
          await sleep(300);
          check(await page.evaluate(() => document.documentElement.dataset.shell === 'phone' && !!document.querySelector('.main > .libnav')), `${at}: the phone shell, with the Library chips`);
          // From the page's last control back up with Shift+Tab: each stop the page scrolls to is
          // clear of the pinned chips
          await page.evaluate(() => { const m = document.getElementById('main'); m.style.scrollBehavior = 'auto'; m.scrollTop = m.scrollHeight; m.style.scrollBehavior = ''; });
          await page.locator('#main button:visible, #main a[href]:visible').last().focus(); await settle(page);
          const under = []; let scrolled = 0; let pinnedSeen = false; let shot = false;
          let was = await page.evaluate(() => document.getElementById('main').scrollTop);
          for (let i = 0; i < p.n * 3 + 6; i++) {
            await page.keyboard.press('Shift+Tab'); await settle(page);
            const w = await page.evaluate(where);
            if (!w || w.noNav) continue;
            const moved = Math.abs(w.top - was) > 0.5; was = w.top;
            if (w.pinned) pinnedSeen = true;
            if (!moved || !w.pinned || !w.fits) continue;
            scrolled++;
            if (w.under > 0.5) { under.push(`${w.label} ${w.under.toFixed(1)}px`); if (!shot) { shot = true; await page.screenshot({ path: path.join(SHOTS, `under-${width}-${p.name}.png`) }); } }
          }
          check(pinnedSeen && scrolled > 5, `${at}: going up with Shift+Tab scrolls the page under the pinned chips (${scrolled} scrolls)`);
          check(under.length === 0, `${at}: no control Shift+Tab scrolls to is under the Library chips${under.length ? ` (${under.length}: ${under.slice(0, 3).join('; ')})` : ''}`);
          // A row scrolled into view from code (as a kept focus is) sits 8 px below the chips
          const toTop = (sel, k) => page.evaluate(([s, k]) => { const rows = document.querySelectorAll(s); const r = rows[Math.floor(rows.length * k)]; const m = document.getElementById('main'); m.style.scrollBehavior = 'auto'; r.scrollIntoView({ block: 'start' }); m.style.scrollBehavior = '';
            const nav = m.querySelector(':scope > .libnav').getBoundingClientRect(); return { gap: Math.round(r.getBoundingClientRect().top - nav.bottom), h: Math.round(nav.height) }; }, [sel, k]);
          const row = await toTop(p.rows, 0.5);
          check(row.gap >= 6 && row.gap <= 10, `${at}: a row scrolled to the top sits just below the chips (${row.gap} px)`);
          await page.screenshot({ path: path.join(SHOTS, `after-${width}-${p.name}.png`) });
          // Larger text makes the chips taller: the room above the row grows with them
          await page.evaluate(() => { const s = document.createElement('style'); s.textContent = '.libnav .chip { font-size: 30px; }'; document.body.append(s); });
          await sleep(200);
          const big = await toTop(p.rows, 0.25);
          check(big.h >= 70 && big.gap >= 6 && big.gap <= 10, `${at}: with larger text (chips ${big.h} px tall) a row scrolled to the top still sits just below them (${big.gap} px)`);
          // The chips grow again with nothing added to the page (no DOM insert, no window resize)
          await page.evaluate(() => document.querySelectorAll('.main > .libnav .chip').forEach((c) => { c.style.fontSize = '44px'; }));
          await sleep(200);
          const bigger = await toTop(p.rows, 0.6);
          check(bigger.h > big.h + 10 && bigger.gap >= 6 && bigger.gap <= 10, `${at}: the chips growing alone (${big.h} to ${bigger.h} px) keep a row scrolled to the top just below them (${bigger.gap} px)`);
          await page.screenshot({ path: path.join(SHOTS, `chips-grown-${width}-${p.name}.png`) });
          check(!state.errors.length, `${at}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Open Identify: the IDENTIFY heading lands right below the chips, not a chips' height lower
      for (const desk of width === 412 ? [false, true] : [false]) {
        // (1280 px: the chips scroll with the page and the heading stops 8 px below the top; a
        // shorter window, so the page can scroll that far)
        const w = desk ? 1280 : width;
        const at = `${w} Open Identify`;
        const { context, page, state } = await fx.create(browser, w, { fixtures: nodeLib });
        try {
          if (desk) await page.setViewportSize({ width: w, height: 560 });
          await fx.go(page, '/identify');
          const shown = await page.waitForSelector('.main .empty button', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${at}: Nothing to confirm shows`);
          if (!shown) continue;
          await page.locator('.main .empty button').click();
          const landed = await page.waitForFunction(() => location.pathname === '/library/node' && document.querySelector('#idcard'), null, { timeout: 8000 }).then(() => true, () => false);
          check(landed, `${at}: Open Identify opens Node library`);
          if (!landed) continue;
          await sleep(400);
          const m = await page.evaluate((desk) => { const main = document.getElementById('main'); const nav = main.querySelector(':scope > .libnav'); const h = document.querySelector('#idcard').closest('.set-group').querySelector('.set-h').getBoundingClientRect();
            return { gap: Math.round(h.top - (desk ? main.getBoundingClientRect().top : nav.getBoundingClientRect().bottom)), scrolled: main.scrollTop > 0, shell: document.documentElement.dataset.shell }; }, desk);
          if (desk) check(m.shell === 'desktop' && m.scrolled && m.gap >= 6 && m.gap <= 10, `${at}: the IDENTIFY heading sits 8 px below the top of the page (${m.gap} px, ${m.shell} shell)`);
          else check(m.scrolled && m.gap >= 4 && m.gap <= 12, `${at}: the IDENTIFY heading sits just below the chips (${m.gap} px)`);
          await page.screenshot({ path: path.join(SHOTS, `identify-${w}.png`) });
          check(!state.errors.length, `${at}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
