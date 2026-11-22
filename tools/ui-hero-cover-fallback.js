// A page's big picture (album, song and artist pages, a person on Pubky) that fails to load leaves the
// page's monogram (its initials on its colour), as the tiles and song rows do, not an empty grey box
// (album, song) or nothing at all (artist, person). Also when the picture comes in late with About
// (the page asked again): the monogram stays under it, with the source badge. A picture that loads
// covers the monogram and gets its own backing (.loaded). 1280, 1024 and 768 px.
// Isolated API fixtures; no node or external network required.
//   node tools/ui-hero-cover-fallback.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/hero-cover-fallback-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
// A 4 × 4 opaque PNG: a cover that loads
const crc = (b) => { let c = ~0; for (const x of b) { c ^= x; for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1)); } return ~c >>> 0; };
const chunk = (t, d) => { const len = Buffer.alloc(4); len.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([len, td, c]); };
const ihdr = Buffer.from([0, 0, 0, 4, 0, 0, 0, 4, 8, 2, 0, 0, 0]);
const raw = Buffer.concat(Array.from({ length: 4 }, () => Buffer.from([0, ...Array(12).fill(200)])));
const PNG = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);

const sefa = { id: 11, name: 'Sefa' };
const tracks = [
  { ...fx.track, id: 1, title: 'Journey Through Time', artists: [sefa], album: { id: 7, title: 'Journey' } },
  { ...fx.track, id: 3, title: 'Feathers', artists: [{ id: 22, name: 'Dr. Peacock' }], album: { id: 8, title: 'Plumage' } },
];
const PID = 'a'.repeat(52);
const BROKEN = '/v1/library/albums/99/cover'; // the fixture answers every cover 404 but album 8's (below)
const base = () => ({
  ...Object.fromEntries(tracks.map((t) => [`/library/tracks/${t.id}`, { body: t }])),
  // An artist page with a portrait that fails
  '/item/artist/lib-11': { body: { ...fx.itemPage('artist', 'lib-11', tracks), image: BROKEN } },
  ['/profiles/' + PID]: { body: { name: 'Ada Lovelace', avatar: BROKEN } },
  ['/shared/' + PID + '/playlists']: { body: [] },
});

// What the hero's picture box shows: its initials (seen), its picture (if any, and whether it covers the box), its badge
const hero = (page) => page.evaluate(() => {
  const art = document.querySelector('#main .hero .art'); if (!art) return null;
  const r = art.getBoundingClientRect(); const span = art.querySelector(':scope > span:not([class])');
  const sr = span && span.getBoundingClientRect(); const img = art.querySelector(':scope > img');
  const ir = img && img.getBoundingClientRect(); const badge = art.querySelector('.src-badge');
  // (the badge lets clicks through, pointer-events: none, so it is made hit-testable for the look)
  const top = (el) => { const was = el.style.pointerEvents; el.style.pointerEvents = 'auto'; const b = el.getBoundingClientRect(); const at = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2); el.style.pointerEvents = was; return !!at && (at === el || el.contains(at)); };
  return {
    w: Math.round(r.width), h: Math.round(r.height),
    initials: span ? span.textContent.trim() : '', initialsShown: !!sr && sr.width > 0 && getComputedStyle(span).visibility !== 'hidden',
    gradient: /gradient/.test(getComputedStyle(art).backgroundImage),
    img: !!img, imgCovers: !!ir && Math.abs(ir.width - r.width) < 2 && Math.abs(ir.height - r.height) < 2 && img.naturalWidth > 0, loaded: !!img && img.classList.contains('loaded'),
    badge: !!badge, badgeOnTop: !!badge && top(badge), shadow: getComputedStyle(art).boxShadow !== 'none',
  };
});
const settle = async (page, ms = 900) => { await page.locator('#main .hero .art').first().waitFor({ timeout: 5000 }).catch(() => {}); await page.waitForTimeout(ms); };

