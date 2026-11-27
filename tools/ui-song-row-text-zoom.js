// A song row (Songs, Bookmarks, a service's album with its track numbers) grows with Android's font
// size: the app's WebView applies it as text zoom, which scaled the row's words but not its fixed 54 px
// height or its 26 px number and 38 px length columns, so at 150-200 % the artist line was cut off by
// the next row and the length ran under the ⋯. The row's height and those columns now follow the text
// (lh does, em and px don't); at 100 % the row is 54 px and the columns 26 and 38 px as before, on a
// phone, and the row 54 px (42 px dense) on a desktop.
// Real candidate assets, isolated responses: node tools/ui-song-row-text-zoom.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'song-row-text-zoom-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Songs 1-30; song 12 lasts over ten minutes (a five-character length)
const many = (n) => Array.from({ length: n }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, duration_ms: i === 11 ? 754000 : 240000 }));
// A SoundCloud set of 14 songs (a service's album page: compact rows with their track numbers)
const set = { title: 'Journey', creator: 'Sefa', year: 2020, url: 'https://soundcloud.com/sefa/sets/journey', image: null,
  tracks: Array.from({ length: 14 }, (_, i) => ({ url: `https://soundcloud.com/sefa/song-${i + 1}`, title: `Song ${i + 1}`, track_no: i + 1, duration_ms: i === 11 ? 754000 : 200000, format: 'mp3' })) };
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

// Each drawn row in the list: its box, and every piece of text (and the ⋯) that sticks out of it or
// into the next column
const rows = (page, list) => page.evaluate((list) => {
  const out = [];
  for (const row of [...document.querySelectorAll(`${list} .track`)].slice(0, 14)) {
    row.scrollIntoView({ block: 'center', behavior: 'instant' });
    const r = row.getBoundingClientRect();
    const bad = [];
    const shown = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden' && parseFloat(getComputedStyle(el).opacity) > 0;
    for (const el of row.querySelectorAll('.title, .by, .num, .dur, [data-act="more"]')) {
      if (!shown(el)) continue;
      const b = el.getBoundingClientRect();
      if (b.top < r.top - 0.5 || b.bottom > r.bottom + 0.5) bad.push(`${el.className || el.dataset.act} ${b.top.toFixed(0)}..${b.bottom.toFixed(0)} outside ${r.top.toFixed(0)}..${r.bottom.toFixed(0)}`);
      // nor out of its side (the list clips its rows, so #main doesn't get wider when a row does)
      if (b.left < r.left - 0.5 || b.right > r.right + 0.5) bad.push(`${el.className || el.dataset.act} ${b.left.toFixed(0)}..${b.right.toFixed(0)} wider than the row ${r.left.toFixed(0)}..${r.right.toFixed(0)}`);
      // the length's words fit its own box (no length running out of its column)
      if (el.matches('.dur') && el.scrollWidth > el.clientWidth + 1) bad.push(`${el.className} "${el.textContent.trim()}" ${el.scrollWidth} px in ${el.clientWidth}`);
      // the number sits inside its column (the .no box: .num is an item of that flex box, so its own
      // scrollWidth never exceeds its width; it runs out of the column instead)
      const no = el.matches('.num') && el.closest('.no');
      if (no) { const c = no.getBoundingClientRect(); if (b.left < c.left - 0.5 || b.right > c.right + 0.5) bad.push(`number "${el.textContent.trim()}" ${b.left.toFixed(0)}..${b.right.toFixed(0)} outside its column ${c.left.toFixed(0)}..${c.right.toFixed(0)}`); }
    }
    // the length's words end before the ⋯ starts
    const dur = row.querySelector('.dur'); const more = row.querySelector('[data-act="more"]');
    if (dur && more && shown(dur) && shown(more)) {
      const rg = document.createRange(); rg.selectNodeContents(dur); const t = rg.getBoundingClientRect(); const m = more.getBoundingClientRect();
      if (t.right > m.left + 0.5) bad.push(`length "${dur.textContent.trim()}" ends at ${t.right.toFixed(0)}, ⋯ starts at ${m.left.toFixed(0)}`);
    }
    const w = (sel) => { const el = row.querySelector(sel); return el && shown(el) ? Number(el.getBoundingClientRect().width.toFixed(1)) : null; };
    out.push({ h: r.height, bad, dur: w('.dur'), no: row.matches('.seq') ? null : w('.no') }); // (a library list's .no is the keyboard's Play over the row's start, not a column)
  }
  window.scrollTo(0, 0); document.querySelector('#main')?.scrollTo?.(0, 0);
  // (the page and #main, which scrolls on its own)
  const main = document.querySelector('#main');
  return { rows: out, fits: document.documentElement.scrollWidth <= innerWidth && (!main || main.scrollWidth <= main.clientWidth) };
}, list);

