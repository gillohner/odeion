// The guided setup pages for Qobuz and TIDAL full tracks (TASK-qobuz-tidal-setup-ux): each state
// of both at desktop and phone width. The node's answers are mocked here (no Qobuz or TIDAL account, no app
// credentials); pm-api's Rust tests cover the checks and sign-ins against mock servers.
//   node tools/ui-stream-setup.js <api token> <node URL> [screenshot directory]
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const SHOTS = process.argv[4];
const WARN = 'This uses a private API the service has not approved for other apps.';
// A Qobuz app secret is 32 characters; the splitter wants at least 8
const SECRET = 'abcdef0123456789abcdef0123456789';
const problems = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// Waits until `fn` is true (polling), at most `ms`
const until = async (fn, ms = 8000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(150); } };
const pillOf = (p, root) => p.locator(`${root} .set-name .pill`).first().textContent().then((t) => t.trim()).catch(() => '');
// A paste into a field, as the browser would deliver it
const pasteInto = (p, sel, text) => p.evaluate(([s, t]) => { const el = document.querySelector(s); el.focus(); const dt = new DataTransfer(); dt.setData('text/plain', t); el.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true })); }, [sel, text]);

const qobuzMocks = async (p, log) => {
  const U = { enabled: false, signed_in: false, hires: false, quality: 'max', warning: WARN };
  const SI = { ways: [], imports: [], places: ['streamrip: ~/.config/streamrip/config.toml (not read)'], in_use: null, chosen: null, line: 'Not signed in', details: {} };
  await p.route(/\/v1\/sources\/qobuz\/unofficial$/, (r) => {
    if (r.request().method() === 'PUT') {
      const b = r.request().postDataJSON(); log.push(['PUT', b]);
      if (b.app_id !== undefined) { if (b.app_id) { U.app_from = 'settings'; U.app_id = b.app_id; } else { delete U.app_from; delete U.app_id; } }
      if (b.enabled !== undefined) { U.enabled = b.enabled; if (b.enabled) delete U.health; }
      if (b.quality) U.quality = b.quality;
    }
    return r.fulfill({ json: U });
  });
  await p.route(/\/v1\/sources\/qobuz\/unofficial\/app\/check$/, (r) => {
    const b = r.request().postDataJSON(); log.push(['CHECK', b]);
    const out = !/^\d+$/.test(b.app_id) ? { ok: false, detail: 'A Qobuz app ID is digits only' }
      : b.app_id !== '100000000' ? { ok: false, detail: "Qobuz doesn't know this app ID. Check it, or ask Qobuz for API access." }
      : b.app_secret !== SECRET ? { ok: false, detail: "Qobuz knows this app ID, but the secret doesn't go with it. Check the secret." }
      : { ok: true, detail: 'OK: Qobuz accepts this app ID; the secret is checked at the first play. Your Qobuz account is next.' };
    return r.fulfill({ json: out });
  });
  await p.route(/\/v1\/sources\/qobuz\/unofficial\/login$/, (r) => {
    if (r.request().method() === 'DELETE') { U.signed_in = false; delete U.user; delete U.plan; U.enabled = false; return r.fulfill({ json: U }); }
    const b = r.request().postDataJSON(); log.push(['LOGIN', b.email]);
    if (!U.app_from) return r.fulfill({ status: 412, json: { error: 'Enter a Qobuz app ID and secret first' } });
    if (b.email === 'free@example.com') return r.fulfill({ status: 403, json: { error: 'This Qobuz account has no streaming plan: full tracks need a paid Qobuz plan' } });
    if (b.password === 'wrong') return r.fulfill({ status: 401, json: { error: "Qobuz doesn't know that email and password. Check them and try again (Invalid Username or password)" } });
    Object.assign(U, { signed_in: true, user: 'Listener', plan: 'Studio', hires: true, enabled: true, health: { ok: true, detail: 'streams play', at: 1 } });
    return r.fulfill({ json: U });
  });
  await p.route(/\/v1\/sources\/qobuz\/signin(\/import)?$/, (r) => {
    if (r.request().method() === 'POST') {
      const b = r.request().postDataJSON(); log.push(['IMPORT', b.text]);
      if (b.text === 'not a sign-in') return r.fulfill({ status: 400, json: { error: 'Not usable: no app values and no sign-in in this' } });
      Object.assign(U, { app_from: 'import', app_id: '100000000', signed_in: true, user: 'listener', plan: 'Studio', hires: true, enabled: true }); delete U.health;
      SI.in_use = { way: 'import', label: 'Pasted config', user: 'listener', gives: 'hi-res' };
    }
    return r.fulfill({ json: SI });
  });
  return U;
};

