// The install button and its help (crates/pm-api/static/pwa.js) in jsdom, without a node.
// The browser's install events are fakes here: this covers the page's own behaviour (only on a
// click, the deferred prompt used once and dropped, nothing for an installed app or inside the
// desktop app, the right steps per browser), NOT whether a browser will install the app. The
// real-browser check is tools/ui-pwa-browser.js.
//
//   npm install jsdom@24     (anywhere; then run from that directory)
//   node /path/to/tools/ui-pwa.js [repo root]
const { JSDOM, VirtualConsole } = require(require.resolve('jsdom', { paths: [process.cwd(), __dirname] }));
const fs = require('fs');
const ROOT = process.argv[2] || require('path').resolve(__dirname, '..');
const STATIC = ROOT + '/crates/pm-api/static/';
const html = fs.readFileSync(STATIC + 'index.html', 'utf8');
const pwa = fs.readFileSync(STATIC + 'pwa.js', 'utf8');
const UA = {
  chrome: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  edge: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0',
  chromeAndroid: 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36',
  samsung: 'Mozilla/5.0 (Linux; Android 14; SM-S911B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/26.0 Chrome/122.0.0.0 Mobile Safari/537.36',
  iphone: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1',
  ipadDesktop: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15',
  safariMac: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15',
  firefoxLinux: 'Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0',
  firefoxWindows: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:143.0) Gecko/20100101 Firefox/143.0',
  firefoxAndroid: 'Mozilla/5.0 (Android 14; Mobile; rv:132.0) Gecko/132.0 Firefox/132.0',
};
const failures = [];
let checks = 0;
const ok = (cond, what) => { checks++; if (!cond) failures.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// One page: index.html with pwa.js only (app.js is not needed for the install button).
function page({ url = 'https://music.example.org/', ua = UA.chrome, platform = 'Linux x86_64', touch = 0, standalone = false, iosStandalone, desktop = false, storage = {} } = {}) {
  const errors = [];
  const vc = new VirtualConsole();
  vc.on('jsdomError', (e) => errors.push('jsdomError: ' + (e.message || e)));
  vc.on('error', (...a) => errors.push('console.error: ' + a.join(' ')));
  const dom = new JSDOM(html.replace(/<script src="\/(app|pwa)\.js"( defer)?><\/script>/g, ''), { url, runScripts: 'outside-only', pretendToBeVisual: true, virtualConsole: vc });
  const w = dom.window;
  for (const [k, v] of Object.entries(storage)) w.localStorage.setItem(k, v);
  Object.defineProperty(w.navigator, 'userAgent', { value: ua, configurable: true });
  Object.defineProperty(w.navigator, 'platform', { value: platform, configurable: true });
  Object.defineProperty(w.navigator, 'maxTouchPoints', { value: touch, configurable: true });
  if (iosStandalone !== undefined) Object.defineProperty(w.navigator, 'standalone', { value: iosStandalone, configurable: true });
  const secure = /^https:|^http:\/\/(localhost|127\.0\.0\.1)[:/]/.test(url);
  Object.defineProperty(w, 'isSecureContext', { value: secure, configurable: true });
  const mq = new Map();
  w.matchMedia = (q) => { if (!mq.has(q)) { const l = new Set(); mq.set(q, { media: q, matches: standalone && /display-mode: (standalone|fullscreen|minimal-ui|window-controls-overlay)/.test(q), addEventListener: (_, f) => l.add(f), removeEventListener: (_, f) => l.delete(f), _l: l }); } return mq.get(q); };
  const registered = [];
  if (secure) {
    const sw = new w.EventTarget();
    sw.register = async (src, opts) => { registered.push([src, opts]); return { update: async () => {}, addEventListener() {} }; };
    Object.defineProperty(w.navigator, 'serviceWorker', { value: sw, configurable: true });
  }
  let reloads = 0;
  try { Object.defineProperty(w.location, 'reload', { value: () => { reloads++; }, configurable: true }); } catch { /* jsdom: location is unforgeable */ }
  if (desktop) w.pmDesktop = { version: '0.2.2', platform: 'linux' };
  if (!w.HTMLDialogElement.prototype.showModal) {
    w.HTMLDialogElement.prototype.showModal = function () { this.open = true; };
    w.HTMLDialogElement.prototype.close = function () { this.open = false; this.dispatchEvent(new w.Event('close')); };
  }
  w.addEventListener('error', (e) => errors.push('window.error: ' + e.message));
  w.eval(pwa);
  w.document.dispatchEvent(new w.Event('DOMContentLoaded'));
  const $ = (s) => w.document.querySelector(s);
  const btn = () => $('#installbtn');
  const shown = () => !!btn() && !btn().hidden;
  const help = () => $('#installhelp');
  const helpText = () => (help() ? help().textContent.replace(/\s+/g, ' ') : '');
  const setMode = (on) => { for (const m of mq.values()) { if (/display-mode: standalone/.test(m.media)) { m.matches = on; for (const f of m._l) f({ matches: on, media: m.media }); } } };
  // A fake BeforeInstallPromptEvent: records preventDefault and prompt() calls
  const bip = (outcome) => {
    const e = new w.Event('beforeinstallprompt', { cancelable: true });
    e.prompts = 0;
    e.prompt = async () => { e.prompts++; return { outcome }; };
    e.userChoice = Promise.resolve({ outcome, platform: 'web' });
    w.dispatchEvent(e);
    return e;
  };
  return { w, $, btn, shown, help, helpText, bip, setMode, registered, errors, reloads: () => reloads };
}

(async () => {
  // A. Chrome on a secure node, before the browser offers its prompt: the button opens the steps
  {
    const p = page();
    ok(p.shown(), 'A: the Install app button shows on a secure, not-installed page');
    ok(p.btn() && p.btn().getAttribute('aria-label') === 'Install app', 'A: the button is labelled "Install app"');
    ok(p.btn() && p.btn().nextElementSibling === p.$('#me'), 'A: the button sits in the top bar before the account button');
    ok(!p.help() || !p.help().open, 'A: no help opens by itself');
    p.btn().click(); await sleep(10);
    ok(p.help() && p.help().open, 'A: a click opens the install help');
    ok(/Install/.test(p.helpText()) && /address bar|menu/.test(p.helpText()), 'A: Chrome steps name the address bar or menu: ' + p.helpText().slice(0, 200));
    ok(/doesn.t play offline|does not play offline|needs your node/i.test(p.helpText()), 'A: the help says music does not play offline');
    ok(p.registered.length === 1 && p.registered[0][0] === '/sw.js' && p.registered[0][1].scope === '/' && p.registered[0][1].updateViaCache === 'none', 'A: the worker is registered once at /sw.js for /: ' + JSON.stringify(p.registered));
    ok(!p.errors.length, 'A: no errors ' + p.errors.join('; '));
  }
  // B. The browser's prompt: kept, shown only on a click, used once, dropped after either answer
  {
    const p = page();
    const e1 = p.bip('dismissed');
    ok(e1.defaultPrevented, 'B: the browser\'s own mini-infobar is held back (preventDefault)');
    await sleep(50);
    ok(e1.prompts === 0, 'B: nothing prompts without a click');
    p.btn().click(); await sleep(10);
    ok(e1.prompts === 1, 'B: a click shows the browser prompt');
    ok(!p.help() || !p.help().open, 'B: the prompt, not the help, answers the click');
    ok(p.shown(), 'B: after "dismissed" the button stays (the app is not installed)');
    p.btn().click(); await sleep(10);
    ok(e1.prompts === 1, 'B: a used prompt is never shown again');
    ok(p.help() && p.help().open, 'B: the next click shows the manual steps instead');
    p.help().close();
    const e2 = p.bip('accepted');
    p.btn().click(); await sleep(10);
    ok(e2.prompts === 1, 'B: a fresh prompt from the browser is used on the next click');
    ok(!p.shown(), 'B: accepted: the button goes');
    ok(!p.errors.length, 'B: no errors ' + p.errors.join('; '));
  }
  // C. Installed from the browser's own menu: appinstalled drops the prompt and the button
  {
    const p = page();
    const e = p.bip('accepted');
    p.w.dispatchEvent(new p.w.Event('appinstalled'));
    ok(!p.shown(), 'C: appinstalled hides the button');
    ok(e.prompts === 0, 'C: and never prompts');
  }
  // D. Already the installed app (standalone window, iOS home screen): no install button
  {
    ok(!page({ standalone: true }).shown(), 'D: no button in the standalone app window');
    ok(!page({ ua: UA.iphone, platform: 'iPhone', touch: 5, iosStandalone: true }).shown(), 'D: no button opened from the iOS home screen');
    const p = page();
    p.setMode(true);
    ok(!p.shown(), 'D: the button goes when the page moves into the app window');
  }
  // E. Inside the desktop or Android app: no install UI and no worker
  {
    const p = page({ desktop: true });
    ok(!p.btn(), 'E: no button inside the desktop app');
    ok(p.registered.length === 0, 'E: no service worker inside the desktop app');
    const t = page({ url: 'http://tauri.localhost/index.html' });
    ok(!t.btn() && t.registered.length === 0, 'E: nothing on the app\'s own pages');
  }
  // F–H. The right manual steps per browser
  const steps = async (opts) => { const p = page(opts); p.btn().click(); await sleep(10); return p.helpText(); };
  {
    const t = await steps({ ua: UA.iphone, platform: 'iPhone', touch: 5 });
    ok(/Share/.test(t) && /Add to Home Screen/.test(t), 'F: iPhone: Share, Add to Home Screen: ' + t.slice(0, 200));
    const ip = await steps({ ua: UA.ipadDesktop, platform: 'MacIntel', touch: 5 });
    ok(/Add to Home Screen/.test(ip), 'F: iPad (desktop-class Safari) gets the iOS steps');
    const mac = await steps({ ua: UA.safariMac, platform: 'MacIntel', touch: 0 });
    ok(/Add to Dock/.test(mac) && /File/.test(mac), 'G: Safari on a Mac: File, Add to Dock: ' + mac.slice(0, 200));
    const fx = await steps({ ua: UA.firefoxLinux });
    ok(/Firefox/.test(fx) && /can.t install|cannot install/i.test(fx) && /Chrome|Edge/.test(fx), 'H: Firefox on Linux cannot install; names a browser that can: ' + fx.slice(0, 200));
    const fxw = await steps({ ua: UA.firefoxWindows, platform: 'Win32' });
    ok(/Add to taskbar/.test(fxw), 'H: Firefox on Windows: Add to taskbar: ' + fxw.slice(0, 200));
    const fxa = await steps({ ua: UA.firefoxAndroid, platform: 'Linux armv8l', touch: 5 });
    // support.mozilla.org "Use Web Apps with Firefox for Android": menu > Install > Add to Home
    // Screen > add automatically; a plain shortcut only where no Install is offered
    ok(/Install/.test(fxa) && /Add to Home Screen/.test(fxa) && /automatically/i.test(fxa), 'H: Firefox on Android: menu, Install, Add to Home Screen, automatically: ' + fxa.slice(0, 260));
    ok(/no Install|isn.t offered|not offered|without Install/i.test(fxa) && /shortcut/i.test(fxa), 'H: Firefox on Android: the shortcut only as the fallback when Install is missing');
    ok(!/Firefox adds a shortcut that opens in Firefox, not a separate app window/.test(fxa), 'H: Firefox on Android: no blanket "only a shortcut" claim');
    const edge = await steps({ ua: UA.edge, platform: 'Win32' });
    ok(/Apps/.test(edge) && /Install this site as an app/.test(edge), 'H: Edge: Apps, Install this site as an app: ' + edge.slice(0, 200));
    const and = await steps({ ua: UA.chromeAndroid, platform: 'Linux armv8l', touch: 5 });
    ok(/Add to Home screen|Install app/.test(and), 'H: Chrome on Android: menu, Install app or Add to Home screen');
    const sam = await steps({ ua: UA.samsung, platform: 'Linux armv8l', touch: 5 });
    ok(/Add page to/.test(sam), 'H: Samsung Internet: Add page to');
  }
  // I. A plain-HTTP LAN address: explains why, promises nothing, registers no worker
  {
    const p = page({ url: 'http://192.168.1.20:7788/' });
    ok(p.shown(), 'I: the button still explains itself on a LAN address');
    p.btn().click(); await sleep(10);
    const t = p.helpText();
    ok(/HTTPS/.test(t) && /192\.168\.1\.20:7788/.test(t), 'I: names HTTPS and this address: ' + t.slice(0, 260));
    ok(!/Add to Dock|address bar/.test(t), 'I: no install steps that cannot work here');
    ok(p.registered.length === 0, 'I: no worker on an insecure page');
    const lo = page({ url: 'http://localhost:7788/' });
    lo.btn().click(); await sleep(10);
    ok(!/HTTPS/.test(lo.helpText().split('Music')[0]) || /address bar|menu/.test(lo.helpText()), 'I: localhost counts as secure');
    ok(lo.registered.length === 1, 'I: localhost registers the worker');
  }
  // J. Hide the button for good on this browser; #install brings the help (and the button) back
  {
    const p = page();
    p.btn().click(); await sleep(10);
    const hide = [...p.help().querySelectorAll('button')].find((b) => /Hide this button/.test(b.textContent));
    ok(!!hide, 'J: the help offers "Hide this button"');
    if (hide) { hide.click(); await sleep(10); }
    ok(!p.shown(), 'J: hidden');
    ok(p.w.localStorage.getItem('pm.installHidden') === '1', 'J: remembered in this browser');
    ok(!page({ storage: { 'pm.installHidden': '1' } }).shown(), 'J: stays hidden on the next visit');
    const back = page({ url: 'https://music.example.org/#install', storage: { 'pm.installHidden': '1' } });
    await sleep(10);
    ok(back.help() && back.help().open && back.shown(), 'J: /#install opens the help and shows the button again');
    ok(back.w.localStorage.getItem('pm.installHidden') === null, 'J: and forgets the hiding');
    ok(back.w.localStorage.getItem('pm.token') === null, 'J: nothing else in storage is touched');
  }
  // K. No reload, ever: a new worker taking over leaves the page (and its music) alone
  {
    const p = page();
    p.w.navigator.serviceWorker.dispatchEvent(new p.w.Event('controllerchange'));
    await sleep(10);
    ok(p.reloads() === 0, 'K: controllerchange does not reload');
    // The one reload: the Reconnect button a person presses once the node answers again
    const PRESSED = "querySelector('#pwareconnect').addEventListener('click', () => location.reload())";
    ok(pwa.split(PRESSED).length === 2 && !/location\.reload|\.reload\(/.test(pwa.split(PRESSED).join('')), 'K: pwa.js reloads only on the Reconnect button, never by itself');
  }
  console.log(`${checks - failures.length}/${checks} install UI checks passed`);
  if (failures.length) { for (const f of failures) console.log('FAIL ' + f); process.exit(1); }
})().catch((e) => { console.error(e); process.exit(1); });
