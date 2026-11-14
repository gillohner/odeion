// Opening another page from a scrolled one: the page on screen stays where it is until the new one
// (or its skeleton) replaces it, and the new one starts at its top. No frame shows the old page at
// its own top first (it was put there at once, before the new page drew: up to 120 ms of the old
// page's top). Also: a page opened and left before it drew comes back (Back) at its top, not at the
// place of the page before it.
// Real candidate assets, isolated responses, the pages' reads delayed: node tools/ui-open-place.js [repo]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fx = require('./ui-fixture');
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const tracks = Array.from({ length: 80 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
const PLACE = 900;
const SLOW = 400; // ms before a page's reads answer
// Every frame for 1.5 s from the click: #main's scroll, and whether what shows is still the old page
const record = (page) => page.evaluate(() => {
  const m = document.querySelector('#main');
  [...m.children].forEach((c) => { c.dataset.oldpage = ''; });
  window.__f = []; window.__t0 = performance.now();
  const tick = () => { window.__f.push([Math.round(m.scrollTop), !!m.querySelector(':scope > [data-oldpage]')]); if (performance.now() - window.__t0 < 1500) requestAnimationFrame(tick); };
  tick();
});
const frames = async (page) => { await page.waitForFunction(() => performance.now() - window.__t0 > 1600, null, { timeout: 6000 }).catch(() => {}); return page.evaluate(() => window.__f); };
const toPlace = (page) => page.evaluate((y) => { const m = document.querySelector('#main'); m.style.scrollBehavior = 'auto'; m.scrollTop = y; m.style.scrollBehavior = ''; }, PLACE);
const songs = async (page) => {
  await page.evaluate(() => document.querySelector('.nav-item[data-view="tracks"]').click());
  await page.waitForFunction(() => /Song 80/.test(document.querySelector('#main').textContent), null, { timeout: 7000 }).catch(() => {});
  await page.waitForTimeout(SLOW + 200);
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 412]) {
      const { context, page, state } = await fx.create(browser, width, { tracks, mobile: width < 600, delay: (v, m) => (m === 'GET' && /^\/(playlists|library\/albums)$/.test(v) ? SLOW : 0) });
      try {
        await page.setViewportSize({ width, height: width < 600 ? 800 : 600 });
        await fx.go(page, '/');
        await page.waitForSelector('.nav-item[data-view="tracks"]', { state: 'attached' });
        // (Collections waits for its read; Albums draws its own placeholder at once)
        for (const [open, name, slow] of [['.nav-item[data-view="playlists"]', 'Collections', true], ['.nav-item[data-view="albums"]', 'Albums', false]]) {
          const at = `${width} Songs → ${name}`;
          await songs(page);
          await toPlace(page);
          const room = await page.evaluate(() => document.querySelector('#main').scrollTop);
          check(room === PLACE, `${at}: Songs is scrolled to ${PLACE} px (${room})`);
          await record(page);
          await page.evaluate((sel) => document.querySelector(sel).click(), open);
          const f = await frames(page);
          const old = f.filter(([, o]) => o); const fresh = f.filter(([, o]) => !o);
          if (slow) check(old.length >= 2, `${at}: the new page is slow, so Songs stays on screen a moment (${old.length} frames)`);
          const oldMoved = [...new Set(old.map(([y]) => y).filter((y) => y !== PLACE))];
          check(!oldMoved.length, `${at}: until it is replaced, Songs stays where it was (${oldMoved.length ? `it showed at ${oldMoved.slice(0, 6).join(', ')}` : `${PLACE} px`})`);
          const newMoved = [...new Set(fresh.map(([y]) => y).filter((y) => y !== 0))];
          check(fresh.length > 10 && !newMoved.length, `${at}: ${name} (or its skeleton) starts at its top (${newMoved.length ? `it showed at ${newMoved.slice(0, 6).join(', ')}` : '0 px'})`);
        }
        // Songs (scrolled) → Collections, and Artists before Collections drew: Back to Collections is at its top
        {
          const at = `${width} Songs → Collections → Artists, Back`;
          await songs(page);
          await toPlace(page);
          await page.evaluate(() => { document.querySelector('.nav-item[data-view="playlists"]').click(); setTimeout(() => document.querySelector('.nav-item[data-view="artists"]').click(), 30); });
          await page.waitForTimeout(SLOW + 400);
          const saved = await page.evaluate(() => { const s = history.state; history.back(); return s && s.v; });
          await page.waitForFunction(() => location.pathname.startsWith('/playlists'), null, { timeout: 4000 }).catch(() => {});
          await page.waitForTimeout(SLOW + 400);
          const y = await page.evaluate(() => [location.pathname, document.querySelector('#main').scrollTop]);
          check(saved === 'artists' && y[0].startsWith('/playlists') && y[1] === 0, `${at}: Collections is at its top, not at Songs' place (${y[0]} at ${y[1]} px)`);
        }
        check(!state.errors.length, `${width}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
