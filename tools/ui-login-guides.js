// Synthetic browser check of the inline login and setup guides (docs/ux/login-guides.md): the
// real web app (crates/pm-api/static) served from a made-up origin, every /v1 request answered
// here by SYNTHETIC fixtures. No node is started, no service or device is called, nothing leaves
// this machine, and no sign-in ever really succeeds. Every check is an assertion: the script exits
// non-zero if any fails.
//   npm install playwright@1.49.1 && npx playwright install chromium   (anywhere; run from there)
//   node /path/to/tools/ui-login-guides.js [<repository dir>] [<screenshots dir>]
// It renders every guide in LOGIN_GUIDES (read from app.js), at 1440, 820 and 390 px wide.
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');

const REPO = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = process.argv[3] ? path.resolve(process.argv[3]) : null;
const STATIC = path.join(REPO, 'crates/pm-api/static');
const ORIGIN = 'http://login-guides.test';
const TOKEN = 'synthetic-owner-token';
const CANARY = 'SECRETCANARY-7f3a';
const EVIL = 'https://evil.test/cb"><img src=x onerror="window.__pwned=1">';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let failed = 0;
const check = (ok, msg, detail) => {
  console.log((ok ? 'ok     ' : 'FAILED ') + msg + (!ok && detail !== undefined ? ` (${typeof detail === 'string' ? detail : JSON.stringify(detail)})` : ''));
  if (!ok) failed += 1;
  return ok;
};

// The guide table, from the app itself
const appJs = fs.readFileSync(path.join(STATIC, 'app.js'), 'utf8');
const block = /\/\/ LOGIN-GUIDES-BEGIN\n([\s\S]*?)\/\/ LOGIN-GUIDES-END/.exec(appJs);
const box = {};
if (block) vm.runInNewContext(`${block[1]}\nbox.g = LOGIN_GUIDES; box.s = typeof LOGIN_SOURCES === 'undefined' ? {} : LOGIN_SOURCES;`, { box });
const GUIDES = box.g || {}; const BY_SOURCE = box.s || {};
// Every source the node has (streaming::services()), as the checker reads it
const RS = path.join(REPO, 'crates/pm-api/src');
const RUNTIME = [...(/pub fn services\(\)[\s\S]*?\n\}/.exec(fs.readFileSync(path.join(RS, 'streaming.rs'), 'utf8')) || [''])[0].matchAll(/Box::new\(crate::(\w+)::\w+\)/g)]
  .map((m) => (/pub(?:\(crate\))? const SOURCE_ID: &str = "([^"]+)"/.exec(fs.readFileSync(path.join(RS, `${m[1]}.rs`), 'utf8')) || [])[1] || m[1]);
check(Object.keys(GUIDES).length > 0, 'app.js has a LOGIN_GUIDES table');

