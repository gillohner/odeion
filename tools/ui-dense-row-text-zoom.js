// Dense lists (Settings › Appearance › Dense lists) on a screen wider than a phone, as on an Android
// tablet with a larger font: the app's WebView applies Android's font size as text zoom, which scaled
// a song row's words but not its fixed 42 px dense height, so at 150-200 % the artist line was cut off
// by the next row. A dense row now grows with the text as a normal one does (lh follows text zoom, em
// and px don't); at 100 % it is 42 px, as before.
// Songs, Bookmarks and a service's album (its compact rows with track numbers), 768-1280 px.
// Real candidate assets, isolated responses: node tools/ui-dense-row-text-zoom.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'dense-row-text-zoom-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const many = (n) => Array.from({ length: n }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, duration_ms: i === 11 ? 754000 : 240000 }));
// A SoundCloud set of 14 songs (a service's album page: compact rows with their track numbers)
const set = { title: 'Journey', creator: 'Sefa', year: 2020, url: 'https://soundcloud.com/sefa/sets/journey', image: null,
  tracks: Array.from({ length: 14 }, (_, i) => ({ url: `https://soundcloud.com/sefa/song-${i + 1}`, title: `Song ${i + 1}`, track_no: i + 1, duration_ms: 200000, format: 'mp3' })) };
const sources = {
  '/sources': { body: [{ id: 'soundcloud', name: 'SoundCloud', enabled: true, status: 'signed in' }] },
  '/sources/soundcloud/likes': { body: { tracks: [], playlists: [{ id: 'abc', title: 'Journey', creator: 'Sefa' }] } },
  '/sources/soundcloud/playlists': { body: [] },
  '/sources/soundcloud/items/abc': { body: set },
};
const marked = (ts) => ({ uris: ts.map((t) => `${fx.SONG}?${t.id}`), tracks: ts, albums: [], artists: [], other: [] });
// Android's font size as the app's WebView applies it (text zoom; see tools/ui-phone-library-head.js)
const ZOOM_ARGS = ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'];
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });

// The first rows of the list: each row's height, and every piece of its text (and its ⋯) that is
// missing or sticks out of the row's top or bottom
const rows = (page, list) => page.evaluate((list) => {
  const out = [];
  const shown = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden' && getComputedStyle(el).display !== 'none';
  for (const row of [...document.querySelectorAll(`${list} .track`)].slice(0, 14)) {
    row.scrollIntoView({ block: 'center', behavior: 'instant' });
    const r = row.getBoundingClientRect();
    const bad = [];
    // (a library row's title is a link with its Match beside it, a service's song plain words)
    const title = row.querySelector('.title .lnk') || row.querySelector('.title');
    if (!title || !shown(title) || !title.textContent.trim()) bad.push('no title shown');
    for (const sel of ['.by', '[data-act="more"]']) {
      if (![...row.querySelectorAll(sel)].some((el) => shown(el) && (sel.startsWith('[') ? el.querySelector('svg') : el.textContent.trim()))) bad.push(`no ${sel} shown`);
    }
    // the words themselves (the title's link, not its Match: the boxes have padding and negative
    // margins for their touch room, so they reach past the row and each other at any size)
    const words = (el) => { const rg = document.createRange(); rg.selectNodeContents(el); return rg.getBoundingClientRect(); };
    const parts = [title, ...row.querySelectorAll('.by, .num, .dur')].filter((el) => el && shown(el)).map((el) => [el, words(el)]);
    for (const el of row.querySelectorAll('[data-act="more"]')) if (shown(el)) parts.push([el, el.getBoundingClientRect()]);
    for (const [el, b] of parts) {
      if (b.top < r.top - 0.5 || b.bottom > r.bottom + 0.5) bad.push(`${el.className || el.dataset.act} "${el.textContent.trim().slice(0, 20)}" ${b.top.toFixed(0)}..${b.bottom.toFixed(0)} outside the row ${r.top.toFixed(0)}..${r.bottom.toFixed(0)}`);
    }
    // the artist line's words start under the title's
    const t = title, by = row.querySelector('.by');
    if (t && by && shown(t) && shown(by) && words(by).top < words(t).bottom - 0.5) bad.push(`the artist line (${words(by).top.toFixed(0)}) overlaps the title (to ${words(t).bottom.toFixed(0)})`);
    out.push({ h: r.height, bad });
  }
  window.scrollTo(0, 0); document.querySelector('#main')?.scrollTo?.(0, 0);
  const main = document.querySelector('#main');
  return { rows: out, dense: document.documentElement.hasAttribute('data-dense'), shell: document.documentElement.dataset.shell, fits: document.documentElement.scrollWidth <= innerWidth && (!main || main.scrollWidth <= main.clientWidth) };
}, list);

