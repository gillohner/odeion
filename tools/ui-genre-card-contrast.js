// The Genres page's genre cards: each card's name and its "N albums · N songs" line must be
// readable on the card's own tint. The tint used to be a dark colour in every theme, so in the
// light theme the dark name and the grey counts sat on dark green/red/blue (about 2:1). Checks
// the text against the tint where the text starts (the gradient's first colour), the middle and
// the end, in the light and the dark theme, for genre names that land on many hues, at 360, 412,
// 768 and 1280 px; also with Settings › Appearance's Light on a dark device and Dark on a light one.
//   node tools/ui-genre-card-contrast.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'genre-card-contrast-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const NAMES = ['Jazz', 'Rock', 'Hip-Hop', 'Classical', 'Electronic', 'Folk', 'Soul', 'Ambient', 'Metal', 'Reggae', 'Blues', 'Pop'];
const genres = { body: NAMES.map((name, i) => ({ id: i + 1, name, albums: i % 3 + 1, tracks: 4 + i })) };

(async () => {
  const browser = await chromium.launch();
  try {
    // [label, the device's scheme, Settings › Appearance's pick (pm.theme) or none]
    for (const [scheme, device, pick] of [['light', 'light', null], ['dark', 'dark', null], ['picked-light', 'dark', 'light'], ['picked-dark', 'light', 'dark']]) {
      for (const width of pick ? [360, 1280] : [360, 412, 768, 1280]) {
        const { page, context, state } = await fx.create(browser, width, { fixtures: { '/library/genres': genres }, ...(width < 600 ? { mobile: true } : {}) });
        try {
          await page.emulateMedia({ colorScheme: device });
          if (pick) await context.addInitScript((t) => localStorage.setItem('pm.theme', t), pick);
          await fx.go(page, '/genres');
          const drawn = await page.waitForFunction((n) => document.querySelectorAll('.card.genre').length === n, NAMES.length, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${scheme} ${width}: the Genres page draws ${NAMES.length} genre cards`);
          if (!drawn) continue;
          if (pick) check(await page.evaluate(() => document.documentElement.dataset.theme) === pick, `${scheme} ${width}: the page follows the picked theme`);
          const cards = await page.evaluate(() => {
            const rgb = (s) => (s.match(/[\d.]+/g) || []).slice(0, 4).map(Number);
            const lum = ([r, g, b]) => { const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
            const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
            const mix = (a, b) => a.map((v, i) => (v + b[i]) / 2);
            return [...document.querySelectorAll('.card.genre')].map((c) => {
              const bg = getComputedStyle(c).backgroundImage;
              const stops = (bg.match(/rgba?\([^)]*\)/g) || []).map(rgb);
              const name = c.querySelector('h3'); const sub = c.querySelector('.muted');
              const fg = [name, sub].map((e) => rgb(getComputedStyle(e).color));
              const at = stops.length >= 2 ? [stops[0], mix(stops[0], stops[stops.length - 1]), stops[stops.length - 1]] : [];
              return { name: name.textContent, sub: sub.textContent, stops: stops.length, nameMin: Math.min(...at.map((s) => ratio(fg[0], s))), subMin: Math.min(...at.map((s) => ratio(fg[1], s))), left: getComputedStyle(c).textAlign, cursor: getComputedStyle(c).cursor };
            });
          });
          for (const c of cards) {
            check(c.stops >= 2, `${scheme} ${width} ${c.name}: the card has its tint (${c.stops} colours)`);
            check(c.nameMin >= 4.5, `${scheme} ${width} ${c.name}: the name reads on the tint (${c.nameMin.toFixed(2)}:1, needs 4.5)`);
            check(c.subMin >= 4.5, `${scheme} ${width} ${c.name}: "${c.sub}" reads on the tint (${c.subMin.toFixed(2)}:1, needs 4.5)`);
            check(c.left === 'left' && c.cursor === 'pointer', `${scheme} ${width} ${c.name}: still left-aligned with a pointer (${c.left}, ${c.cursor})`);
          }
          const names = new Set(cards.map((c) => c.name)); check(names.size === NAMES.length, `${scheme} ${width}: every genre drawn once`);
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${scheme} ${width}: nothing wider than the screen (${o})`);
          await page.locator('.card.genre').first().click();
          const opened = await page.waitForFunction(() => location.pathname.startsWith('/genre/') || (document.querySelector('.main .eyebrow') || {}).textContent === 'Genre', null, { timeout: 4000 }).then(() => true, () => false);
          check(opened, `${scheme} ${width}: a tap on a genre card still opens the genre`);
          await fx.go(page, '/genres'); await page.waitForSelector('.card.genre');
          await page.screenshot({ path: path.join(SHOTS, `genres-${scheme}-${width}.png`), fullPage: true });
          check(!state.errors.length, `${scheme} ${width}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})();
