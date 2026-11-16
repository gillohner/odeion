// A song row's artist line cut with "…": with two long artist names, Tab to the second artist left
// most of its name under the "…" at 1024-1280 px (Chromium scrolls a focused element into its box only
// when less than ~32 px of it shows: 86 of 252 px at 1280). Reached by keyboard, the line now moves
// just enough to show the whole link, and goes back to its start when focus leaves the line. A click
// moves nothing (the text under the pointer stays put), and a line that isn't cut never moves. Where
// the browser did move the line (under 900 px), the "…" stayed where the line ended and the moved
// line drew blank; a moved line is now cut without "…", so the name is drawn.
// Real UI in Chromium against fixtures (no node), 1280 to 768 px.
//   node tools/ui-row-line-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'row-line-focus-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const longTracks = [{ ...fx.track, id: 1, title: 'Song', artists: [{ id: 31, name: 'Somebody With A Rather Long Artist Name Ensemble' }, { id: 32, name: 'Second Long Named Orchestra Of Somewhere' }], album: { id: 9, title: 'An Album' } }];
// Where the focused element sits in its row's artist line: how much of it shows inside the line's box
const seen = () => {
  const a = document.activeElement; const by = a && a.closest && a.closest('.by');
  if (!by) return { inLine: false, text: a && a.textContent && a.textContent.trim(), scroll: (document.querySelector('#main .track .by') || {}).scrollLeft };
  const b = by.getBoundingClientRect(); const r = a.getBoundingClientRect();
  const lo = b.left + by.clientLeft; const hi = lo + by.clientWidth;
  return { inLine: true, text: a.textContent.trim(), width: Math.round(r.width), shown: Math.round(Math.max(0, Math.min(r.right, hi) - Math.max(r.left, lo))), lineWidth: Math.round(by.clientWidth), scroll: Math.round(by.scrollLeft) };
};

