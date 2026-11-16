// Node library's top line ("12,840 songs in the library · last scan 3 h ago · 37 files could not be
// read · Review 64 unsure") at phone widths (and in the Android app) and desktop. When it wraps, no
// row starts with "·" (it did: "· Review 64 unsure" began the second or third row), a part moves
// down whole, the first part of each row lines up with the page's text, the Review button's focus
// ring has room, and big counts read with separators (12,840, not 12840). A count drawn again by
// the Identify card (none left) leaves no stray dot.
// Real candidate assets, isolated responses: node tools/ui-libstatus-wrap.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'libstatus-wrap-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const now = Math.floor(Date.now() / 1000);
const info = (scan) => ({ body: { name: 'Fixture node', library: { tracks: 12840, albums: 900, artists: 300, files: 12900, missing_files: 0 }, scanning: false, ...scan } });
const identify = (suggested) => ({ body: { phase: suggested ? 'needs_review' : 'ready', progress: { running: false, total: 30, done: 30, identified: 20, suggested, unknown: 4, errors: 0, finished_at: 1 }, counts: { total: 12840, identified: 12000, suggested, unknown: 776, unchecked: 0, by: { tags: 10, album: 60, fingerprint: 8, you: 2, title_artist: 10 } }, acoustid: true, acoustid_own: true, acoustid_state: 'ok', write_ids: false } });
const CASES = {
  errors: { '/info': info({ last_scan: { finished_at: now - 3 * 3600, error_count: 37, errors: [['/music/A/b.flac', 'bad header']] } }), '/library/identify': identify(64) },
  scan: { '/info': info({ last_scan: { finished_at: now - 3 * 3600, error_count: 0, errors: [] } }), '/library/identify': identify(64) },
  scanning: { '/info': info({ scanning: true }), '/library/identify': identify(64) },
};
const common = { '/library/webdav': { body: [] }, '/library/drives': { body: { roots: [], providers: [] } }, '/library/hidden': { body: [] }, '/library/identify/settings': { body: {} } };