async function phone(browser, width, k) {
  const at = `android-${width}${k !== 1 ? ` at ${Math.round(k * 100)} % text zoom` : ''}`;
  const ts = many(30);
  const { page, context, state } = await fx.create(browser, width, { mobile: true, tracks: ts, fixtures: { ...sources, '/bookmarks': { body: marked(ts.slice(0, 14)) }, '/favourites': { body: marked(ts.slice(0, 14)) } } });
  try {
    await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    const pages = [['Songs', '/tracks', '#tracklist'], ['Bookmarks', '/favourites', '[data-list="fav"]'], ['Service album', '/sources/soundcloud', '#main .src-tracks']];
    for (const [name, url, list] of pages) {
      await fx.go(page, url);
      if (name === 'Service album') await page.click('[data-sitem="abc"]');
      await page.waitForSelector(`${list} .track`);
      if (k !== 1) await textZoom(page, k);
      await page.waitForTimeout(300);
      const got = await rows(page, list);
      const bad = got.rows.flatMap((r, i) => r.bad.map((b) => `row ${i + 1}: ${b}`));
      check(got.rows.length >= 10 && !bad.length, `${at} ${name}: every row holds its title, artist, length and ⋯ (${got.rows.length} rows${bad.length ? `; ${bad.slice(0, 3).join('; ')}` : ''})`);
      const hs = [...new Set(got.rows.map((r) => r.h.toFixed(1)))];
      if (k === 1) {
        check(hs.length === 1 && hs[0] === '54.0', `${at} ${name}: a row is 54 px, as before (${hs.join(', ')})`);
        const durs = [...new Set(got.rows.map((r) => r.dur))]; const nos = [...new Set(got.rows.map((r) => r.no).filter((x) => x != null))];
        // (song 12's "12:34" gets 38.5: the column grows to fit its words, tools/ui-phone-hour-length.js)
        check(durs.includes(38) && durs.every((d) => d === 38 || d === 38.5) && (!nos.length || (nos.length === 1 && nos[0] === 26)), `${at} ${name}: the length column is 38 px${nos.length ? ' and the number column 26 px' : ''}, as before (${durs.join(', ')}${nos.length ? `; ${nos.join(', ')}` : ''})`);
      }
      else check(hs.length === 1 && Number(hs[0]) > 54, `${at} ${name}: the rows grow with the text, all the same height (${hs.join(', ')} px)`);
      check(got.fits, `${at} ${name}: no sideways overflow`);
      await page.evaluate((list) => document.querySelector(`${list} .track`).scrollIntoView({ block: 'start', behavior: 'instant' }), list);
      await page.screenshot({ path: path.join(SHOTS, `${name.toLowerCase()}-${at}.png`.replace(/ /g, '-')) });
    }
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

// A desktop window and its dense list: the row heights are what they were (54 px; 42 px dense)
async function desktop(browser, width, dense) {
  const at = `desktop-${width}${dense ? ' dense' : ''}`;
  const { page, context, state } = await fx.create(browser, width, { tracks: many(30) });
  try {
    if (dense) await context.addInitScript(() => { localStorage.setItem('pm.dense', '1'); });
    await fx.go(page, '/tracks'); await page.waitForSelector('#tracklist .track');
    if (dense) await page.evaluate(() => document.documentElement.setAttribute('data-dense', ''));
    await page.waitForTimeout(200);
    const hs = await page.evaluate(() => [...new Set([...document.querySelectorAll('#tracklist .track')].slice(0, 10).map((r) => r.getBoundingClientRect().height.toFixed(1)))]);
    check(hs.length === 1 && hs[0] === (dense ? '42.0' : '54.0'), `${at} Songs: a row is ${dense ? 42 : 54} px, as before (${hs.join(', ')})`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  const zoomed = await chromium.launch({ args: ZOOM_ARGS });
  try {
    for (const w of [360, 412]) await phone(browser, w, 1);
    for (const [w, k] of [[360, 1.3], [360, 1.5], [412, 1.5], [412, 2], [360, 2]]) await phone(zoomed, w, k);
    await desktop(browser, 1280, false);
    await desktop(browser, 1280, true);
  } finally { await browser.close(); await zoomed.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
