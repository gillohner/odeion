// Sign-in pages, simple (owner, 2026-11-08: "make sure ui is simpler. remove all options that don't
// work at the moment"): at phone, large phone and desktop width, for Spotify, TIDAL, Qobuz, YouTube,
// Last.fm and Jamendo, with the node's answers mocked as the owner's node gave them:
//   * no way that can't be used here is drawn (no greyed row, no "install X first");
//   * no radio buttons: one way is its button or form; more sit under one closed "Other ways";
//   * Spotify without librespot and without a sound output shows no librespot or Soloist;
//   * the first line of each sign-in says what it gives and what it needs;
//   * signed in is one row, with Change only when another way works.
// Run against a disposable fixture node:
//   node tools/ui-signin-simple.js <api token> <node URL> [screenshot directory]
const path = require('node:path');
const fs = require('node:fs');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const SHOTS = process.argv[4];
const problems = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 8000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(150); } };

const way = (id, label, note, o = {}) => ({ id, label, note, action: o.action || 'setup', recommended: !!o.recommended, advanced: !!o.advanced, available: o.available !== false, ...(o.available === false ? { reason: o.reason } : {}) });
const REDIRECT = 'https://node.example/spotify/callback';
// As the owner's node answered on 2026-11-08 (main 46f6e631)
const spotifySi = (o = {}) => ({
  source: 'spotify', chosen: null, in_use: o.inUse || null, line: 'Not signed in', imports: [], places: [],
  ways: [
    way('app', 'Your own Spotify app', 'Official API: Connect control of your Spotify devices, your library and playlists. Set up a free developer app first (two minutes, below).', { recommended: true, action: o.configured ? 'browser' : 'setup' }),
    way('librespot', 'librespot, picked in a Spotify app on this network', 'Unofficial: librespot appears as a speaker in your Spotify apps.', { available: !!o.librespot, reason: 'Install librespot first (the Spotify tab says how).' }),
    way('librespot-file', 'librespot with a credentials.json you pick', 'The credentials.json another librespot saved after its sign-in.', { action: 'import', available: !!o.librespot, reason: 'Install librespot first (the Spotify tab says how).' }),
  ],
  details: { status: { configured: !!o.configured, signed_in: !!o.inUse, display_name: o.inUse ? o.inUse.user : null, product: 'premium', redirect_uri: REDIRECT, requested_scopes: 'user-library-read', app_from: o.configured ? 'settings' : null, client_id: o.configured ? '0123456789abcdef0123456789abcdef' : null }, librespot: { enabled: true, supported: !!o.librespot } },
});
const soloist = (o = {}) => ({ supported: true, enabled: false, key_set: false, device_name: 'NUC', pipewire_device: '', can_download: true, sound_output: !!o.sound, running: false, note: "Spotify's official headless player. It plays on this machine's own sound output." });
const librespot = (o = {}) => ({ supported: true, enabled: true, device_name: 'NUC', output: 'local', zone_id: '', bitrate: '320', zones: [{ id: 'z1', name: 'Living room' }], running: false, warning: 'librespot is a community-made Spotify client, not approved by Spotify. It can stop working at any time.', ...(o.installed ? { librespot: { path: '/usr/bin/librespot', from: 'path', version: '0.6.0' } } : { waiting_for: 'librespot itself (install it on this machine)' }) });
const tidalSi = (o = {}) => ({
  source: 'tidal', chosen: o.inUse ? o.inUse.way : null, in_use: o.inUse || null, line: 'Not signed in', imports: [], places: ['tidal-dl-ng: ~/.config/tidal_dl_ng/token.json'],
  ways: [
    way('browser', 'Sign in with TIDAL', 'Full tracks, lossless and hi-res: sign in at TIDAL in a new tab, then paste the address of the page you land on.', { recommended: true, action: 'browser', available: o.browser !== false, reason: 'This build has no client for Sign in with TIDAL.' }),
    way('device', 'Sign in with a code', 'Fallback, which TIDAL may send AAC only (lossy): a code you enter at link.tidal.com.', { action: 'code' }),
    way('app', 'Your own TIDAL app', 'Official API, sign in at TIDAL in this browser: search, albums, your collection and playlists.', { action: 'browser', advanced: true }),
    way('own-device', 'Your own device client', 'A TIDAL device client id and secret you are allowed to use, for the code sign-in.', { advanced: true }),
    way('paste', 'Paste a token', "Another app's token file (tidal-dl-ng, beets, tidal-dl, streamrip) or its contents.", { action: 'paste', advanced: true }),
  ],
  details: { status: { configured: true, signed_in: false }, unofficial: { service: 'tidal', enabled: !!o.inUse, signed_in: !!o.inUse, quality: 'max', browser_available: o.browser !== false, session_client: o.inUse ? 'pkce' : undefined, user: o.inUse ? '176801529' : undefined, warning: 'Unofficial.' } },
});
const youtubeSi = (o = {}) => ({
  source: 'youtube', chosen: null, in_use: o.inUse || null, line: 'Not signed in', imports: [], places: [],
  ways: [
    way('key', 'Your YouTube Data API key', 'Optional, for searching YouTube here: a free key from the Google Cloud console.', { recommended: true }),
    way('cookies', 'yt-dlp with a cookies file you pick', 'For playback on your speakers through yt-dlp (unofficial) as your YouTube account.', { action: 'import', available: o.cookies !== false, reason: 'Switch on On your speakers first.' }),
  ],
  details: { key: { set: !!o.inUse }, cookies: {} },
});
const lastfmSi = () => ({
  source: 'lastfm', chosen: null, in_use: null, line: 'Not signed in', imports: [], places: ['rescrobbled: ~/.config/rescrobbled/config.toml'],
  ways: [
    way('own', 'Your own Last.fm API account', "An API key and shared secret from last.fm/api/account/create, then Last.fm's permission page in the browser.", { recommended: true, action: 'browser' }),
    way('import', 'A sign-in from another app on this machine', 'Nothing in the known places on this machine (rescrobbled).', { action: 'import' }),
    way('paste', 'Paste a key, secret and session key', 'Three lines: the API key, its shared secret, and a session key made with that key.', { action: 'paste' }),
  ],
  details: { lastfm: { configured: false, connected: false } },
});
const qobuzU = { enabled: true, signed_in: false, hires: false, quality: 'max', warning: 'Unofficial.' };
const qobuzSi = { source: 'qobuz', chosen: null, in_use: null, line: 'Not signed in', imports: [], places: ['streamrip: ~/.config/streamrip/config.toml (not found)'], ways: [way('own', 'Your own Qobuz app and account', 'A Qobuz app id and secret.', { recommended: true }), way('import', 'A sign-in from another app on this machine', 'Nothing found.', { action: 'import' }), way('paste', "Paste streamrip's or qobuz-dl's config", 'The contents of the config.', { action: 'paste' })], details: { unofficial: qobuzU } };