async function run(browser, width) {
  const { context, page, state } = await fx.create(browser, width, { tracks, fixtures: base() });
  await page.route('**/v1/library/albums/8/cover**', (r) => r.fulfill({ status: 200, contentType: 'image/png', body: PNG }));
  const at = `${width}`;
  try {
    // An album whose cover fails (the fixture keeps no artwork): its monogram, with the library badge
    await fx.go(page, '/album/lib-7'); await settle(page);
    let h = await hero(page);
    check(h && !h.img && h.initials === 'J' && h.initialsShown && h.gradient, `${at} album, cover fails: the hero shows its monogram J (${JSON.stringify(h)})`);
    check(h && h.badge && h.badgeOnTop, `${at} album, cover fails: the source badge is still on it`);
    check(h && h.w >= 140 && h.h === h.w, `${at} album, cover fails: the box keeps its square size (${h && h.w}×${h && h.h})`);
    await page.screenshot({ path: path.join(shots, `album-broken-${width}.png`) });
    // A song page whose album cover fails
    await fx.go(page, '/song/lib-1'); await settle(page);
    h = await hero(page);
    check(h && !h.img && h.initials === 'JT' && h.initialsShown && h.gradient, `${at} song, cover fails: the hero shows its monogram JT (${JSON.stringify(h)})`);
    // An album whose cover loads: the picture covers the monogram, with a backing, under the badge
    await fx.go(page, '/album/lib-8'); await settle(page);
    h = await hero(page);
    check(h && h.img && h.imgCovers && h.loaded, `${at} album, cover loads: the picture covers the whole box and has its backing (${JSON.stringify(h)})`);
    check(h && h.badge && h.badgeOnTop, `${at} album, cover loads: the source badge is over the picture`);
    await page.screenshot({ path: path.join(shots, `album-cover-${width}.png`) });
    // An artist whose portrait fails: a round monogram, not an empty space
    await fx.go(page, '/artist/lib-11'); await settle(page);
    h = await hero(page);
    check(h && !h.img && h.initials === 'S' && h.initialsShown && h.gradient, `${at} artist, portrait fails: the hero shows its monogram S (${JSON.stringify(h)})`);
    check(h && h.shadow, `${at} artist, portrait fails: the monogram has the shadow of one without a portrait`);
    await page.screenshot({ path: path.join(shots, `artist-broken-${width}.png`) });
    // A person on Pubky whose picture fails
    await fx.go(page, '/person/' + PID); await settle(page);
    h = await hero(page);
    check(h && !h.img && h.initials === 'AL' && h.initialsShown && h.gradient, `${at} person, picture fails: the hero shows their monogram AL (${JSON.stringify(h)})`);
    check(h && h.shadow, `${at} person, picture fails: the monogram has the shadow of one without a picture`);
    await page.screenshot({ path: path.join(shots, `person-broken-${width}.png`) });
    // An artist whose portrait loads: round, over the monogram, no shadow (as before)
    state.fixtures['/item/artist/lib-22'] = { body: { ...fx.itemPage('artist', 'lib-22', tracks), image: '/v1/library/albums/8/cover' } };
    await fx.go(page, '/artist/lib-22'); await settle(page);
    h = await hero(page);
    check(h && h.img && h.imgCovers && h.loaded && !h.shadow, `${at} artist, portrait loads: it covers the round box, no shadow (${JSON.stringify(h)})`);
    // The picture comes with About (the page asks again): a failing one leaves the monogram and the badge
    for (const [kind, id, img, loads] of [['album', 'lib-7', BROKEN, false], ['album', 'lib-8', '/v1/library/albums/8/cover', true]]) {
      const key = `/item/${kind}/${id}`; const full = fx.itemPage(kind, id, tracks);
      state.fixtures[key] = { body: { ...full, image: null, about_pending: true } };
      await fx.go(page, `/${kind}/${id}`); await settle(page, 300);
      const before = await hero(page);
      state.fixtures[key] = { body: { ...full, image: img, about_pending: false } };
      // (asked again after 3 s: wait until the picture went in, then for it to load or fail)
      await page.waitForFunction(() => { const a = document.querySelector('#main .hero .art'); return a && !a.classList.contains('placeholder'); }, null, { timeout: 10000 }).catch(() => {});
      await page.waitForTimeout(800);
      h = await hero(page);
      const what = `${at} ${kind} ${id}, picture comes with About (${loads ? 'loads' : 'fails'})`;
      check(before && !before.img && before.initialsShown, `${what}: the monogram while About is pending`);
      if (loads) check(h && h.img && h.imgCovers && h.loaded && h.badgeOnTop, `${what}: the picture over the monogram, under the badge (${JSON.stringify(h)})`);
      else check(h && !h.img && h.initials === 'J' && h.initialsShown && h.gradient && h.badge, `${what}: the monogram and the badge stay (${JSON.stringify(h)})`);
      delete state.fixtures[key];
    }
    await fx.cleanCopy(page);
    check(!state.errors.length, `${at} no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768]) await run(browser, w); } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} FAIL`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
