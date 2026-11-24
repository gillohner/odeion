// An address that is no page says so. /settings/you, /settings/zones (Zones and outputs is
// /settings/playback), a mistyped /library/<tab> or a song/album/artist address with an id the app
// can't read opened Home (or the bare page) without a word, and the address bar kept the dead path.
// Now the nearest page opens (Settings for /settings/..., Your library for /library/..., else Home;
// a listener's Albums),
// its address replaces the dead one (a reload or a copied link lands there), and a toast says
// "No page at <path>". Real pages (/, /app, /settings/about, /library/import) open as before, no toast.
// Phone (360, 412; a browser and the Android app) and desktop (1280).
// Real candidate assets, isolated responses: node tools/ui-unknown-address.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'unknown-address-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// An id the app can't read, one unbroken piece longer than a phone's toast (it wraps inside it)
const LONG = '/album/' + 'notarealalbumidnotarealalbumidnotarealalbumidnotarealalbum';
// [address, the view that opens, its address afterwards, the toast's words or null for none]
const cases = [
  ['/settings/you', 'settings', '/settings', 'No page at /settings/you'],
  ['/settings/zones', 'settings', '/settings', 'No page at /settings/zones'],
  ['/library/foo', 'library', '/library', 'No page at /library/foo'],
  ['/album/not-an-id', 'home', '/', 'No page at /album/not-an-id'],
  ['/nope', 'home', '/', 'No page at /nope'],
  ['/Settings/about', 'settings', '/settings', 'No page at /Settings/about'],
  [LONG, 'home', '/', 'No page at /album/notareal'],
  ['/x#settings/about', 'settings', '/settings/about', null],
  ['/', 'home', '/', null],
  ['/app', 'home', '/', null],
  ['/settings/about', 'settings', '/settings/about', null],
  ['/library/import', 'library', '/library/import', null],
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
      const phone = width < 600;
      const at = `${android ? 'android' : phone ? 'browser' : 'desktop'}-${width}`;
      const { context, page, state } = await fx.create(browser, width, { mobile: phone });
      try {
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
        for (const [addr, view, after, said] of cases) {
          await fx.go(page, addr);
          await page.waitForFunction(() => history.state && history.state.v && document.querySelector('#main > *'), null, { timeout: 8000 }).catch(() => {});
          await page.waitForTimeout(400);
          const got = await page.evaluate(() => { const t = document.querySelector('#toast'); const r = t.getBoundingClientRect(); return { v: history.state && history.state.v, path: location.pathname, toast: t.classList.contains('show') ? t.textContent.trim() : '', inside: r.left >= 0 && r.right <= innerWidth && t.scrollWidth <= t.clientWidth }; });
          check(got.v === view, `${at} ${addr}: opens ${view} (${got.v})`);
          check(got.path === after, `${at} ${addr}: the address is ${after} (${got.path})`);
          if (said) check(got.toast.startsWith(said) && got.inside, `${at} ${addr}: says "${said}", inside the screen (${got.toast || 'nothing'}${got.inside ? '' : ', cut off'})`);
          else check(!/No page at/.test(got.toast), `${at} ${addr}: no "No page" word (${got.toast || 'nothing'})`);
          check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at} ${addr}: no sideways overflow`);
          if (said && (addr === '/settings/you' || addr === LONG)) await page.screenshot({ path: path.join(SHOTS, `${at}${addr === LONG ? '-album-long-id' : addr.replace(/\//g, '-')}.png`) });
        }
        // The dead address leaves no entry behind: Back goes to the page before it, not to it again
        await fx.go(page, '/');
        await page.waitForFunction(() => history.state && history.state.v === 'home', null, { timeout: 8000 }).catch(() => {});
        await fx.go(page, '/settings/you');
        await page.waitForFunction(() => history.state && history.state.v === 'settings', null, { timeout: 8000 }).catch(() => {});
        await page.goBack();
        await page.waitForFunction(() => history.state && history.state.v && document.querySelector('#main > *'), null, { timeout: 8000 }).catch(() => {});
        const back = await page.evaluate(() => ({ path: location.pathname, v: history.state && history.state.v }));
        check(back.path === '/' && back.v === 'home', `${at} Back after /settings/you: the page before it (${back.path}, ${back.v})`);
        // A reload of the page it opened is that page, said once
        await fx.go(page, '/settings/you');
        await page.waitForFunction(() => history.state && history.state.v === 'settings', null, { timeout: 8000 }).catch(() => {});
        await page.evaluate(() => { document.querySelector('#toast').classList.remove('show'); });
        await page.reload();
        await page.waitForFunction(() => history.state && history.state.v === 'settings' && document.querySelector('#main > *'), null, { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(400);
        const re = await page.evaluate(() => ({ path: location.pathname, toast: document.querySelector('#toast.show') ? document.querySelector('#toast').textContent.trim() : '' }));
        check(re.path === '/settings' && !/No page at/.test(re.toast), `${at} reload after /settings/you: Settings, said once (${re.path}, ${re.toast || 'nothing'})`);
        check(state.errors.length === 0, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // A listener (no Home): a dead address opens their first page, Albums, and says so
    {
      const { context, page, state } = await fx.create(browser, 360, { mobile: true, role: 'listener' });
      try {
        await fx.go(page, '/nope');
        await page.waitForFunction(() => history.state && history.state.v && document.querySelector('#main > *'), null, { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(400);
        const got = await page.evaluate(() => ({ v: history.state && history.state.v, toast: document.querySelector('#toast.show') ? document.querySelector('#toast').textContent.trim() : '' }));
        check(got.v === 'albums' && got.toast.startsWith('No page at /nope'), `listener-360 /nope: Albums, says "No page at /nope" (${got.v}, ${got.toast || 'nothing'})`);
        check(state.errors.length === 0, `listener-360: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