// ---- SYNTHETIC fixtures ----
const way = (id, label, action, recommended = false, available = true, reason) => ({ id, label, note: `SYNTHETIC ${label}`, action, recommended, available, ...(reason ? { reason } : {}) });
const LIBRESPOT_MISSING = "librespot isn't installed where the node looks: follow Set up first: librespot below (Linux install or Docker), then reload.";
const fresh = () => ({
  tidalIn: false, librespot: true, ytFailOnce: false, ytDelay: 0, outs: 0, scClient: false, imported: [],
});
let st = fresh();
const signin = (source) => {
  if (source === 'tidal') return { source, chosen: null, line: st.tidalIn ? 'Odeion’s app' : 'Not signed in',
    in_use: st.tidalIn ? { way: 'app', label: 'Odeion’s app', user: 'synthetic-listener' } : null,
    ways: [way('app', 'Odeion’s app', 'browser', true), way('import', 'A sign-in from another app on this machine', 'import'), way('device', 'Your own device client', 'code'), way('paste', 'Paste a token', 'paste')],
    details: { status: { signed_in: st.tidalIn, user_id: '1', redirect_uri: EVIL, requested_scopes: 'search.read', client_secret: CANARY }, unofficial: { enabled: false, signed_in: false, client_from: null } } };
  if (source === 'spotify') return { source, chosen: null, line: 'Not signed in', in_use: null,
    ways: [way('app', 'Your own Spotify app', 'setup', true), way('librespot', 'librespot, picked in a Spotify app on this network', 'setup', false, st.librespot, st.librespot ? null : LIBRESPOT_MISSING), way('librespot-file', 'librespot with a credentials.json you pick', 'import', false, st.librespot, st.librespot ? null : LIBRESPOT_MISSING)],
    details: { status: { configured: false, signed_in: false, redirect_uri: EVIL, client_id: '' }, librespot: { enabled: false, supported: st.librespot } } };
  if (source === 'youtube') return { source, chosen: null, line: 'Not signed in', in_use: null,
    ways: [way('key', 'Your YouTube Data API key', 'setup', true), way('cookies', 'yt-dlp with a cookies file you pick', 'import')], details: { key: { set: false } } };
  if (source === 'lastfm') return { source, chosen: null, line: 'Not signed in', in_use: null,
    ways: [way('own', 'Your own Last.fm API account', 'setup', true), way('import', 'A sign-in from another app on this machine', 'import'), way('paste', 'Paste a key, secret and session key', 'paste')],
    details: { status: { configured: false, signed_in: false, api_key: '', shared_secret: CANARY } } };
  if (source === 'qobuz') return { source, chosen: null, line: 'Not signed in', in_use: null, ways: [way('own', 'Your own Qobuz app and account', 'setup', true), way('import', 'import', 'import'), way('paste', 'paste', 'paste')], details: {} };
  return null;
};
const NAMES = { archive: 'The Internet Archive', audius: 'Audius', tidal: 'TIDAL', qobuz: 'Qobuz', deezer: 'Deezer', spotify: 'Spotify', jamendo: 'Jamendo', bandcamp: 'Bandcamp', soundcloud: 'SoundCloud', subsonic: 'Subsonic', jellyfin: 'Jellyfin', plex: 'Plex', podcasts: 'Podcasts', radio: 'Radio', audiobookshelf: 'Audiobookshelf', youtube: 'YouTube' };
const SOURCES = RUNTIME.map((id) => ({ id, name: NAMES[id] || id, enabled: true, status: 'ready', features: ['search'], stage: 'stable', description: 'SYNTHETIC TEST FIXTURE' }));
const app = (id, name, extra = {}) => ({ id, name, configured: false, from: null, builtin: false, fields: [], redirects: [], ...extra });
const APPS = [
  app('spotify', 'Spotify', { set: 'PUT /v1/sources/spotify/app', fields: [{ name: 'client_id', label: 'Client ID' }], redirects: [EVIL], get_one: 'https://developer.spotify.com/dashboard', limits: 'SYNTHETIC limits' }),
  app('tidal', 'TIDAL', { builtin: true, configured: true, from: 'builtin', set: 'PUT /v1/node/apps/tidal', fields: [{ name: 'client_id', label: 'Client ID' }, { name: 'client_secret', label: 'Client secret', secret: true, optional: true }], redirects: ['https://synthetic.test/callback'], client_secret: CANARY }),
  // As apps.rs answers without node.toml: every row but AcoustID (its own page) and the built-ins has a `set`
  app('tidal-full', 'TIDAL full tracks', { set: 'PUT /v1/sources/tidal/unofficial' }), app('qobuz', 'Qobuz', { set: 'PUT /v1/sources/qobuz/unofficial' }), app('soundcloud', 'SoundCloud', { set: 'PUT /v1/sources/soundcloud/unofficial' }), app('youtube', 'YouTube Data', { set: 'PUT /v1/sources/youtube/key', get_one: 'https://console.cloud.google.com/apis/library/youtube.googleapis.com' }), app('lastfm', 'Last.fm', { set: 'PUT /v1/lastfm' }),
  app('jamendo', 'Jamendo', { set: 'PUT /v1/node/apps/jamendo', redirects: ['https://synthetic.test/jamendo/callback'] }), app('acoustid', 'AcoustID', { page: 'library:node' }),
  app('google-drive', 'Google Drive', { set: 'PUT /v1/node/apps/google-drive', fields: [{ name: 'client_id', label: 'Client ID' }, { name: 'client_secret', label: 'Client secret', secret: true, optional: true }], redirects: ['https://synthetic.test/return', 'https://synthetic.test/drives/google/callback'] }), app('onedrive', 'OneDrive', { set: 'PUT /v1/node/apps/onedrive', fields: [{ name: 'client_id', label: 'Application (client) ID' }], redirects: ['https://synthetic.test/return'] }),
  app('fanart', 'fanart.tv', { configured: true, from: 'builtin' }), app('audiodb', 'TheAudioDB', { configured: true, from: 'builtin' }),
];