async function tablet(browser, width, k) {
  const at = `android-${width} dense${k !== 1 ? ` at ${Math.round(k * 100)} % text zoom` : ''}`;
  const ts = many(30);
  const { page, context, state } = await fx.create(browser, width, { mobile: true, tracks: ts, fixtures: { ...sources, '/bookmarks': { body: marked(ts.slice(0, 14)) }, '/favourites': { body: marked(ts.slice(0, 14)) } } });
  try {
    await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; localStorage.setItem('pm.dense', '1'); });
    const pages = [['Songs', '/tracks', '#tracklist'], ['Bookmarks', '/favourites', '[data-list="fav"]'], ['Service album', '/sources/soundcloud', '#main .src-tracks']];
    for (const [name, url, list] of pages) {
      await fx.go(page, url);
      if (name === 'Service album') await page.click('[data-sitem="abc"]');
      await page.waitForSelector(`${list} .track`);
      if (k !== 1) await textZoom(page, k);
      await page.waitForTimeout(300);
      await page.mouse.move(1, 1);
      const got = await rows(page, list);
      check(got.dense && got.shell !== 'phone', `${at} ${name}: the dense list is on, in the ${got.shell} layout`);
      const bad = got.rows.flatMap((r, i) => r.bad.map((b) => `row ${i + 1}: ${b}`));
      check(got.rows.length >= 10 && !bad.length, `${at} ${name}: every row holds its title, artist line and ⋯ (${got.rows.length} rows${bad.length ? `; ${bad.slice(0, 3).join('; ')}` : ''})`);
      const hs = [...new Set(got.rows.map((r) => r.h.toFixed(1)))];
      if (k === 1) check(hs.length === 1 && hs[0] === '42.0', `${at} ${name}: a row is 42 px, as before (${hs.join(', ')})`);
      else {
        // (and stay shorter than a normal row at the same text size)
        const normal = await page.evaluate((list) => { const de = document.documentElement; de.removeAttribute('data-dense'); const h = document.querySelector(`${list} .track`).getBoundingClientRect().height; de.dataset.dense = '1'; return h; }, list);
        check(hs.length === 1 && Number(hs[0]) > 42 && Number(hs[0]) < normal, `${at} ${name}: the rows grow with the text, all the same height, and stay shorter than a normal row (${hs.join(', ')} px; normal ${normal.toFixed(1)})`);
      }
      check(got.fits, `${at} ${name}: no sideways overflow`);
      await page.evaluate((list) => document.querySelector(`${list} .track`).scrollIntoView({ block: 'start', behavior: 'instant' }), list);
      await page.screenshot({ path: path.join(SHOTS, `${name.toLowerCase()}-${at}.png`.replace(/ /g, '-')) });
    }
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

// A desktop browser's dense list (no text zoom): 42 px rows, as before; a normal list 54 px
async function desktop(browser, width, dense) {
  const at = `desktop-${width}${dense ? ' dense' : ''}`;
  const { page, context, state } = await fx.create(browser, width, { tracks: many(30) });
  try {
    if (dense) await context.addInitScript(() => { localStorage.setItem('pm.dense', '1'); });
    await fx.go(page, '/tracks'); await page.waitForSelector('#tracklist .track');
    await page.waitForTimeout(200);
    const got = await rows(page, '#tracklist');
    const hs = [...new Set(got.rows.map((r) => r.h.toFixed(1)))];
    check(got.dense === dense && hs.length === 1 && hs[0] === (dense ? '42.0' : '54.0'), `${at} Songs: a row is ${dense ? 42 : 54} px, as before (${hs.join(', ')})`);
    { const bad = got.rows.flatMap((r, i) => r.bad.map((b) => `row ${i + 1}: ${b}`)); check(got.rows.length >= 10 && !bad.length, `${at} Songs: every row holds its title, artist line and ⋯ (${got.rows.length} rows${bad.length ? `; ${bad.slice(0, 3).join("; ")}` : ""})`); }
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  const zoomed = await chromium.launch({ args: ZOOM_ARGS });
  try {
    for (const w of [768, 1024]) await tablet(browser, w, 1);
    for (const [w, k] of [[768, 1.3], [768, 2], [1024, 1.5], [1280, 2]]) await tablet(zoomed, w, k);
    for (const w of [1024, 1280]) { await desktop(browser, w, true); await desktop(browser, w, false); }
  } finally { await browser.close(); await zoomed.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
