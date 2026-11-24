// Back from a music service's sign-in page. TIDAL's and Spotify's callbacks send the browser to the
// node's Home with ?tidal=… / ?spotify=…; once the first page is drawn the page opens that service's
// Sources panel (its sign-in said "comes back here"), signed in or, after a failed sign-in, with the
// error toast and the panel's Sign in button to try again. Before, Spotify's (both ways) and TIDAL's
// failed sign-in stayed on Home with only the toast. Checked at 360/412 px (touch; the plain web UI
// and with the Android app's pmDesktop stub, the WebView shell's rules) and 1280 px, then once more at
// 360 px with a slow node (startup answers late) and as a listener (no Sources: their Settings, where
// My streaming accounts is, opens instead):
//   * the service's panel is shown at /sources/<service>, and the ?… is gone from the address;
//   * the toast says signed in, or why it failed, and a failed one's panel shows its Sign in button;
//   * a failed one's panel also says why, in a line above the way to try again ("Last sign-in failed:
//     …"), which stays after the 6 s toast, on a later visit (in this page load) and when another
//     button of the sign-in is pressed (Spotify's Change), and goes once Sign in is pressed there
//     (before, the reason was only in the toast); a sign-in that worked shows no such line; with no
//     reason given, the line says so plainly;
//   * Jamendo's callback (its own panel, /source/jamendo?jamendo=error…) says why the same way;
//   * Back returns to Home (the panel is one step over it; before, Back from Home left the app);
//   * no page errors.
// Real candidate assets on the fixture origin (tools/ui-fixture.js), no live node.
//   node tools/ui-signin-return.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'signin-return-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const sources = [{ id: 'tidal', name: 'TIDAL', description: '', status: 'ready', enabled: true }, { id: 'spotify', name: 'Spotify', description: '', status: 'ready', enabled: true }, { id: 'jamendo', name: 'Jamendo', description: '', status: 'ready', enabled: true }];
// The service's sign-in ways (GET /v1/sources/<service>/signin): its own app, signed in or not
// (Spotify's app set in Settings: its step 1 has Change and Forget, buttons that are not a new try)
const signin = (ok) => ({ ways: [{ id: 'app', label: 'Odeion app', available: true, recommended: true }], in_use: ok ? { way: 'app' } : null, details: { status: { configured: true, signed_in: ok, user_id: ok ? 'fixture' : null, app_from: 'settings', client_id: 'fixture-client' } } });
// A listener's own streaming accounts (Settings › My streaming accounts)
const mine = (svc, ok) => [{ source: 'tidal', name: 'TIDAL', connected: svc === 'tidal' && ok, available: true, setup: '/v1/sources/tidal/signin' }, { source: 'spotify', name: 'Spotify', connected: svc === 'spotify' && ok, available: true, setup: '/v1/sources/spotify/signin' }];
const fixturesFor = (svc, ok) => ({ '/sources': { body: sources }, [`/sources/${svc}/signin`]: { body: signin(ok) }, '/me/services': { body: mine(svc, ok) }, '/sources/jamendo/status': { body: { configured: true, signed_in: false, can_sign_in: true } } });
// The panel's "Last sign-in failed" line: its text, and whether it is in the panel above the given button
const failNote = (page, svc, button) => page.evaluate(([s, b]) => {
  const n = document.querySelector(`#src-${s} .src-failed`); if (!n || !n.getClientRects().length) return null;
  const btn = document.querySelector(b); const r = n.getBoundingClientRect();
  return { text: n.textContent.trim(), above: !!btn && r.bottom <= btn.getBoundingClientRect().top, inside: r.left >= 0 && r.right <= innerWidth };
}, [svc, button]);
// [service, query, toast it shows]; a failed one shows the panel's Sign in button (#spotin / #tidalin)
const CASES = [
  ['spotify', '?spotify=ok', /Spotify signed in/],
  ['spotify', '?spotify=error&message=Spotify%20said%20no', /Spotify: Spotify said no/],
  ['tidal', '?tidal=signed_in', /TIDAL signed in/],
  ['tidal', '?tidal=error&detail=TIDAL%20said%20no', /TIDAL sign-in failed: TIDAL said no/],
  ['tidal', '?tidal=error', /TIDAL sign-in failed: error/, 'TIDAL gave no reason'],
];
const APP = { platform: 'android', version: '0.0.0' };
// [shell, width, pmDesktop stub, extra fixture options]
const SHELLS = [['web', 360, null], ['web', 412, null], ['android', 360, APP], ['android', 412, APP], ['web', 1280, null],
  ['slow node', 360, null, { delay: (v) => (['/info', '/me', '/zones'].includes(v) ? 700 : 0) }], ['listener', 360, null, { role: 'listener' }]];