const reply = (route, code, body) => route.fulfill({ status: code, contentType: 'application/json', body: body === undefined ? '' : JSON.stringify(body) }).catch(() => {});
const types = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml', '.webmanifest': 'application/manifest+json' };
const external = [];
async function handle(route) {
  const req = route.request(); const u = new URL(req.url());
  if (u.origin !== ORIGIN) { external.push(u.origin); return route.abort(); } // nothing leaves this machine
  const p = u.pathname;
  if (!p.startsWith('/v1/')) {
    const f = path.join(STATIC, p === '/' || !path.extname(p) ? 'index.html' : p.slice(1));
    if (!f.startsWith(STATIC) || !fs.existsSync(f)) return route.fulfill({ status: 404, body: '' });
    return route.fulfill({ status: 200, contentType: types[path.extname(f)] || 'application/octet-stream', body: fs.readFileSync(f) });
  }
  const v = p.slice(3); const m = req.method();
  const auth = req.headers().authorization || '';
  if (v === '/auth/pubky/start') return reply(route, 503, { error: 'SYNTHETIC: no Pubky identity on this fixture' });
  if (auth !== 'Bearer ' + TOKEN && !/token=/.test(u.search)) return reply(route, 401, { error: 'missing or wrong bearer token' });
  if (/^\/sources\/\w+\/signin\/import$/.test(v) || v === '/sources/qobuz/signin/import') { st.imported.push((req.postDataJSON() || {}).text || ''); return reply(route, 400, { error: 'SYNTHETIC: refused, nothing was signed in' }); }
  const si = /^\/sources\/(\w+)\/signin$/.exec(v);
  if (si) {
    if (si[1] === 'youtube' && st.ytFailOnce) { st.ytFailOnce = false; await sleep(st.ytDelay); return reply(route, 500, { error: 'SYNTHETIC: the node could not answer' }); }
    if (m === 'DELETE') { st.outs += 1; if (si[1] === 'tidal') st.tidalIn = false; }
    const body = signin(si[1]); return body ? reply(route, 200, body) : reply(route, 404, { error: 'none' });
  }
  const R = {
    '/me': { role: 'owner', key: null, zones: null }, '/info': { name: 'SYNTHETIC node', version: '0.0.0-fixture', library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0 } },
    '/sources': SOURCES, '/play-order': { order: [], services: [] }, '/node/apps': APPS,
    '/sources/qobuz/unofficial': { signed_in: false, enabled: false, app_id: '', app_from: null, health: null },
    '/sources/spotify/status': { configured: false, signed_in: false, redirect_uri: EVIL },
    '/sources/spotify/librespot': { enabled: false, supported: st.librespot, librespot: null, warning: 'SYNTHETIC unofficial', zones: [], device_name: 'SYNTHETIC', output: 'local', running: false },
    '/sources/subsonic/server': { connected: false }, '/sources/jellyfin/server': { connected: false },
    '/sources/soundcloud/unofficial': { enabled: true, warning: 'SYNTHETIC unofficial', client_from: st.scClient ? 'settings' : null, client_id: st.scClient ? 'synthetic-client' : '', signed_in: false, health: null },
    '/sources/bandcamp/unofficial': { enabled: true, warning: 'SYNTHETIC unofficial', signed_in: false, formats: ['flac'], format: 'flac', folder: '/music/Bandcamp', health: null },
    '/sources/spotify/soloist': { supported: true, enabled: false, running: false, note: 'SYNTHETIC Soloist', key_set: false, soloist: null, can_download: true, device_name: 'SYNTHETIC', pipewire_device: '' },
    '/sources/youtube/unofficial': { enabled: true, warning: 'SYNTHETIC unofficial', ytdlp: null, can_download: true, health: null },
    '/library/identify': { counts: { total: 0, identified: 0, suggested: 0, unknown: 0, unchecked: 0 }, progress: { running: false }, acoustid: false, acoustid_state: 'missing', acoustid_own: false, write_ids: false },
    '/sources/jamendo/status': { configured: true, can_sign_in: true, signed_in: false, redirect_uri: 'https://synthetic.test/jamendo/callback' },
    '/listenbrainz': { connected: false }, '/lastfm': { configured: false, signed_in: false },
    '/library/drives': { providers: [{ id: 'google', name: 'Google Drive', configured: true, connected: false }, { id: 'onedrive', name: 'OneDrive', configured: true, connected: false }], roots: [] },
    '/library/folders': [], '/library/webdav': [], '/library/hidden': [], '/node/settings': { contact: '', discovery: false },
    '/outputs': [], '/zones': [], '/playlists': [], '/presence': [], '/favourites': { uris: [] }, '/library/albums': [], '/library/genres': [], '/library/tracks': { tracks: [], next: null },
  };
  if (v in R) return reply(route, 200, R[v]);
  return reply(route, 404, { error: 'SYNTHETIC: not in this fixture' });
}

