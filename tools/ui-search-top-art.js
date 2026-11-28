// Search's Top result is never an empty square. With no picture (or one that doesn't load) it
// showed a plain grey square or circle, while the same artist or album right under it in Artists
// or Albums, and on its own page, shows its initials on a coloured cover. Now the Top result's
// cover is drawn the same way as the tile: the same colours and initials, the picture over them
// when there is one. At 360, 412 (phone, also in the Android app) and 1280 px, for an artist (round),
// an album and a song:
//   * no picture: the cover has the tile's colours and initials, in the middle of the cover;
//   * a picture that doesn't load: the same, once the broken picture is gone;
//   * a picture: it covers the whole cover;
//   * the cover is decoration for screen readers (the title next to it is the link), the page has
//     no sideways scroll and no page errors.
// Real candidate assets, isolated responses: node tools/ui-search-top-art.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'search-top-art-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// The query matches each Top result exactly; the hit is also the first tile of its kind (songs
// are rows, so a song's Top result is compared with a tile of an album with the same title)
const PICTURES = { none: undefined, broken: '/v1/library/albums/999/cover', loads: '/icon-192.png' };
const CASES = [
  { kind: 'artist', q: 'journey artists', title: 'Journey Artists', initials: 'JA', round: true },
  { kind: 'album', q: 'night record', title: 'Night Record', initials: 'NR', round: false },
  { kind: 'song', q: 'tears of time', title: 'Tears Of Time', initials: 'TT', round: false },
];
const hitsFor = (c, image) => {
  const top = { kind: c.kind, id: c.kind === 'artist' ? 'lib-11' : c.kind === 'album' ? 'lib-7' : 'lib-1', title: c.title, subtitle: c.kind === 'artist' ? '' : 'Sefa', keys: [`${c.kind === 'song' ? 'track' : c.kind}:1`], exact: true, ...(image ? { image } : {}), ...(c.kind === 'song' ? { track_id: 1 } : {}) };
  // A tile of the same title (and no picture), so the two covers can be compared
  const twin = c.kind === 'song' ? { kind: 'album', id: 'mb-album-twin', title: c.title, subtitle: 'Sefa', keys: ['album:twin'] } : null;
  return [top, ...(twin ? [twin] : [])];
};

const read = (page, title) => page.evaluate((title) => {
  const art = document.querySelector('#stop .art');
  if (!art) return null;
  const cs = getComputedStyle(art); const b = art.getBoundingClientRect();
  const span = [...art.children].find((e) => e.tagName === 'SPAN' && !e.className);
  const sb = span && span.getBoundingClientRect();
  const img = art.querySelector('img');
  const ib = img && img.getBoundingClientRect();
  const tile = [...document.querySelectorAll('#sresults .tile')].find((t) => t.title === title);
  const tileArt = tile && tile.querySelector('.art');
  return {
    bg: cs.backgroundImage, tileBg: tileArt ? getComputedStyle(tileArt).backgroundImage : null,
    text: span ? span.textContent : '', tileText: tileArt ? (tileArt.querySelector(':scope > span:first-child') || {}).textContent : null,
    textShown: !!span && getComputedStyle(span).visibility !== 'hidden' && sb.width > 0,
    centred: !!sb && Math.abs((sb.left + sb.width / 2) - (b.left + b.width / 2)) <= 1 && Math.abs((sb.top + sb.height / 2) - (b.top + b.height / 2)) <= 1,
    radius: cs.borderTopLeftRadius, size: [Math.round(b.width), Math.round(b.height)],
    img: img ? { loaded: img.complete && img.naturalWidth > 0, covers: Math.abs(ib.width - b.width) <= 1 && Math.abs(ib.height - b.height) <= 1 } : null,
    hidden: art.getAttribute('aria-hidden') === 'true' || !!art.closest('[aria-hidden="true"]'),
    overflowX: document.documentElement.scrollWidth - innerWidth,
  };
}, title);

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [w, android] of [[360, false], [412, false], [360, true], [412, true], [1280, false]]) {
      for (const c of CASES) for (const [pic, image] of Object.entries(PICTURES)) {
        const at = `${w}${android ? ' android' : ''} ${c.kind}, picture ${pic}`;
        const run = await fx.create(browser, w, { mobile: w < 600, searchHits: hitsFor(c, image) });
        if (android) await run.context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
        const { page, context, state } = run;
        try {
          await fx.go(page, `/search?q=${encodeURIComponent(c.q)}`);
          await page.waitForSelector('#stop .art', { timeout: 6000 });
          // A picture settles: it loads or is taken away (data-err="remove")
          await page.waitForFunction(() => { const i = document.querySelector('#stop .art img'); return !i || (i.complete && (i.naturalWidth > 0 || !i.isConnected)); }, null, { timeout: 4000 }).catch(() => {});
          await page.waitForTimeout(150);
          const r = await read(page, c.title);
          if (!r) { check(false, `${at}: the Top result is drawn`); continue; }
          check(/linear-gradient/.test(r.bg), `${at}: the cover has colours, not a plain grey ${pic === 'loads' ? 'under the picture' : 'square'} (${r.bg.slice(0, 40)})`);
          if (r.tileBg) check(r.bg === r.tileBg, `${at}: the same colours as the tile of the same name below`);
          if (pic === 'loads') {
            check(!!r.img && r.img.loaded && r.img.covers, `${at}: the picture loads over the whole cover`);
          } else {
            check(!r.img, `${at}: no picture left on the cover (${JSON.stringify(r.img)})`);
            check(r.text === c.initials && r.textShown, `${at}: the cover says ${c.initials} (says "${r.text}")`);
            if (r.tileText != null) check(r.text === r.tileText, `${at}: the same initials as the tile below ("${r.tileText}")`);
            check(r.centred, `${at}: the initials sit in the middle of the cover`);
          }
          check(c.round ? r.radius === '50%' : r.radius !== '50%', `${at}: ${c.round ? 'an artist is round' : 'square corners'} (${r.radius})`);
          check(r.hidden, `${at}: screen readers skip the cover (the title is the link)`);
          check(r.overflowX <= 0, `${at}: no sideways scroll (${r.overflowX})`);
          check(!state.errors.length, `${at}: no page errors ${state.errors.join(' | ')}`);
          if (pic !== 'loads') await page.locator('#stop').screenshot({ path: path.join(SHOTS, `top-${c.kind}-${pic}-${w}${android ? '-android' : ''}.png`) });
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
