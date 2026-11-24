// Albums and Artists on a phone: the Sort row under the Filter field keeps a 12 px gap, as the
// Quality chips under it do (it touched the field: 0 px); under the Quality chips it keeps its 16 px,
// and nothing runs past the screen. Desktop keeps its spacing.
// An album tile's second line (artist · year, then the quality badge) keeps the badge whole: a long
// artist name is shortened with "…" instead of the badge being cut off behind a bare "…", the line
// keeps its height with or without a badge, and a tile with no badge still shortens its text.
// Real candidate assets, isolated responses: node tools/ui-list-head-tiles.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'list-head-tiles-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// 52 albums and artists (a Filter field, the A–Z strip), long artist names on some.
const NAMES = ['Abba', 'Blur', 'Cream', 'Doors', 'Eels', 'Faust', 'Genesis', 'Hole', 'Iggy', 'Jet', 'Kiss', 'Love', 'Muse', 'Nena', 'Oasis', 'Pixies', 'Queen', 'Rush', 'Sade', 'Toto', 'U2', 'Vangelis', 'Wire', 'Xtc', 'Yes', 'Zappa'];
const LONG = 'The Paper Lanterns Orchestra';
// (Yes 1: no artist and no year, so its line holds the badge alone)
const albums = Array.from({ length: 52 }, (_, i) => ({ id: i + 1, title: `${NAMES[i % 26]} ${Math.floor(i / 26) + 1}`, album_artist: i === 24 ? null : i % 2 ? LONG : NAMES[i % 26], year: i === 24 ? null : 1990 + (i % 30), tracks: 1 }));
const artists = Array.from({ length: 52 }, (_, i) => ({ id: i + 1, name: `${NAMES[i % 26]} ${Math.floor(i / 26) + 1}`, albums: 1, tracks: 1 }));
// One quality for every album (no Quality chips), or two and none for the last two (the chips show)
const formats = (mixed) => albums.slice(0, mixed ? 50 : 52).map((a, i) => ({ album_id: a.id, label: mixed && i % 2 ? '24/96' : 'FLAC', hi_res: mixed && i % 2 === 1, lossless: true, dsd: false }));
const library = (mixed) => ({ '/library/albums': { body: albums }, '/library/albums/formats': { body: formats(mixed) }, '/library/artists': { body: artists } });

// The gap from what sits above the Sort row (the head with the Filter, or the Quality chips) to the
// Sort row's first control, both by their drawn boxes.
const gaps = (page) => page.evaluate(() => {
  const bar = document.querySelector('.listbar');
  const above = bar.previousElementSibling;
  const aboveBox = above.matches('.qualchips') ? [...above.querySelectorAll('.chip')].reduce((b, c) => Math.max(b, c.getBoundingClientRect().bottom), 0) : Math.max(...[...above.querySelectorAll('#filter, h2, .btn')].map((el) => el.getBoundingClientRect().bottom));
  const top = Math.min(...[...bar.querySelectorAll('select, [data-lfav]')].map((el) => el.getBoundingClientRect().top));
  return { above: above.matches('.qualchips') ? 'Quality chips' : 'Filter row', gap: top - aboveBox, fits: document.documentElement.scrollWidth <= innerWidth };
});

// Each tile's second line: its box, its text's box and whether the text is shortened, the badge's box.
const tiles = (page) => page.evaluate(() => [...document.querySelectorAll('#albumgrid .tile')].slice(0, 12).map((t) => {
  const a = t.querySelector('.a'), b = a.getBoundingClientRect();
  const badge = a.querySelector('.fmt'); const fb = badge && badge.getBoundingClientRect();
  const text = a.querySelector('.at') || a;
  return { title: t.querySelector('.t').textContent, h: b.height, left: b.left, right: b.right, badge: fb && { left: fb.left, right: fb.right, label: badge.textContent }, textCut: text.scrollWidth > text.clientWidth + 0.5, textShown: text.clientWidth, textFull: text.scrollWidth, text: [...a.childNodes].filter((n) => !(n.classList && n.classList.contains('fmt'))).map((n) => n.textContent).join('') };
}));

