// The installable web app in a real Chromium against a real node (docs/guides/install-web-app.md).
// It starts the node itself (the command after `--`), so it can stop it and start it again.
//   1. Chromium's own manifest and installability diagnostics (CDP Page.getAppManifest,
//      Page.getInstallabilityErrors): no errors on loopback; "not secure" on a plain-HTTP LAN
//      address. This is the browser's verdict, not an install on a phone or desktop (that needs a
//      device);
//   2. the Install app button and its help at phone, tablet and desktop widths (screenshots);
//   3. the service worker keeps only the public page files: never /v1, art, audio or a token;
//   4. the node stopped: the installed app still opens and says it can't reach the node; the
//      node back: it reconnects by itself; logging out leaves nothing private behind;
//   5. a new worker takes over while audio plays: no reload, the audio keeps going;
//   6. inside the desktop app (window.pmDesktop): no button and no worker.
//   7. the approved Odeion icons (docs/brand/odeion/sources.json) are what the page, the
//      manifest and the sidebar actually use;
//   8. behind a same-origin sign-in proxy (synthetic), the worker keeps no login page, private
//      page, redirect target or anything from a query: only the public files as anyone gets them;
//   9. switching accounts (log out the owner, log in a listener) leaves only the listener's
//      session: no replay of the owner's token, nothing private kept.
//  10. negative controls: a page without a manifest must be reported not installable (so the
//      verdicts come from a browser whose checks answer; the headless shell fails here), and under
//      CI the plain-HTTP LAN check must run;
//  11. signed out with no token: an Install app action a person can see and press on the login page;
//      #install typed on the open page brings the button back without a reload;
//  12. a proxy or tunnel answering 5xx (500, 502, 520–530) opens the kept page; 401/403 pass as they
//      are; a silent node gets a visible "can't reach" note and a way back (Reconnect, or the app's
//      own retry), never a reload by itself; a file changed to the same length still updates the
//      copy; a page from the node never mixes in an older kept script; behind a cookie sign-in proxy
//      Chromium reads the manifest and finds the app installable; icons are served no-cache; an
//      Android WebView user agent without the app's bridge gets no install action or worker.
// The audio during the worker update is a generated tone over HTTP (synthetic, not a provider).
// The install events of the "fake prompt" section and the standalone window are simulated:
// they check the page's reaction only.
//
// Full Chromium (channel "chromium", new headless): Playwright's default headless shell answers
// the installability calls with empty results whatever the page, so it proves nothing.
//   npm install playwright@1.49.1 && npx playwright install chromium   (anywhere; run from there)
//   node tools/ui-pwa-browser.js <token> http://127.0.0.1:7841 <out dir> [--lan http://<lan ip>:7841]
//        [--data <the node's data_dir>] -- <node command…>
// --data lets it add a listener to the (stopped) fixture node for 9.; it needs node:sqlite
// (Node 22.13+) and @noble/hashes (BLAKE3, as the node hashes tokens), resolved like playwright.
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const { spawn } = require('child_process');
const http = require('http');
const os = require('os');
const fs = require('fs');
const path = require('path');
const args = process.argv.slice(2);
const sep = args.indexOf('--');
const [TOKEN, BASE, OUT] = args;
const lanAt = args.indexOf('--lan');
const LAN = lanAt > 0 && lanAt < sep ? args[lanAt + 1] : null;
const dataAt = args.indexOf('--data');
const DATA = dataAt > 0 && dataAt < sep ? args[dataAt + 1] : null;
const REPO = path.resolve(__dirname, '..');
const BRAND = JSON.parse(fs.readFileSync(path.join(REPO, 'docs', 'brand', 'odeion', 'sources.json'), 'utf8'));
const sha256 = (b) => require('crypto').createHash('sha256').update(b).digest('hex');
const PAGES = new RegExp(fs.readFileSync(path.join(REPO, 'crates', 'pm-api', 'static', 'sw.js'), 'utf8').match(/^const PAGES = \/(.+)\/;$/m)[1]); // the worker's own list
const CMD = args.slice(sep + 1);
if (!TOKEN || !BASE || !OUT || sep < 0 || !CMD.length) { console.error('usage: ui-pwa-browser.js <token> <base> <out> [--lan url] -- <node command…>'); process.exit(2); }
fs.mkdirSync(OUT, { recursive: true });
const SHELL = new Set(['/', '/app.js', '/app.css', '/pwa.js', '/pwa.css', '/feedback.js', '/feedback.css', '/usage.js', '/manifest.webmanifest', '/icon.svg', '/icon-192.png', '/icon-512.png', '/icon-maskable-512.png', '/apple-touch-icon.png']);
const failures = [];
let checks = 0;
const ok = (cond, what) => { checks++; console.log(`${cond ? 'ok  ' : 'FAIL'} ${what}`); if (!cond) failures.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const report = {};
const save = (name, v) => { report[name] = v; fs.writeFileSync(path.join(OUT, 'report.json'), JSON.stringify(report, null, 2)); };

let node = null;
const nodeLog = fs.createWriteStream(path.join(OUT, 'node.log'), { flags: 'a' });
async function startNode() {
  node = spawn(CMD[0], CMD.slice(1), { stdio: ['ignore', 'pipe', 'pipe'] });
  node.stdout.pipe(nodeLog, { end: false }); node.stderr.pipe(nodeLog, { end: false });
  for (let i = 0; i < 120; i++) {
    try { const r = await fetch(BASE + '/v1/info', { headers: { authorization: 'Bearer ' + TOKEN } }); if (r.ok) return; } catch { /* starting */ }
    await sleep(500);
  }
  throw new Error('the node did not start');
}
async function stopNode() { if (!node) return; const n = node; node = null; n.kill('SIGTERM'); await new Promise((r) => n.once('exit', r)); }

// What the page's caches hold: [cache name, path] pairs
const cached = (page) => page.evaluate(async () => { const out = []; for (const n of await caches.keys()) { const c = await caches.open(n); for (const r of await c.keys()) out.push([n, new URL(r.url).pathname + new URL(r.url).search]); } return out; });
// Everything the worker kept, with its type and (for text) its body
const kept = (page) => page.evaluate(async () => { const out = []; for (const n of await caches.keys()) { const c = await caches.open(n); for (const q of await c.keys()) { const r = await c.match(q); const type = r.headers.get('content-type') || ''; out.push({ path: new URL(q.url).pathname + new URL(q.url).search, type, cacheControl: r.headers.get('cache-control') || '', url: r.url, responseType: r.type, redirected: r.redirected, body: /^image\/png/.test(type) ? '' : await r.text() }); } } return out; });
const TYPE = (p) => (p === '/' ? 'text/html' : { js: 'text/javascript', css: 'text/css', png: 'image/png', svg: 'image/svg+xml', webmanifest: 'application/manifest+json' }[p.split('.').pop()]);
// Every request a page makes: same origin only, and the worker answers only its public files
// (Other origins are blocked, not fetched: the node's public catalogue lookups name artist images
// on a CDN; this test asks no third party for anything.)
const watch = (page, origin) => { const log = []; page.route((u) => u.origin !== origin && /^https?:/.test(u.protocol), (r) => r.abort('blockedbyclient')); page.on('response', (r) => { const u = new URL(r.url()); log.push({ origin: u.origin, path: u.pathname, search: u.search, status: r.status(), sw: r.fromServiceWorker(), type: r.request().resourceType() }); }); page.on('requestfailed', (q) => { const u = new URL(q.url()); log.push({ origin: u.origin, path: u.pathname, search: u.search, type: q.resourceType(), failed: q.failure() && q.failure().errorText }); }); return () => ({ log, foreign: log.filter((x) => x.origin !== origin && /^https?:/.test(x.origin) && x.type !== 'image'), swOutside: log.filter((x) => x.sw && !(SHELL.has(x.path) || (x.type === 'document' && PAGES.test(x.path)))) }); };
// The worker active, or an error within 15 s (a worker that never installs must not hang the run)
const swReady = (page) => Promise.race([page.evaluate(() => navigator.serviceWorker.ready.then(() => true)), sleep(15000).then(() => { throw new Error('the service worker never became active'); })]);
// An "Install app" button a person can actually see and press (not a hidden flag that is off)
const visibleInstall = async (page) => { const l = page.getByRole('button', { name: /^Install app/ }); const n = await l.count(); for (let i = 0; i < n; i++) if (await l.nth(i).isVisible()) return l.nth(i); return null; };
const noOverflow = (page) => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1);

