// A song row's cover that doesn't load (a library without artwork: the node sends
// /v1/library/albums/<n>/cover and it 404s) leaves the song's initials on its gradient, as the
// mini player and queue draw a song without art, in the same 44 px slot (36 px on a phone), with
// the source badge still in its corner. Before, the <img> was hidden in place and the row kept an
// empty slot with a lone badge floating in it (artist page Top songs). A cover that loads still
// covers the whole slot. 1280, 1024 and 768 px, plus 412 for the phone's smaller slot.
// Isolated API fixtures; no node or external network required.
//   node tools/ui-row-cover-mono.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/row-cover-mono-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const peacock = { id: 22, name: 'Dr. Peacock' };
// Two songs whose album cover 404s (the fixture keeps no artwork) and one whose cover loads
const songs = [
  { no: null, title: 'Journey Through Time', artist: 'Sefa, Dr. Peacock', duration_ms: 240000, uri: null, track_id: 1, page: null, image: '/v1/library/albums/7/cover', id: 'lib-1', artist_id: 'lib-11', album: 'Journey', album_id: 'lib-7', versions: [] },
  { no: null, title: 'Feathers', artist: 'Dr. Peacock', duration_ms: 200000, uri: null, track_id: 3, page: null, image: '/v1/library/albums/8/cover', id: 'lib-3', artist_id: 'lib-22', album: 'Plumage', album_id: 'lib-8', versions: [] },
  { no: null, title: 'Lit Window', artist: 'Dr. Peacock', duration_ms: 180000, uri: null, track_id: 4, page: null, image: '/icon-192.png', id: 'lib-4', artist_id: 'lib-22', album: 'Lamps', album_id: 'lib-9', versions: [] },
];
// The node's library artist page shape (crates/pm-api/src/item.rs library_artist)
const artistPage = { kind: 'artist', id: 'lib-22', source: 'Your library', source_id: 'library', connect: [], reading: false, pending: false, versions: [], also_on: [], open_on: [], related: [], concerts: [], about: [], target: { artist_id: 22 }, title: peacock.name, artist: null, album: null, year: null, plays_from: { service: 'library', kind: 'artist', id: '22', label: 'Your library', url: null, mode: 'full' }, total_ms: 620000, songs, albums: [], section: 'artists' };
const fixtures = { '/item/artist/lib-22': { body: artistPage }, '/item/artist/lib-22/extras': { body: { related: [], concerts: [], similar: [] } } };

// Each Top songs row's cover slot: what is drawn there and where
const slots = (page) => page.locator('#isongs .srow').evaluateAll((rs) => rs.map((r) => {
  const w = r.querySelector('.thumbwrap'); if (!w) return { title: r.querySelector('.name')?.textContent.trim(), wrap: false };
  const box = (e) => { const b = e.getBoundingClientRect(); return { x: Math.round(b.x), y: Math.round(b.y), w: Math.round(b.width), h: Math.round(b.height) }; };
  const mono = w.querySelector('.mono'); const img = w.querySelector('img'); const badge = w.querySelector('.src-badge.cover');
  const visible = (e) => !!e && getComputedStyle(e).visibility !== 'hidden' && e.getClientRects().length > 0;
  return {
    title: r.querySelector('.name').textContent.trim(), wrap: box(w),
    mono: mono ? { text: mono.textContent.trim(), box: box(mono), visible: visible(mono), gradient: /gradient/.test(getComputedStyle(mono).backgroundImage), hidden: mono.getAttribute('aria-hidden') === 'true' } : null,
    img: img ? { box: box(img), visible: visible(img), loaded: img.complete && img.naturalWidth > 0 } : null,
    // the initials' own glyph box (a Range over the text) against the badge's: does the badge cover them?
    clear: mono && badge ? (() => { const rg = document.createRange(); rg.selectNodeContents(mono); const t = rg.getBoundingClientRect(); const b = badge.getBoundingClientRect(); return t.right <= b.left + 1 || t.left >= b.right - 1 || t.bottom <= b.top + 1 || t.top >= b.bottom - 1; })() : null,
    badge: badge ? (() => { const b = box(badge); const a = box(w); return b.x >= a.x && b.y >= a.y && b.x + b.w <= a.x + a.w + 1 && b.y + b.h <= a.y + a.h + 1; })() : null,
  };
}));

