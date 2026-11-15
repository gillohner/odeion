// Opening a page puts it at its top at once, and Back puts it where it was scrolled at once, as a
// browser's own Back does: the main area's smooth scrolling must not glide there. Every frame
// after Back shows either the page not yet drawn (0) or the place it was left at, nothing between.
// The same for an album opened on Downloads (offline) from a scrolled list.
// Real candidate assets, isolated responses: node tools/ui-back-place.js [repo]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fx = require('./ui-fixture');
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const tracks = Array.from({ length: 80 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
// Every frame's #main scroll from the next navigation on (a click now, or the popstate of Back), for 3 s
const record = (page, on) => page.evaluate((onPop) => {
  window.__frames = []; window.__t0 = null;
  const tick = () => { window.__frames.push(Math.round(document.querySelector('#main').scrollTop)); if (performance.now() - window.__t0 < 3000) requestAnimationFrame(tick); };
  const start = () => { window.__t0 = performance.now(); tick(); };
  if (onPop) addEventListener('popstate', start, { once: true }); else window.__start = start;
}, on === 'back');
const frames = async (page) => { await page.waitForFunction(() => window.__t0 != null && performance.now() - window.__t0 > 3100, null, { timeout: 9000 }).catch(() => {}); return page.evaluate(() => window.__frames || []); };
const open = (page, view) => page.evaluate((v) => document.querySelector(`.nav-item[data-view="${v}"]`).click(), view);
const PLACE = 900;

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, height] of [[1280, 600], [1024, 600], [768, 600], [360, 640]]) {
      const at = `${width}`;
      const { context, page, state } = await fx.create(browser, width, { tracks, mobile: width < 600 });
      try {
        await page.setViewportSize({ width, height });
        await fx.go(page, '/');
        await page.waitForSelector('.nav-item[data-view="tracks"]', { state: 'attached' });
        await open(page, 'tracks');
        await page.waitForFunction(() => /Song 80/.test(document.querySelector('#main').textContent), null, { timeout: 7000 }).catch(() => {});
        const room = await page.evaluate(() => { const m = document.querySelector('#main'); return m.scrollHeight - m.clientHeight; });
        check(room > PLACE, `${at}: Songs is long enough to scroll ${PLACE} px (${room})`);
        await page.evaluate((y) => { const m = document.querySelector('#main'); m.style.scrollBehavior = 'auto'; m.scrollTop = y; m.style.scrollBehavior = ''; }, PLACE);
        // Home from there: it starts at its top, at once
        await record(page, 'click');
        await page.evaluate(() => { window.__start(); document.querySelector('.nav-item[data-view="home"]').click(); });
        const opened = await frames(page);
        // (Songs stays where it was until Home replaces it: tools/ui-open-place.js; once at 0, it stays)
        const glide = [...new Set(opened.slice(1).filter((y, i, a) => y !== 0 && (y !== PLACE || a.slice(0, i).includes(0))))];
        check(opened.length > 10 && opened.at(-1) === 0 && glide.length === 0, `${at}: a page opened from a scrolled one starts at its top at once (${glide.length ? `passed ${glide.slice(0, 6).join(', ')}…` : 'no frame in between'})`);
        // Back to Songs: where it was left, at once
        await record(page, 'back');
        await page.goBack();
        await page.waitForFunction(() => /Song 80/.test(document.querySelector('#main').textContent), null, { timeout: 7000 }).catch(() => {});
        const back = await frames(page);
        const between = [...new Set(back.filter((y) => y !== 0 && y !== PLACE))];
        check(back.length > 10 && back.at(-1) === PLACE, `${at}: Back to Songs is where it was left (${back.at(-1)} px)`);
        check(between.length === 0, `${at}: it gets there at once, without gliding from the top (${between.length ? `passed ${between.slice(0, 6).join(', ')}…` : 'no frame in between'})`);
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // Downloads (offline, the app's own page): an album opened from a scrolled list starts at its
    // top at once (its tile's click reset #main with scrollTo(0, 0), which glided)
    for (const width of [1280, 768]) {
      const at = `${width} Downloads`;
      const offline = { offline: true, device_id: 'fixture-device', synced_at: null, bytes: 1, queued: 0, tracks, albums: [{ id: 7, title: 'Journey', track_ids: tracks.map((t) => t.id), cover: false }], playlists: [], favourites: { track_ids: [] } };
      const { context, page, state } = await fx.create(browser, width, { offline: true, tracks, fixtures: { '/offline': { body: offline } } });
      try {
        await page.setViewportSize({ width, height: 600 });
        await fx.go(page, '/downloads');
        await page.waitForSelector('[data-dlalbum="7"]', { timeout: 8000 });
        await page.waitForFunction(() => /Song 80/.test(document.querySelector('#main').textContent), null, { timeout: 7000 }).catch(() => {});
        const room = await page.evaluate(() => { const m = document.querySelector('#main'); return m.scrollHeight - m.clientHeight; });
        check(room > PLACE, `${at}: the list is long enough to scroll ${PLACE} px (${room})`);
        await page.evaluate((y) => { const m = document.querySelector('#main'); m.style.scrollBehavior = 'auto'; m.scrollTop = y; m.style.scrollBehavior = ''; }, PLACE);
        await record(page, 'click');
        await page.evaluate(() => { window.__start(); document.querySelector('[data-dlalbum="7"]').click(); });
        const opened = await frames(page);
        const glide = [...new Set(opened.slice(1).filter((y) => y !== 0))];
        check(await page.evaluate(() => !!document.querySelector('[data-dlclose]')), `${at}: the album opened`);
        check(opened.length > 10 && glide.length === 0, `${at}: an album opened from a scrolled list starts at its top at once (${glide.length ? `passed ${glide.slice(0, 6).join(', ')}…` : 'no frame in between'})`);
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
