// Sources on the phone app without a node (phone mode, /v1/info mode "phone"): there is no Your
// library tab. Its two buttons, "Open Your library › Node library" and "Open Settings › Play from",
// led to pages the phone doesn't have (the first fell back to Search, the second to the phone's
// one Settings page, which has no Play from). At 360, 412 and 1280 px:
//   * phone mode: the tabs are the phone's services only; /sources/library is plain /sources; at
//     1280 the page opens on the first service (TIDAL), not on an empty panel; no page errors;
//   * a node: Your library is still the first tab and, at 1280, the one it opens on, with both
//     buttons; /sources/library opens it.
//   node tools/ui-phone-sources-library.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-sources-library-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// The phone's own backend: its capabilities as crates/pm-offline/src/phone/mod.rs lists them
const PHONE_INFO = { mode: 'phone', capabilities: ['search', 'items', 'sources', 'bookmarks', 'listens', 'songs', 'add-songs', 'homeserver-files'], name: 'This phone', api: 'v1', version: '0.2.4', offline: false,
  library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0 }, zones: 0, outputs: 0, scanning: false };
// The phone's rows and TIDAL sign-in answer as crates/pm-offline/src/phone/{tidal,qobuz}.rs send them
const SOURCES = [
  { id: 'tidal', name: 'TIDAL', description: 'Your TIDAL plan: search, albums and full tracks on this phone', status: 'ready', enabled: true, features: ['search', 'play', 'artists'], stage: 'unofficial', downloadable: false, sign_in: 'Sign in with TIDAL · full tracks', healthy: true },
  { id: 'qobuz', name: 'Qobuz', description: 'Unofficial: search, albums and full tracks in FLAC up to 24-bit/192 kHz with your Qobuz plan', status: 'needs sign-in', enabled: true, features: ['search', 'play', 'artists'], stage: 'unofficial', downloadable: false, sign_in: 'Not signed in', healthy: false },
];
const PLAY_ORDER = { order: ['tidal', 'qobuz'], services: [{ id: 'tidal', name: 'TIDAL', connected: true }, { id: 'qobuz', name: 'Qobuz', connected: false }] };
const TIDAL_SIGNIN = { source: 'tidal', line: 'Sign in with TIDAL · full tracks', details: { unofficial: null },
  ways: [{ id: 'browser', label: 'Sign in with TIDAL', note: '', action: 'browser', recommended: true, available: true, reason: null }, { id: 'device', label: 'Sign in with a code', note: '', action: 'code', recommended: false, available: true, reason: null }],
  in_use: { way: 'browser', label: 'Sign in with TIDAL', user: 1, expires_at: 0, gives: 'full tracks, lossless' } };
const shared = { '/sources': { body: SOURCES }, '/play-order': { body: PLAY_ORDER }, '/sources/tidal/signin': { body: TIDAL_SIGNIN } };
const PHONE = { ...shared, '/info': { body: PHONE_INFO }, '/me': { body: { role: 'owner', zones: [] } } };

const read = (p) => p.evaluate(() => ({
  phone: document.body.classList.contains('phone'),
  tabs: [...document.querySelectorAll('#main [data-srctab]')].map((t) => t.dataset.srctab),
  current: (document.querySelector('#main [data-srctab][aria-current="page"]') || {}).dataset?.srctab || null,
  title: (document.querySelector('#main .set-title') || {}).textContent || null,
  libButtons: [...document.querySelectorAll('#main [data-view-go="library"], #main [data-goto-settings="playfrom"]')].length,
  path: location.pathname,
}));
const settle = (p) => p.waitForFunction(() => document.querySelector('#main .src-nav'), null, { timeout: 6000 }).catch(() => {});

(async () => {
  const browser = await chromium.launch();
  for (const w of [360, 412, 1280]) {
    const at = `${w}px`;
    // ---- Phone mode ----
    const run = await fx.create(browser, w, { mobile: w < 600, fixtures: PHONE }); const p = run.page;
    await fx.go(p, '/sources'); await settle(p); await p.waitForTimeout(300);
    let r = await read(p);
    check(r.phone, `${at}: phone mode: body.phone is set`);
    check(r.tabs.length === 2 && !r.tabs.includes('library'), `${at}: phone mode: the tabs are the phone's services only (${r.tabs.join(', ')})`);
    check(r.libButtons === 0, `${at}: phone mode: no Node library or Play from button`);
    if (w >= 600) check(r.current === 'tidal' && r.title === 'TIDAL', `${at}: phone mode: the page opens on the first service (${r.current}, ${r.title})`);
    await p.screenshot({ path: path.join(SHOTS, `phone-${w}.png`) });
    // An old link to the Your library tab: plain /sources, no Your library panel
    await fx.go(p, '/sources/library'); await settle(p); await p.waitForTimeout(300);
    r = await read(p);
    check(r.path === '/sources' && !r.tabs.includes('library') && r.title !== 'Your library' && r.libButtons === 0, `${at}: phone mode: /sources/library opens plain /sources (${r.path}, ${r.title})`);
    check(!run.state.errors.length, `${at}: phone mode: no page errors (${run.state.errors.join(' | ')})`);
    await run.context.close();

    // ---- A node: unchanged ----
    const node = await fx.create(browser, w, { mobile: w < 600, fixtures: shared }); const n = node.page;
    await fx.go(n, '/sources'); await settle(n); await n.waitForTimeout(300);
    r = await read(n);
    check(!r.phone && r.tabs[0] === 'library', `${at}: node: Your library is the first tab (${r.tabs.join(', ')})`);
    if (w >= 600) check(r.current === 'library' && r.libButtons === 2, `${at}: node: the page opens on Your library with both buttons (${r.current}, ${r.libButtons})`);
    else {
      await n.click('[data-srctab="library"]'); await n.waitForFunction(() => document.querySelector('#main .set-title'), null, { timeout: 5000 }).catch(() => {});
      r = await read(n);
      check(r.title === 'Your library' && r.libButtons === 2, `${at}: node: Your library opens with both buttons (${r.title}, ${r.libButtons})`);
    }
    await n.screenshot({ path: path.join(SHOTS, `node-${w}.png`) });
    // A link to the Your library tab still opens it on a node
    await fx.go(n, '/sources/library'); await n.waitForFunction(() => document.querySelector('#main .set-title'), null, { timeout: 5000 }).catch(() => {});
    r = await read(n);
    check(r.path === '/sources/library' && r.title === 'Your library' && r.libButtons === 2, `${at}: node: /sources/library opens Your library (${r.path}, ${r.title})`);
    check(!node.state.errors.length, `${at}: node: no page errors (${node.state.errors.join(' | ')})`);
    await node.context.close();
  }
  await browser.close();
  if (failed) { console.error(`${failed} check(s) failed`); process.exit(1); }
  console.log('phone-mode Sources lists only the phone\'s services');
})();
