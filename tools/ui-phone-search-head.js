// On a phone the search words stay in the search box at the top, so Search doesn't repeat them in a
// "Results for …" heading: the heading is kept for screen readers only, and the source chips and then the
// results start right under the top bar. Tablet and desktop keep the visible heading.
// Real candidate assets, isolated responses: node tools/ui-phone-search-head.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-search-head-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const searchHits = [
  { kind: 'song', id: 'lib-1', title: 'Journey Through Time', subtitle: 'Sefa', keys: ['track:1'], track_id: 1 },
  { kind: 'song', id: 'lib-2', title: 'Journey Home', subtitle: 'Sefa', keys: ['track:2'], track_id: 2 },
  { kind: 'album', id: 'lib-a7', title: 'Journey', subtitle: 'Sefa', keys: ['album:7'] },
  { kind: 'artist', id: 'lib-r11', title: 'Journey Band', subtitle: '', keys: ['artist:11'] },
];
const HEADING = 'Results for “journey”';

// Where the search box, the hidden heading, the chips and the results are
const measure = (page) => page.evaluate(() => {
  const box = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { top: Math.round(b.top), bottom: Math.round(b.bottom), w: Math.round(b.width), h: Math.round(b.height) }; };
  const head = document.querySelector('#main .page-head');
  const h2 = head && head.querySelector('h2');
  const search = document.querySelector('#search');
  return {
    main: box(document.querySelector('#main')),
    head: box(head), h2Text: h2 && h2.textContent,
    search: box(search), words: search && search.value,
    chips: box(document.querySelector('#scopechips')),
    results: box(document.querySelector('#sresults')),
    firstChip: box(document.querySelector('#scopechips .chip')),
    overflowX: document.documentElement.scrollWidth - innerWidth,
  };
});
const fmt = (m) => `box ${m.search && m.search.bottom}, heading ${m.head && `${m.head.w}x${m.head.h}`}, chips ${m.chips && m.chips.top}-${m.chips && m.chips.bottom}, results ${m.results && m.results.top}`;

(async () => {
  const browser = await chromium.launch();
  try {
    for (const android of [false, true]) for (const [w, h] of [[360, 640], [412, 732]]) {
      const tag = `${android ? 'android' : 'browser'}-${w}`;
      const run = await fx.create(browser, w, { mobile: true, searchHits });
      if (android) await run.context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
      const { page, context } = run;
      try {
        await page.setViewportSize({ width: w, height: h });
        await fx.go(page, '/search?q=journey');
        await page.waitForSelector('#sresults section'); await page.waitForTimeout(300);
        const shell = await page.evaluate(() => document.documentElement.dataset.shell);
        check(shell === 'phone', `${tag}: phone shell (${shell})`);
        const m = await measure(page);
        check(m.words === 'journey' && m.search.bottom <= m.chips.top, `${tag}: the words stay in the search box above the chips ("${m.words}", ${fmt(m)})`);
        check(m.head && m.head.w <= 1 && m.head.h <= 1, `${tag}: no visible "Results for" heading (${fmt(m)})`);
        const named = await page.getByRole('heading', { name: HEADING }).count();
        check(named === 1 && m.h2Text === HEADING, `${tag}: screen readers still get the heading "${HEADING}" (${named})`);
        check(m.chips.top - m.main.top <= 20, `${tag}: the source chips are the first thing on the page (${m.chips.top - m.main.top} px below the top bar)`);
        check(m.results.top - m.chips.bottom <= 12, `${tag}: the results start right under the chips (${m.results.top - m.chips.bottom} px gap)`);
        check(m.firstChip.h >= 40, `${tag}: the chips keep their size (${m.firstChip.h} px)`);
        // (the hidden heading sits at the top: it doesn't lengthen the page as the results' sr-only live region does)
        check(m.overflowX <= 0 && m.head.bottom <= m.chips.bottom, `${tag}: nothing runs off the screen, the hidden heading stays at the top (x ${m.overflowX}, heading at ${m.head.top})`);
        await page.screenshot({ path: path.join(SHOTS, `${tag}-search.png`) });
        // Another source redraws the page: still no visible heading, the results follow
        await page.locator('#scopechips .chip', { hasText: 'Library' }).tap();
        await page.waitForFunction(() => new URLSearchParams(location.search).get('in') === 'library');
        await page.waitForSelector('#sresults section'); await page.waitForTimeout(200);
        const l = await measure(page);
        check(l.head.w <= 1 && l.head.h <= 1 && l.h2Text === HEADING && l.results.top - l.chips.bottom <= 12, `${tag}: Library chip redraws the same way (${fmt(l)})`);
        check(!run.state.errors.length, `${tag}: no page errors (${run.state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // Tablet and desktop keep the visible heading
    for (const w of [768, 1280]) {
      const { page, context } = await fx.create(browser, w, { searchHits });
      try {
        await fx.go(page, '/search?q=journey');
        await page.waitForSelector('#sresults section'); await page.waitForTimeout(300);
        const m = await measure(page);
        check(m.head.h >= 20 && m.h2Text === HEADING && m.chips.top >= m.head.bottom, `desktop-${w}: "${HEADING}" shows above the chips (${fmt(m)})`);
        await page.screenshot({ path: path.join(SHOTS, `desktop-${w}-search.png`) });
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed; ${failed} failed`);
  if (failed) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