const seen = new Set();
const banner = (page) => page.evaluate(() => {
  if (document.getElementById('synthbanner')) return;
  const b = document.createElement('div'); b.id = 'synthbanner'; b.textContent = 'SYNTHETIC TEST FIXTURE · no real sign-in';
  b.setAttribute('style', 'position:fixed;top:0;left:50%;transform:translateX(-50%);z-index:99999;background:#b00020;color:#fff;font:700 12px system-ui;padding:3px 10px;border-radius:0 0 6px 6px;pointer-events:none');
  document.body.appendChild(b);
});
const collect = async (page, label) => {
  const keys = await page.$$eval('.lguide', (els) => els.map((e) => e.dataset.guide));
  keys.forEach((k) => seen.add(k));
  // Every rendered guide: a heading in its summary, numbered steps, safe links, nothing evil
  const bad = await page.$$eval('.lguide', (els) => els.flatMap((e) => {
    const out = [];
    if (!e.querySelector(':scope > summary .lguide-h:is(h2,h4)')) out.push(`${e.dataset.guide}: no heading`);
    // Headings never skip a level after the page's last one before the guide
    const hs = [...document.querySelectorAll('h1,h2,h3,h4,h5')]; const at = hs.indexOf(e.querySelector('.lguide-h'));
    const prev = hs.slice(0, at).filter((h) => !h.closest('.lguide')).pop();
    if (prev && +e.querySelector('.lguide-h').tagName[1] > +prev.tagName[1] + 1) out.push(`${e.dataset.guide}: heading skips from ${prev.tagName}`);
    if (e.querySelectorAll('ol.lguide-steps > li').length < 2) out.push(`${e.dataset.guide}: <2 steps`);
    for (const a of e.querySelectorAll('a')) if (a.target !== '_blank' || !/noopener/.test(a.rel) || !/^https:/.test(a.href)) out.push(`${e.dataset.guide}: unsafe link ${a.href}`);
    if (e.querySelector('img,script,iframe')) out.push(`${e.dataset.guide}: injected element`);
    return out;
  }));
  check(bad.length === 0, `${label}: every guide has a heading (no skipped level), numbered steps and safe official links`, bad);
  return keys;
};
const noOverflow = async (page, label) => {
  const r = await page.evaluate(() => {
    const w = document.documentElement.clientWidth;
    const over = [...document.querySelectorAll('.lguide')].filter((e) => e.getClientRects().length && e.getBoundingClientRect().right > w + 1).map((e) => e.dataset.guide);
    return { w, sw: document.documentElement.scrollWidth, over };
  });
  check(r.sw <= r.w + 1 && r.over.length === 0, `${label}: no sideways overflow`, r);
};
const shot = async (page, name) => {
  if (!SHOTS) return;
  await banner(page);
  await page.evaluate(() => { document.activeElement?.blur(); });
  // The shell scrolls #main itself: grow the viewport to the whole page, then restore it
  const viewport = page.viewportSize();
  const height = await page.evaluate(() => { const m = document.querySelector('#main'); return innerHeight + (m ? Math.max(0, m.scrollHeight - m.clientHeight) : 0); });
  await page.setViewportSize({ width: viewport.width, height: Math.min(6000, Math.ceil(height)) });
  await page.screenshot({ path: path.join(SHOTS, `${name}.png`), fullPage: true });
  await page.setViewportSize(viewport);
};
const guide = (page, key) => page.locator(`.lguide[data-guide="${key}"]`).first();
const isOpen = (page, key) => guide(page, key).evaluate((e) => e.open);