(async () => {
  const browser = await chromium.launch();
  for (const [shell, width, desktop, more] of SHELLS) {
    const listener = !!(more && more.role === 'listener');
    for (const [svc, query, said, plain] of CASES) {
      const at = `${shell} ${width} px, back with ${query.split('&')[0]}`;
      const ok = !query.includes('error');
      const { page, context, state } = await fx.create(browser, width, { touch: width < 600, fixtures: fixturesFor(svc, ok), ...(more || {}) });
      if (desktop) await context.addInitScript((d) => { window.pmDesktop = d; }, desktop);
      await fx.go(page, '/' + query);
      const where = listener ? '/settings' : `/sources/${svc}`;
      const shown = listener ? '#main .svc-own' : `#src-${svc}`;
      await page.waitForFunction(([w, sel]) => location.pathname === w && !!document.querySelector(sel), [where, shown], { timeout: 6000 }).catch(() => {});
      const now = await page.evaluate(([sel]) => ({ path: location.pathname, search: location.search, panel: !!document.querySelector(sel), toast: (document.querySelector('#toast') || {}).textContent || '' }), [shown]);
      check(now.path === where && now.panel, `${at}: ${listener ? 'Settings with My streaming accounts' : `the ${svc} panel`} is open (${now.path}, shown ${now.panel})`);
      check(now.search === '', `${at}: the address has no ?… left (${now.search})`);
      check(said.test(now.toast), `${at}: the toast says ${said} (${now.toast.trim()})`);
      const again = svc === 'spotify' ? '#spotin' : '#tidalin';
      const why = (query.match(/(?:message|detail)=([^&]*)/) || [])[1];
      if (!ok && !listener) {
        await page.waitForSelector(again, { timeout: 3000 }).catch(() => {});
        check(await page.$eval(again, (b) => !b.disabled && b.getBoundingClientRect().height > 0).catch(() => false), `${at}: the panel's Sign in button is there to try again (${again})`);
        const n = await failNote(page, svc, again);
        check(!!n && n.text === `Last sign-in failed: ${plain || decodeURIComponent(why)}` && n.above && n.inside, `${at}: the panel says why, above Sign in (${JSON.stringify(n)})`);
        // (once: the line doesn't go with the toast)
        if (shell === 'web' && width === 360) {
          await page.waitForFunction(() => !document.querySelector('#toast.show'), null, { timeout: 9000 }).catch(() => {});
          const gone = await page.evaluate(() => !document.querySelector('#toast.show'));
          const still = await failNote(page, svc, again);
          check(gone && !!still, `${at}: the toast is gone, the panel's line stays (toast gone ${gone}, line ${!!still})`);
        }
      }
      if (ok && !listener) check(!(await failNote(page, svc, again)), `${at}: no "Last sign-in failed" line on the panel`);
      if (width === 360 || width === 1280) await page.screenshot({ path: path.join(SHOTS, `${svc}-${ok ? 'ok' : 'error'}-${shell.replace(' ', '-')}-${width}.png`) });
      await page.goBack();
      // A listener's Home is their Albums (the same page, its address replaced)
      const home = listener ? ['/', '/albums'] : ['/'];
      await page.waitForFunction((h) => location.origin === 'http://odeion-fixture.test' && h.includes(location.pathname) && !!document.querySelector('#main > :not(.navskel)'), home, { timeout: 3000 }).catch(() => {});
      const back = await page.evaluate(() => ({ url: location.href, drawn: !!document.querySelector('#main > :not(.navskel)') }));
      check(home.some((h) => back.url === fx.ORIGIN + h) && back.drawn, `${at}: Back returns to Home, in the app (${back.url})`);
      // A later visit still says why; pressing Sign in there (a new try) puts the line away
      if (!ok && !listener) {
        await page.goForward();
        await page.waitForSelector(again, { timeout: 3000 }).catch(() => {});
        const n = await failNote(page, svc, again);
        check(!!n, `${at}: back on the panel, it still says why (${n && n.text})`);
        if (svc === 'spotify') {
          await page.click('#spotappchange', { noWaitAfter: true }).catch(() => {});
          await page.waitForTimeout(300);
          check(!!(await failNote(page, svc, again)), `${at}: pressing Change (Spotify's app, not a new try) keeps the line`);
          await page.waitForSelector(again, { timeout: 3000 }).catch(() => {});
        }
        await page.click(again, { noWaitAfter: true }).catch(() => {});
        await page.waitForTimeout(300);
        check(!(await failNote(page, svc, again)), `${at}: pressing Sign in there puts the line away`);
      }
      check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
      await context.close();
    }
  }
  // Jamendo comes back to its own panel: a failed sign-in says why there too, until Sign in is pressed
  for (const [shell, width, desktop] of SHELLS.filter(([sh]) => sh === 'web' || sh === 'android')) {
    const at = `${shell} ${width} px, back with ?jamendo=error`;
    const { page, context, state } = await fx.create(browser, width, { touch: width < 600, fixtures: fixturesFor('jamendo', false) });
    if (desktop) await context.addInitScript((d) => { window.pmDesktop = d; }, desktop);
    await fx.go(page, '/source/jamendo?jamendo=error&message=Jamendo%20said%20no');
    await page.waitForSelector('#src-jamendo #jamin', { timeout: 6000 }).catch(() => {});
    const n = await failNote(page, 'jamendo', '#jamin');
    check(!!n && n.text === 'Last sign-in failed: Jamendo said no' && n.above && n.inside, `${at}: the Jamendo panel says why, above Sign in (${JSON.stringify(n)})`);
    await page.waitForFunction(() => /Jamendo sign-in failed: Jamendo said no/.test(document.querySelector('#toast').textContent), null, { timeout: 3000 }).catch(() => {});
    check(/Jamendo sign-in failed: Jamendo said no/.test(await page.textContent('#toast')), `${at}: the toast still says it too`);
    if (width === 360) await page.screenshot({ path: path.join(SHOTS, `jamendo-error-${shell}-${width}.png`) });
    await page.click('#jamin', { noWaitAfter: true }).catch(() => {});
    await page.waitForTimeout(300);
    check(!(await failNote(page, 'jamendo', '#jamin')), `${at}: pressing Sign in to Jamendo puts the line away`);
    check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAILED, ${passed} passed` : `all passed (${passed})`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