const tidalMocks = async (p, log, si0) => {
  const U = { service: 'tidal', enabled: false, signed_in: false, quality: 'max', browser_available: true, warning: WARN };
  const si = structuredClone(si0); si.in_use = null; si.chosen = null; si.line = 'Not signed in';
  // As a release build answers: both built-in clients there, Sign in with TIDAL recommended
  si.ways.forEach((w) => { w.available = true; delete w.reason; w.recommended = w.id === 'browser'; });
  si.details = { status: { configured: false, signed_in: false }, unofficial: U };
  const T = { si, U, log };
  await p.route(/\/v1\/sources\/tidal\/signin$/, (r) => {
    if (r.request().method() === 'PUT') { si.chosen = r.request().postDataJSON().way; log.push(['WAY', si.chosen]); }
    if (r.request().method() === 'DELETE') { si.in_use = null; U.signed_in = false; U.enabled = false; }
    return r.fulfill({ json: si });
  });
  await p.route(/\/v1\/sources\/tidal\/unofficial$/, (r) => {
    if (r.request().method() === 'PUT') { const b = r.request().postDataJSON(); log.push(['PUT', b]); if (b.client_id) { U.client_from = 'settings'; si.ways.forEach((w) => { if (w.id === 'device') { w.available = true; delete w.reason; w.label = 'Your own device client'; } }); } else if (b.client_id === '') delete U.client_from; if (b.enabled !== undefined) U.enabled = b.enabled; }
    return r.fulfill({ json: U });
  });
  await p.route(/\/v1\/sources\/tidal\/unofficial\/client\/check$/, (r) => {
    const b = r.request().postDataJSON(); log.push(['CHECK', b]);
    const out = b.client_id === 'portal-app' ? { ok: false, detail: "This client can't use the code sign-in. Apps from TIDAL's developer portal are not device clients; enter a TIDAL device client instead." }
      : b.client_id !== 'dev-id' ? { ok: false, detail: "TIDAL doesn't know this client ID (401: invalid_client)" }
      : b.client_secret !== 'dev-secret' ? { ok: false, detail: "TIDAL knows this client ID, but the secret doesn't go with it. Check the secret." }
      : { ok: true, detail: 'OK: TIDAL accepts this device client. Sign in with a code next.' };
    return r.fulfill({ json: out });
  });
  // Sign in with TIDAL: the address to open, then the pasted address back
  await p.route(/\/v1\/sources\/tidal\/unofficial\/browser$/, (r) => {
    log.push(['BROWSER']); U.browser_pending = Math.floor(Date.now() / 1000) + 600;
    return r.fulfill({ json: { url: 'https://login.tidal.com/authorize?response_type=code&client_id=ui-test&code_challenge_method=S256', expires_at: U.browser_pending } });
  });
  await p.route(/\/v1\/sources\/tidal\/unofficial\/browser\/finish$/, (r) => {
    const b = r.request().postDataJSON(); log.push(['FINISH', b.text]);
    if (!/^https:\/\/tidal\.com\//.test(b.text)) return r.fulfill({ status: 400, json: { error: "That isn't a TIDAL address (evil.example). Paste the address of the tidal.com page you landed on after signing in." } });
    Object.assign(U, { signed_in: true, enabled: true, user: '77', session_client: 'pkce', health: { ok: true, detail: 'signed in', at: 1 } }); delete U.browser_pending;
    si.in_use = { way: 'browser', label: 'Sign in with TIDAL', user: '77', gives: 'full tracks, lossless' }; si.line = 'Sign in with TIDAL · full tracks'; si.chosen = 'browser';
    return r.fulfill({ json: U });
  });
  await p.route(/\/v1\/sources\/tidal\/unofficial\/login$/, (r) => {
    if (r.request().method() === 'DELETE') { U.signed_in = false; delete U.pending; U.enabled = false; si.in_use = null; return r.fulfill({ json: U }); }
    log.push(['LOGIN']);
    U.pending = { user_code: 'ABCDE', uri: 'https://link.tidal.com/ABCDE', expires_at: Math.floor(Date.now() / 1000) + (T.codeSeconds || 300) };
    return r.fulfill({ json: U });
  });
  return T;
};

(async () => {
  if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
  const r = await fetch(`${BASE}/v1/sources/tidal/signin`, { headers: { authorization: `Bearer ${TOKEN}` } });
  assert.equal(r.status, 200); const tidalSi = await r.json();
  const browser = await chromium.launch();
  try {
    for (const width of [1440, 390]) {
      const ctx = await browser.newContext({ viewport: { width, height: width === 390 ? 844 : 1000 }, reducedMotion: 'reduce' });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); }, TOKEN);
      const p = await ctx.newPage();
      const errors = []; p.on('pageerror', (e) => errors.push(e.message));
      const w = `${width}:`;
      const shot = async (name) => { if (SHOTS) await p.screenshot({ path: path.join(SHOTS, `${name}-${width}.png`), fullPage: true }); };

      // ---- Qobuz ----
      {
        const log = []; const U = await qobuzMocks(p, log);
        await p.goto(BASE + '/sources/qobuz');
        const root = '#signin-qobuz';
        await p.waitForSelector(`${root} .setup-step`, { timeout: 20000 });
        check(await pillOf(p, root) === 'Needs app', `${w} Qobuz: fresh, the status says it needs an app`);
        check(await p.locator(`${root} .setup-step[data-step="1"]:not(.done) #qbapp`).count() === 1 && await p.locator(`${root} .setup-step[data-step="2"] input[name=email]`).isDisabled(), `${w} Qobuz: step 1 open, step 2 waits for it`);
        check(/api@qobuz\.com/.test(await p.getAttribute(`${root} .setup-step[data-step="1"] a[href^="mailto:"]`, 'href')) && await p.locator(`${root} a[href*="guides/qobuz.md"]`).count() >= 1, `${w} Qobuz: says whom to ask, and links the guide`);
        check(await p.getAttribute(`${root} #qbapp input[name=secret]`, 'type') === 'password', `${w} Qobuz: the secret is hidden`);
        await p.click(`${root} #qbapp .showpw`);
        check(await p.getAttribute(`${root} #qbapp input[name=secret]`, 'type') === 'text' && await p.getAttribute(`${root} #qbapp .showpw`, 'aria-pressed') === 'true', `${w} Qobuz: Show shows it`);
        await p.click(`${root} #qbapp .showpw`);
        // A pasted block or JSON splits into the two fields
        await pasteInto(p, `${root} #qbapp input[name=id]`, `app_id = 100000000\napp_secret = ${SECRET}\n`);
        check(await p.inputValue(`${root} #qbapp input[name=id]`) === '100000000' && await p.inputValue(`${root} #qbapp input[name=secret]`) === SECRET, `${w} Qobuz: a pasted app_id=… app_secret=… block is split`);
        await p.fill(`${root} #qbapp input[name=id]`, ''); await p.fill(`${root} #qbapp input[name=secret]`, '');
        await pasteInto(p, `${root} #qbapp input[name=secret]`, `{"app_id": "100000000", "app_secret": "${SECRET}"}`);
        check(await p.inputValue(`${root} #qbapp input[name=id]`) === '100000000' && await p.inputValue(`${root} #qbapp input[name=secret]`) === SECRET, `${w} Qobuz: pasted JSON is split too`);
        // Check says the exact problem, or OK
        await p.fill(`${root} #qbapp input[name=secret]`, 'nope'); await p.click('#qbcheck');
        check(await until(async () => /secret doesn't go with it/.test(await p.textContent('#qbcheckout'))) && await p.$eval('#qbcheckout', (e) => e.classList.contains('bad')), `${w} Qobuz: Check names a wrong secret`);
        await p.fill(`${root} #qbapp input[name=id]`, '200000000'); await p.click('#qbcheck');
        check(await until(async () => /doesn't know this app ID/.test(await p.textContent('#qbcheckout'))), `${w} Qobuz: Check names an unknown app ID`);
        await p.fill(`${root} #qbapp input[name=id]`, '100000000'); await p.fill(`${root} #qbapp input[name=secret]`, SECRET); await p.click('#qbcheck');
        check(await until(async () => /^OK/.test(await p.textContent('#qbcheckout'))) && await p.$eval('#qbcheckout', (e) => e.classList.contains('ok')), `${w} Qobuz: Check says OK`);
        await shot('qobuz-step1');
        await p.click('#qbapp button[type=submit]');
        check(await until(async () => (await pillOf(p, root)) === 'Needs sign-in'), `${w} Qobuz: saved, the status moves on to the sign-in`);
        const put = log.find((l) => l[0] === 'PUT' && l[1].app_id);
        check(!!put && put[1].app_secret === SECRET, `${w} Qobuz: the app went to the node once`);
        check(await p.locator(`${root} .setup-step[data-step="1"].done`).count() === 1 && /100000000/.test(await p.textContent(`${root} .setup-step[data-step="1"]`)) && await p.locator(`${root} .setup-step[data-step="2"] input[name=email]`).isEnabled(), `${w} Qobuz: step 1 done with the ID shown, step 2 open`);
        check(await p.evaluate(() => document.activeElement && document.activeElement.name === 'email'), `${w} Qobuz: the email field has the focus`);
        // The account: no plan, wrong password, then in
        await p.fill('#qbin input[name=email]', 'free@example.com'); await p.fill('#qbin input[name=password]', 'hunter2'); await p.click('#qbin button[type=submit]');
        check(await until(async () => /no streaming plan/.test(await p.textContent('#qbinerr'))) && await p.isVisible('#qbinerr'), `${w} Qobuz: an account without a plan is told so, in place`);
        await p.fill('#qbin input[name=email]', 'me@example.com'); await p.fill('#qbin input[name=password]', 'wrong'); await p.click('#qbin button[type=submit]');
        check(await until(async () => /doesn't know that email and password/.test(await p.textContent('#qbinerr'))), `${w} Qobuz: a wrong password is told so`);
        await p.fill('#qbin input[name=password]', 'hunter2'); await p.click('#qbin button[type=submit]');
        check(await until(async () => (await pillOf(p, root)) === 'Working'), `${w} Qobuz: signed in, Working`);
        check(/up to 24-bit\/192 kHz/.test(await p.textContent(`${root} .set-row`)) && await p.isVisible('#qbout') && await p.locator('#qbq').count() === 1, `${w} Qobuz: the quality, Sign out and the quality setting`);
        check(/Unofficial: can stop working, uses your Qobuz account/.test(await p.textContent(`${root}`)), `${w} Qobuz: the one-line warning`);
        check(await p.locator(`${root} .setup-step.done`).count() === 2 && await p.locator('#qbin').count() === 0, `${w} Qobuz: both steps done, no password field left`);
        await shot('qobuz-working');
        // Off, a problem, sign out
        await p.click('#qbon');
        check(await until(async () => (await pillOf(p, root)) === 'Off'), `${w} Qobuz: switched off`);
        U.health = { ok: false, detail: 'Qobuz refused the app secret', at: 1 }; U.enabled = false;
        await p.reload(); await p.waitForSelector(`${root} .setup-step`, { timeout: 20000 });
        check(await pillOf(p, root) === 'Problem' && await p.isVisible('#qbretry'), `${w} Qobuz: a failure is a Problem with Try again`);
        await p.click('#qbout');
        check(await until(async () => (await pillOf(p, root)) === 'Needs sign-in'), `${w} Qobuz: Sign out goes back to step 2`);
        // streamrip's / qobuz-dl's config, under step 1: a refusal stays in place; a good one signs in
        await p.click(`${root} #qbimport summary`);
        await p.fill('#qbimppaste textarea', 'not a sign-in'); await p.click('#qbimppaste button');
        check(await until(async () => await p.isVisible('#qbimperr') && /Not usable/.test(await p.textContent('#qbimperr'))), `${w} Qobuz: a refused config is said in place`);
        await p.fill('#qbimppaste textarea', `[qobuz]\napp_id = "100000000"\nsecrets = ["${SECRET}"]\nemail_or_userid = "listener@example.org"\npassword_or_token = "0000"\n`); await p.click('#qbimppaste button');
        check(await until(async () => (await pillOf(p, root)) === 'Working'), `${w} Qobuz: streamrip’s config signs in at once`);
        check(await p.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${w} Qobuz: no horizontal overflow`);
        for (const re of [/\/v1\/sources\/qobuz\/unofficial$/, /\/v1\/sources\/qobuz\/unofficial\/app\/check$/, /\/v1\/sources\/qobuz\/unofficial\/login$/, /\/v1\/sources\/qobuz\/signin(\/import)?$/]) await p.unroute(re);
      }

      // ---- TIDAL full tracks ----
      {
        const log = []; const T = await tidalMocks(p, log, tidalSi);
        // TIDAL's sign-in page opens in a new tab: answered here, never the real one
        await ctx.route(/^https:\/\/login\.tidal\.com\//, (r) => r.fulfill({ contentType: 'text/html', body: '<title>TIDAL</title><p>sign in</p>' }));
        await p.goto(BASE + '/sources/tidal');
        const root = '#tidalsignin';
        await p.waitForSelector(`${root} input[type=radio]`, { timeout: 20000 });
        // The chooser: Sign in with TIDAL first and Recommended, picked; the code sign-in second; the
        // rest under a closed Advanced; nothing found, so no import way
        const ids = await p.$$eval(`${root} .ways input[type=radio]`, (es) => es.map((e) => e.value));
        check(JSON.stringify(ids.slice(0, 2)) === '["browser","device"]' && !ids.includes('import'), `${w} TIDAL: Sign in with TIDAL first, the code sign-in second (${ids.join(', ')})`);
        check(/Sign in with TIDAL\s*Recommended/.test(await p.textContent(`${root} label.way:has(input[value="browser"]) .way-name`)) && await p.locator(`${root} .pill.ok:text("Recommended")`).count() === 1, `${w} TIDAL: only Sign in with TIDAL is Recommended`);
        check(await p.isChecked(`${root} input[value="browser"]`), `${w} TIDAL: and picked`);
        check(/AAC/.test(await p.textContent(`${root} label.way:has(input[value="device"]) .way-note`)), `${w} TIDAL: the code sign-in says it may be AAC only`);
        const adv = await p.$$eval(`${root} details.ways-adv input[type=radio]`, (es) => es.map((e) => e.value));
        check(JSON.stringify(adv) === '["app","own-device","paste"]' && !(await p.$eval(`${root} details.ways-adv`, (d) => d.open)) && !(await p.isVisible(`${root} input[value="paste"]`)), `${w} TIDAL: the official app, your own device client and Paste a token under a closed Advanced (${adv.join(', ')})`);
        check(await p.locator(`${root} details.ways-adv .pill`).count() === 0, `${w} TIDAL: no badge under Advanced`);
        check(await pillOf(p, root) === 'Needs sign-in', `${w} TIDAL: the status asks for the sign-in`);
        check(await p.locator(`${root} input[name=secret], ${root} input[name=id]`).count() === 0, `${w} TIDAL: no client id or secret to enter for Sign in with TIDAL`);
        await shot('tidal-chooser');
        // Step 1 opens TIDAL's sign-in in a new tab; step 2 asks for its address
        const [tab] = await Promise.all([ctx.waitForEvent('page'), p.click('#tbin')]);
        check(await until(async () => /^https:\/\/login\.tidal\.com\/authorize\?/.test(tab.url()), 5000), `${w} TIDAL: TIDAL's sign-in opens in a new tab (${tab.url()})`);
        await tab.close();
        check(await until(async () => await p.isVisible('#tbfinish')), `${w} TIDAL: then a field for the address`);
        check(await until(async () => p.evaluate(() => document.activeElement && document.activeElement.matches('#tbfinish input[name=text]'))), `${w} TIDAL: and the address field has the focus`);
        check(log.some((l) => l[0] === 'WAY' && l[1] === 'browser') && log.some((l) => l[0] === 'BROWSER'), `${w} TIDAL: the way was picked and the sign-in started`);
        check(await pillOf(p, root) === 'Waiting' && await p.locator(`${root} .setup-step[data-step="1"].done`).count() === 1, `${w} TIDAL: waiting for the address, step 1 done`);
        await shot('tidal-browser-paste');
        await p.fill('#tbfinish input[name=text]', 'https://evil.example/android/login/auth?code=x'); await p.click('#tbfinish button[type=submit]');
        check(await until(async () => await p.isVisible('#tbfinisherr') && /isn't a TIDAL address/.test(await p.textContent('#tbfinisherr'))), `${w} TIDAL: a wrong address is said in place`);
        await p.fill('#tbfinish input[name=text]', 'https://tidal.com/android/login/auth?code=ui-code'); await p.click('#tbfinish button[type=submit]');
        check(await until(async () => (await pillOf(p, root)) === 'Working'), `${w} TIDAL: pasted, signed in, Working`);
        check(/Sign in with TIDAL · full tracks, lossless/.test(await p.textContent(root)) && await p.locator('#tbagain').count() === 0, `${w} TIDAL: in use, no "again" line`);
        await shot('tidal-browser-working');
        // A code sign-in sees one line: Sign in with TIDAL again for lossless, with the button
        T.U.session_client = 'device'; T.si.in_use = { way: 'device', label: 'Sign in with a code', user: '42', gives: 'full tracks' };
        await p.reload(); await p.waitForSelector(`${root} .set-name .pill`, { timeout: 20000 });
        const againLines = async () => (await p.textContent(root)).split('Sign in with TIDAL again for lossless').length - 1;
        check(await p.isVisible('#tbagain') && await againLines() === 1, `${w} TIDAL: a code sign-in gets one "Sign in with TIDAL again for lossless" line with the button`);
        // AAC only: the health line says so and what to do, still one line
        T.U.health = { ok: true, lossy: true, detail: 'TIDAL sends this sign-in AAC only (lossy). Sign in with TIDAL again for lossless', at: 1 };
        await p.reload(); await p.waitForSelector(`${root} .set-name .pill`, { timeout: 20000 });
        check(await pillOf(p, root) === 'AAC only' && !/full tracks are playing/.test(await p.textContent(root)) && await againLines() === 1 && await p.isVisible('#tbagain'), `${w} TIDAL: AAC only is said on the health line, with the button, once`);
        await shot('tidal-aac');
        // A browser session's AAC track is that track: Working, the Health line says why
        T.U.session_client = 'pkce'; T.si.in_use = { way: 'browser', label: 'Sign in with TIDAL', user: '77', gives: 'full tracks, lossless' }; T.U.health = { ok: true, lossy: true, detail: 'TIDAL sent the last track as AAC (lossy): it may have no lossless version', at: 1 };
        await p.reload(); await p.waitForSelector(`${root} .set-name .pill`, { timeout: 20000 });
        check(await pillOf(p, root) === 'Working' && /may have no lossless version/.test(await p.textContent(root)), `${w} TIDAL: a browser session's AAC track is not "AAC only"`);
        T.U.session_client = 'device'; T.si.in_use = { way: 'device', label: 'Sign in with a code', user: '42', gives: 'full tracks' }; T.U.health = { ok: true, lossy: true, detail: 'TIDAL sends this sign-in AAC only (lossy). Sign in with TIDAL again for lossless', at: 1 };
        await p.reload(); await p.waitForSelector(`${root} .set-name .pill`, { timeout: 20000 });
        // The button: the chooser opens on Sign in with TIDAL and TIDAL's tab opens
        const [tab2] = await Promise.all([ctx.waitForEvent('page'), p.click('#tbagain')]);
        await tab2.close();
        check(await until(async () => await p.isVisible('#tbfinish') && await p.isChecked(`${root} input[value="browser"]`)), `${w} TIDAL: "Sign in with TIDAL" from there goes straight to the paste step`);
        // Back to nothing: your own device client, under Advanced
        Object.assign(T.U, { signed_in: false, enabled: false, browser_available: false }); delete T.U.session_client; delete T.U.health; delete T.U.browser_pending; T.si.in_use = null; T.si.chosen = null;
        T.si.ways.forEach((w2) => { w2.recommended = false; if (w2.id === 'browser' || w2.id === 'device') { w2.available = false; w2.reason = w2.id === 'browser' ? 'This build has no client for Sign in with TIDAL (a release build carries it). Use the code sign-in instead.' : 'This build has no device client: add one under Advanced › Your own device client.'; } });
        await p.reload(); await p.waitForSelector(`${root} input[type=radio]`, { timeout: 20000 });
        check(await pillOf(p, root) === 'Needs client', `${w} TIDAL: a build without its clients needs a device client, the status says so`);
        check(await p.isDisabled(`${root} input[value="browser"]`) && /no client for Sign in with TIDAL/.test(await p.textContent(`${root} label.way:has(input[value="browser"])`)), `${w} TIDAL: Sign in with TIDAL is there, unavailable, saying why`);
        // Nothing everyday usable: the first usable way is an Advanced one, so Advanced is open
        check(await p.$eval(`${root} details.ways-adv`, (d) => d.open), `${w} TIDAL: Advanced opens by itself when only its ways are usable`);
        await p.locator(`${root} input[value="own-device"]`).check();
        await p.waitForSelector('#tfclient', { timeout: 5000 });
        check(await p.locator(`${root} .setup-step[data-step="1"]`).count() === 1 && await p.locator(`${root} a[href*="guides/tidal-full-tracks.md"]`).count() >= 1, `${w} TIDAL: the client step card, with the guide`);
        await pasteInto(p, '#tfclient input[name=id]', 'client_id=dev-id\nclient_secret=dev-secret');
        check(await p.inputValue('#tfclient input[name=id]') === 'dev-id' && await p.inputValue('#tfclient input[name=secret]') === 'dev-secret', `${w} TIDAL: a pasted client_id=… client_secret=… block is split`);
        await p.fill('#tfclient input[name=id]', 'portal-app'); await p.click('#tfcheck');
        check(await until(async () => /not device clients/.test(await p.textContent('#tfcheckout'))), `${w} TIDAL: Check says a portal app is not a device client`);
        await p.fill('#tfclient input[name=id]', 'dev-id'); await p.fill('#tfclient input[name=secret]', 'nope'); await p.click('#tfcheck');
        check(await until(async () => /secret doesn't go with it/.test(await p.textContent('#tfcheckout'))), `${w} TIDAL: Check names a wrong secret`);
        await p.fill('#tfclient input[name=secret]', 'dev-secret'); await p.click('#tfcheck');
        check(await until(async () => /^OK/.test(await p.textContent('#tfcheckout'))), `${w} TIDAL: Check says OK`);
        await shot('tidal-client');
        // Save and sign in with a code: the code big, the link that fills it in, Copy, the time left
        await p.click('#tfclient button[type=submit]');
        check(await until(async () => await p.isVisible('#tfcode')), `${w} TIDAL: the code shows`);
        check((await p.textContent('#tfcode')).trim() === 'ABCDE' && await p.$eval('#tfcode', (e) => parseFloat(getComputedStyle(e).fontSize) >= 30), `${w} TIDAL: big`);
        check(await p.getAttribute('#tfopen', 'href') === 'https://link.tidal.com/ABCDE' && await p.getAttribute('#tfopen', 'target') === '_blank', `${w} TIDAL: Open link.tidal.com carries the code`);
        check(await p.locator('[data-copylink="ABCDE"]').count() === 1, `${w} TIDAL: a Copy button`);
        check(await until(async () => /good for 4:5\d/.test(await p.textContent('#tfcount'))), `${w} TIDAL: the time left counts down (${(await p.textContent('#tfcount')).trim()})`);
        check(await pillOf(p, root) === 'Waiting', `${w} TIDAL: the status waits for the code`);
        check(log.some((l) => l[0] === 'PUT' && l[1].client_id === 'dev-id' && l[1].client_secret === 'dev-secret') && log.some((l) => l[0] === 'WAY' && l[1] === 'device') && log.some((l) => l[0] === 'LOGIN'), `${w} TIDAL: the client was saved, the code sign-in picked and started`);
        await shot('tidal-code');
        // TIDAL says yes: the page continues by itself
        Object.assign(T.U, { signed_in: true, enabled: true, user: '42', session_client: 'device', health: { ok: true, detail: 'signed in', at: 1 } }); delete T.U.pending;
        T.si.in_use = { way: 'device', label: 'Your own device client', user: '42', gives: 'full tracks' }; T.si.line = 'Your own device client · full tracks';
        check(await until(async () => (await pillOf(p, root)) === 'Working', 8000), `${w} TIDAL: signed in by itself, Working`);
        check(/up to 24-bit\/192 kHz/.test(await p.textContent(`${root} .set-row`)) && await p.isVisible('#tfout'), `${w} TIDAL: the quality and Sign out`);
        await shot('tidal-working');
        // A session imported from another app's file needs no device client here: Working
        delete T.U.client_from; await p.reload(); await p.waitForSelector(`${root} .set-name .pill`, { timeout: 20000 });
        check(await pillOf(p, root) === 'Working', `${w} TIDAL: an imported session without a client is Working, not Needs client`);
        T.U.client_from = 'settings';
        // Off, a problem, sign out, then a code that runs out
        // (signed in, the chooser shows its status rows, not the ways)
        T.U.enabled = false; await p.reload(); await p.waitForSelector(`${root} .set-name .pill`, { timeout: 20000 });
        check(await pillOf(p, root) === 'Off', `${w} TIDAL: signed in but switched off`);
        T.U.health = { ok: false, detail: 'TIDAL refused the client', at: 1 }; await p.reload(); await p.waitForSelector(`${root} .set-name .pill`, { timeout: 20000 });
        check(await pillOf(p, root) === 'Problem' && await p.isVisible('#tfretry'), `${w} TIDAL: a failure is a Problem with Try again`);
        delete T.U.health;
        await p.click('#tfout');
        check(await until(async () => (await pillOf(p, root)) === 'Needs sign-in'), `${w} TIDAL: signed out, needs the sign-in again`);
        T.codeSeconds = 2;
        await p.locator(`${root} input[value="device"]`).check(); await p.waitForSelector('#tfin', { timeout: 5000 }); await p.click('#tfin');
        check(await until(async () => await p.isVisible('#tfcount')), `${w} TIDAL: a new code`);
        check(await until(async () => /expired/.test(await p.textContent('#tfcount')), 5000), `${w} TIDAL: the countdown says when it expired`);
        delete T.U.pending;
        check(await until(async () => await p.isVisible('#tfin') && /New code/.test(await p.textContent('#tfin')) && /expired/.test(await p.textContent(`${root} [data-sg="action"]`)), 8000), `${w} TIDAL: then offers a new code`);
        check(await p.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${w} TIDAL: no horizontal overflow`);
      }
      check(errors.length === 0, `${w} no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
      await ctx.close();
    }
  } finally { await browser.close(); }
  if (problems.length) { console.log(`${problems.length} failed`); process.exit(1); }
  console.log('stream setup ok');
})().catch((e) => { console.error(e); process.exit(1); });
