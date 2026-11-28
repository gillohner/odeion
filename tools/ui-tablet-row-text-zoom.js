// A song row wider than a phone (an Android tablet, or the app on a wide window) grows its number,
// format and length columns with Android's font size: the app's WebView applies it as text zoom,
// which scaled the words but not the fixed 40 px number, 92 px format and 64 px length columns, so at
// 150-200 % a format ("16/44.1k", "24/176.4k") ran over the length, and a length ("12:34", "1:02:34") or a
// four-digit number ran out of its column, the length under the row's buttons. The columns now follow
// the text (lh does, em and px don't); at 100 % they are 40, 92, 64 and 112 px as before.
// Songs (the list's own numbers) and a service's album (compact rows with track numbers), 768-1280 px.
// Real candidate assets, isolated responses: node tools/ui-tablet-row-text-zoom.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'tablet-row-text-zoom-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// 1200 songs (four-digit numbers); every third lasts 12:34, every fifth over an hour, and every
// fourth is a hi-res file with the longest format ("24/176.4k")
const len = (i) => (i % 5 === 4 ? 3754000 : i % 3 === 2 ? 754000 : 240000);
const hires = { container: 'flac', format: { kind: 'pcm', bits: 24, rate_hz: 176400, channels: 2 } };
const many = (n) => Array.from({ length: n }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, duration_ms: len(i), ...(i % 4 === 1 ? { file: hires } : {}) }));
// A SoundCloud set of 14 songs (a service's album page: compact rows with their track numbers)
const set = { title: 'Journey', creator: 'Sefa', year: 2020, url: 'https://soundcloud.com/sefa/sets/journey', image: null,
  tracks: Array.from({ length: 14 }, (_, i) => ({ url: `https://soundcloud.com/sefa/song-${i + 1}`, title: `Song ${i + 1}`, track_no: i + 1, duration_ms: len(i), format: 'mp3' })) };
const sources = {
  '/sources': { body: [{ id: 'soundcloud', name: 'SoundCloud', enabled: true, status: 'signed in' }] },
  '/sources/soundcloud/likes': { body: { tracks: [], playlists: [{ id: 'abc', title: 'Journey', creator: 'Sefa' }] } },
  '/sources/soundcloud/playlists': { body: [] },
  '/sources/soundcloud/items/abc': { body: set },
};
// Android's font size as the app's WebView applies it (text zoom; see tools/ui-phone-library-head.js)
const ZOOM_ARGS = ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'];
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });

// The chosen rows of the list: each piece of text inside its own column, and the columns' widths
const rows = (page, list, picks) => page.evaluate(({ list, picks }) => {
  const all = [...document.querySelectorAll(`${list} .track`)];
  const out = [];
  const shown = (el) => el && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden' && getComputedStyle(el).display !== 'none';
  const text = (el) => { const rg = document.createRange(); rg.selectNodeContents(el); return rg.getBoundingClientRect(); };
  for (const i of picks) {
    const row = all[i]; if (!row) continue;
    row.scrollIntoView({ block: 'center', behavior: 'instant' });
    const bad = [];
    const q = (s) => row.querySelector(s);
    const no = q('.no'), num = q('.num'), fcol = q('.fmt-col'), chip = q('.fmt-col .fmt'), dur = q('.dur'), acts = q('.row-actions');
    const inside = (a, b, what) => { const x = a.getBoundingClientRect(), y = b.getBoundingClientRect(); if (x.left < y.left - 0.5 || x.right > y.right + 0.5) bad.push(`${what} ${x.left.toFixed(0)}..${x.right.toFixed(0)} outside ${y.left.toFixed(0)}..${y.right.toFixed(0)}`); };
    if (shown(num) && num.textContent.trim()) inside(num, no, `number "${num.textContent.trim()}"`);
    if (shown(chip) && chip.textContent.trim()) inside(chip, fcol, `format "${chip.textContent.trim()}"`);
    if (shown(dur)) {
      const t = text(dur), d = dur.getBoundingClientRect();
      if (t.left < d.left - 0.5 || t.right > d.right + 0.5) bad.push(`length "${dur.textContent.trim()}" ${t.left.toFixed(0)}..${t.right.toFixed(0)} outside ${d.left.toFixed(0)}..${d.right.toFixed(0)}`);
      if (shown(acts) && t.right > acts.getBoundingClientRect().left + 0.5) bad.push(`length "${dur.textContent.trim()}" ends at ${t.right.toFixed(0)}, the buttons start at ${acts.getBoundingClientRect().left.toFixed(0)}`);
    }
    const w = (el) => (shown(el) ? Number(el.getBoundingClientRect().width.toFixed(1)) : null);
    out.push({ i, bad, no: w(no), fmt: w(fcol), dur: w(dur), acts: w(acts) });
  }
  window.scrollTo(0, 0); document.querySelector('#main')?.scrollTo?.(0, 0);
  const m = document.querySelector('#main');
  return { rows: out, fits: document.documentElement.scrollWidth <= innerWidth && (!m || m.scrollWidth <= m.clientWidth) };
}, { list, picks });