async function heads(browser, width, android, mixed) {
  const at = `${width}${android ? ' Android' : ''}${mixed ? ' (Quality chips)' : ''}`;
  const { page, context } = await fx.create(browser, width, { mobile: width < 600, fixtures: library(mixed) });
  try {
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    for (const route of ['/albums', '/artists']) {
      if (route === '/artists' && mixed) continue;
      await fx.go(page, route);
      await page.locator('.listbar').waitFor();
      await page.locator(route === '/albums' ? '#albumgrid .tile' : '#artistlist > *').first().waitFor();
      if (route === '/albums') await page.waitForFunction((m) => !!document.querySelector('.qualchips') === m, mixed);
      const g = await gaps(page);
      // (under the Quality chips it was 16 px already, and stays)
      const want = g.above === 'Quality chips' ? 16 : 12;
      if (width < 600) check(Math.abs(g.gap - want) <= 0.5, `${at} ${route}: ${g.above} to the Sort row ${g.gap.toFixed(1)} px (${want})`);
      else check(g.gap >= 0, `${at} ${route}: ${g.above} to the Sort row ${g.gap.toFixed(1)} px (not overlapping)`);
      check(g.fits, `${at} ${route}: nothing past the screen`);
      await page.screenshot({ path: path.join(SHOTS, `head-${route.slice(1)}-${width}${android ? '-android' : ''}${mixed ? '-quality' : ''}.png`) });
    }
    if (!mixed) return;
    // The tiles (Albums again)
    await fx.go(page, '/albums');
    await page.locator('#albumgrid .tile .fmt').first().waitFor();
    const ts = await tiles(page);
    const withBadge = ts.filter((t) => t.badge);
    const cut = withBadge.filter((t) => t.text.includes(LONG));
    check(withBadge.length >= 6 && withBadge.every((t) => t.badge.left >= t.left - 0.5 && t.badge.right <= t.right + 0.5), `${at}: every tile's quality badge is whole inside its line (${withBadge.map((t) => `${t.title} ${t.badge.label} ${(t.right - t.badge.right).toFixed(1)}`).join(', ')})`);
    if (width < 600) check(cut.length > 0 && cut.every((t) => t.textCut), `${at}: a long artist name is shortened with … before the badge (${cut.map((t) => `${t.textShown.toFixed(0)}/${t.textFull.toFixed(0)}`).join(', ')})`);
    const short = withBadge.filter((t) => !t.text.includes(LONG));
    check(short.length > 0 && short.every((t) => !t.textCut && t.badge.left > t.left + 20), `${at}: a short line shows its whole text and the badge after it`);
    const hs = [...new Set(ts.map((t) => t.h.toFixed(1)))];
    check(hs.length === 1, `${at}: the tiles' second lines with a badge are all the same height (${hs.join(', ')} px)`);
    // An album with no quality (Zappa 2, the long artist) shortens its text too
    await page.locator('#filter').fill('Zappa 2');
    await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length === 1);
    const plainTiles = (await tiles(page)).filter((t) => !t.badge);
    check(plainTiles.length > 0 && plainTiles.every((t) => t.h.toFixed(1) === hs[0] && (!t.text.includes(LONG) || width >= 600 || t.textCut)), `${at}: a tile without a badge keeps the line's height and shortens a long name (${plainTiles.map((t) => `${t.title} ${t.h.toFixed(1)}`).join(', ')})`);
    // An album with the badge alone (no artist, no year): the same height again
    await page.locator('#filter').fill('Yes 1');
    await page.waitForFunction(() => document.querySelectorAll('#albumgrid .tile').length === 1);
    const lone = (await tiles(page))[0];
    check(!!lone.badge && !lone.text && lone.h.toFixed(1) === hs[0], `${at}: a line with the badge alone is as tall as the others (${lone.h.toFixed(1)} px)`);
    await page.locator('#filter').fill('');
    await page.waitForTimeout(200);
    await page.screenshot({ path: path.join(SHOTS, `tiles-${width}${android ? '-android' : ''}.png`) });
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [w, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
      await heads(browser, w, android, false);
      await heads(browser, w, android, true);
    }
  } finally { await browser.close(); }
  console.log(`${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