// The line's rows as drawn: the visible characters (not clipped by the line) grouped by row, and
// each row's first visible character's left edge
const rows = (page) => page.evaluate(() => {
  const line = document.querySelector('.libstatus');
  const cs = getComputedStyle(line); const lr = line.getBoundingClientRect();
  const clipLeft = cs.overflowX === 'clip' || cs.overflowX === 'hidden' ? lr.left + parseFloat(cs.borderLeftWidth) : -Infinity;
  const out = {};
  const add = (c, r) => {
    if (!r.width || r.left + r.width / 2 < clipLeft) return; // nothing drawn, or clipped off
    const top = Math.round(r.top + r.height / 2);
    const key = Object.keys(out).find((t) => Math.abs(t - top) < 8) ?? top;
    (out[key] ||= []).push({ c, left: r.left, right: r.right });
  };
  const walk = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
  for (let n = walk.nextNode(); n; n = walk.nextNode()) {
    for (let i = 0; i < n.data.length; i++) {
      if (!n.data[i].trim()) continue;
      const rg = document.createRange(); rg.setStart(n, i); rg.setEnd(n, i + 1);
      add(n.data[i], rg.getBoundingClientRect());
    }
  }
  line.querySelectorAll('.spin').forEach((e) => add('◌', e.getBoundingClientRect())); // the scan's spinner starts its row
  const text = document.querySelector('.libstatus + .muted.small'); const textLeft = text ? text.getBoundingClientRect().left : lr.left;
  const parts = [...line.querySelectorAll('.lspart')].map((p) => { const t = p.lastElementChild.getBoundingClientRect(); return { h: t.height, lh: parseFloat(getComputedStyle(p).lineHeight) || 18, text: p.lastElementChild.textContent }; });
  const rv = document.querySelector('#libreview'); const rvr = rv && rv.getBoundingClientRect();
  const clipRight = clipLeft === -Infinity ? Infinity : lr.right - parseFloat(cs.borderRightWidth);
  return { clipLeft, clipRight, textLeft, parts, review: rvr ? { left: rvr.left, right: rvr.right } : null, rows: Object.keys(out).sort((a, b) => a - b).map((k) => { const cs2 = out[k].sort((a, b) => a.left - b.left); return { text: cs2.map((x, i) => (i && x.left - cs2[i - 1].right > 2 ? ' ' : '') + x.c).join(''), first: cs2[0].c, left: cs2[0].left }; }) };
});

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, true], [412, false], [1280, false]]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      for (const [name, fixtures0] of Object.entries(CASES)) {
        const fixtures = { ...common, ...fixtures0 };
        const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures });
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
        try {
          await fx.go(page, '/library/node');
          const drawn = await page.waitForFunction(() => document.querySelector('.libstatus #libreview'), null, { timeout: 6000 }).then(() => true, () => false);
          check(drawn, `${at} ${name}: the top line offers Review 64 unsure`);
          if (!drawn) continue;
          const g = await rows(page);
          const all = g.rows.map((r) => r.text).join(' / ');
          check(g.rows.every((r) => r.first !== '·'), `${at} ${name}: no row starts with "·" (${all})`);
          check(g.rows.every((r) => Math.abs(r.left - g.textLeft) < 4), `${at} ${name}: each row starts where the page's text does (${g.rows.map((r) => Math.round(r.left)).join(', ')} vs ${Math.round(g.textLeft)})`);
          check(/12,840 songs/.test(all) && !/12840/.test(all), `${at} ${name}: the song count reads 12,840`);
          if (name === 'errors') check(/37 files could not be read/.test(all) && /last scan 3 h ago/.test(all), `${at} ${name}: every part is there`);
          if (width < 600) check(g.parts.length > 0 && g.parts.every((p) => p.h < p.lh * 1.6), `${at} ${name}: a part moves down whole, not split over two rows (${g.parts.map((p) => Math.round(p.h)).join(', ')} px)`);
          if (width >= 1280) check(g.rows.length === 1, `${at} ${name}: one row on a desktop (${g.rows.length})`);
          // The Review button's focus ring (2px, 2px off) has room, wherever the button lands
          // (and its tap reach on a phone, 6px out: #libreview's ::before on a phone)
          check(g.review && g.review.left - 6 >= g.clipLeft - 0.5 && g.review.right + 6 <= g.clipRight + 0.5, `${at} ${name}: Review's focus ring and tap reach aren't cut (${g.review && Math.round(g.review.left - g.clipLeft)} px left, ${g.review && Math.round(g.clipRight - g.review.right)} px right)`);
          if (name === 'scan' && width === 360) {
            await page.locator('#libreview').focus();
            await page.keyboard.press('Shift+Tab'); await page.keyboard.press('Tab');
            await page.screenshot({ path: path.join(SHOTS, `${at}-${name}-focus.png`), clip: { x: 0, y: 260, width, height: 140 } });
          }
          await page.screenshot({ path: path.join(SHOTS, `${at}-${name}.png`) });
          await fx.cleanCopy(page).then(() => check(true, `${at} ${name}: clean copy, no horizontal overflow`), (e) => check(false, `${at} ${name}: ${e.message}`));
          if (name === 'errors') {
            // The Identify card drawn again with none unsure: the review goes and leaves no dot behind
            fixtures['/library/identify'] = identify(0);
            await page.evaluate(() => { document.querySelector('#idphase').dataset.old = '1'; const a = document.querySelector('#idauto'); a.checked = !a.checked; a.dispatchEvent(new Event('change')); });
            await page.waitForFunction(() => !document.querySelector('#libreview') && document.querySelector('#idphase') && !document.querySelector('#idphase').dataset.old, null, { timeout: 6000 }).catch(() => {});
            const g0 = await rows(page);
            const t0 = g0.rows.map((r) => r.text).join(' / ');
            check(!/unsure/.test(t0) && !/·\s*$/.test(t0) && g0.rows.every((r) => r.first !== '·'), `${at} ${name}: with none unsure left the line drops the review and no dot stays (${t0})`);
            fixtures['/library/identify'] = identify(5);
            await page.evaluate(() => { document.querySelector('#idphase').dataset.old = '1'; const a = document.querySelector('#idauto'); a.checked = !a.checked; a.dispatchEvent(new Event('change')); });
            await page.waitForFunction(() => document.querySelector('#libreview'), null, { timeout: 6000 }).catch(() => {});
            const g5 = await rows(page);
            const t5 = g5.rows.map((r) => r.text).join(' / ');
            check(/(^|· |\/ )Review 5 unsure$/.test(t5) && g5.rows.every((r) => r.first !== '·'), `${at} ${name}: …and back with 5 it reads as before (${t5})`);
          }
          check(state.errors.length === 0, `${at} ${name}: no page errors (${state.errors.join('; ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
