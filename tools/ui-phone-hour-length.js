// A song row on a phone or in a window up to 720 px (Songs, Bookmarks, a service's album) shows a song's length in a 38 px column
// before its ⋯. A length over an hour ("1:02:34", a mix or an audiobook chapter; "10:00:00") is wider
// than that: it ran out of its column, ended to the right of every other length and touched the ⋯.
// The column now grows to fit such a length, in that row only (its title gives up the room), so every
// length ends at the same place; a row under an hour keeps its 38 px column, at Android's larger font
// sizes too (the column follows the text there, see tools/ui-song-row-text-zoom.js).
// Real candidate assets, isolated responses: node tools/ui-phone-hour-length.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-hour-length-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Songs 1-12: song 3 lasts 1:02:34 (with a long title), song 5 10:00:00, song 8 12:34, the others 4:00
const LEN = { 3: 3754000, 5: 36000000, 8: 754000 };
const many = (n) => Array.from({ length: n }, (_, i) => ({ ...fx.track, id: i + 1, title: i === 2 ? 'A long mix with a title that runs on and on' : `Song ${i + 1}`, duration_ms: LEN[i + 1] || 240000 }));
// A SoundCloud set (a service's album page: compact rows with their track numbers), the same lengths
const set = { title: 'Journey', creator: 'Sefa', year: 2020, url: 'https://soundcloud.com/sefa/sets/journey', image: null,
  tracks: many(12).map((t, i) => ({ url: `https://soundcloud.com/sefa/song-${i + 1}`, title: t.title, track_no: i + 1, duration_ms: t.duration_ms, format: 'mp3' })) };
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

// Each row's length: its words, where they end, its column's width, the ⋯'s start and the title's width
const rows = (page, list) => page.evaluate((list) => [...document.querySelectorAll(`${list} .track`)].slice(0, 12).map((row) => {
  const dur = row.querySelector('.dur'); const more = row.querySelector('[data-act="more"]'); const title = row.querySelector('.title');
  const rg = document.createRange(); rg.selectNodeContents(dur); const t = rg.getBoundingClientRect();
  return { len: dur.textContent.trim(), right: t.right, col: dur.getBoundingClientRect().width, fits: dur.scrollWidth <= dur.clientWidth + 1,
    more: more ? more.getBoundingClientRect().left : null, title: title.getBoundingClientRect().width };
}), list);

// (k: Android's font size; wide: a browser window of that width with a mouse, where the page has its
// desktop layout from 600 px on: up to 720 px its song rows are the same four columns)
async function phone(browser, width, k, wide) {
  const at = `${wide ? 'window' : 'android'}-${width}${k !== 1 ? ` at ${Math.round(k * 100)} % text zoom` : ''}`;
  const ts = many(12);
  const { page, context, state } = await fx.create(browser, width, { mobile: !wide, tracks: ts, fixtures: { ...sources, '/bookmarks': { body: marked(ts) }, '/favourites': { body: marked(ts) } } });
  try {
    if (!wide) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    for (const [name, url, list] of [['Songs', '/tracks', '#tracklist'], ['Bookmarks', '/favourites', '[data-list="fav"]'], ['Service album', '/sources/soundcloud', '#main .src-tracks']]) {
      await fx.go(page, url);
      if (name === 'Service album') await page.click('[data-sitem="abc"]');
      await page.waitForSelector(`${list} .track`);
      if (k !== 1) await textZoom(page, k);
      await page.waitForTimeout(300);
      const got = await rows(page, list);
      const lens = got.map((r) => r.len);
      check(['1:02:34', '10:00:00', '12:34', '4:00'].every((l) => lens.includes(l)), `${at} ${name}: the list shows the hour-long lengths (${lens.join(' ')})`);
      const bad = got.filter((r) => !r.fits || (r.more != null && r.right > r.more + 0.5));
      check(!bad.length, `${at} ${name}: each length fits its column and ends before the ⋯ (${bad.map((r) => `"${r.len}" ends at ${r.right.toFixed(0)}, ⋯ at ${r.more?.toFixed(0)}`).join('; ') || 'all'})`);
      const ends = got.map((r) => r.right);
      check(Math.max(...ends) - Math.min(...ends) <= 0.6, `${at} ${name}: every length ends at the same place (${got.map((r) => `${r.len} ${r.right.toFixed(1)}`).join(', ')})`);
      // (under ten minutes exactly as before; "12:34" is 0.5 px wider than 38 px with its padding, so its
      // column grows that much, where its words stuck out of it by 0.5 px)
      const col = (re) => [...new Set(got.filter((r) => re.test(r.len)).map((r) => r.col.toFixed(1)))];
      const mins = col(/^\d:\d\d$/); const tens = col(/^\d\d:\d\d$/);
      if (k === 1) check(mins.length === 1 && mins[0] === '38.0' && tens.length === 1 && Number(tens[0]) <= 39, `${at} ${name}: a length under an hour keeps its 38 px column (${mins.join(', ')}; 12:34 ${tens.join(', ')})`);
      else check(mins.length === 1 && Number(mins[0]) > 38, `${at} ${name}: under ten minutes, the length column is one width, grown with the text (${mins.join(', ')})`);
      // the long-titled hour-long row keeps most of its title's room (a few px less, not half)
      const mix = got[2]; const plain = got[0];
      check(mix.title > plain.title - 30 * k && mix.title > 100, `${at} ${name}: the 1:02:34 row's title keeps its room (${mix.title.toFixed(0)} px; a 4:00 row's ${plain.title.toFixed(0)})`);
      const fits = await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && (() => { const m = document.querySelector('#main'); return !m || m.scrollWidth <= m.clientWidth; })());
      check(fits, `${at} ${name}: no sideways overflow`);
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
    for (const w of [360, 412]) await phone(browser, w, 1);
    for (const [w, k] of [[360, 1.5], [412, 2]]) await phone(zoomed, w, k);
    await phone(browser, 680, 1, true);
  } finally { await browser.close(); await zoomed.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
