// A listener's own bookmarks and sign-ins (TASK-listener-personal-library-1104):
//   * a listener sees Favourites and the hearts; before their homeserver is connected the page
//     asks them to connect it (one Pubky Ring approval), and a heart that needs it opens that;
//   * connected: their own bookmarks; lost (revoked in Ring): the node's copy and "Approve again";
//     changes waiting on the node say so;
//   * Settings › You: My homeserver (connect, disconnect), My streaming accounts (their own
//     sign-ins, never the node's apps), and no "your own come in a later version";
//   * the owner's pages are as they were.
// The node's /v1/me, /v1/bookmarks (/favourites), /v1/me/homeserver… and /v1/me/services are
// mocked here (pm-api's tests/listener_library.rs covers them against a fake homeserver).
//   node tools/ui-listener-library.js <api token> <node URL> [screenshot dir]
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const OUT = process.argv[4] || null;
const ANN = 'o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy';
const SONG = 'https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const QR = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4" width="120" height="120"><rect width="4" height="4" fill="#fff"/><rect width="2" height="2" fill="#000"/></svg>';

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 820, 390]) {
      const ctx = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: 'reduce', colorScheme: 'dark', hasTouch: width < 1100 });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); HTMLMediaElement.prototype.play = async function () {}; }, TOKEN);
      const page = await ctx.newPage();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      const shot = async (name) => { if (OUT) { fs.mkdirSync(OUT, { recursive: true }); await page.screenshot({ path: path.join(OUT, `${width}-${name}.png`) }); } };
      // ---- the mocked node: Ann, a listener ----
      let listener = true; let hs = { connected: false, lost: false, pending: 0, stale: false }; let mine = []; let polls = 0; let refuse = null;
      const writes = [];
      await page.route(/\/v1\/me(\?.*)?$/, (r) => (listener ? r.fulfill({ json: { role: 'listener', key: ANN, zones: null } }) : r.fallback()));
      await page.route(/\/v1\/(bookmarks|favourites)(\?.*)?$/, (r) => {
        if (!listener) return r.fallback();
        const req = r.request(); const m = req.method();
        if (m === 'GET') return r.fulfill({ json: { tracks: [], albums: [], artists: [], other: mine.slice(), posts: [], uris: mine.slice(), homeserver: hs } });
        if (!hs.connected || hs.lost || refuse) { writes.push({ m, refused: true }); return r.fulfill({ status: 409, json: { error: 'Connect your homeserver first', needs_connect: true, lost: hs.lost } }); }
        const uri = m === 'PUT' ? req.postDataJSON().uri : new URL(req.url()).searchParams.get('uri');
        writes.push({ m, uri });
        mine = mine.filter((u) => u !== uri); if (m === 'PUT') mine.push(uri);
        return r.fulfill({ json: { uri, bookmarked: m === 'PUT', favourite: m === 'PUT', ...(hs.pending ? { pending: true } : {}) } });
      });
      await page.route(/\/v1\/me\/homeserver(\?.*)?$/, (r) => { if (r.request().method() === 'DELETE') { hs = { ...hs, connected: false }; writes.push({ m: 'DISCONNECT' }); } return r.fulfill({ json: hs }); });
      await page.route(/\/v1\/me\/homeserver\/connect$/, (r) => { polls = 0; return r.fulfill({ json: { id: 'c1', url: 'pubkyauth:///?caps=/priv/social/v1/bookmarks/:rw', qr_svg: QR } }); });
      await page.route(/\/v1\/me\/homeserver\/connect\/poll\?/, (r) => { polls += 1; if (polls < 2) return r.fulfill({ json: { state: 'pending' } }); hs = { connected: true, lost: false, pending: 0, stale: false }; return r.fulfill({ json: { state: 'ok' } }); });
      await page.route(/\/v1\/me\/services$/, (r) => (listener ? r.fulfill({ json: [
        { source: 'tidal', name: 'TIDAL', connected: false, available: true, setup: '/v1/sources/tidal/signin' },
        { source: 'qobuz', name: 'Qobuz', connected: false, available: false, reason: 'the owner hasn’t set up a Qobuz app', setup: '/v1/sources/qobuz/signin' },
        { source: 'spotify', name: 'Spotify', connected: true, user: 'ann-spot', way: 'app', available: true, setup: '/v1/sources/spotify/signin' }] }) : r.fallback()));
      await page.route(/\/v1\/sources\/tidal\/signin$/, (r) => (listener && r.request().method() === 'GET' ? r.fulfill({ json: { source: 'tidal', chosen: 'own-device', in_use: null, ways: [
        { id: 'app', label: 'TIDAL’s own sign-in', available: true }, { id: 'device', label: 'A device code', available: true, recommended: true }, { id: 'own-device', label: 'A device client of your own', available: true }],
        details: { unofficial: { client_from: null }, status: { signed_in: false } }, imports: [], places: [] } }) : r.fallback()));
      await page.route(/\/v1\/sources\/spotify\/signin$/, (r) => (listener && r.request().method() === 'GET' ? r.fulfill({ json: { source: 'spotify', chosen: 'app', in_use: null, ways: [{ id: 'app', label: 'Spotify’s own sign-in', available: true, recommended: true }],
        details: { status: { configured: true, signed_in: false, app_from: 'settings', client_id: 'nodesclientid0123', redirect_uri: 'http://node.example/v1/sources/spotify/callback' }, librespot: {} }, imports: [], places: [] } }) : r.fallback()));
      const go = async (p) => { await page.goto(BASE + p); await page.locator('#main h1, #main h2').first().waitFor({ timeout: 20000 }); await sleep(400); };

      // ---- not connected: Favourites asks to connect ----
      await go('/favourites');
      assert.ok(await page.locator('#main').innerText().then((t) => /Favourites/.test(t)), 'a listener has Favourites (not sent to Albums)');
      assert.match(await page.locator('#hsnote').innerText(), /own homeserver/);
      await shot('bookmarks-connect');
      // Connect: the Ring QR, the link, then theirs
      await page.locator('[data-hsconnect]').click();
      const dlg = page.locator('dialog.dialog');
      await dlg.locator('.qr svg').waitFor();
      assert.match(await dlg.locator('a.btn').getAttribute('href'), /^pubkyauth:/);
      await shot('connect-dialog');
      await dlg.waitFor({ state: 'detached', timeout: 10000 });
      await page.locator('#main h1, #main h2').first().waitFor(); await sleep(500);
      assert.equal(await page.locator('#hsnote').count(), 0, 'connected, nothing to say');
      // ---- a heart on a song page is theirs ----
      await go('/song/lib-1');
      const heart = page.locator('#ifav');
      assert.ok(await heart.isVisible(), 'the heart is there for a listener');
      await heart.click(); await sleep(500);
      assert.ok(writes.some((w) => w.m === 'PUT' && !w.refused), JSON.stringify(writes));
      // Revoked in Ring: the heart opens "Approve this node again"
      hs = { ...hs, lost: true };
      await go('/song/lib-1');
      await page.locator('#ifav').click();
      await page.locator('dialog.dialog h2').filter({ hasText: /Approve this node again/ }).waitFor();
      await page.locator('[data-hsclose]').click(); await sleep(200);
      await go('/favourites');
      assert.match(await page.locator('#hsnote').innerText(), /no longer lets this node in/);
      await shot('bookmarks-lost');
      hs = { connected: true, lost: false, pending: 2, stale: false };
      await go('/favourites');
      assert.match(await page.locator('#hsnote').innerText(), /2 changes wait on this node/);

      // ---- Settings › You ----
      hs = { connected: true, lost: false, pending: 0, stale: false };
      await go('/settings');
      const txt = await page.locator('#main').innerText();
      assert.doesNotMatch(txt, /later version/, 'the old promise is gone');
      assert.match(txt, /My homeserver/); assert.match(txt, /Connected: your bookmarks are kept there/);
      assert.match(txt, /My streaming accounts/i); assert.match(txt, /Signed in as ann-spot/); assert.match(txt, /hasn’t set up a Qobuz app/);
      assert.doesNotMatch(txt, /Service apps|Device client ID|App ID and secret/, 'never the node’s own apps');
      await shot('settings-listener');
      // TIDAL: their own sign-in ways; the node's device client is the owner's to set
      await page.locator('details[data-svc="tidal"] > summary').click();
      await page.locator('details[data-svc="tidal"] .way').first().waitFor();
      assert.equal(await page.locator('#tfclient').count(), 0, 'no device client form for a listener');
      assert.match(await page.locator('details[data-svc="tidal"]').innerText(), /set up by whoever runs this node/);
      await shot('settings-tidal');
      // Spotify: their own sign-in; the node's app (its client id, Change, Forget) stays the owner's
      await page.locator('details[data-svc="spotify"] > summary').click();
      await page.locator('details[data-svc="spotify"] #spotin').waitFor();
      const sp = await page.locator('details[data-svc="spotify"]').innerText();
      assert.doesNotMatch(sp, /nodesclientid0123|Your Spotify app/, 'never the node’s Spotify app');
      assert.equal(await page.locator('details[data-svc="spotify"] #spotapp, details[data-svc="spotify"] #spotappchange').count(), 0);
      // Disconnect
      await page.locator('#hsoff').click(); await sleep(400);
      assert.ok(writes.some((w) => w.m === 'DISCONNECT'));
      assert.match(await page.locator('#main').innerText(), /Not connected/);
      if (width < 1100) {
        for (const sel of ['#hson', 'details[data-svc] > summary']) {
          const h = await page.locator(sel).first().evaluate((e) => e.getBoundingClientRect().height);
          assert.ok(h >= 40, `${sel} is 40 px to touch at ${width} (${h})`);
        }
      }
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth) <= 0, `no sideways scroll at ${width}`);

      // ---- the owner's pages are as they were ----
      listener = false;
      await go('/settings');
      assert.doesNotMatch(await page.locator("#main").innerText(), /My homeserver|My streaming accounts/i);
      await go('/favourites');
      assert.equal(await page.locator('#hsnote').count(), 0);
      await shot('owner-bookmarks');

      assert.deepEqual(errors, []);
      await ctx.close();
      console.log(`ok ${width}: listener bookmarks (connect, lost, pending), hearts, Settings › You, owner unchanged`);
    }
  } finally { await browser.close(); }
})().catch((e) => { console.error(e); process.exit(1); });
