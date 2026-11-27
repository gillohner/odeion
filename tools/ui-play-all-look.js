// Songs' Play all looks like every other page's Play: the filled (primary) button with the play icon,
// as Bookmarks' Play songs, a genre's Play all and an album's or collection's Play. It was the one
// page-level Play drawn as an outline button with no icon. The head still fits one row: Play all and
// Add music (desktop), the filter, Play all and + (narrow screens), nothing wider than the screen.
// Below 380 px a phone shows the word only (as Shuffle there).
// The buttons sit level with the title from 600 px, on their own row under it on a phone.
// At 1280, 1024, 768, 412 and 360 px. Isolated API fixtures; no node needed.
//   node tools/ui-play-all-look.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'play-all-look-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// 20 songs, so Songs also draws its filter (from 13) beside Play all
const tracks = Array.from({ length: 20 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
const look = (sel) => { const b = document.querySelector(sel); if (!b) return null; const s = getComputedStyle(b); const r = b.getBoundingClientRect(); const ic = b.querySelector(':scope > svg.ic use'); return { primary: b.classList.contains('primary'), icon: ic ? ic.getAttribute('href') : null, text: b.textContent.trim(), bg: s.backgroundColor, color: s.color, border: s.borderTopColor, weight: s.fontWeight, h: Math.round(r.height), top: Math.round(r.top), bottom: Math.round(r.bottom), right: Math.round(r.right) }; };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 412, 360]) {
      const { page, context, state } = await fx.create(browser, width, { tracks, favs: [fx.SONG], mobile: width < 600 });
      try {
        await fx.go(page, '/favourites');
        await page.waitForSelector('#favplay');
        const fav = await page.evaluate(look, '#favplay');
        await fx.go(page, '/tracks');
        await page.waitForSelector('#shuffle');
        await page.waitForTimeout(200);
        const all = await page.evaluate(look, '#shuffle');
        const head = await page.evaluate(() => {
          const h = document.querySelector('.main .page-head'); const t = h.querySelector('h1, h2').getBoundingClientRect();
          const f = h.querySelector('input'); const a = h.querySelector('[data-addmusic]');
          const mid = (r) => r && Math.round(r.top + r.height / 2);
          return { title: mid(t), filter: f ? mid(f.getBoundingClientRect()) : null, add: a ? mid(a.getBoundingClientRect()) : null, sw: document.documentElement.scrollWidth, iw: innerWidth };
        });
        await page.screenshot({ path: path.join(SHOTS, `songs-${width}.png`) });
        const at = `${width}`;
        check(all && all.primary, `${at}: Songs' Play all is a primary button`);
        // (below 380 px a phone shows the word only, as an album's or collection's Shuffle, so the row fits at a larger font size)
        const iconShown = await page.evaluate(() => { const i = document.querySelector('#shuffle > svg.ic'); return !!i && getComputedStyle(i).display !== 'none'; });
        check(all && all.icon === '#i-play', `${at}: it has the play icon (${all && all.icon})`);
        check(iconShown === (width >= 380), `${at}: the icon is ${width >= 380 ? 'shown' : 'hidden on the narrowest phones'} (${iconShown})`);
        check(all && all.text === 'Play all', `${at}: its label is still "Play all" (${all && all.text})`);
        for (const k of ['bg', 'color', 'border', 'weight', 'h']) check(all && fav && all[k] === fav[k], `${at}: ${k} matches Bookmarks' Play songs (${all && all[k]} / ${fav && fav[k]})`);
        const mid = Math.round((all.top + all.bottom) / 2);
        // (on a phone the head's buttons go on a row of their own under the title)
        if (width >= 600) check(Math.abs(mid - head.title) <= 3, `${at}: Play all is level with the Songs title (${mid} / ${head.title})`);
        if (head.filter != null) check(Math.abs(mid - head.filter) <= 3, `${at}: the filter shares Play all's row (${head.filter} / ${mid})`);
        if (width >= 600 || head.add != null) check(head.add != null && Math.abs(mid - head.add) <= 3, `${at}: Add music shares Play all's row (${head.add} / ${mid})`);
        check(all.right <= width && head.sw <= head.iw, `${at}: nothing wider than the screen (right ${all.right}, scrollWidth ${head.sw} / ${head.iw})`);
        check(!state.errors.length, `${at}: no page errors ${state.errors.join(' | ')}`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