// Launch safety (review round 3): a launch while the node or a proxy misbehaves always ends, in
// bounded time, in a usable page (the kept app with its note, or the "can't reach" page); the kept
// copy survives unrelated answers; a proxy's sign-in page is not "the node is back"; an explicit
// #install survives the sign-in screen. Every read is bounded, every held connection is the test's own.
const peek = (page) => Promise.race([page.evaluate(() => ({ h1: ((document.querySelector('h1') || {}).textContent || '').trim(), topbar: !!document.querySelector('.topbar'), note: ((document.getElementById('pwanotice') || {}).textContent || '').replace(/\s+/g, ' ').trim(), reconnect: !!document.getElementById('pwareconnect'), help: !!(document.getElementById('installhelp') && document.getElementById('installhelp').open) })), sleep(3000).then(() => null)]).catch(() => null);
const halt = (page) => Promise.race([page.evaluate(() => window.stop()), sleep(1000)]).catch(() => {});
const usable = (v) => !!v && (/can.t reach your node/i.test(v.h1) || (v.topbar && /can.t reach your node/i.test(v.note)));
function caseProxy() {
  const st = { mode: null, stuck: [] };
  const up = new URL(BASE);
  const hold = (sock) => { st.stuck.push(sock); };
  const server = http.createServer((req, res) => {
    const cookie = /(?:^|;\s*)pagecookie=1/.test(req.headers.cookie || '');
    const u = new URL(req.url, 'http://x'); const html = /text\/html/.test(req.headers.accept || '');
    const m = st.mode;
    if (m === 'hang') return hold(req.socket);
    if (m === 'stallBody') { if (html) { res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }); res.flushHeaders(); } hold(req.socket); return; }
    if (m === 'loginOnly') { if (u.pathname === '/login') { res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' }); res.end('<!doctype html><title>Sign in</title><p>PROXY-LOGIN'); } else { res.writeHead(302, { location: '/login?next=' + encodeURIComponent(req.url) }); res.end(); } return; }
    if (!cookie && m === 'noPwaJs' && u.pathname === '/pwa.js') { res.writeHead(404, { 'content-type': 'text/plain' }); res.end('not here'); return; }
    if (!cookie && m === 'portal') { if (u.pathname === '/') { res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }); res.end('<!doctype html><title>Wi-Fi</title><p>PORTAL-LOGIN'); } else { res.writeHead(429, { 'content-type': 'text/plain' }); res.end('slow down'); } return; }
    const out = http.request({ host: up.hostname, port: up.port, path: req.url, method: req.method, headers: req.headers }, (r) => { res.writeHead(r.statusCode, r.headers); r.pipe(res); });
    out.on('error', () => { res.writeHead(502); res.end(); });
    req.pipe(out);
  });
  server.on('upgrade', (req, sock, head) => {
    if (st.mode) { hold(sock); return; }
    const o = require('net').connect(Number(up.port), up.hostname, () => { o.write(`${req.method} ${req.url} HTTP/1.1\r\n` + Object.entries(req.headers).map(([k, v]) => `${k}: ${v}`).join('\r\n') + '\r\n\r\n'); if (head && head.length) o.write(head); o.pipe(sock); sock.pipe(o); });
    o.on('error', () => sock.destroy()); sock.on('error', () => o.destroy());
  });
  return { st, server, release: () => st.stuck.splice(0).forEach((x) => x.destroy()), close: () => { st.stuck.splice(0).forEach((x) => x.destroy()); server.closeAllConnections && server.closeAllConnections(); server.close(); } };
}
async function caseContext(browser, via) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await ctx.addCookies([{ name: 'pagecookie', value: '1', url: via }]);
  await ctx.addInitScript((t) => { try { localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
  const page = await ctx.newPage();
  await page.goto(via + '/', { waitUntil: 'load', timeout: 20000 }); await swReady(page);
  await page.reload({ waitUntil: 'load', timeout: 20000 }); await sleep(1500);
  return { ctx, page };
}
// A launch: until the page is usable (or `ms` passed); returns what showed and when
async function launchUntil(page, url, ms, done = usable) {
  const t0 = Date.now();
  page.goto(url, { waitUntil: 'commit', timeout: ms }).catch(() => {});
  let v = null;
  while (Date.now() - t0 < ms) { await sleep(400); v = await peek(page); if (done(v)) break; }
  return { ms: Date.now() - t0, seen: v };
}
async function launchSafety(browser) {
  // M9: the kept page without one of its blocking files (here /pwa.js was never kept), node silent
  {
    const P = caseProxy(); await new Promise((r) => P.server.listen(0, '127.0.0.1', r));
    const VIA = `http://127.0.0.1:${P.server.address().port}`;
    P.st.mode = 'noPwaJs';
    const { ctx, page } = await caseContext(browser, VIA);
    const k = (await kept(page)).map((x) => x.path);
    ok(k.includes('/') && !k.includes('/pwa.js'), 'partial copy (precondition): the page is kept, /pwa.js is not ' + JSON.stringify(k));
    P.st.mode = 'hang';
    const r = await launchUntil(page, VIA + '/albums', 10000);
    save('launchPartialCopySilent', r);
    ok(usable(r.seen) && r.ms < 9000, `partial copy, silent node: a usable page (the "can't reach" page) in bounded time, not a blank launch: ${r.ms} ms ${JSON.stringify(r.seen)}`);
    await halt(page); P.release(); P.st.mode = null; await ctx.close(); P.close();
  }
  // M10, then the kept copy against unrelated answers, then a proxy sign-in page while away
  {
    const P = caseProxy(); await new Promise((r) => P.server.listen(0, '127.0.0.1', r));
    const VIA = `http://127.0.0.1:${P.server.address().port}`;
    const { ctx, page } = await caseContext(browser, VIA);
    const k = (await kept(page)).map((x) => x.path);
    ok(['/', '/pwa.js', '/app.js', '/app.css', '/pwa.css', '/feedback.js', '/feedback.css', '/usage.js'].every((f) => k.includes(f)), 'full copy (precondition): the page and its blocking files are kept');
    // M10: the proxy sends 200 headers for the page, then nothing
    P.st.mode = 'stallBody';
    const r = await launchUntil(page, VIA + '/albums', 10000);
    save('launchHeadersThenNothing', r);
    ok(usable(r.seen) && r.seen.topbar && r.ms < 9000, `200 headers then no body: the kept app opens with its note in bounded time: ${r.ms} ms ${JSON.stringify(r.seen)}`);
    // A proxy that answers only with its sign-in page (302 -> 200 HTML) is not the node coming back
    if (r.seen && r.seen.topbar) {
      P.st.mode = 'loginOnly';
      await sleep(12000);
      const v = await peek(page);
      save('noteWhileProxyLogin', v);
      ok(!!v && /can.t reach your node/i.test(v.note) && !v.reconnect, 'proxy sign-in page while away: the note still says the node can\'t be reached (no "answers again") ' + JSON.stringify(v));
      P.st.mode = null;
      let back = null; for (let i = 0; i < 30; i++) { await sleep(500); back = await peek(page); if (back && back.reconnect) break; }
      ok(!!back && back.reconnect, 'and the real node answering again is still recognised (Reconnect offered)');
    } else ok(false, 'proxy sign-in page while away: not checked (the kept app did not open)');
    // Unrelated answers to the worker's own refresh (a captive portal's 200 page, 429) leave the copy alone
    await halt(page); P.release(); P.st.mode = 'portal';
    await page.goto(VIA + '/albums', { waitUntil: 'load', timeout: 20000 }).catch(() => {}); await sleep(2000);
    const root = (await kept(page)).find((x) => x.path === '/');
    save('copyAfterPortal', root ? { url: root.url, type: root.type, bytes: root.body.length, portal: root.body.includes('PORTAL-LOGIN') } : null);
    ok(!!root && root.body.includes('<script src="/app.js"></script>') && !root.body.includes('PORTAL-LOGIN'), 'unrelated answer (portal 200 / 429): the valid kept page is not dropped or replaced');
    P.st.mode = null;
    await halt(page); P.release(); P.st.mode = null; await ctx.close(); P.close();
  }
  // #install on arrival, signed out: the sign-in screen does not close the help that was asked for
  {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    await page.goto(BASE + '/search?q=kept#install', { waitUntil: 'load', timeout: 20000 }); await sleep(2500);
    const v = await peek(page); const u = new URL(page.url());
    save('installHashSignedOut', { v, url: page.url() });
    ok(!!v && v.help && await page.locator('#loginpage').isVisible() && u.pathname === '/search' && u.search === '?q=kept' && !u.hash, '#install on arrival, signed out: the help stays open over the login page, path and query kept ' + JSON.stringify(v));
    await ctx.close();
  }
}

// The browser: full Chromium. PWA_CHANNEL="" picks Playwright's default headless shell instead: only to
// show that the negative control below then fails (CI never sets it).
const CHANNEL = process.env.PWA_CHANNEL === undefined ? 'chromium' : process.env.PWA_CHANNEL || undefined;

// A run never hangs: past the limit it records why, stops its node and fails
setTimeout(async () => { failures.push('watchdog: the run took over ' + (Number(process.env.PWA_WATCHDOG_MS) || 480000) + ' ms'); save('summary', { checks, failed: failures.length, failures }); console.error('FAIL watchdog'); await stopNode(); process.exit(1); }, Number(process.env.PWA_WATCHDOG_MS) || 480000).unref();

(async () => {
  await startNode();
  const browser = await chromium.launch({ channel: CHANNEL });
  // PWA_ONLY=launch: only the launch-safety cases (focused RED/GREEN loops; CI runs everything)
  if (process.env.PWA_ONLY === 'launch') {
    await launchSafety(browser);
    await browser.close(); await stopNode();
    save('summary', { checks, failed: failures.length, failures, only: 'launch' });
    console.log(`
${checks - failures.length}/${checks} browser checks passed (launch safety only)`);
    process.exit(failures.length ? 1 : 0);
  }
  // Chromium's install checks: a normal (not incognito) profile, as a person's browser is
  const verdict = async (url, cookies) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'pwa-profile-'));
    // Skipping only the engagement wait (a person's clicks over 30 s), so a real prompt can arrive
    const ctx = await chromium.launchPersistentContext(dir, { channel: CHANNEL, viewport: { width: 1440, height: 900 }, args: ['--bypass-app-banner-engagement-checks'] });
    if (cookies) await ctx.addCookies(cookies); // signed in to a sign-in proxy in front of the node
    await ctx.addInitScript(() => { window.pwaRealPrompts = []; addEventListener('beforeinstallprompt', (e) => { setTimeout(() => window.pwaRealPrompts.push({ prevented: e.defaultPrevented }), 0); }); });
    const page = ctx.pages()[0] || await ctx.newPage();
    await page.goto(url, { waitUntil: 'load' });
    if (await page.evaluate(() => !!navigator.serviceWorker)) await swReady(page);
    await sleep(1000);
    const cdp = await ctx.newCDPSession(page);
    const man = await cdp.send('Page.getAppManifest');
    const inst = await cdp.send('Page.getInstallabilityErrors');
    let icon = null; try { const r = await cdp.send('Page.getManifestIcons'); icon = r.primaryIcon ? r.primaryIcon.length : null; } catch { /* older Chromium */ }
    const secure = await page.evaluate(() => window.isSecureContext);
    let prompts = []; for (let i = 0; i < 16 && !prompts.length && secure; i++) { await sleep(500); prompts = await page.evaluate(() => window.pwaRealPrompts); }
    const button = !!(await visibleInstall(page)); // this profile has no token: the login page
    await ctx.close();
    return { man, inst: inst.installabilityErrors, icon, secure, prompts, button, profile: 'persistent, not incognito' };
  };
  const version = browser.version();
  save('browser', { chromium: version, headless: true, base: BASE, note: 'CDP installability verdict from a real Chromium; not an OS-level install on a device' });

  // 0. Negative controls: the installability verdicts below mean something only if this browser's
  // checks answer at all. A page without a manifest must be reported not installable (Playwright's
  // headless shell answers "no errors" for every page); and CI must also run the plain-HTTP control.
  {
    const bare = http.createServer((q, r) => { r.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }); r.end('<!doctype html><title>No manifest</title><p>A page with no manifest'); });
    await new Promise((r) => bare.listen(0, '127.0.0.1', r));
    const ctx = await browser.newContext(); const page = await ctx.newPage();
    await page.goto(`http://127.0.0.1:${bare.address().port}/`, { waitUntil: 'load' }); await sleep(500);
    const errs = (await (await ctx.newCDPSession(page)).send('Page.getInstallabilityErrors')).installabilityErrors;
    save('negativeControl', { page: 'no manifest, loopback', installabilityErrors: errs, executable: chromium.executablePath(), channel: CHANNEL || 'default (headless shell)' });
    ok(Array.isArray(errs) && errs.some((e) => e.errorId === 'no-manifest'), 'negative control: Chromium reports a page without a manifest not installable ' + JSON.stringify(errs));
    await ctx.close(); bare.close();
    if (process.env.CI) ok(!!LAN, 'negative control: CI runs the plain-HTTP LAN check (--lan)');
  }

  // 1. Chromium's verdict: the manifest as it parsed it, and why it would not install
  {
    const { man, inst, icon, prompts, button } = await verdict(BASE + '/');
    const parsed = JSON.parse(man.data || '{}');
    save('loopback', { url: man.url, manifestErrors: man.errors, installabilityErrors: inst, parsed: man.parsed || null, manifest: parsed, primaryIconBase64Length: icon, realBeforeInstallPrompt: prompts, installButtonShown: button });
    ok(prompts.length >= 1 && prompts[0].prevented, 'real prompt: Chromium fired beforeinstallprompt and pwa.js kept it for the button ' + JSON.stringify(prompts));
    ok(button === true, 'real prompt: an Install app button is actually visible on the signed-out page');
    ok(man.url === BASE + '/manifest.webmanifest', 'CDP: Chromium found the manifest at ' + man.url);
    ok(Array.isArray(man.errors) && man.errors.length === 0, 'CDP: no manifest errors ' + JSON.stringify(man.errors));
    ok(parsed.name === 'Odeion' && parsed.id === '/' && parsed.start_url === '/', 'CDP: manifest name Odeion, id and start_url on the node root');
    ok(Array.isArray(inst) && inst.length === 0, 'CDP: Chromium reports no installability errors on loopback ' + JSON.stringify(inst));
    ok(icon > 0, 'CDP: Chromium picked a primary icon from the manifest (' + icon + ' base64 bytes)');
    if (man.parsed) ok(!/token/i.test(JSON.stringify(man.parsed)), 'CDP: no token anywhere in the parsed manifest');
  }

  // 2. The button and its help, signed in, at three widths
  for (const [name, w, h, touch] of [['phone', 390, 844, true], ['tablet', 820, 1180, true], ['desktop', 1440, 900, false]]) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h }, hasTouch: touch, isMobile: touch && w < 600, deviceScaleFactor: 2 });
    await ctx.addInitScript((t) => { try { if (!localStorage.getItem('pm.token') && !sessionStorage.getItem('pwa.loggedOut')) localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
    const page = await ctx.newPage();
    const errors = []; page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(BASE + '/', { waitUntil: 'load' }); await sleep(1500);
    const btn = page.locator('#installbtn');
    ok(await btn.isVisible(), `${name}: Install app button visible`);
    const box = await btn.boundingBox();
    ok(!!box && box.width >= 32 && box.height >= 32, `${name}: button is a usable target ${JSON.stringify(box)}`);
    ok(await noOverflow(page), `${name}: no horizontal overflow with the button`);
    await page.screenshot({ path: path.join(OUT, `${name}-topbar.png`) });
    await btn.click(); await sleep(300);
    ok(await page.locator('#installhelp').isVisible(), `${name}: a click opens the install help`);
    const text = (await page.locator('#installhelp').textContent()).replace(/\s+/g, ' ');
    ok(/address bar|menu/.test(text) && !/HTTPS address/.test(text), `${name}: Chromium on loopback gets Chromium steps, not the HTTP note`);
    await page.screenshot({ path: path.join(OUT, `${name}-install-help.png`) });
    await page.locator('#installhelp [data-install="close"]').click();
    ok(!(await page.locator('#installhelp').isVisible()), `${name}: Done closes it`);
    ok(!errors.length, `${name}: no page errors ${errors.join('; ')}`);
    await ctx.close();
  }

  // 2b. Signed out, no token at all: a fresh visitor sees an Install app action on the login page
  for (const [name, w, h, touch] of [['phone', 390, 844, true], ['desktop', 1440, 900, false]]) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h }, hasTouch: touch, isMobile: touch, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    const errors = []; page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(BASE + '/', { waitUntil: 'load' }); await sleep(1500);
    ok(await page.locator('#loginpage').isVisible() && await page.evaluate(() => !localStorage.getItem('pm.token') && document.body.classList.contains('signedout')), `signed out ${name}: the login page shows and this browser has no token`);
    const entry = await visibleInstall(page);
    ok(!!entry, `signed out ${name}: an Install app button is actually visible (not only unhidden)`);
    if (entry) {
      const box = await entry.boundingBox();
      ok(!!box && box.width >= 32 && box.height >= 32 && box.y + box.height <= h, `signed out ${name}: it is a usable target on screen ${JSON.stringify(box)}`);
      ok(await page.evaluate(() => !(document.activeElement && /install/i.test(document.activeElement.id || ''))), `signed out ${name}: it does not take the focus from the login`);
      ok(await noOverflow(page), `signed out ${name}: no horizontal overflow`);
      await page.screenshot({ path: path.join(OUT, `${name}-signed-out-install.png`) });
      await entry.click(); await sleep(300);
      const help = page.locator('#installhelp');
      ok(await help.isVisible() && /address bar|menu/.test((await help.textContent()) || ''), `signed out ${name}: clicking it opens the install help with this browser's steps`);
      await page.screenshot({ path: path.join(OUT, `${name}-signed-out-install-help.png`) });
      await page.locator('#installhelp [data-install="close"]').click(); await sleep(200);
      ok(!(await help.isVisible()) && await page.locator('#loginpage').isVisible(), `signed out ${name}: Done closes it and the login page is still there`);
    }
    ok(await page.evaluate(() => !localStorage.getItem('pm.token')), `signed out ${name}: still no token (nothing signed in)`);
    ok(!errors.length, `signed out ${name}: no page errors ${errors.join('; ')}`);
    await ctx.close();
  }

  // 2c. "Hide this button", then #install typed on the SAME page (a same-document hash change): the
  // help opens and the button returns, the path and query stay, the page is not reloaded
  {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    await ctx.addInitScript((t) => { try { localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
    const page = await ctx.newPage();
    const errors = []; page.on('pageerror', (e) => errors.push(e.message));
    let loads = 0; page.on('load', () => { loads++; });
    await page.goto(BASE + '/search?q=wasted', { waitUntil: 'load' }); await sleep(1500);
    await page.locator('#installbtn').click(); await sleep(300);
    await page.locator('#installhelp [data-install="hide"]').click(); await sleep(300);
    ok(!(await visibleInstall(page)), 'same-page #install: "Hide this button" hides it');
    await page.evaluate(() => { window.pwaMarker = 'same document'; location.hash = 'install'; }); await sleep(800);
    const u = new URL(page.url());
    const st = await page.evaluate(() => ({ marker: window.pwaMarker, hidden: localStorage.getItem('pm.installHidden') }));
    ok(await page.locator('#installhelp').isVisible(), 'same-page #install: the help opens without leaving the page');
    ok(!!(await visibleInstall(page)) && st.hidden === null, 'same-page #install: the button is back ' + JSON.stringify(st));
    ok(st.marker === 'same document' && loads === 1, `same-page #install: no reload (${loads} load)`);
    ok(u.pathname === '/search' && u.search === '?q=wasted' && u.hash === '', 'same-page #install: path and query kept, #install dropped: ' + page.url());
    await page.keyboard.press('Escape');
    ok(!errors.length, 'same-page #install: no page errors ' + errors.join('; '));
    await ctx.close();
  }

  // 3–4. One context as the installed app would use: worker, node down, node back, log out
  {
    const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true, deviceScaleFactor: 2 });
    await ctx.addInitScript((t) => { try { if (!sessionStorage.getItem('pwa.loggedOut') && !localStorage.getItem('pm.token')) localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
    const page = await ctx.newPage();
    const errors = []; page.on('pageerror', (e) => errors.push(e.message));
    const audit = watch(page, new URL(BASE).origin);
    await page.goto(BASE + '/', { waitUntil: 'load' });
    await swReady(page);
    await page.reload({ waitUntil: 'load' }); await sleep(1500);
    ok(await page.evaluate(() => !!navigator.serviceWorker.controller), 'worker: controls the page after a reload');
    await page.goto(BASE + '/albums', { waitUntil: 'load' }); await sleep(1200);
    const albums = await page.evaluate(() => document.querySelector('#main').textContent.replace(/\s+/g, ' ').trim());
    ok(albums.length > 20 && !/error|went wrong/i.test(albums), 'populated: the fixture library shows on /albums: ' + albums.slice(0, 90));
    // Browse a few views: the API, covers and the queue go through the network, never the cache
    for (const v of ['albums', 'tracks', 'queue', 'settings']) { await page.goto(BASE + '/' + v, { waitUntil: 'load' }); await sleep(700); }
    // A full load (from /settings to /albums: a hash on the same path would not load the page)
    await page.goto(BASE + '/albums#install', { waitUntil: 'load' }); await sleep(600);
    ok(await page.locator('#installhelp').isVisible(), 'hash: /albums#install opens the install help');
    ok(new URL(page.url()).pathname === '/albums' && new URL(page.url()).hash === '', 'hash: the path stays, #install is dropped from the address ' + page.url());
    await page.keyboard.press('Escape');
    await page.goto(BASE + '/search?q=wasted', { waitUntil: 'load' }); await sleep(800);
    ok(new URL(page.url()).search === '?q=wasted', 'query: a search link keeps its query through the worker');
    const held = await cached(page);
    save('cacheWhileSignedIn', held);
    ok(held.length > 0 && held.every(([, p]) => SHELL.has(p)), 'worker: caches only the public page files ' + JSON.stringify(held.map(([, p]) => p)));
    ok(!held.some(([, p]) => /\/v1\/|token|\/art\//.test(p)), 'worker: nothing from /v1, art or with a token is cached');
    const shellKept = await kept(page);
    const root = shellKept.find((k) => k.path === '/');
    ok(!!root && root.body.includes('<script src="/app.js"></script>') && root.body.includes('<link rel="manifest" href="/manifest.webmanifest" crossorigin="use-credentials">'), 'worker: the kept page is the node\'s own public page');
    ok(shellKept.every((k) => k.type.split(';')[0].trim() === TYPE(k.path.split('?')[0]) && !/no-store|private/i.test(k.cacheControl) && !k.path.includes('?')), 'worker: every kept file has its own type, no query, no private/no-store ' + JSON.stringify(shellKept.map((k) => [k.path, k.type])));
    ok(!shellKept.some((k) => k.body.includes(TOKEN)), 'worker: no kept file contains the sign-in token');
    save('keptMetadata', shellKept.map((k) => ({ path: k.path, url: k.url, responseType: k.responseType, redirected: k.redirected, type: k.type, cacheControl: k.cacheControl, bytes: k.body.length })));
    ok(shellKept.every((k) => k.url === new URL(BASE).origin + k.path && k.responseType === 'basic' && k.redirected === false), 'worker: every kept response is the plain public file itself: its URL is the path (no query), type basic, not redirected');
    // The node goes away: a fresh launch still opens and says so
    await stopNode();
    await page.goto(BASE + '/albums', { waitUntil: 'load' }); await sleep(4000);
    const conn = (await page.locator('#conn').textContent()).replace(/\s+/g, ' ').trim();
    const bar = await page.locator('#connbar').isVisible() ? (await page.locator('#connbar').textContent()).replace(/\s+/g, ' ').trim() : '';
    save('nodeDown', { conn, connbar: bar });
    ok(await page.locator('.topbar').isVisible(), 'node down: the app opens from the cached page');
    ok(/reach|offline|connect/i.test(conn + ' ' + bar), 'node down: it says it cannot reach the node: ' + (bar || conn));
    await page.screenshot({ path: path.join(OUT, 'phone-node-down.png') });
    const noNode = await ctx.newPage();
    await noNode.goto(BASE + '/party/abc', { waitUntil: 'load' }).catch(() => {}); await sleep(800);
    const partyShell = await noNode.evaluate(() => !!document.querySelector('.topbar')).catch(() => false);
    ok(!partyShell, 'node down: a party link is not answered with the app page (' + noNode.url() + ')');
    await noNode.close();
    // …and comes back
    // (polled: when the next retry is due depends on how long the node was away)
    await startNode();
    let back = '';
    for (let i = 0; i < 40; i++) { await sleep(500); back = (await page.locator('#conn').textContent()).replace(/\s+/g, ' ').trim(); if (i >= 11 && !/reach|offline|lost/i.test(back)) break; }
    save('nodeBack', { conn: back });
    ok(!/reach|offline|lost/i.test(back), 'node back: reconnected by itself: ' + back);
    await page.screenshot({ path: path.join(OUT, 'phone-node-back.png') });
    // Log out: the token goes; the worker kept nothing that needs it
    await page.evaluate(() => { sessionStorage.setItem('pwa.loggedOut', '1'); });
    await page.locator('#me').click(); await sleep(300);
    const out = page.locator('.popover .pop-item', { hasText: 'Log out' });
    if (await out.count()) { await out.first().click(); await sleep(800); }
    await page.reload({ waitUntil: 'load' }); await sleep(1200);
    ok(await page.evaluate(() => !localStorage.getItem('pm.token')), 'log out: no token left in this browser');
    ok(await page.locator('#loginpage').isVisible(), 'log out: the next launch asks to log in (no cached private view)');
    const after = await cached(page);
    ok(after.every(([, p]) => SHELL.has(p)), 'log out: the cache still holds only public page files');
    await page.screenshot({ path: path.join(OUT, 'phone-logged-out.png') });
    ok(!errors.length, 'worker context: no page errors ' + errors.join('; '));
    const a = audit();
    save('requestAudit', { total: a.log.length, foreign: a.foreign, workerAnsweredOutsideShell: a.swOutside, failedWhileNodeDown: a.log.filter((x) => x.failed).length });
    ok(a.foreign.length === 0, 'requests: nothing but (blocked) images from other origins: no script, fetch or page ' + JSON.stringify(a.foreign.slice(0, 3)));
    ok(!a.log.some((x) => x.sw && x.origin !== new URL(BASE).origin), 'requests: the worker answered nothing from another origin');
    ok(a.swOutside.length === 0, 'requests: the worker answered nothing but its public files and app pages ' + JSON.stringify(a.swOutside.slice(0, 3)));
    ok(!a.log.some((x) => x.sw && x.search && x.type !== 'document'), 'requests: no file with a query came from the worker');
    await ctx.close();
  }

  // 5. A new worker arrives while something plays: no reload, the audio keeps going
  {
    // A node updated under the open page: a small proxy that, once told, serves a changed sw.js
    let v2 = false;
    let slowApp = false; // a new app.js that takes 5.5 s to come (longer than the launch wait)
    let samelen = false; // a changed file of exactly the same length (pwa.css: one word in capitals)
    let hang = false; // a node that is off but doesn't refuse: requests just never get an answer
    const hung = []; // their connections: a host that comes back resets them (unknown to it by then)
    const up = new URL(BASE);
    // Synthetic music over HTTP, API-shaped (/v1/…, so it never goes through the worker): 20 s of a
    // generated tone, with byte ranges as a media element asks for them. Not a provider stream.
    const TONE = (() => { const rate = 8000; const n = rate * 20; const b = Buffer.alloc(44 + n * 2); b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVE', 8); b.write('fmt ', 12); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); for (let i = 0; i < n; i++) b.writeInt16LE(Math.round(Math.sin(i / 8) * 3000), 44 + i * 2); return b; })();
    const toneHits = [];
    const proxy = http.createServer((req, res) => {
      if (hang) { hung.push(req.socket); return; }
      if (req.url === '/v1/pwa-test-tone.wav') {
        const m = /bytes=(\d+)-(\d*)/.exec(req.headers.range || ''); toneHits.push(req.headers.range || 'all');
        if (!m) { res.writeHead(200, { 'content-type': 'audio/wav', 'content-length': TONE.length, 'accept-ranges': 'bytes' }); res.end(TONE); return; }
        const a = Number(m[1]); const z = m[2] ? Math.min(Number(m[2]), TONE.length - 1) : TONE.length - 1;
        res.writeHead(206, { 'content-type': 'audio/wav', 'content-length': z - a + 1, 'content-range': `bytes ${a}-${z}/${TONE.length}`, 'accept-ranges': 'bytes' }); res.end(TONE.subarray(a, z + 1)); return;
      }
      const out = http.request({ host: up.hostname, port: up.port, path: req.url, method: req.method, headers: req.headers }, (r) => {
        if (slowApp && req.url === '/app.js') { const parts = []; r.on('data', (d) => parts.push(d)); r.on('end', () => setTimeout(() => { const body = Buffer.concat([...parts, Buffer.from("\n;window.pwaAppVersion = 'new';\n")]); res.writeHead(r.statusCode, { ...r.headers, 'content-length': body.length }); res.end(body); }, 5500)); return; }
        if (samelen && req.url === '/pwa.css') { const parts = []; r.on('data', (d) => parts.push(d)); r.on('end', () => { const body = Buffer.from(Buffer.concat(parts).toString('utf8').replace('Odeion', 'ODEION')); res.writeHead(r.statusCode, { ...r.headers, 'content-length': body.length }); res.end(body); }); return; }
        if (v2 && req.url === '/sw.js') { const parts = []; r.on('data', (d) => parts.push(d)); r.on('end', () => { const body = Buffer.concat([...parts, Buffer.from('\n// next version\n')]); const h = { ...r.headers, 'content-length': body.length }; res.writeHead(r.statusCode, h); res.end(body); }); return; }
        res.writeHead(r.statusCode, r.headers); r.pipe(res);
      });
      out.on('error', () => { res.writeHead(502); res.end(); });
      req.pipe(out);
    });
    // The app's event channel is a WebSocket: passed through too (unless the node is silent)
    proxy.on('upgrade', (req, sock, head) => {
      if (hang) { hung.push(sock); return; }
      const o = require('net').connect(Number(up.port), up.hostname, () => { o.write(`${req.method} ${req.url} HTTP/1.1\r\n` + Object.entries(req.headers).map(([k, v]) => `${k}: ${v}`).join('\r\n') + '\r\n\r\n'); if (head && head.length) o.write(head); o.pipe(sock); sock.pipe(o); });
      o.on('error', () => sock.destroy()); sock.on('error', () => o.destroy());
    });
    await new Promise((r) => proxy.listen(0, '127.0.0.1', r));
    const VIA = `http://127.0.0.1:${proxy.address().port}`;
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    await ctx.addInitScript((t) => { try { localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN); // signed in, as an installed app is
    const page = await ctx.newPage();
    await page.goto(VIA + '/', { waitUntil: 'load' });
    await swReady(page);
    await page.reload({ waitUntil: 'load' });
    // Synthetic music over HTTP from the node's side, standing in for a stream playing in this tab
    const toneFromWorker = []; page.on('response', (r) => { if (r.url().endsWith('/v1/pwa-test-tone.wav')) toneFromWorker.push(r.fromServiceWorker()); });
    await page.evaluate(async () => {
      const a = new Audio('/v1/pwa-test-tone.wav'); a.loop = true; a.muted = true; window.pwaTestAudio = a; window.pwaMarker = 'same page';
      await a.play();
      // …and one paused part-way, as a paused queue is
      const p = new Audio('/v1/pwa-test-tone.wav'); p.muted = true; await p.play(); await new Promise((r) => setTimeout(r, 300)); p.pause(); window.pwaPaused = p;
    });
    const before = await page.evaluate(() => ({ t: window.pwaTestAudio.currentTime, paused: window.pwaTestAudio.paused, pausedAt: window.pwaPaused.currentTime }));
    v2 = true;
    const swapped = await page.evaluate(() => new Promise((res) => { let c = 0; navigator.serviceWorker.addEventListener('controllerchange', () => { c++; }); navigator.serviceWorker.getRegistration().then((r) => r.update()).then(() => setTimeout(() => res(c), 3000)); }));
    const afterUpd = await page.evaluate(() => ({ t: window.pwaTestAudio.currentTime, paused: window.pwaTestAudio.paused, marker: window.pwaMarker, pausedStill: window.pwaPaused.paused, pausedAt: window.pwaPaused.currentTime }));
    save('update', { controllerChanges: swapped, before, after: afterUpd, audio: 'synthetic generated WAV over HTTP at /v1/pwa-test-tone.wav (test proxy), not a provider stream', toneRequests: toneHits.length, toneRanges: toneHits.slice(0, 6), toneAnsweredByWorker: toneFromWorker.filter(Boolean).length });
    ok(toneHits.length > 0 && toneFromWorker.length > 0 && !toneFromWorker.some(Boolean), `update: the (synthetic) HTTP audio came from the network, never the worker (${toneHits.length} requests)`);
    ok(swapped >= 1, 'update: the changed worker took over (controllerchange) ' + swapped);
    ok(afterUpd.marker === 'same page', 'update: the page was not reloaded');
    ok(!afterUpd.paused && afterUpd.t > before.t, `update: the audio kept playing (${before.t.toFixed(2)}s -> ${afterUpd.t.toFixed(2)}s)`);
    ok(afterUpd.pausedStill && Math.abs(afterUpd.pausedAt - before.pausedAt) < 0.01, `update: the paused audio stayed paused at ${before.pausedAt.toFixed(2)}s`);
    // The node changes a file without changing its length: the kept copy still follows it
    samelen = true;
    await page.reload({ waitUntil: 'load' }); await sleep(2000);
    const css = (await kept(page)).find((k) => k.path === '/pwa.css');
    save('sameLengthUpdate', { kept: css ? css.body.slice(0, 40) : null });
    ok(!!css && css.body.includes('ODEION') && !css.body.includes('Odeion as an app'), 'update: a file changed to the same length replaces the kept copy: ' + (css ? css.body.slice(0, 30) : 'none'));
    samelen = false;
    // A page that came from the node runs the node's files, even slow ones: never a new page with
    // an older kept script (the kept files are for a page opened from the copy)
    slowApp = true;
    await page.reload({ waitUntil: 'load', timeout: 20000 }); await sleep(500);
    const appV = await page.evaluate(() => window.pwaAppVersion || null);
    save('slowFileFromNetworkPage', { appVersion: appV });
    ok(appV === 'new', 'update: a page from the node waits for its slow new app.js instead of mixing in the kept one (' + appV + ')');
    slowApp = false;
    // The node stops answering without refusing: the kept page opens after the short wait
    hang = true;
    const t0 = Date.now();
    await page.goto(VIA + '/albums', { waitUntil: 'load', timeout: 20000 }).catch((e) => { save('hangError', String(e.message)); });
    const waited = Date.now() - t0;
    const opened = await page.locator('.topbar').isVisible().catch(() => false);
    save('silentNode', { waitedMs: waited, opened });
    ok(opened && waited < 6500, `silent node: the app opened from the kept page after ${waited} ms (one wait, not one per file)`);
    // The app's own start waits for the node without a limit ("Connecting…", empty page): the page
    // itself must say, within a bounded time, that the node can't be reached…
    const noteText = async () => ((await page.locator('#pwanotice').isVisible().catch(() => false)) ? (await page.locator('#pwanotice').textContent()).replace(/\s+/g, ' ').trim() : '');
    let note = '';
    for (let i = 0; i < 24 && !/can.t reach your node/i.test(note); i++) { await sleep(250); note = await noteText(); }
    const shownAfter = Date.now() - t0;
    save('silentNodeNote', { note, shownAfterMs: shownAfter, conn: await page.locator('#conn').textContent().catch(() => '') });
    ok(/can.t reach your node/i.test(note) && shownAfter < 10000, `silent node: a visible note says it can't reach the node, ${shownAfter} ms after the launch: ${note}`);
    await page.screenshot({ path: path.join(OUT, 'desktop-silent-node.png') });
    // …and once the node answers again (its old connections still open, silent), say so and offer
    // to reconnect: nothing reloads by itself; Reconnect opens the app afresh
    await page.evaluate(() => { window.pwaMarker = 'opened from the copy'; });
    hang = false;
    let again = '';
    for (let i = 0; i < 40 && !/answers again/i.test(again); i++) { await sleep(500); again = await noteText(); }
    const stillSame = await page.evaluate(() => window.pwaMarker);
    ok(/answers again/i.test(again) && await page.locator('#pwareconnect').isVisible() && stillSame === 'opened from the copy', 'silent node back: the note says the node answers again and offers Reconnect, without reloading by itself: ' + again);
    await page.screenshot({ path: path.join(OUT, 'desktop-silent-node-back.png') });
    if (await page.locator('#pwareconnect').isVisible()) await Promise.all([page.waitForEvent('load', { timeout: 15000 }).catch(() => {}), page.locator('#pwareconnect').click()]);
    const settled = async () => { let r = {}; for (let i = 0; i < 30; i++) { await sleep(500); r = await page.evaluate(() => ({ main: document.querySelector('#main').textContent.replace(/\s+/g, ' ').trim(), note: !!document.getElementById('pwanotice'), state: document.getElementById('conn').dataset.state })); if (r.state === 'connected' && r.main.length > 20 && !r.note) break; } return r; };
    const recovered = await settled();
    save('silentNodeRecovered', { ...recovered, main: recovered.main.slice(0, 120), stuckConnections: hung.length });
    ok(recovered.state === 'connected' && recovered.main.length > 20 && !recovered.note, 'silent node back: Reconnect opens the app afresh, connected, the note gone: ' + recovered.main.slice(0, 60));
    // A host that comes back resets the connections it no longer knows: the app's own retry then
    // gets through, and the note gives way to the app's connection state (no click, no reload)
    hang = true;
    await page.goto(VIA + '/albums', { waitUntil: 'load', timeout: 20000 }).catch(() => {});
    note = ''; for (let i = 0; i < 24 && !/can.t reach your node/i.test(note); i++) { await sleep(250); note = await noteText(); }
    await page.evaluate(() => { window.pwaMarker = 'reset path'; });
    hang = false; hung.splice(0).forEach((sock) => sock.destroy());
    const byItself = await settled();
    const same2 = await page.evaluate(() => window.pwaMarker);
    save('silentNodeReset', { noteBefore: note, ...byItself, main: byItself.main.slice(0, 120), reloaded: same2 !== 'reset path' });
    ok(/can.t reach your node/i.test(note) && byItself.state === 'connected' && byItself.main.length > 20 && !byItself.note && same2 === 'reset path', 'silent node, reset connections: the app reconnects by itself, the note gives way, no reload: ' + byItself.main.slice(0, 60));
    hung.splice(0).forEach((sock) => sock.destroy());
    await ctx.close();
    proxy.closeAllConnections && proxy.closeAllConnections();
    proxy.close();
  }

  // 5b. A proxy or tunnel in front of a node that is away answers with its own 5xx (Cloudflare: 520–530):
  // the kept page opens instead of that error page. A sign-in answer (401/403) is passed on as it is.
  {
    let mode = 0; // 0: pass through; else answer everything with this status
    const up = new URL(BASE);
    const proxy = http.createServer((req, res) => {
      if (mode) { res.writeHead(mode, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' }); res.end(`<!doctype html><title>Error</title><p>EDGE-ANSWER-${mode}</p>`); return; }
      const out = http.request({ host: up.hostname, port: up.port, path: req.url, method: req.method, headers: req.headers }, (r) => { res.writeHead(r.statusCode, r.headers); r.pipe(res); });
      out.on('error', () => { res.writeHead(502); res.end(); });
      req.pipe(out);
    });
    await new Promise((r) => proxy.listen(0, '127.0.0.1', r));
    const VIA = `http://127.0.0.1:${proxy.address().port}`;
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    await ctx.addInitScript((t) => { try { localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
    const page = await ctx.newPage();
    await page.goto(VIA + '/', { waitUntil: 'load' }); await swReady(page);
    await page.reload({ waitUntil: 'load' }); await sleep(1000);
    const seen = {};
    for (const status of [500, 502, 520, 521, 522, 523, 524, 530]) {
      mode = status;
      const t0 = Date.now();
      await page.goto(VIA + '/albums', { waitUntil: 'load', timeout: 15000 }).catch(() => {});
      // (the app may quote the edge's answer to its API calls in its own notice: what counts is which page this is)
      const doc = await page.evaluate(() => ({ title: document.title, topbar: !!document.querySelector('.topbar') })).catch(() => ({}));
      seen[status] = { ms: Date.now() - t0, keptPage: !!doc.topbar, edgePage: doc.title === 'Error' };
    }
    save('edge5xx', seen);
    ok(Object.values(seen).every((v) => v.keptPage && !v.edgePage), 'proxy 5xx (500, 502, 520–524, 530): the kept page opens, not the edge\'s error page ' + JSON.stringify(seen));
    const auth = {};
    for (const status of [401, 403]) {
      mode = status;
      await page.goto(VIA + '/albums', { waitUntil: 'load', timeout: 15000 }).catch(() => {});
      const doc = await page.evaluate(() => ({ title: document.title, topbar: !!document.querySelector('.topbar'), text: document.body.textContent })).catch(() => ({}));
      auth[status] = { edgePage: doc.title === 'Error' && (doc.text || '').includes('EDGE-ANSWER-' + status), keptPage: !!doc.topbar };
    }
    save('edgeAuth', auth);
    ok(Object.values(auth).every((v) => v.edgePage && !v.keptPage), 'proxy 401/403: its answer is shown as it is, never replaced by the kept page ' + JSON.stringify(auth));
    mode = 0;
    await page.goto(VIA + '/albums', { waitUntil: 'load' }); await sleep(1500);
    const albums = await page.evaluate(() => document.querySelector('#main').textContent.replace(/\s+/g, ' ').trim());
    ok(albums.length > 20 && !/cannot reach|went wrong/i.test(albums), 'proxy back to normal: the node\'s albums show again: ' + albums.slice(0, 60));
    await ctx.close();
    proxy.closeAllConnections && proxy.closeAllConnections(); proxy.close();
  }

  await launchSafety(browser);

  // 6. Inside the desktop app: no install button, no worker
  {
    const ctx = await browser.newContext();
    await ctx.addInitScript(() => { window.pmDesktop = { version: 'test', platform: 'linux' }; });
    const page = await ctx.newPage();
    await page.goto(BASE + '/', { waitUntil: 'load' }); await sleep(800);
    ok(!(await page.locator('#installbtn').count()), 'desktop app: no Install app button');
    ok(!(await page.evaluate(async () => (await navigator.serviceWorker.getRegistrations()).length)), 'desktop app: no service worker registered');
    await ctx.close();
  }

  // 6b. The Android app's WebView where its early script did not run (no window.pmDesktop): its user
  // agent still says "; wv)". A fake user agent: this checks the page's reaction, not a device.
  {
    const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, userAgent: 'Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/AP2A; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/131.0.0.0 Mobile Safari/537.36' });
    const page = await ctx.newPage();
    await page.goto(BASE + '/', { waitUntil: 'load' }); await sleep(1000);
    const r = await page.evaluate(async () => ({ buttons: document.querySelectorAll('#installbtn, #installlogin').length, workers: (await navigator.serviceWorker.getRegistrations()).length }));
    save('androidWebViewUa', r);
    ok(r.buttons === 0 && r.workers === 0, 'Android WebView user agent without the app bridge: no Install app action, no worker ' + JSON.stringify(r));
    await ctx.close();
  }

  // 7. Fakes, for the UI only: the browser's prompt on a click, used once; gone once installed
  {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    await page.goto(BASE + '/', { waitUntil: 'load' }); await sleep(500);
    const r = await page.evaluate(async () => {
      const e = new Event('beforeinstallprompt', { cancelable: true }); let n = 0; e.prompt = async () => { n++; return { outcome: 'accepted' }; };
      dispatchEvent(e); const btn = document.getElementById('installbtn'); btn.click(); await new Promise((r) => setTimeout(r, 100));
      return { prevented: e.defaultPrevented, prompts: n, hidden: btn.hidden };
    });
    save('fakePromptUiOnly', r);
    ok(r.prevented && r.prompts === 1 && r.hidden, 'fake prompt (UI only): held back, shown on the click, button gone after accept ' + JSON.stringify(r));
    await ctx.close();
  }

  // Simulated installed window (display-mode: standalone): no install button, the worker still
  // serves the app. A real app window can't be opened headless: this checks the page's reaction.
  {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    await ctx.addInitScript(() => { const mm = window.matchMedia.bind(window); window.matchMedia = (q) => (/display-mode: standalone/.test(q) ? { matches: true, media: q, addEventListener() {}, removeEventListener() {} } : mm(q)); });
    const page = await ctx.newPage();
    await page.goto(BASE + '/', { waitUntil: 'load' }); await sleep(800);
    await swReady(page);
    const regs = await page.evaluate(async () => (await navigator.serviceWorker.getRegistrations()).length);
    ok(!(await page.locator('#installbtn').isVisible()), 'standalone (simulated): no Install app button');
    ok(regs === 1, 'standalone (simulated): the worker is registered for the app window');
    await ctx.close();
  }

  // 7. The approved icons, as the page, the manifest and the sidebar use them
  {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 });
    await ctx.addInitScript((t) => { try { localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
    const page = await ctx.newPage();
    await page.goto(BASE + '/', { waitUntil: 'load' }); await sleep(1000);
    const links = await page.evaluate(() => ({ icon: document.querySelector('link[rel="icon"]').getAttribute('href'), apple: document.querySelector('link[rel="apple-touch-icon"]').getAttribute('href'), manifest: document.querySelector('link[rel="manifest"]').getAttribute('href') }));
    const man = JSON.parse(await (await page.request.get(BASE + links.manifest)).text());
    const want = (rel) => BRAND.installed[`crates/pm-api/static${rel}`];
    const served = {};
    for (const src of [links.icon, links.apple, ...man.icons.map((i) => i.src)]) { const r = await page.request.get(BASE + src); served[src] = { status: r.status(), type: r.headers()['content-type'], cacheControl: r.headers()['cache-control'], sha256: sha256(await r.body()) }; }
    save('icons', { links, served, approved: BRAND.installed });
    ok(Object.entries(served).every(([src, v]) => v.status === 200 && want(src) && v.sha256 === want(src).sha256), 'icons: favicon, Apple icon and every manifest icon the node serves are the approved files ' + JSON.stringify(Object.fromEntries(Object.entries(served).map(([k, v]) => [k, v.sha256.slice(0, 12)]))));
    // Fixed file names whose picture changed (the new logo): browsers must ask again, not keep a day-old icon
    ok(Object.values(served).every((v) => v.cacheControl === 'no-cache'), 'icons: every icon is served no-cache, so a changed logo shows at once ' + JSON.stringify(Object.fromEntries(Object.entries(served).map(([k, v]) => [k, v.cacheControl]))));
    const brand = await page.evaluate(() => { const m = document.querySelector('.sidebar .brand-mark'); const cs = getComputedStyle(m); const svg = m.querySelector('svg'); const b = svg ? svg.getBoundingClientRect() : { width: 0, height: 0 }; return { use: m.querySelector('use') && m.querySelector('use').getAttribute('href'), color: cs.color, background: cs.backgroundImage, w: b.width, h: b.height, grooves: document.querySelectorAll('#i-odeion path').length }; });
    save('sidebarMark', brand);
    ok(brand.use === '#i-odeion' && brand.grooves === 3 && brand.background === 'none' && brand.color === 'rgb(164, 150, 255)' && brand.w >= 20, 'sidebar: the Odeion mark, three grooves in #a496ff ' + JSON.stringify(brand));
    await page.locator('.sidebar .brand').screenshot({ path: path.join(OUT, 'desktop-sidebar-mark.png') });
    await ctx.close();
  }

  // 8. Behind a same-origin sign-in proxy (synthetic): the worker keeps nothing private
  {
    const tag = Math.random().toString(36).slice(2, 10);
    const CANARY = { login: `LOGIN-CANARY-${tag}`, priv: `PRIVATE-CANARY-${tag}`, query: `QUERY-CANARY-${tag}`, redirect: `REDIRECT-CANARY-${tag}` };
    let down = false;
    const up = new URL(BASE);
    const signIn = (res, extra = '') => { res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' }); res.end(`<!doctype html><html><head><title>Sign in</title></head><body><p>${CANARY.login} ${extra}</p></body></html>`); };
    let edge = null; // 'hang': the proxy never answers; a number: it answers everything with that status
    const stuck = [];
    const proxy = http.createServer((req, res) => {
      if (down) { req.socket.destroy(); return; }
      if (edge === 'hang') { stuck.push(req.socket); return; }
      if (edge) { res.writeHead(edge, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' }); res.end(`<!doctype html><title>Error</title><p>EDGE-ANSWER-${edge}</p>`); return; }
      const u = new URL(req.url, 'http://proxy');
      if (u.searchParams.has('ticket')) return signIn(res, CANARY.redirect); // where its redirect lands
      if (!/(?:^|;\s*)pwaauth=1/.test(req.headers.cookie || '')) return signIn(res); // no cookie: its sign-in page, for everything
      if (u.pathname === '/library') { res.writeHead(302, { location: `/?next=%2Flibrary&ticket=${CANARY.redirect}` }); res.end(); return; }
      const html = req.method === 'GET' && /text\/html/.test(req.headers.accept || '');
      const out = http.request({ host: up.hostname, port: up.port, path: req.url, method: req.method, headers: req.headers }, (r) => {
        if (!html || !/text\/html/.test(r.headers['content-type'] || '')) { res.writeHead(r.statusCode, r.headers); r.pipe(res); return; }
        const parts = []; r.on('data', (d) => parts.push(d)); r.on('end', () => {
          // A signed-in page, personalised by the proxy and marked private
          const body = Buffer.from(Buffer.concat(parts).toString('utf8').replace('</body>', `<!-- ${CANARY.priv} user=alice -->\n</body>`));
          const h = { ...r.headers, 'content-length': body.length, 'cache-control': 'private, no-store' }; res.writeHead(r.statusCode, h); res.end(body);
        });
      });
      out.on('error', () => { res.writeHead(502); res.end(); });
      req.pipe(out);
    });
    await new Promise((r) => proxy.listen(0, '127.0.0.1', r));
    const VIA = `http://127.0.0.1:${proxy.address().port}`;
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    await ctx.addCookies([{ name: 'pwaauth', value: '1', url: VIA }]);
    await ctx.addInitScript((t) => { try { localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
    const page = await ctx.newPage();
    const errors = []; page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(VIA + '/', { waitUntil: 'load' });
    await swReady(page);
    await page.reload({ waitUntil: 'load' }); await sleep(800);
    const deep = await page.goto(VIA + `/albums?sso=${CANARY.query}&token=${CANARY.query}#x`, { waitUntil: 'load' }); await sleep(800);
    ok(deep && deep.status() === 200 && new URL(deep.url()).search.includes(CANARY.query), 'proxy: a signed-in deep link with a credential in its query was answered (the scenario ran)');
    await page.goto(VIA + '/library', { waitUntil: 'load' }); await sleep(800);
    const landed = await page.evaluate(() => document.body.textContent);
    ok(landed.includes(CANARY.redirect), 'proxy: its redirect landed on its sign-in page (the scenario ran)');
    await sleep(1500);
    const all = await kept(page);
    const leaks = all.filter((k) => Object.values(CANARY).some((c) => k.body.includes(c) || k.path.includes(c)));
    save('authProxy', { kept: all.map((k) => ({ path: k.path, url: k.url, responseType: k.responseType, redirected: k.redirected, type: k.type, cacheControl: k.cacheControl, bytes: k.body.length })), leaks: leaks.map((k) => k.path) });
    ok(all.every((k) => k.url === VIA + k.path && k.responseType === 'basic' && k.redirected === false && !Object.values(CANARY).some((c) => k.url.includes(c))), 'proxy: every kept response URL is the plain path, type basic, not redirected');
    ok(leaks.length === 0, 'proxy: no sign-in page, private page, redirect or query kept ' + JSON.stringify(leaks.map((k) => [k.path, k.type])));
    ok(all.every((k) => !/no-store|private/i.test(k.cacheControl) && k.type.split(';')[0].trim() === TYPE(k.path)), 'proxy: whatever is kept has its own type and no private/no-store');
    // Installing behind that proxy: Chromium itself reads the manifest (it needs the proxy's cookie)
    // and finds nothing in the way. The worker still keeps nothing there (above).
    const pv = await verdict(VIA + '/', [{ name: 'pwaauth', value: '1', url: VIA }]);
    const pm = JSON.parse(pv.man.data || '{}');
    save('authProxyInstall', { manifestUrl: pv.man.url, manifestErrors: pv.man.errors, manifestName: pm.name || null, installabilityErrors: pv.inst, primaryIconBase64Length: pv.icon, realBeforeInstallPrompt: pv.prompts });
    ok(pv.man.url === VIA + '/manifest.webmanifest' && Array.isArray(pv.man.errors) && pv.man.errors.length === 0 && pm.name === 'Odeion', 'cookie sign-in proxy: Chromium reads the Odeion manifest with the proxy cookie ' + JSON.stringify({ errors: pv.man.errors, name: pm.name }));
    ok(Array.isArray(pv.inst) && pv.inst.length === 0 && pv.icon > 0, 'cookie sign-in proxy: Chromium reports no installability errors and picks an icon ' + JSON.stringify(pv.inst));
    // Nothing public could be kept behind this proxy (no copy of the page). A launch must still end
    // in the generic "can't reach" page, in bounded time, when the proxy is silent or answers 5xx;
    // its 401/403 is shown as it is.
    ok(!all.some((k) => k.path === '/'), 'proxy: no kept page here (the precondition for the next checks)');
    const launch = async (mode) => {
      edge = mode; const t0 = Date.now();
      await page.goto(VIA + '/albums', { waitUntil: 'load', timeout: 20000 }).catch(() => {});
      // (bounded: a navigation that never ends must fail this check, not stall the run)
      const doc = await Promise.race([page.evaluate(() => ({ title: document.title, h1: (document.querySelector('h1') || {}).textContent || '', text: document.body.textContent })), sleep(3000).then(() => ({}))]).catch(() => ({}));
      edge = null; stuck.splice(0).forEach((sock) => sock.destroy()); // (its silent connections end, or they would hold the browser's connection slots)
      return { ms: Date.now() - t0, offlinePage: /can.t reach your node/i.test(doc.h1 || ''), edgePage: (doc.text || '').includes('EDGE-ANSWER-') };
    };
    const noCopy = { hang: await launch('hang'), s530: await launch(530), s502: await launch(502), s401: await launch(401) };
    stuck.splice(0).forEach((sock) => sock.destroy());
    save('authProxyNoCopy', noCopy);
    ok(noCopy.hang.offlinePage && noCopy.hang.ms < 7000, `proxy silent, no kept page: the "can't reach your node" page after ${noCopy.hang.ms} ms, not an endless wait`);
    ok(noCopy.s530.offlinePage && !noCopy.s530.edgePage && noCopy.s502.offlinePage && !noCopy.s502.edgePage, 'proxy 530/502, no kept page: the generic "can\'t reach" page, not the proxy\'s error page ' + JSON.stringify({ s530: noCopy.s530, s502: noCopy.s502 }));
    ok(noCopy.s401.edgePage && !noCopy.s401.offlinePage, 'proxy 401, no kept page: its answer is shown as it is');
    await page.screenshot({ path: path.join(OUT, 'desktop-auth-proxy-silent-no-copy.png') });
    // The proxy and node out of reach: the next launch shows no one's page
    down = true;
    await page.goto(VIA + '/albums', { waitUntil: 'load' }).catch(() => {}); await sleep(1500);
    const shown = await page.evaluate(() => document.documentElement.outerHTML).catch(() => '');
    save('authProxyDown', { url: page.url(), excerpt: shown.replace(/\s+/g, ' ').slice(0, 300) });
    ok(!Object.values(CANARY).some((c) => shown.includes(c)), 'proxy down: the launch replays no sign-in or private page');
    await page.screenshot({ path: path.join(OUT, 'desktop-auth-proxy-down.png') });
    await ctx.close();
    proxy.closeAllConnections && proxy.closeAllConnections(); proxy.close();
  }

  // 9. Switching accounts: the owner logs out, a listener logs in
  if (DATA) {
    const { DatabaseSync } = require('node:sqlite');
    const { blake3 } = require(require.resolve('@noble/hashes/blake3', { paths: [process.cwd(), __dirname] }));
    const KEY = 'tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy'; // a test key, as in pm-api's tests
    const B = 'pml_pwafixturelistener' + Math.random().toString(36).slice(2);
    const hex = (u) => Buffer.from(u).toString('hex');
    await stopNode();
    const db = new DatabaseSync(path.join(DATA, 'library.db'));
    const put = db.prepare('INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value');
    put.run('people', JSON.stringify({ people: [{ key: KEY, access: 'music', music_enabled: true, added_at: 1 }] }));
    put.run('accounts.tokens', JSON.stringify([{ hash: hex(blake3(Buffer.from(B))), key: KEY, created_at: Math.floor(Date.now() / 1000) }]));
    db.close();
    await startNode();
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    await ctx.addInitScript((t) => { try { if (!sessionStorage.getItem('pwa.switched')) localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
    const page = await ctx.newPage();
    const errors = []; page.on('pageerror', (e) => errors.push(e.message));
    const auth = []; page.on('request', (q) => { const a = q.headers().authorization; if (a) auth.push({ path: new URL(q.url()).pathname, who: a === 'Bearer ' + TOKEN ? 'A' : a === 'Bearer ' + B ? 'B' : 'other' }); });
    await page.goto(BASE + '/', { waitUntil: 'load' }); await swReady(page); await sleep(800);
    for (const v of ['albums', 'settings']) { await page.goto(BASE + '/' + v, { waitUntil: 'load' }); await sleep(700); }
    const meA = await page.evaluate(async () => (await fetch('/v1/me', { headers: { authorization: 'Bearer ' + localStorage.getItem('pm.token') } })).json());
    // A logs out
    await page.evaluate(() => sessionStorage.setItem('pwa.switched', '1'));
    await page.locator('#me').click(); await sleep(300);
    await page.locator('.popover .pop-item', { hasText: 'Log out' }).first().click(); await sleep(1500);
    ok(await page.locator('#loginpage').isVisible(), 'switch: after A logs out the page asks to log in');
    const mark = auth.length;
    // B logs in with their token
    await page.evaluate(() => { document.getElementById('logintoken').open = true; });
    await page.locator('#tok').fill(B);
    await page.locator('#tokform button[type="submit"]').click(); await sleep(2000);
    await page.goto(BASE + '/albums', { waitUntil: 'load' }); await sleep(1500);
    const meB = await page.evaluate(async () => (await fetch('/v1/me', { headers: { authorization: 'Bearer ' + localStorage.getItem('pm.token') } })).json());
    const stored = await page.evaluate(() => localStorage.getItem('pm.token'));
    const meTitle = await page.locator('#me').getAttribute('title');
    await page.locator('#me').click(); await sleep(300);
    const menu = (await page.locator('.popover:visible').first().textContent().catch(() => '')).replace(/\s+/g, ' ');
    await page.keyboard.press('Escape');
    const afterB = auth.slice(mark);
    const all = await kept(page);
    save('accountSwitch', { meA: { role: meA.role }, meB: { role: meB.role, key: meB.key }, meTitle, menu: menu.slice(0, 160), requestsAfterSwitch: afterB.length, byWho: afterB.reduce((m, x) => ({ ...m, [x.who]: (m[x.who] || 0) + 1 }), {}), kept: all.map((k) => k.path) });
    ok(meA.role === 'owner', 'switch: A was the owner');
    ok(stored === B && meB.role === 'listener' && meB.key === KEY, 'switch: B is signed in as the listener ' + JSON.stringify({ role: meB.role }));
    ok(/listening here/.test(menu) && !/this node.s owner/.test(menu), 'switch: the account menu names B as a listener, not the owner: ' + menu.slice(0, 120));
    ok(afterB.length > 0 && afterB.every((x) => x.who === 'B'), 'switch: every request after the switch carries B\'s token, none A\'s ' + JSON.stringify(afterB.filter((x) => x.who !== 'B').slice(0, 3)));
    ok(all.every((k) => SHELL.has(k.path)) && !all.some((k) => k.body.includes(TOKEN) || k.body.includes(B)), 'switch: the worker kept only public files, neither token');
    ok(!errors.length, 'switch: no page errors ' + errors.join('; '));
    await page.screenshot({ path: path.join(OUT, 'desktop-switched-to-listener.png') });
    await ctx.close();
  }

  // Plain HTTP on the LAN: Chromium itself refuses to install; the help says why
  if (LAN) {
    const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true, deviceScaleFactor: 2 });
    await ctx.addInitScript((t) => { try { localStorage.setItem('pm.token', t); } catch { /* */ } }, TOKEN);
    const page = await ctx.newPage();
    await page.goto(LAN + '/', { waitUntil: 'load' }); await sleep(1500);
    const v = await verdict(LAN + '/');
    save('lan', { url: LAN, isSecureContext: v.secure, installabilityErrors: v.inst, profile: v.profile });
    ok(!v.secure, 'LAN: Chromium treats ' + LAN + ' as not secure');
    ok(v.inst.some((e) => e.errorId === 'not-from-secure-origin'), 'LAN: Chromium reports it not installable: ' + JSON.stringify(v.inst));
    ok(!(await page.evaluate(() => !!navigator.serviceWorker)), 'LAN: no service worker API, none registered');
    await page.locator('#installbtn').click(); await sleep(300);
    const text = (await page.locator('#installhelp').textContent()).replace(/\s+/g, ' ');
    ok(/HTTPS/.test(text) && text.includes(new URL(LAN).host), 'LAN: the help names HTTPS and this address');
    await page.screenshot({ path: path.join(OUT, 'phone-lan-http-help.png') });
    await ctx.close();
  }

  await browser.close();
  await stopNode();
  save('summary', { checks, failed: failures.length, failures });
  console.log(`\n${checks - failures.length}/${checks} browser checks passed (Chromium ${version})`);
  process.exit(failures.length ? 1 : 0);
})().catch(async (e) => { console.error(e); await stopNode(); process.exit(1); });