// Whether the focused link's text is drawn in the right half of its box (inside the ring): a screenshot
// there, then the same with the link's text transparent; the same pixels mean nothing was drawn (a line
// scrolled under a "…" that stayed put is blank)
const drawn = async (page) => {
  const clip = await page.evaluate(() => {
    const a = document.activeElement; const by = a.closest('.by'); const b = by.getBoundingClientRect(); const r = a.getBoundingClientRect();
    const x0 = Math.max(r.left + r.width / 2, b.left + by.clientLeft); const x1 = Math.min(r.right, b.left + by.clientLeft + by.clientWidth) - 5;
    return x1 - x0 < 8 ? null : { x: Math.round(x0), y: Math.round(r.top + 2), width: Math.round(x1 - x0), height: Math.max(4, Math.round(r.height - 4)) };
  });
  if (!clip) return false;
  const shown = await page.screenshot({ clip });
  await page.evaluate(() => { document.activeElement.style.color = 'transparent'; });
  const blank = await page.screenshot({ clip });
  await page.evaluate(() => { document.activeElement.style.color = ''; });
  return !shown.equals(blank);
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 901, 768]) {
      const T = `${width}`;
      const { page, context, state } = await fx.create(browser, width, { tracks: longTracks });
      try {
        await fx.go(page, '/tracks');
        await page.waitForSelector('#main .track .by [data-artist-credit]', { state: 'attached', timeout: 8000 });
        const cut = await page.evaluate(() => { const by = document.querySelector('#main .track .by'); return by.scrollWidth > by.clientWidth; });
        check(cut, `${T}: the artist line is cut with "…" (the case under test)`);
        // Keyboard: the first artist, then Tab to the second
        await page.locator('#main .track .by [data-artist-credit]').first().focus();
        const first = await page.evaluate(seen);
        check(first.inLine && first.scroll === 0 && first.shown >= Math.min(first.width, first.lineWidth - 8), `${T}: the first artist shows from the line's start (${first.shown} of ${first.width} px, scroll ${first.scroll})`);
        await page.keyboard.press('Tab');
        const second = await page.evaluate(seen);
        check(second.text === 'Second Long Named Orchestra Of Somewhere', `${T}: Tab reaches the second artist (${second.text})`);
        check(second.inLine && second.shown >= Math.min(second.width, second.lineWidth - 8), `${T}: focused, the second artist shows whole (${second.shown} of ${second.width} px in a ${second.lineWidth} px line)`);
        check(await drawn(page), `${T}: and its name is drawn there, not blank behind a "…" (scroll ${second.scroll})`);
        await page.locator('#main .track').first().screenshot({ path: path.join(SHOTS, `second-artist-${width}.png`) });
        // Back with Shift+Tab: the first artist is in view again
        await page.keyboard.press('Shift+Tab');
        const back = await page.evaluate(seen);
        check(back.text === first.text && back.shown >= Math.min(back.width, back.lineWidth - 8), `${T}: Shift+Tab, the first artist shows again (${back.shown} of ${back.width} px)`);
        check(await drawn(page), `${T}: and its name is drawn`);
        // On past the line: Tab until focus leaves it; the line goes back to its start
        await page.keyboard.press('Tab');
        let out = null;
        for (let i = 0; i < 4; i++) {
          await page.keyboard.press('Tab');
          const s = await page.evaluate(seen);
          if (!s.inLine) { out = s; break; }
          check(s.shown >= Math.min(s.width, s.lineWidth - 8) && await drawn(page), `${T}: ${s.text} shows whole and drawn while focused (${s.shown} of ${s.width} px)`);
        }
        const after = await page.evaluate(() => { const by = document.querySelector('#main .track .by'); return { scroll: by.scrollLeft, overflow: getComputedStyle(by).textOverflow }; });
        check(!!out && after.scroll === 0 && after.overflow === 'ellipsis', `${T}: focus moved on (${out && out.text}), the line is back at its start with its "…" (scroll ${after.scroll}, ${after.overflow})`);
        // A click on the part of the second artist that shows: nothing moves under the pointer, and it opens the artist
        await page.locator('#main .track .title [data-act="song"]').focus();
        const box = await page.evaluate(() => {
          const by = document.querySelector('#main .track .by'); const a = by.querySelectorAll('[data-artist-credit]')[1];
          const b = by.getBoundingClientRect(); const r = a.getClientRects()[0];
          const right = Math.min(r.right, b.left + by.clientLeft + by.clientWidth - 30);
          return { x: (Math.max(r.left, b.left) + right) / 2, y: (r.top + r.bottom) / 2, shows: right - r.left };
        });
        if (box.shows > 4) {
          await page.mouse.move(box.x, box.y); await page.mouse.down();
          const during = await page.evaluate(() => document.querySelector('#main .track .by').scrollLeft);
          await page.mouse.up();
          const opened = await page.waitForFunction(() => location.pathname === '/artist/lib-32', null, { timeout: 6000 }).then(() => true, () => false);
          check(during === 0 && opened, `${T}: a click on the second artist moves nothing (scroll ${during}) and opens it (${await page.evaluate(() => location.pathname)})`);
        }
        const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
        check(over <= 0, `${T}: nothing wider than the window (${over})`);
        check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
    // Short names (the fixture's "Sefa, Dr. Peacock"): the line isn't cut, so Tab moves nothing
    for (const width of [1280, 768]) {
      const T = `${width} short names`;
      const { page, context, state } = await fx.create(browser, width, {});
      try {
        await fx.go(page, '/tracks');
        await page.waitForSelector('#main .track .by [data-artist-credit]', { state: 'attached', timeout: 8000 });
        await page.locator('#main .track .by [data-artist-credit]').first().focus();
        await page.keyboard.press('Tab');
        const s = await page.evaluate(seen);
        check(s.text === 'Dr. Peacock' && s.scroll === 0 && s.shown === s.width, `${T}: Tab to the second artist moves nothing (${s.text}, scroll ${s.scroll})`);
        check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `row line focus: ${failed} FAIL` : 'row line focus: all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