(async () => {
  if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
  const browser = await chromium.launch();
  const newPage = async (vp, token = TOKEN) => {
    const ctx = await browser.newContext({ viewport: vp, reducedMotion: 'reduce', colorScheme: 'dark' });
    await ctx.addInitScript((t) => { if (t) localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); HTMLMediaElement.prototype.play = async function () {}; }, token);
    await ctx.route('**/*', handle);
    await ctx.routeWebSocket(/\/v1\/events/, (ws) => { ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'b1', library_rev: 0, zones: [] })); });
    const page = await ctx.newPage();
    const errors = []; page.on('pageerror', (e) => errors.push(e.message));
    return { ctx, page, errors };
  };
  try {
    const VPS = [['desktop', { width: 1440, height: 1000 }], ['tablet', { width: 820, height: 1180 }], ['phone', { width: 390, height: 844 }]];
    for (const [vpName, vp] of VPS) {
      st = fresh();
      const { ctx, page, errors } = await newPage(vp);
      const go = async (url, sel) => { await page.goto(ORIGIN + url); await page.waitForSelector(sel, { timeout: 15000 }).catch(() => null); };

      // 1. TIDAL: the recommended way's guide open, an advanced way's closed until asked
      await go('/sources/tidal', '.lguide[data-guide="tidal:app"]');
      check(await guide(page, 'tidal:app').isVisible().catch(() => false), `${vpName}: TIDAL shows the recommended way's guide beside the chooser`);
      check(await isOpen(page, 'tidal:app').catch(() => false), `${vpName}: the recommended guide is open`);
      await collect(page, `${vpName} TIDAL`); await noOverflow(page, `${vpName} TIDAL`);
      await shot(page, `tidal-recommended-${vp.width}`);
      for (const w of ['import', 'device', 'paste']) {
        await page.locator(`input[name="way-tidal"][value="${w}"]`).check();
        await guide(page, `tidal:${w}`).waitFor({ timeout: 5000 }).catch(() => null);
        check(await guide(page, `tidal:${w}`).count() === 1 && await guide(page, 'tidal:app').count() === 0, `${vpName}: picking ${w} swaps in its guide`);
        check(!(await isOpen(page, `tidal:${w}`).catch(() => true)), `${vpName}: tidal:${w} is an advanced disclosure, closed`);
        await collect(page, `${vpName} TIDAL ${w}`);
      }
      await guide(page, 'tidal:paste').locator('summary').click();
      check(await isOpen(page, 'tidal:paste'), `${vpName}: the advanced guide opens from its heading`);
      await noOverflow(page, `${vpName} TIDAL paste open`);
      await shot(page, `tidal-advanced-${vp.width}`);
      // Signed in, then Sign out: the guide (with its relogin steps) is back
      st.tidalIn = true; await go('/sources/tidal', '[data-sg="out"]');
      check(await page.locator('.lguide').count() === 0, `${vpName}: signed in, no guide crowds the status line`);
      await page.locator('[data-sg="out"]').click();
      await guide(page, 'tidal:app').waitFor({ timeout: 5000 }).catch(() => null);
      check(st.outs === 1 && await guide(page, 'tidal:app').isVisible().catch(() => false), `${vpName}: after Sign out the sign-in guide is back`);
      check(/Sign out, then Sign in at TIDAL again/.test(await guide(page, 'tidal:app').innerText()), `${vpName}: the guide says how to recover by signing out and in again`);

      // 2. Spotify: own-app guide with the escaped redirect URI; librespot missing → install guide
      st.librespot = false;
      await go('/sources/spotify', '.lguide[data-guide="spotify:app"]');
      check(!(await isOpen(page, 'spotify:app')), `${vpName}: own-app help starts closed`);
      await guide(page, 'spotify:app').locator(':scope > summary').click();
      const code = await guide(page, 'spotify:app').locator('pre.lguide-code code').first().innerText().catch(() => '');
      check(code === EVIL, `${vpName}: a hostile redirect URI shows as text`, code);
      check(await page.evaluate(() => window.__pwned === undefined), `${vpName}: nothing injected ran`);
      check(await page.locator('input[name="way-spotify"][value="librespot"]').isDisabled(), `${vpName}: librespot can't be picked while it is missing`);
      const missG = page.locator('.lguide[data-guide="spotify:librespot"]', { has: page.locator('.lguide-missing') });
      check(await missG.count() === 1 && !(await missG.evaluate((e) => e.open)), `${vpName}: the install guide is there, closed, not crowding the chosen way's guide`);
      await missG.locator(':scope > summary').click({ timeout: 3000 }).catch(() => null);
      const subs = missG.locator('details.lguide-sub');
      check(await subs.count() === 2 && !(await subs.nth(0).evaluate((e) => e.open)) && !(await subs.nth(1).evaluate((e) => e.open)), `${vpName}: Linux install and Docker are separate closed disclosures`);
      for (let i = 0; i < 2; i++) await subs.nth(i).locator(':scope > summary').click({ timeout: 3000 }).catch(() => null);
      const miss = missG.locator('.lguide-missing');
      check(await miss.isVisible().catch(() => false), `${vpName}: the missing librespot shows its install guide`);
      check(/uname -m/.test(await miss.first().innerText().catch(() => '')) && /same architecture/.test(await miss.first().innerText().catch(() => '')), `${vpName}: host and container architecture are checked`);
      check(await guide(page, 'spotify:app').locator('ol.lguide-steps').count() === 1 && await page.locator('#spotapp ol').count() === 0, `${vpName}: Spotify's app steps show once (the guide), not twice`);
      const mt = await miss.innerText().catch(() => '');
      check(/Linux install/.test(mt) && /Docker/.test(mt) && /docker exec "\$NODE_CONTAINER" \/data\/bin\/librespot --version/.test(mt) && /rustls/.test(mt) && /not visible in Docker/.test(mt), `${vpName}: native and Docker steps with their checks`, mt.slice(0, 200));
      check(!/tab says how/i.test(await page.locator('#main').innerText()), `${vpName}: no vague pointer to another tab`);
      await collect(page, `${vpName} Spotify missing`); await noOverflow(page, `${vpName} Spotify missing`);
      await shot(page, `spotify-librespot-missing-${vp.width}`);
      st.librespot = true;
      await go('/sources/spotify', 'input[name="way-spotify"][value="librespot"]:not([disabled])');
      check(await page.locator('input[name="way-spotify"]:checked').inputValue() === 'app', `${vpName}: backend-recommended authentication default is preserved`);
      check(await page.locator('.ways input[name="way-spotify"]').first().inputValue() === 'librespot', `${vpName}: Connect pairing is the first method`);
      await page.locator('input[name="way-spotify"][value="app"]').check();
      check(!(await isOpen(page, 'spotify:app')), `${vpName}: own developer app instructions stay Advanced and closed`);
      const appWayText = await page.locator('.way', { has: page.locator('input[value="app"]') }).textContent();
      check(/Advanced/.test(appWayText), `${vpName}: own developer app is clearly marked Advanced`, appWayText);
      for (const w of ['librespot', 'librespot-file']) {
        await page.locator(`input[name="way-spotify"][value="${w}"]`).check();
        await guide(page, `spotify:${w}`).waitFor({ timeout: 5000 }).catch(() => null);
        check(await guide(page, `spotify:${w}`).count() === 1, `${vpName}: spotify:${w} guide shown when picked`);
        await collect(page, `${vpName} Spotify ${w}`);
      }
      check(await isOpen(page, 'spotify:librespot-file').then((o) => !o).catch(() => false), `${vpName}: credentials.json is advanced (closed)`);

      // 3. YouTube: loading, then an error with Try again, then the guide
      st.ytFailOnce = true; st.ytDelay = 600;
      await page.goto(ORIGIN + '/sources/youtube');
      await page.waitForSelector('#signin-youtube', { timeout: 15000 }).catch(() => null);
      check(await page.locator('#signin-youtube .lguide').count() === 0, `${vpName}: while loading, no guide shows yet`);
      await page.waitForSelector('#signin-youtube [data-sg="retry"]', { timeout: 5000 }).catch(() => null);
      check(await page.locator('#signin-youtube [role="alert"]').isVisible().catch(() => false), `${vpName}: a failed load says so, with Try again`);
      await shot(page, `youtube-error-${vp.width}`);
      await page.locator('#signin-youtube [data-sg="retry"]').click();
      await guide(page, 'youtube:key').waitFor({ timeout: 5000 }).catch(() => null);
      check(await guide(page, 'youtube:key').isVisible().catch(() => false), `${vpName}: Try again brings the sign-in and its guide`);
      await collect(page, `${vpName} YouTube key`);
      await page.locator('input[name="way-youtube"][value="cookies"]').check();
      await guide(page, 'youtube:cookies').waitFor({ timeout: 5000 }).catch(() => null);
      await collect(page, `${vpName} YouTube`);

      // 4. Qobuz, Subsonic, Jellyfin, Jamendo
      for (const [url, keys] of [['/sources/qobuz', ['qobuz:own', 'qobuz:import', 'qobuz:paste']], ['/sources/subsonic', ['subsonic:server']], ['/sources/jellyfin', ['jellyfin:server']], ['/sources/jamendo', ['jamendo:signin']]]) {
        await go(url, `.lguide[data-guide="${keys[0]}"]`);
        const got = await collect(page, `${vpName} ${url}`);
        check(keys.every((k) => got.includes(k)), `${vpName}: ${url} shows ${keys.join(', ')}`, got);
        await noOverflow(page, `${vpName} ${url}`);
      }
      await go('/sources/qobuz', '.lguide[data-guide="qobuz:own"]');
      await page.locator('#qbimport > summary').click().catch(() => null);
      await guide(page, 'qobuz:import').locator('summary').click().catch(() => null);
      check(/not issued to you, so don’t use such a file/.test(await guide(page, 'qobuz:import').locator('.lguide-needs').innerText().catch(() => '')), `${vpName}: Qobuz import guide warns about configs not issued to you`);

      // 5. Settings › Accounts (Last.fm, ListenBrainz) and Service apps
      await go('/settings/accounts', '.lguide[data-guide="lastfm:own"]');
      for (const w of ['import', 'paste']) { await page.locator(`input[name="way-lastfm"][value="${w}"]`).check().catch(() => null); await guide(page, `lastfm:${w}`).waitFor({ timeout: 5000 }).catch(() => null); await collect(page, `${vpName} Last.fm ${w}`); }
      await page.waitForSelector('.lguide[data-guide="listenbrainz:token"]', { timeout: 5000 }).catch(() => null);
      await collect(page, `${vpName} accounts`); await noOverflow(page, `${vpName} accounts`);
      await go('/settings/apps', '.lguide[data-guide^="app:"]');
      const appKeys = await collect(page, `${vpName} Service apps`);
      check(APPS.filter((a) => GUIDES[`app:${a.id}`]).every((a) => appKeys.includes(`app:${a.id}`)), `${vpName}: every Service app with setup has its guide`, appKeys);
      check(!(await page.locator('#main').innerText()).includes(CANARY) && !(await page.content()).includes(CANARY), `${vpName}: no secret canary reaches the page`);
      // As tools/ui-service-apps.js requires: service links only under Advanced
      const loose = await page.$$eval('#main a[href*="developer"], #main a[href*="console."]', (as) => as.filter((a) => !a.closest('details.app-adv')).map((a) => a.href));
      check(loose.length === 0, `${vpName}: Service apps' developer links sit under Advanced`, loose);
      await page.locator('details.app-adv', { has: page.locator('.lguide[data-guide="app:spotify"]') }).locator('> summary').click();
      check(await guide(page, 'app:spotify').isVisible(), `${vpName}: Advanced opens with the app's guide`);
      await noOverflow(page, `${vpName} Service apps`);
      await shot(page, `service-apps-${vp.width}`);

      // 6. Your library › Node library: WebDAV and the cloud drives
      await go('/library/node', '.lguide[data-guide="drive:google"]');
      const libKeys = await collect(page, `${vpName} library`);
      check(['drive:google', 'drive:onedrive', 'webdav:share'].every((k) => libKeys.includes(k)), `${vpName}: drives and WebDAV show their guides`, libKeys);
      await noOverflow(page, `${vpName} library`);

      // 6b. Identify: the AcoustID guide beside its key field, ending at Save key
      await go('/library/node', '.lguide[data-guide="acoustid:key"]');
      const ac = page.locator('#acidrow .lguide[data-guide="acoustid:key"]');
      check(await ac.count() === 1 && /Check, then Save key/.test(await ac.innerText()), `${vpName}: AcoustID guide sits in Identify's key row and ends with Save key`);
      await collect(page, `${vpName} Identify`);

      // 8. Every source the node has: its page shows its guides (ways once picked)
      for (const id of RUNTIME) {
        const keys = BY_SOURCE[id] || [];
        check(keys.length > 0, `source ${id}: classified in LOGIN_SOURCES`);
        for (const pass of id === 'soundcloud' ? [false, true] : [false]) {
          st.scClient = pass;
          await go(`/sources/${id}`, `#src-${id}`);
          await sleep(300);
          for (const k of keys.filter((x) => !(id === 'soundcloud' && (x === 'soundcloud:token') !== pass))) {
            const radio = page.locator(`#src-${id} input[name="way-${id}"][value="${k.split(':')[1]}"]`);
            if (await radio.count() && await radio.isEnabled()) await radio.check();
            await page.locator(`#src-${id} .lguide[data-guide="${k}"]`).first().waitFor({ state: 'attached', timeout: 5000 }).catch(() => null);
            check(await page.locator(`#src-${id} .lguide[data-guide="${k}"]`).count() >= 1, `source ${id} (${vpName}): ${k} on its own page`);
            await collect(page, `${vpName} ${id} ${k}`);
          }
          await noOverflow(page, `${vpName} ${id}`);
        }
      }
      await go('/sources/soundcloud', '.lguide[data-guide="soundcloud:client"]');
      await shot(page, `soundcloud-client-${vp.width}`);
      await go('/sources/plex', '.lguide[data-guide="plex:server"]');
      check(/getpass.getpass/.test(await guide(page, 'plex:server').textContent()), `${vpName}: Plex's guide reads secrets at a hidden prompt`);
      await shot(page, `plex-${vp.width}`);
      await go('/sources/archive', '.lguide[data-guide="source:open"]');
      await shot(page, `open-source-${vp.width}`);

      // 9. A pasted secret is masked until Show, and goes to the node unchanged (SYNTHETIC refusal)
      await go('/sources/tidal', 'input[name="way-tidal"][value="paste"]');
      await page.locator('input[name="way-tidal"][value="paste"]').check();
      const ta = page.locator('#tidalsignin textarea[name="text"]');
      await ta.fill(`${CANARY}-pasted-token`);
      check(await ta.evaluate((e) => getComputedStyle(e).webkitTextSecurity === 'disc'), `${vpName}: the pasted token is masked`);
      check(!(await page.locator('#main').innerText()).includes(CANARY), `${vpName}: the pasted token is not in the page text`);
      await shot(page, `paste-masked-${vp.width}`);
      const rv = page.locator('#tidalsignin [data-reveal]');
      await rv.click({ timeout: 3000 }).catch(() => null);
      check(await rv.getAttribute('aria-pressed', { timeout: 3000 }).catch(() => null) === 'true' && await ta.evaluate((e) => getComputedStyle(e).webkitTextSecurity !== 'disc'), `${vpName}: Show what I pasted reveals it, on request only`);
      await rv.click({ timeout: 3000 }).catch(() => null);
      check(await ta.evaluate((e) => getComputedStyle(e).webkitTextSecurity === 'disc'), `${vpName}: Hide masks it again`);
      await page.locator('#tidalsignin [data-sg=paste] button[type=submit]').click();
      await page.locator('#tidalsignin [data-sg="error"]:not([hidden])').waitFor({ timeout: 5000 }).catch(() => null);
      check(st.imported.includes(`${CANARY}-pasted-token`), `${vpName}: the paste still reaches the node as typed (form name text)`);
      check(await page.locator('#tidalsignin [data-sg="error"]').isVisible(), `${vpName}: the synthetic refusal shows; nothing pretends to sign in`);
      st.imported = [];

      // 10. Drives and token: the addresses and places that actually work
      await go('/settings/apps', '.lguide[data-guide="app:google-drive"]');
      check(/drives\/google\/callback/.test(await guide(page, 'app:google-drive').locator('pre').textContent().catch(() => '')), `${vpName}: Google Drive's guide gives the node's own callback (used with a secret)`);
      check(/Mobile and desktop applications/.test(await guide(page, 'app:onedrive').textContent()) && !/Add a single-page application/.test(await guide(page, 'app:onedrive').textContent()), `${vpName}: OneDrive's guide registers a public client, not a single-page app`);
      check(errors.length === 0, `${vpName}: no page errors`, errors);
      await ctx.close();

      // 7. The login page: Pubky Ring can't start here (SYNTHETIC), the token guide is beside the field
      const lp = await newPage(vp, null);
      await lp.page.goto(ORIGIN + '/');
      await lp.page.waitForSelector('.lguide[data-guide="node:ring"]', { timeout: 15000 }).catch(() => null);
      const lk = await collect(lp.page, `${vpName} login`);
      check(lk.includes('node:ring') && lk.includes('node:token'), `${vpName}: the login page has the Ring and token guides`, lk);
      await lp.page.locator('#logintoken > summary').click();
      check(await lp.page.locator('#logintoken .lguide[data-guide="node:token"]').isVisible().catch(() => false), `${vpName}: the token guide opens with the token field`);
      check(await lp.page.locator('#tok').getAttribute('type') === 'password', `${vpName}: the token field stays masked`);
      const tg = await lp.page.locator('#logintoken .lguide[data-guide="node:token"]').innerText();
      check(/\/etc\/pubky-music\/node\.toml/.test(tg) && /\/var\/lib\/pubky-music\/api-token/.test(tg) && /PM_TOKEN/.test(tg) && /docker exec pubky-music cat \/data\/api-token/.test(tg), `${vpName}: the token guide covers the Linux config, the generated file, Docker and the bundle's .env`);
      await noOverflow(lp.page, `${vpName} login`);
      await shot(lp.page, `login-${vp.width}`);
      check(lp.errors.length === 0, `${vpName}: no page errors on login`, lp.errors);
      await lp.ctx.close();
    }
    const missing = Object.keys(GUIDES).filter((k) => !seen.has(k));
    check(missing.length === 0, `every guide in LOGIN_GUIDES rendered (${seen.size}/${Object.keys(GUIDES).length})`, missing);
    check(external.length === 0, 'no request left the synthetic origin', [...new Set(external)]);
  } finally {
    await browser.close();
  }
  console.log(failed ? `\n${failed} check(s) FAILED` : '\nall checks passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
