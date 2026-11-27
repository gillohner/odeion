// Every Play shows the same "starting" spinner while its music starts (desktop and tablet, 1280,
// 1024 and 768 px, on This browser). Before, only an album's Play and Shuffle, a row's or a tile's
// Play and the bar's did: a collection's Play and Shuffle, the Collections list's Play, a page's
// sticky Play (the bar that shows when the page's top is scrolled away), Songs' Play all, the
// Top result's Play in search and a radio station's Play gave no sign that the press was taken.
// The album's Play and Shuffle are checked too (they spun before as well). Only the Play pressed
// spins, and a Play with an icon keeps its width: the spinner takes the icon's place and is as big
// (it was 2 px smaller, so the button and the ones beside it moved). Songs' Play all has its play
// icon, as every page-wide Play (below 380 px in the phone layout only its words: a spinner after them).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-play-starting.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'play-starting-shots'));
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };

// An album of 30 songs (the album page is taller than the window, so its sticky bar can show)
const tracks = Array.from({ length: 30 }, (_, k) => ({ ...fx.track, id: k + 1, title: k ? `Song ${k + 1}` : fx.track.title, track_no: k + 1 }));
const station = { id: 'st1', name: 'Fixture FM', url: 'https://radio.example/stream', country: 'CH', tags: ['jazz'] };
const searchHits = [{ kind: 'song', id: 'lib-1', track_id: 1, title: fx.track.title, subtitle: 'Sefa, Dr. Peacock', album: 'Journey', sources: ['library'], exact: true }];
const FIXTURES = { '/radio/stations': { body: { stations: [station] } }, ...Object.fromEntries(tracks.map((t) => [`/library/tracks/${t.id}`, { body: t }])) };

// [what, route, selector, and 'sticky': the page scrolled down first, so its sticky bar shows]
const CASES = [
  ['the album\'s Play', '/album/lib-7', '#iplay'],
  ['the album\'s Shuffle', '/album/lib-7', '#ishuffle'],
  ['the album\'s sticky Play', '/album/lib-7', '.stickyhead:not([hidden]) [data-sticky-play]', 'sticky'],
  ['the collection\'s Play', '/playlist/c1', '#plplay'],
  ['the collection\'s Shuffle', '/playlist/c1', '#plshuffle'],
  ['the Collections list\'s Play', '/playlists', '[data-plplay]'],
  ['Songs\' Play all', '/tracks', '#shuffle'],
  ['the Top result\'s Play', '/search?q=Journey%20Through%20Time', '[data-splaytop]'],
  ['a radio station\'s Play', '/radio', '[data-st-play]'],
];

async function run(browser, width) {
  for (const [what, route, sel, before] of CASES) {
    const { context, page, state } = await fx.create(browser, width, { tracks, searchHits, collections: [fx.summary], fixtures: FIXTURES });
    try {
      await fx.go(page, route);
      if (before === 'sticky') {
        await page.locator('#iplay').waitFor();
        await page.evaluate(() => { const m = document.querySelector('.main') || document.scrollingElement; m.scrollTop = m.scrollHeight; });
      }
      const b = page.locator(sel).first();
      const found = await b.waitFor({ timeout: 5000 }).then(() => true, () => false);
      check(found, `${width} ${what}: on screen (${route})`);
      if (!found) continue;
      await page.waitForTimeout(300);
      check(!(await b.evaluate((el) => el.classList.contains('starting'))), `${width} ${what}: no spinner before the press`);
      const w0 = (await b.boundingBox()).width; const hasIcon = await b.evaluate((el) => !!el.querySelector('.ic'));
      await b.click();
      // (This browser never starts here: the fixture's audio does not play, so the wait stays)
      const spun = await page.waitForFunction((s) => { const el = document.querySelector(s); return el && el.classList.contains('starting'); }, sel, { timeout: 2000 }).then(() => true, () => false);
      check(spun, `${width} ${what}: spins while the music starts`);
      if (spun) {
        const w1 = (await b.boundingBox()).width;
        if (hasIcon) check(Math.abs(w1 - w0) < 1, `${width} ${what}: keeps its width while it spins (${w0.toFixed(1)} → ${w1.toFixed(1)} px)`);
        check(await page.evaluate(() => document.querySelectorAll('.starting').length) === 1, `${width} ${what}: only the Play pressed spins`);
      }
      if (spun) await page.screenshot({ path: path.join(shots, `${what.replace(/\W+/g, '-')}-${width}.png`) });
      check(!state.errors.length, `${width} ${what}: no page errors ${state.errors.join(' | ')}`);
    } finally { await context.close(); }
  }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768]) await run(browser, w); } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} FAIL`);
  process.exit(fail ? 1 : 0);
})();