(async () => {
  if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
  const browser = await chromium.launch();
  try {
    for (const [width, height] of [[360, 800], [412, 915], [1440, 1000]]) {
      const ctx = await browser.newContext({ viewport: { width, height }, reducedMotion: 'reduce' });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); HTMLMediaElement.prototype.play = async function () {}; }, TOKEN);
      const p = await ctx.newPage();
      const errors = []; p.on('pageerror', (e) => errors.push(e.message));
      const w = `${width}:`;
      const M = { spotify: spotifySi(), soloist: soloist(), librespot: librespot(), tidal: tidalSi(), youtube: youtubeSi(), ytOn: false, lastfm: lastfmSi() };
      const log = [];
      await p.route(/\/v1\/sources\/spotify\/signin(\?way=[\w-]+)?$/, (r) => { const m = r.request().method(); const named = /\?way=([\w-]+)$/.exec(r.request().url()); // as spotify.rs: the way in use follows the chosen one where it is signed in
        const inUseWay = () => { const c = M.spotify.chosen || ''; return /^librespot/.test(c) && M.spotify.details.librespot.signed_in ? c : c === 'app' && M.spotify.details.status.signed_in ? 'app' : (M.spotify.in_use || {}).way; };
        log.push([m, 'spotify', m === 'PUT' ? r.request().postDataJSON().way : named ? named[1] : inUseWay()]); if (m === 'PUT') M.spotify.chosen = r.request().postDataJSON().way; if (m === 'DELETE') { M.spotify = spotifySi({ configured: true }); } return r.fulfill({ json: M.spotify }); });
      await p.route(/\/v1\/sources\/spotify\/status$/, (r) => r.fulfill({ json: M.spotify.details.status }));
      await p.route(/\/v1\/sources\/spotify\/soloist$/, (r) => r.fulfill({ json: M.soloist }));
      await p.route(/\/v1\/sources\/spotify\/librespot$/, (r) => r.fulfill({ json: M.librespot }));
      await p.route(/\/v1\/sources\/spotify\/login$/, (r) => { log.push(['LOGIN', 'spotify']); return r.fulfill({ json: { url: BASE + '/sources/spotify?spotify-login-test=1' } }); });
      await p.route(/\/v1\/sources\/spotify\/app$/, (r) => { const b = r.request().postDataJSON(); log.push(['APP', b.client_id]); M.spotify = spotifySi({ configured: !!b.client_id }); return r.fulfill({ json: M.spotify.details.status }); });
      await p.route(/\/v1\/sources\/tidal\/signin$/, (r) => { if (r.request().method() === 'PUT') { M.tidal.chosen = r.request().postDataJSON().way; } return r.fulfill({ json: M.tidal }); });
      await p.route(/\/v1\/sources\/qobuz\/unofficial$/, (r) => r.fulfill({ json: qobuzU }));
      await p.route(/\/v1\/sources\/qobuz\/signin(\/import)?$/, (r) => { if (r.request().method() === 'POST') { log.push(['QIMPORT', r.request().postDataJSON().text]); return r.fulfill({ status: 400, json: { error: 'Not usable: no app values and no sign-in in this' } }); } return r.fulfill({ json: qobuzSi }); });
      await p.route(/\/v1\/sources\/youtube\/signin$/, (r) => r.fulfill({ json: M.youtube }));
      await p.route(/\/v1\/sources\/youtube\/unofficial$/, (r) => r.fulfill({ json: { enabled: M.ytOn, can_download: true, warning: 'This plays YouTube on your speakers through yt-dlp, outside YouTube’s own player. More words.' } }));
      await p.route(/\/v1\/sources\/youtube\/key$/, (r) => { const b = r.request().postDataJSON(); log.push(['YTKEY', b.key]); return r.fulfill({ status: 400, json: { error: 'YouTube refused this key' } }); });
      await p.route(/\/v1\/sources\/lastfm\/signin$/, (r) => r.fulfill({ json: M.lastfm }));
      await p.route(/\/v1\/sources\/jamendo\/status$/, (r) => r.fulfill({ json: { configured: false, signed_in: false, can_sign_in: false } }));
      const shot = async (name) => {
        if (!SHOTS) return;
        const vp = p.viewportSize();
        const h = await p.evaluate(() => { const m = document.querySelector('#main'); return innerHeight + Math.max(0, m.scrollHeight - m.clientHeight); });
        await p.setViewportSize({ width, height: Math.ceil(h) }); await p.screenshot({ path: path.join(SHOTS, `${name}-${width}.png`) }); await p.setViewportSize(vp);
      };
      // What every sign-in block must hold, whatever the service
      const common = async (root, svc) => {
        check(await p.locator(`${root} input[type=radio]`).count() === 0, `${w} ${svc}: no radio buttons`);
        check(await p.locator(`${root} .way.off, ${root} input:disabled[type=radio]`).count() === 0, `${w} ${svc}: no greyed way`);
        const txt = await p.textContent(root);
        check(!/Install librespot first|Pick how|the node remembers it|Another way|Advanced|Change way/.test(txt), `${w} ${svc}: no "install first", "Pick how", "Advanced" or "Another way" wording`);
        const lead = p.locator(`${root} [data-sg=lead]`).first();
        check(await lead.count() === 1 && /Needs|need/.test(await lead.textContent()), `${w} ${svc}: the first line says what it needs (${await lead.textContent().catch(() => '')})`);
        check(await p.evaluate((r) => { const g = document.querySelector(`${r} .set-rows`); const l = document.querySelector(`${r} [data-sg=lead]`); return !!g && !!l && g.firstElementChild === l; }, root), `${w} ${svc}: and it is the first line of the sign-in`);
        check(await p.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${w} ${svc}: no sideways scroll`);
      };

      // ---- Spotify on the owner's node: not set up, no librespot, Soloist can't play (Docker) ----
      await p.goto(BASE + '/sources/spotify');
      const sp = '#signin-spotify';
      await p.waitForSelector(`${sp} [data-sg=lead]`, { timeout: 20000 });
      await sleep(400);
      await common(sp, 'Spotify');
      check(/Needs Spotify Premium/.test(await p.textContent(`${sp} [data-sg=lead]`)), `${w} Spotify: Premium is said before step 1`);
      check(await p.locator(`${sp} .setup-step[data-step="1"] #spotapp`).count() === 1 && await p.locator(`${sp} .setup-step[data-step="2"]`).count() === 1, `${w} Spotify: step 1 the developer app, step 2 Connect`);
      check((await p.textContent(`${sp} .setup-step[data-step="1"] code`)).trim() === REDIRECT && await p.locator('#spotcopy').count() === 1, `${w} Spotify: the redirect URI to copy`);
      const panel = '#src-spotify';
      check(!/librespot|Soloist/i.test(await p.innerText(panel)), `${w} Spotify: no librespot or Soloist anywhere on the page`);
      check(await p.locator('#spotplay').isHidden(), `${w} Spotify: no Other ways to play`);
      check(await p.locator(`${sp} [data-sg=other]`).count() === 0, `${w} Spotify: one way, no Other ways to sign in`);
      await shot('spotify-new');
      // Step 1 saved: step 2 is the Connect button, going to Spotify's sign-in
      await p.fill('#spotid', '0123456789abcdef0123456789abcdef'); await p.click('#spotapp button.primary');
      check(await until(async () => await p.locator('#spotin').count() === 1), `${w} Spotify: saved, Connect Spotify is there`);
      check(log.some((l) => l[0] === 'APP' && l[1] === '0123456789abcdef0123456789abcdef'), `${w} Spotify: the client ID went to the node`);
      check(await p.locator(`${sp} .setup-step[data-step="1"].done`).count() === 1, `${w} Spotify: step 1 done`);
      // (the page tidies its address at once: watch the navigations)
      const went = []; const nav = (f) => { if (f === p.mainFrame()) went.push(f.url()); }; p.on('framenavigated', nav);
      await p.click('#spotin');
      check(await until(async () => log.some((l) => l[0] === 'LOGIN') && went.some((u) => /spotify-login-test=1/.test(u))), `${w} Spotify: Connect goes to the sign-in address the node gives`);
      p.off('framenavigated', nav);
      // Signed in: one row, no Change (no other sign-in works here)
      M.spotify = spotifySi({ configured: true, inUse: { way: 'app', label: 'Your own Spotify app', user: 'Gil', gives: 'Connect playback on your devices, your library' } });
      await p.goto(BASE + '/sources/spotify'); await p.waitForSelector(`${sp} [data-sg=signed-in]`, { timeout: 20000 });
      check(await p.locator(`${sp} [data-sg=signed-in]`).count() === 1 && /Signed in as Gil/.test(await p.textContent(`${sp} [data-sg=signed-in]`)) && /Connect playback/.test(await p.textContent(`${sp} [data-sg=signed-in]`)), `${w} Spotify: signed in is one row, who and what it gives`);
      check(await p.locator(`${sp} [data-sg=change]`).count() === 0 && await p.locator(`${sp} [data-sg=out]`).count() === 1, `${w} Spotify: Sign out, no Change`);
      check(await p.locator(`${sp} .set-row`).count() === 1, `${w} Spotify: nothing else in the sign-in (${await p.locator(`${sp} .set-row`).count()} rows)`);
      await shot('spotify-signed-in');
      // librespot installed and a sound output here: both under Other ways to play, not in the sign-in
      M.spotify = spotifySi({ configured: true, librespot: true }); M.librespot = librespot({ installed: true }); M.soloist = soloist({ sound: true });
      await p.goto(BASE + '/sources/spotify'); await p.waitForSelector('#spotplay:not([hidden]) #lson', { timeout: 20000 });
      await p.waitForSelector('#spotsoloist #slon', { timeout: 10000 });
      check(/Other ways to play/.test(await p.textContent('#spotplay')), `${w} Spotify: librespot and Soloist under Other ways to play`);
      check(!/librespot|Soloist/i.test(await p.innerText(sp)), `${w} Spotify: and not in the sign-in`);
      check(await p.locator('#lsfile').count() === 1, `${w} Spotify: a credentials.json from another librespot, with librespot`);
      await shot('spotify-other-ways-to-play');
      // librespot signed in while the app is the way in use: its account still shows, with Sign out
      M.spotify = spotifySi({ configured: true, librespot: true, inUse: { way: 'app', label: 'Your own Spotify app', user: 'Gil', gives: 'Connect playback on your devices, your library' } });
      Object.assign(M.spotify.details.librespot, { signed_in: true, user: 'gil-ls' });
      await p.goto(BASE + '/sources/spotify'); await p.waitForSelector('#spotplay:not([hidden]) #lsout', { timeout: 20000 });
      check(/Signed in as gil-ls/.test(await p.textContent('#splibrespot')) && await p.locator('#lsfile').count() === 0, `${w} Spotify: librespot's own account shows while the app is in use`);
      // Its Sign out picks librespot's way first, then signs out of it (not the app)
      log.length = 0; M.spotify.chosen = 'app';
      await p.click('#lsout');
      check(await until(async () => log.some((l) => l[0] === 'DELETE')) && JSON.stringify(log.filter((l) => l[0] !== 'GET').map((l) => l[0] + ':' + l[2])) === '["DELETE:librespot"]', `${w} Spotify: librespot's Sign out signs out of librespot's way, not the app (${JSON.stringify(log.filter((l) => l[0] !== 'GET'))})`);
      // The app's Sign out while librespot was the way picked last: the app is picked, then signed out
      M.spotify = spotifySi({ configured: true, librespot: true, inUse: { way: 'librespot-file', label: 'librespot, from a credentials file', user: 'gil-ls', gives: 'playback into a zone' } });
      M.spotify.details.status.signed_in = true; M.spotify.details.status.display_name = 'Gil';
      await p.goto(BASE + '/sources/spotify'); await p.waitForSelector(`${sp} [data-sg=signed-in]`, { timeout: 20000 });
      check(/Signed in as Gil/.test(await p.textContent(`${sp} [data-sg=signed-in]`)), `${w} Spotify: the sign-in shows the app's account, not librespot's`);
      log.length = 0; await p.click(`${sp} [data-sg=out]`);
      check(await until(async () => log.some((l) => l[0] === 'DELETE')) && JSON.stringify(log.filter((l) => l[0] !== 'GET').map((l) => l[0] + ':' + l[2])) === '["DELETE:app"]', `${w} Spotify: Sign out signs out of the app, not librespot`);
      // librespot still signed in after its program went: the switch and Sign out, nothing to set up
      M.spotify = spotifySi({ configured: true }); Object.assign(M.spotify.details.librespot, { signed_in: true, user: 'gil-ls' }); M.librespot = librespot();
      await p.goto(BASE + '/sources/spotify'); await p.waitForSelector('#spotplay:not([hidden]) #lsout', { timeout: 20000 });
      check(await p.locator('#lsdev, #lsfile').count() === 0 && /isn’t installed/.test(await p.textContent('#splibrespot')), `${w} Spotify: librespot signed in without its program: switch and Sign out only`);
      // Soloist left on where nothing can play: just its switch, to turn it off
      M.soloist = { ...soloist(), enabled: true };
      await p.goto(BASE + '/sources/spotify'); await p.waitForSelector('#spotsoloist #slon', { timeout: 20000 });
      check(await p.locator('#slkey, #sldev, #slget').count() === 0 && /no sound output/.test(await p.textContent('#spotsoloist')), `${w} Spotify: Soloist on without a sound output is only its switch`);
      M.librespot = librespot(); M.soloist = soloist(); M.spotify = spotifySi();

      // ---- TIDAL as on the owner's node: Sign in with TIDAL in use ----
      M.tidal = tidalSi({ inUse: { way: 'browser', label: 'Sign in with TIDAL', user: '176801529', gives: 'full tracks, lossless' } });
      await p.goto(BASE + '/sources/tidal');
      const td = '#tidalsignin';
      await p.waitForSelector(`${td} [data-sg=signed-in]`, { timeout: 20000 });
      await common(td, 'TIDAL');
      check(await p.locator(`${td} [data-sg=signed-in]`).count() === 1 && /^Signed in$/.test((await p.textContent(`${td} [data-sg=signed-in] .set-name`)).trim()) && /Sign in with TIDAL · Full tracks, lossless/.test(await p.textContent(`${td} [data-sg=signed-in]`)) && /Full tracks, lossless/.test(await p.textContent(`${td} [data-sg=signed-in]`)), `${w} TIDAL: signed in is one row saying what it gives`);
      check(await p.locator(`${td} [data-sg=change]`).count() === 1, `${w} TIDAL: Change, as other ways work`);
      check(await p.locator(`${td} .set-name .pill`).count() === 0, `${w} TIDAL: no second status row while it works`);
      await shot('tidal-signed-in');
      await p.click(`${td} [data-sg=change]`);
      await p.waitForSelector(`${td} [data-sg=action]`);
      const others = await p.$$eval(`${td} [data-sg-way]`, (bs) => bs.map((b) => b.dataset.sgWay));
      check(await p.locator('#tfin').count() === 1 && JSON.stringify(others) === '["app","paste"]', `${w} TIDAL: Change offers the code sign-in, then the app and a token under Other ways (${others.join(', ')}); no device client form`);
      check(!(await p.$eval(`${td} [data-sg=other]`, (d) => d.open)), `${w} TIDAL: Other ways closed`);
      await p.click(`${td} [data-sg=change]`);
      check(await until(async () => p.evaluate(() => document.activeElement && document.activeElement.dataset.sg === 'change')), `${w} TIDAL: Cancel keeps the focus on the button`);
      // Not signed in: Sign in with TIDAL is the action, the rest a line each
      M.tidal = tidalSi();
      M.tidal.chosen = 'paste';
      await p.goto(BASE + '/sources/tidal'); await p.waitForSelector(`${td} #tbin`, { timeout: 20000 });
      await common(td, 'TIDAL (signed out)');
      check(await p.locator(`${td} [data-sg=action] textarea`).count() === 0, `${w} TIDAL: a way picked before doesn't push the recommended one aside`);
      const others2 = await p.$$eval(`${td} [data-sg-way]`, (bs) => bs.map((b) => b.dataset.sgWay));
      check(JSON.stringify(others2) === '["device","app","paste"]', `${w} TIDAL: Sign in with TIDAL first; the code, the app and a token under Other ways (${others2.join(', ')})`);
      await p.click(`${td} [data-sg=other] summary`);
      check(await p.locator(`${td} [data-sg=other] .list-row`).count() === 3, `${w} TIDAL: one line each`);
      await shot('tidal-signed-out');
      await p.click(`${td} [data-sg-way="paste"]`);
      check(await until(async () => await p.locator(`${td} [data-sg=action] textarea`).count() === 1 && await p.evaluate(() => !!document.activeElement.closest('[data-sg=action]'))), `${w} TIDAL: another way swaps in, the focus on it`);
      check(JSON.stringify(await p.$$eval(`${td} [data-sg-way]`, (bs) => bs.map((b) => b.dataset.sgWay))) === '["browser","device","app"]', `${w} TIDAL: and Sign in with TIDAL is under Other ways now`);
      // A device client set here shows, to forget it
      M.tidal = tidalSi(); M.tidal.details.unofficial.client_from = 'settings';
      await p.goto(BASE + '/sources/tidal'); await p.waitForSelector(`${td} #tbin`, { timeout: 20000 });
      check((await p.$$eval(`${td} [data-sg-way]`, (bs) => bs.map((b) => b.dataset.sgWay))).includes('own-device'), `${w} TIDAL: your own device client shows once one is set here`);

      // ---- Qobuz: the config that works; no app form, no api@qobuz.com ----
      await p.goto(BASE + '/sources/qobuz');
      const qb = '#signin-qobuz';
      await p.waitForSelector(`${qb} .setup-step`, { timeout: 20000 });
      await common(qb, 'Qobuz');
      check(await p.locator(`${qb} #qbapp, ${qb} a[href^="mailto:"], ${qb} #qbcheck`).count() === 0 && !/api@qobuz/.test(await p.textContent(qb)), `${w} Qobuz: no app ID form and no "ask api@qobuz.com"`);
      check(await p.locator(`${qb} .setup-step[data-step="1"] #qbimpfile`).count() === 1 && await p.isVisible(`${qb} #qbimppaste textarea`), `${w} Qobuz: step 1 is streamrip’s or qobuz-dl’s config, open`);
      check(await p.locator(`${qb} .setup-step[data-step="2"] input`).count() === 0, `${w} Qobuz: step 2 waits for it`);
      await p.fill(`${qb} #qbimppaste textarea`, 'not a config'); await p.click(`${qb} #qbimppaste button`);
      check(await until(async () => await p.isVisible('#qbimperr') && /Not usable/.test(await p.textContent('#qbimperr'))), `${w} Qobuz: a refused config is said in place`);
      await shot('qobuz-new');
      // An app from the node config: the account is next; a newer config can still replace the app
      Object.assign(qobuzU, { app_from: 'config', app_id: '100000000' });
      await p.goto(BASE + '/sources/qobuz'); await p.waitForSelector(`${qb} #qbin`, { timeout: 20000 });
      check(await p.locator(`${qb} .setup-step[data-step="1"].done details #qbimppaste`).count() === 1 && await p.locator('#qbappforget').count() === 0, `${w} Qobuz: with an app from the config, "Use a newer config" (no Forget for the config's)`);
      // A listener on a node without an app: the owner's reason, nothing to do
      delete qobuzU.app_from; delete qobuzU.app_id;
      // (as qobuz.rs answers a listener: the owner's ways not available)
      const own = qobuzSi.ways[0]; own.available = false; own.reason = "The node's owner hasn't set up a Qobuz app yet."; qobuzSi.ways[1].available = false;
      await p.goto(BASE + '/sources/qobuz'); await p.waitForSelector(`${qb} [data-sg=none]`, { timeout: 20000 });
      check(await p.locator(`${qb} .setup-step, ${qb} textarea, ${qb} input`).count() === 0 && /owner hasn't set up/.test(await p.textContent(qb)), `${w} Qobuz: a listener without an app here gets the reason, no import`);
      own.available = true; delete own.reason; qobuzSi.ways[1].available = true;

      // ---- YouTube: the key; cookies only with On your speakers ----
      await p.goto(BASE + '/sources/youtube');
      const yt = '#signin-youtube';
      await p.waitForSelector(`${yt} [data-yt=key]`, { timeout: 20000 });
      await common(yt, 'YouTube');
      check(await p.locator(`${yt} [data-sg=other]`).count() === 0 && !/cookies/i.test(await p.textContent(yt)), `${w} YouTube: speakers off, the key only (no cookies)`);
      await p.fill('#ytkeyin', 'not-a-key'); await p.click(`${yt} [data-yt=key] button`);
      check(await until(async () => log.some((l) => l[0] === 'YTKEY' && l[1] === 'not-a-key')), `${w} YouTube: Save sends the key`);
      await shot('youtube-key');
      M.ytOn = true;
      await p.goto(BASE + '/sources/youtube'); await p.waitForSelector(`${yt} [data-sg=other]`, { timeout: 20000 });
      check(JSON.stringify(await p.$$eval(`${yt} [data-sg-way]`, (bs) => bs.map((b) => b.dataset.sgWay))) === '["cookies"]', `${w} YouTube: speakers on, the cookies file under Other ways`);
      M.ytOn = false;
      // A service with one usable way among unusable ones: just its form
      M.youtube = youtubeSi({ cookies: false }); M.ytOn = true;
      await p.goto(BASE + '/sources/youtube'); await p.waitForSelector(`${yt} [data-yt=key]`, { timeout: 20000 });
      check(await p.locator(`${yt} [data-sg=other]`).count() === 0 && !/Switch on On your speakers first/.test(await p.textContent(yt)), `${w} YouTube: an unusable way is not drawn, nor its reason`);
      M.youtube = youtubeSi(); M.ytOn = false;

      // ---- Last.fm (Settings › Accounts): two steps; nothing found, so no import way ----
      await p.goto(BASE + '/settings/accounts');
      const lf = '#lfbody';
      await p.waitForSelector(`${lf} [data-lf=app]`, { timeout: 20000 });
      check(await p.locator(`${lf} input[type=radio]`).count() === 0 && /Needs a free Last\.fm API account/.test(await p.textContent(`${lf} [data-sg=lead]`)), `${w} Last.fm: no radios; the first line says what it needs`);
      check(await p.locator(`${lf} .setup-step`).count() === 2, `${w} Last.fm: two numbered steps`);
      check(JSON.stringify(await p.$$eval(`${lf} [data-sg-way]`, (bs) => bs.map((b) => b.dataset.sgWay))) === '["paste"]', `${w} Last.fm: Paste under Other ways; no import when nothing was found`);
      await shot('lastfm');

      // ---- Jamendo without an app here: no notice about apps ----
      await p.goto(BASE + '/sources/jamendo'); await p.waitForSelector('#src-jamendo', { timeout: 20000 }); await sleep(300);
      check(!/no built-in Jamendo app|Service apps/.test(await p.textContent('#src-jamendo')), `${w} Jamendo: nothing to do here, nothing said`);

      check(errors.length === 0, `${w} no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
      await ctx.close();
    }
  } finally { await browser.close(); }
  if (problems.length) { console.log(`${problems.length} failed`); process.exit(1); }
  console.log('sign-in pages: only what works, one action, one row signed in');
})().catch((e) => { console.error(e); process.exit(1); });
