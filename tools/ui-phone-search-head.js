// On a phone the search words stay in the search box at the top, so Search doesn't repeat them in a
// "Results for …" heading: the heading is kept for screen readers only, and the source chips and then the
// results start right under the top bar. Tablet and desktop keep the visible heading.
// The Filters button ends the source row (outside its "Search in" group) instead of taking a line of
// its own above the results, and opens the filters under that row; the page scrolls inside #main
// only (screen-reader-only text no longer makes the whole document longer than the screen).
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
    filters: box(document.querySelector('[data-sfilters]')),
    filtersRight: (() => { const f = document.querySelector('[data-sfilters]'); return f ? Math.round(f.getBoundingClientRect().right) : null; })(),
    filtersInGroup: !!document.querySelector('[role="group"] [data-sfilters]'),
    expanded: document.querySelector('[data-sfilters]')?.getAttribute('aria-expanded'),
    panel: (() => { const p = document.querySelector('#sfacets'); return p ? { shown: !p.hidden && p.getClientRects().length > 0, chips: p.querySelectorAll('[data-facet]').length } : null; })(),
    docExtra: document.documentElement.scrollHeight - innerHeight,
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
        // Filters: in the source row, after the group, same top as the chips, 14 px from the right edge
        check(m.filters && !m.filtersInGroup && Math.abs(m.filters.top - m.firstChip.top) <= 1 && m.filters.h >= 40 && m.filtersRight <= w - 13, `${tag}: Filters ends the source row, outside "Search in" (${m.filters && `${m.filters.top}-${m.filters.bottom}`} vs chip ${m.firstChip.top}-${m.firstChip.bottom}, right ${m.filtersRight})`);
        check(m.expanded === 'false' && m.panel && !m.panel.shown, `${tag}: the filters start folded (${m.expanded}, ${JSON.stringify(m.panel)})`);
        check(m.results.top - m.firstChip.bottom <= 20, `${tag}: no line of its own for Filters above the results (${m.results.top - m.firstChip.bottom} px from the chips to the results)`);
        check(m.docExtra <= 1, `${tag}: only the page scrolls, not the whole document (${m.docExtra} px past the screen)`);
        await page.screenshot({ path: path.join(SHOTS, `${tag}-search.png`) });
        // Open the filters, pick one: the same Filters button counts it and keeps the panel open
        if (m.filters) {
        await page.evaluate(() => { document.querySelector('[data-sfilters]').dataset.probe = '1'; });
        await page.locator('[data-sfilters]').tap(); await page.waitForTimeout(150);
        const o = await measure(page);
        check(o.expanded === 'true' && o.panel.shown && o.panel.chips >= 2 && o.overflowX <= 0, `${tag}: Filters opens the filters under the row (${o.expanded}, ${JSON.stringify(o.panel)})`);
        await page.screenshot({ path: path.join(SHOTS, `${tag}-search-filters.png`) });
        await page.locator('#sfacets [data-facet="kind:song"]').tap(); await page.waitForTimeout(200);
        const f = await page.evaluate(() => { const b = document.querySelector('[data-sfilters]'); const p = document.querySelector('#sfacets'); return { same: b && b.dataset.probe === '1', text: b && b.textContent, expanded: b && b.getAttribute('aria-expanded'), shown: !!p && !p.hidden, kinds: [...document.querySelectorAll('#sresults [data-skind]')].map((x) => x.dataset.skind).join() }; });
        check(f.same && f.text === 'Filters · 1' && f.expanded === 'true' && f.shown && f.kinds === 'song', `${tag}: a picked filter is counted on the same button, the panel stays open (${JSON.stringify(f)})`);
        await page.locator('#sfacets [data-facet="kind:song"]').tap(); await page.waitForTimeout(200);
        await page.locator('[data-sfilters]').tap(); await page.waitForTimeout(150);
        const c = await measure(page);
        check(c.expanded === 'false' && !c.panel.shown && c.results.top === m.results.top, `${tag}: Filters folds them again, the results are back where they were (${c.results.top} vs ${m.results.top})`);
        }
        // Another source redraws the page: still no visible heading, the results follow
        await page.locator('#scopechips .chip', { hasText: 'Library' }).tap();
        await page.waitForFunction(() => new URLSearchParams(location.search).get('in') === 'library');
        await page.waitForSelector('#sresults section'); await page.waitForTimeout(200);
        const l = await measure(page);
        check(l.head.w <= 1 && l.head.h <= 1 && l.h2Text === HEADING && l.results.top - l.chips.bottom <= 12, `${tag}: Library chip redraws the same way (${fmt(l)})`);
        check(l.filters && !l.filtersInGroup && Math.abs(l.filters.top - l.firstChip.top) <= 1 && l.docExtra <= 1, `${tag}: Library chip keeps Filters in the row (${l.filters && l.filters.top}, ${l.docExtra} px past the screen)`);
        // Wider than a phone and back (a rotated tablet, a resized window): the filters follow the shell
        if (!android) {
          await page.focus('[data-sfilters]');
          await page.setViewportSize({ width: 768, height: h }); await page.waitForTimeout(300);
          const wide = await page.evaluate(() => ({ shell: document.documentElement.dataset.shell, button: !!document.querySelector('[data-sfilters]'), inline: (() => { const r = document.querySelector('#sresults .facets'); return !!r && r.getClientRects().length > 0 && !r.closest('[hidden]'); })(), focus: document.activeElement.matches('#sresults .facets [data-facet]') }));
          check(wide.shell !== 'phone' && !wide.button && wide.inline, `${tag}: at 768 px the filters show with the results, no Filters button (${JSON.stringify(wide)})`);
          check(wide.focus, `${tag}: a focused Filters hands the focus to the filters, not the page (${JSON.stringify(wide)})`);
          await page.setViewportSize({ width: w, height: h }); await page.waitForTimeout(300);
          const back = await measure(page);
          check(back.filters && back.expanded === 'false' && !back.panel.shown && Math.abs(back.filters.top - back.firstChip.top) <= 1, `${tag}: back at ${w} px Filters ends the row again (${back.filters && back.filters.top})`);
        }
        check(!run.state.errors.length, `${tag}: no page errors (${run.state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // Filters arriving with the results narrows the source row: a source button focused and in view
    // before stays in view (and focused) beside it
    for (const w of [360, 412]) {
      const services = ['tidal', 'qobuz', 'spotify', 'deezer'].map((id, i) => ({ id, name: ['TIDAL', 'Qobuz', 'Spotify', 'Deezer'][i], enabled: true, status: 'signed in', features: ['search'] }));
      const run = await fx.create(browser, w, { mobile: true, searchHits, fixtures: { '/sources': { body: services } } });
      const { page, context } = run;
      let release; const held = new Promise((r) => { release = r; });
      await context.route((url) => url.pathname === '/v1/search', async (route) => { await held; await route.fallback(); });
      try {
        await page.setViewportSize({ width: w, height: 640 });
        await fx.go(page, '/search?q=journey');
        await page.locator('#scopechips [data-scope-in="tidal"]').waitFor(); await page.waitForTimeout(200);
        await page.focus('#scopechips [data-scope-in="tidal"]');
        const seen = await page.evaluate(() => { const b = document.querySelector('[data-scope-in="tidal"]').getBoundingClientRect(); return { left: Math.round(b.left), right: Math.round(b.right), filters: !!document.querySelector('[data-sfilters]') }; });
        release();
        await page.waitForSelector('[data-sfilters]'); await page.waitForTimeout(300);
        const r = await page.evaluate(() => { const b = document.activeElement.getBoundingClientRect(), row = document.querySelector('#scopechips').getBoundingClientRect(), f = document.querySelector('[data-sfilters]').getBoundingClientRect(); return { id: document.activeElement.dataset.scopeIn, left: Math.round(b.left), right: Math.round(b.right), rowLeft: Math.round(row.left), filtersLeft: Math.round(f.left) }; });
        check(!seen.filters && seen.right <= w && r.id === 'tidal' && r.left >= r.rowLeft && r.right <= r.filtersLeft, `focus-${w}: the focused TIDAL stays in view when Filters arrives (${seen.left}-${seen.right} before, ${r.left}-${r.right} after, Filters at ${r.filtersLeft})`);
        check(!run.state.errors.length, `focus-${w}: no page errors (${run.state.errors.join('; ')})`);
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
        const facets = await page.evaluate(() => { const r = document.querySelector('#sresults .facets'); return !!r && r.getClientRects().length > 0; });
        check(!m.filters && facets && m.results.top - m.chips.bottom >= 20, `desktop-${w}: no Filters button, the filters show with the results as before (${facets}, ${m.results.top - m.chips.bottom} px)`);
        await page.screenshot({ path: path.join(SHOTS, `desktop-${w}-search.png`) });
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed; ${failed} failed`);
  if (failed) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