async function tablet(browser, width, k) {
  const at = `android-${width}${k !== 1 ? ` at ${Math.round(k * 100)} % text zoom` : ''}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true, tracks: many(1200), fixtures: sources });
  try {
    await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    const pages = [['Songs', '/tracks', '#tracklist', [0, 1, 2, 3, 4, 98, 99, 100, 998, 999, 1000, 1001, 1002, 1003, 1004]], ['Service album', '/sources/soundcloud', '#main .src-tracks', [...Array(14).keys()]]];
    for (const [name, url, list, picks] of pages) {
      await fx.go(page, url);
      if (name === 'Service album') await page.click('[data-sitem="abc"]');
      await page.waitForSelector(`${list} .track`);
      if (k !== 1) await textZoom(page, k);
      await page.waitForTimeout(300);
      const shell = await page.evaluate(() => document.documentElement.dataset.shell);
      // (the zoom took effect: the rows grow with the text, 54 px at 100 %; a zoom that did nothing
      // would otherwise pass as a 100 % run)
      const rowH = await page.evaluate((list) => document.querySelector(`${list} .track`).getBoundingClientRect().height, list);
      check(k === 1 ? Math.abs(rowH - 54) < 0.5 : rowH > 56, `${at} ${name}: the rows are ${k === 1 ? '54 px' : 'taller than at 100 %'} (${rowH.toFixed(1)} px)`);
      const got = await rows(page, list, picks);
      const bad = got.rows.flatMap((r) => r.bad.map((b) => `row ${r.i + 1}: ${b}`));
      check(got.rows.length === picks.length && !bad.length, `${at} ${name} (${shell}): number, format and length stay in their columns, the length before the buttons (${got.rows.length} rows${bad.length ? `; ${bad.slice(0, 3).join('; ')}` : ''})`);
      if (k === 1) {
        const one = (key) => [...new Set(got.rows.map((r) => r[key]))].join(', ');
        check(one('no') === '40' && one('fmt') === '92' && one('dur') === '64' && one('acts') === '112', `${at} ${name}: the columns are 40, 92, 64 and 112 px, as before (${one('no')}; ${one('fmt')}; ${one('dur')}; ${one('acts')})`);
      }
      check(got.fits, `${at} ${name}: no sideways overflow`);
      await page.evaluate((list) => document.querySelector(`${list} .track`).scrollIntoView({ block: 'start', behavior: 'instant' }), list);
      await page.screenshot({ path: path.join(SHOTS, `${name.toLowerCase()}-${at}.png`.replace(/ /g, '-')) });
    }
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  const zoomed = await chromium.launch({ args: ZOOM_ARGS });
  try {
    for (const w of [768, 1024, 1280]) await tablet(browser, w, 1);
    for (const [w, k] of [[768, 1.3], [768, 2], [800, 1.5], [1024, 1.5], [1024, 2], [1280, 2]]) await tablet(zoomed, w, k);
  } finally { await browser.close(); await zoomed.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