async function run(browser, width) {
  const { context, page, state } = await fx.create(browser, width, { fixtures, mobile: width < 600 });
  const at = `${width}`; const size = width < 600 ? 36 : 44;
  try {
    await fx.go(page, '/artist/lib-22');
    await page.locator('#isongs .srow').first().waitFor({ timeout: 5000 }).catch(() => {});
    await page.locator('#isongs').scrollIntoViewIfNeeded().catch(() => {});
    // the covers settle (in view, so lazy loading starts): the two 404s fire their error, the third loads
    const settled = await page.waitForFunction(() => [...document.querySelectorAll('#isongs .thumbwrap img')].every((i) => i.complete), null, { timeout: 5000 }).then(() => true, () => false);
    check(settled, `${at} the row covers finished loading or failing within 5 s`);
    await page.waitForTimeout(150);
    const ss = await slots(page);
    check(ss.length === 3, `${at} Top songs lists the three songs (${ss.map((s) => s.title)})`);
    for (const [i, initials] of [[0, 'JT'], [1, 'F']]) {
      const s = ss[i] || {};
      check(!!s.mono && s.mono.visible && s.mono.text === initials, `${at} ${s.title}: its cover 404s and the slot shows the song's initials "${initials}" (${JSON.stringify(s.mono)})`);
      check(!!s.mono && s.mono.gradient && s.mono.hidden, `${at} ${s.title}: the monogram is on the song's gradient and hidden from screen readers`);
      check(!s.img || !s.img.visible, `${at} ${s.title}: no broken or blank image left over the monogram (${JSON.stringify(s.img)})`);
      check(!!s.wrap && s.wrap.w === size && s.wrap.h === size && !!s.mono && s.mono.box.w === size && s.mono.box.h === size, `${at} ${s.title}: the slot keeps its ${size} px square (${JSON.stringify(s.wrap)})`);
      check(s.badge === true, `${at} ${s.title}: the source badge sits in the monogram's corner`);
      check(s.clear === true, `${at} ${s.title}: the badge doesn't cover the initials`);
    }
    const lit = ss[2] || {};
    check(!!lit.img && lit.img.loaded && lit.img.visible, `${at} ${lit.title}: a cover that loads is shown (${JSON.stringify(lit.img)})`);
    check(!!lit.img && !!lit.wrap && JSON.stringify(lit.img.box) === JSON.stringify(lit.wrap), `${at} ${lit.title}: the loaded cover fills the whole slot, over the monogram (${JSON.stringify(lit.img && lit.img.box)} vs ${JSON.stringify(lit.wrap)})`);
    check(lit.badge === true, `${at} ${lit.title}: its badge is in the cover's corner`);
    // every row's title starts at the same x: the slot is there whether or not the cover loaded
    const xs = await page.locator('#isongs .srow .grow').evaluateAll((gs) => gs.map((g) => Math.round(g.getBoundingClientRect().x)));
    check(new Set(xs).size === 1, `${at} the titles line up (${xs})`);
    // a row dimmed as unavailable (the class playlist pages give it) dims its slot once, as a whole:
    // the monogram under a loaded cover doesn't show through it
    const dim = await page.locator('#isongs .srow').evaluateAll((rs) => rs.map((r) => { r.classList.add('unavail'); const w = r.querySelector('.thumbwrap'); const op = (e) => (e ? Number(getComputedStyle(e).opacity) : null); const res = { wrap: op(w), mono: op(w.querySelector('.mono')), img: op(w.querySelector('img')) }; r.classList.remove('unavail'); return res; }));
    check(dim.length === 3 && dim.every((d) => d.wrap === 0.45 && d.mono === 1 && (d.img === null || d.img === 1)), `${at} an unavailable row dims the cover slot as one (${JSON.stringify(dim)})`);
    const litBg = await page.locator('#isongs .srow').nth(2).locator('.thumbwrap img').evaluate((i) => getComputedStyle(i).backgroundColor).catch(() => null);
    check(litBg === 'rgba(0, 0, 0, 0)', `${at} the cover image has no grey of its own over the monogram while it loads (${litBg})`);
    await fx.cleanCopy(page).then(() => check(true, `${at} no undefined, no overflow`), (e) => check(false, `${at} ${e.message}`));
    check(!state.errors.length, `${at} no page errors (${state.errors})`);
    await page.screenshot({ path: path.join(shots, `after-artist-${width}.png`) });
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768, 412]) await run(browser, w); } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
