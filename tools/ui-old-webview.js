// The Android app on an old Android System WebView says so (TASK-old-webview-1111, option b). Real
// candidate assets on the fixture origin (tools/ui-fixture.js); the user agent and the app's marker
// (window.pmDesktop, as pm-desktop's init script sets it) are set before the page's scripts run:
//   * in the Android app, a WebView below Chrome 111 gets one line above the content, "Update Android
//     System WebView for Odeion to show correctly", with a link to its Google Play page that opens
//     outside the app (target=_blank); it stays on every page and fits a phone;
//   * 111 and newer, a user agent without a Chrome version, the desktop app and a plain browser
//     (whatever its version) get nothing.
//   node tools/ui-old-webview.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'old-webview-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const wv = (chrome) => `Mozilla/5.0 (Linux; Android 11; sdk_gphone_x86 Build/RSR1.201013.001; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/${chrome} Mobile Safari/537.36`;
const PLAY = 'https://play.google.com/store/apps/details?id=com.google.android.webview';
const LINE = 'Update Android System WebView for Odeion to show correctly';
// [what, user agent, the app's platform (null: a browser), the version the note should name (null: no note)]
const CASES = [
  ['Android app, WebView 83 (the API 30 emulator image)', wv('83.0.4103.106'), 'android', 83],
  ['Android app, WebView 87', wv('87.0.4280.101'), 'android', 87],
  ['Android app, WebView 110', wv('110.0.5481.153'), 'android', 110],
  ['Android app, WebView 111', wv('111.0.5563.116'), 'android', null],
  ['Android app, WebView 131', wv('131.0.6778.135'), 'android', null],
  ['Android app, WebView 1000 (not read as 100)', wv('1000.0.0.0'), 'android', null],
  ['Android app, a user agent without a Chrome version', 'Mozilla/5.0 (Linux; Android 11) AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36', 'android', null],
  ['Android app, an empty user agent', '', 'android', null],
  ['desktop app, an old Chrome in its user agent', 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/83.0.4103.106 Safari/537.36', 'linux', null],
  ['a phone browser, Chrome 83', wv('83.0.4103.106').replace('; wv', ''), null, null],
  ['a WebView that is not the app (; wv, no marker), Chrome 83', wv('83.0.4103.106'), null, null],
  ['a desktop browser, Chrome 90', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/90.0.4430.93 Safari/537.36', null, null],
];

(async () => {
  const browser = await chromium.launch();
  for (const [what, ua, platform, want] of CASES) {
    const w = platform === 'android' || /Android/.test(ua) ? 390 : 1280;
    const { page, context, state } = await fx.create(browser, w, { mobile: w === 390 });
    await context.addInitScript(([agent, plat]) => {
      Object.defineProperty(Navigator.prototype, 'userAgent', { get: () => agent, configurable: true });
      if (plat) window.pmDesktop = { platform: plat, version: '0.0.0' };
    }, [ua, platform]);
    await fx.go(page, '/'); await page.waitForSelector('#main'); await sleep(500);
    const read = () => page.evaluate(() => {
      const n = document.querySelector('#webviewnote'); if (!n) return { count: document.querySelectorAll('.webview-note').length };
      const a = n.querySelector('a'); const main = document.querySelector('#main'); const b = n.getBoundingClientRect();
      return { count: document.querySelectorAll('.webview-note').length, text: n.querySelector('span').textContent, version: n.dataset.webview, href: a.getAttribute('href'), target: a.target, label: a.textContent, role: n.getAttribute('role'),
        aboveMain: n.nextElementSibling === main && b.bottom <= main.getBoundingClientRect().top + 1, visible: b.height > 0 && b.width > 0, right: Math.round(b.right), linkH: Math.round(a.getBoundingClientRect().height), wide: document.documentElement.scrollWidth - innerWidth, vw: innerWidth, ua: navigator.userAgent };
    });
    const r = await read();
    if (want == null) check(r.count === 0, `${what}: no note (${r.count})`);
    else {
      check(r.count === 1 && r.text === LINE && r.version === String(want), `${what}: one line, "${r.text}" (read as ${r.version})`);
      check(r.href === PLAY && r.target === '_blank' && r.label === 'Open Google Play', `${what}: a button to the WebView's Google Play page, opened outside the app (${r.label})`);
      check(r.aboveMain && r.visible && r.role === 'status', `${what}: above the content, announced as a status`);
      check(r.wide <= 0 && r.right <= r.vw && r.linkH >= 32, `${what}: fits the phone, the button is ${r.linkH} px tall (page ${r.wide})`);
      if (want === 83) {
        await page.screenshot({ path: path.join(SHOTS, 'old-webview-390.png') });
        // It stays while moving through the app
        for (const p of ['/search', '/library', '/settings']) { await fx.go(page, p); await page.waitForSelector('#main'); await sleep(300); const s = await read(); check(s.count === 1 && s.visible && s.wide <= 0, `${what}: still there on ${p}`); }
      }
    }
    check(!state.errors.length, `${what}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
