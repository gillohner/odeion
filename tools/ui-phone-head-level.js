// Phones: a page's title sits level with the buttons on its row. The page head was aligned to its
// foot, so a 24 px title beside 40-44 px buttons (a short Songs' Play all, Zones' +, Your library's
// Add music, Home's Edit Home) read 8-10 px lower than them. Now their middles meet (within 1 px).
// A head whose buttons wrap onto a row of their own (Songs with a filter) keeps its two rows, and
// the desktop layout is unchanged (the title's foot on the buttons' foot).
// Phone (360, 412; a browser and the Android app), desktop (1280).
// Real candidate assets, isolated responses: node tools/ui-phone-head-level.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-head-level-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// [route, the first button beside the title]
const pages = [['/', 'Edit Home'], ['/library', 'Add music'], ['/tracks', 'Play all'], ['/zones', 'Group outputs']];

const head = (page) => page.evaluate(() => {
  const h = document.querySelector('#main .page-head'); if (!h) return null;
  const t = h.querySelector(':scope > h2').getBoundingClientRect();
  const b = [...h.querySelectorAll(':scope > .actions > *')].find((e) => e.getClientRects().length);
  const r = b && b.getBoundingClientRect();
  const box = h.getBoundingClientRect();
  return { title: t.top + t.height / 2, button: r ? r.top + r.height / 2 : null, foot: r ? t.bottom - r.bottom : null, titleTop: t.top - box.top, below: r ? r.top - t.bottom : null, name: b ? (b.getAttribute('aria-label') || b.textContent || '').trim() : '' };
});

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, false], [412, false], [360, true], [412, true], [1280, false]]) {
      const phone = width < 600;
      const at = `${android ? 'android' : phone ? 'browser' : 'desktop'}-${width}`;
      const { context, page, state } = await fx.create(browser, width, { mobile: phone });
      try {
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
        for (const [route, want] of pages) {
          await fx.go(page, route);
          await page.waitForFunction((w) => [...document.querySelectorAll('#main .page-head > .actions > *')].some((e) => e.getClientRects().length && (e.getAttribute('aria-label') || e.textContent || '').trim().startsWith(w)), want, { timeout: 5000 }).catch(() => {});
          const h = await head(page);
          check(!!h && h.name.startsWith(want), `${at} ${route}: "${want}" is beside the title (${h && h.name})`);
          if (!h || h.button == null) continue;
          const off = h.title - h.button;
          if (phone) check(Math.abs(off) <= 1, `${at} ${route}: the title's middle is level with ${want}'s (${off.toFixed(1)} px)`);
          // Desktop keeps its own layout: the title's foot on the buttons' foot
          else check(Math.abs(h.foot) <= 1, `${at} ${route}: desktop unchanged, the title's foot on ${want}'s (${h.foot.toFixed(1)} px)`);
          check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at} ${route}: no sideways overflow`);
          if (route === '/tracks' || route === '/zones') await page.screenshot({ path: path.join(SHOTS, `${at}${route.replace(/\//g, '-')}.png`) });
        }
        if (phone) {
          // Songs with more than 12 songs: the filter and Play all wrap onto a row of their own under
          // the title, which stays at the head's top (the rows are not pulled together)
          state.tracks = Array.from({ length: 20 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
          await fx.go(page, '/tracks');
          await page.locator('#main .page-head #filter').waitFor().catch(() => {});
          const w = await head(page);
          check(!!w && w.below >= 0 && Math.abs(w.titleTop) <= 1, `${at} /tracks (20 songs): the buttons keep their own row under the title (${w && w.below.toFixed(1)} px below it, the title ${w && w.titleTop.toFixed(1)} px from the head's top)`);
          check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at} /tracks (20 songs): no sideways overflow`);
          state.tracks = undefined;
        }
        check(state.errors.length === 0, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
