// Sign-in block regression checks (Qobuz has its own guided page: tools/ui-stream-setup.js; the
// "only what works" rules: tools/ui-signin-simple.js). Provider replies are mocked here; pm-api's
// Rust tests cover validation against mock provider servers and the private files on disk.
// Run against a disposable fixture node:
//   node tools/ui-signin.js <api token> <node URL> [screenshot directory]
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const SHOTS = process.argv[4];
// A way that is not the main action: opened from "Other ways to sign in", then pasted
const cases = [
  { source: 'youtube', url: '/sources/youtube', mount: '#signin-youtube', way: 'cookies', label: 'yt-dlp cookies', text: '# Netscape HTTP Cookie File\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t4000000000\tSID\tsynthetic-cookie\n' },
  { source: 'lastfm', url: '/settings/accounts', mount: '#lfbody', way: 'paste', label: 'Pasted sign-in', text: '0123456789abcdef0123456789abcdef\nfedcba9876543210fedcba9876543210\nsynthetic-session\n' },
];
const LIBRESPOT = { supported: true, enabled: true, device_name: 'Node', output: 'local', zone_id: '', bitrate: '320', zones: [], running: false, warning: 'librespot is a community-made Spotify client, not approved by Spotify. It can stop working.', librespot: { path: '/usr/bin/librespot', from: 'path', version: '0.6.0' } };
const CREDS = '{"username":"listener","auth_type":1,"auth_data":"c3ludGhldGlj"}';
(async () => {
  if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
  const initial = {};
  for (const source of ['youtube', 'lastfm', 'spotify']) {
    const r = await fetch(`${BASE}/v1/sources/${source}/signin`, { headers: { authorization: `Bearer ${TOKEN}` } });
    assert.equal(r.status, 200);
    initial[source] = await r.json();
  }
  const browser = await chromium.launch();
  try {
    for (const width of [1440, 390]) for (const theme of ['dark', 'light']) {
      const ctx = await browser.newContext({ viewport: { width, height: width === 390 ? 844 : 1000 }, colorScheme: theme, reducedMotion: 'reduce' });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); HTMLMediaElement.prototype.play = async function () {}; }, TOKEN);
      const page = await ctx.newPage();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      // YouTube's cookies file is offered once YouTube plays on your speakers
      await page.route(/\/v1\/sources\/youtube\/unofficial$/, (route) => route.fulfill({ json: { enabled: true, can_download: true, warning: 'On your speakers.' } }));
      for (const c of cases) {
        const si = structuredClone(initial[c.source]); si.in_use = null; si.chosen = null; si.line = 'Not signed in'; si.imports = [];
        si.ways.forEach((w) => { w.available = true; delete w.reason; });
        let submitted = 0; let signedOut = 0;
        const endpoint = new RegExp(`/v1/sources/${c.source}/signin(?:/import)?$`);
        await page.route(endpoint, async (route) => {
          const req = route.request();
          const body = req.method() === 'POST' || req.method() === 'PUT' ? req.postDataJSON() : null;
          if (req.method() === 'POST') {
            if (body.text === 'not a sign-in') return route.fulfill({ status: 400, json: { error: 'Not usable: the service refused this sign-in' } });
            assert.equal(body.text, c.text, `${c.source}: the submitted file/paste reaches the API`);
            submitted++;
            si.chosen = c.way; si.in_use = { way: c.way, label: c.label, user: 'listener', gives: 'playback' }; si.line = `${c.label} · listener`;
          } else if (req.method() === 'PUT') si.chosen = body.way;
          else if (req.method() === 'DELETE') { signedOut++; si.in_use = null; si.line = 'Not signed in'; }
          return route.fulfill({ json: si });
        });
        await page.goto(BASE + c.url);
        const panel = page.locator(c.mount);
        await panel.locator('[data-sg=action]').waitFor();
        assert.equal(await panel.locator('input[type=radio]').count(), 0, 'no radio buttons');
        // The recommended way is the action; the rest a line each under a closed list
        assert.equal(await panel.locator('[data-sg=other]').evaluate((d) => d.open), false);
        await panel.locator('[data-sg=other] summary').click();
        await panel.locator(`[data-sg-way="${c.way}"]`).click();
        await panel.locator('[data-sg=action] textarea').waitFor();
        await page.evaluate(() => { document.activeElement?.blur(); for (const el of document.querySelectorAll('*')) if (el.scrollTop) el.scrollTo({ top: 0, left: 0, behavior: 'instant' }); window.scrollTo({ top: 0, left: 0, behavior: 'instant' }); });
        if (SHOTS) {
          // The shell scrolls #main internally. Extend the capture height so the screenshot
          // includes the whole sign-in at the requested width, then restore the test viewport.
          const viewport = page.viewportSize();
          const height = await page.evaluate(() => { const m = document.querySelector('#main'); return innerHeight + Math.max(0, m.scrollHeight - m.clientHeight); });
          await page.setViewportSize({ width, height: Math.ceil(height) });
          await page.screenshot({ path: path.join(SHOTS, `13-signin-${c.source}-${width}-${theme}.png`), fullPage: true });
          await page.setViewportSize(viewport);
        }
        // A refused import remains visible and keyboard users can return to the other ways.
        await panel.locator('[data-sg=action] textarea').fill('not a sign-in');
        await panel.locator('[data-sg=paste] button').click();
        await panel.locator('[data-sg=error]:not([hidden])').waitFor();
        await panel.locator('[data-sg=another]').click();
        assert.equal(await page.evaluate(() => !!document.activeElement.dataset.sgWay), true, 'Try another way goes to the other ways');
        await panel.locator('[data-sg=action] textarea').fill(c.text);
        await panel.locator('[data-sg=paste] button').click();
        await panel.locator('[data-sg=out]').waitFor();
        assert.equal(submitted, 1);
        // Signed in: one row with who and what it gives
        assert.equal(await panel.locator('[data-sg=signed-in]').count(), 1);
        assert.match(await panel.locator('[data-sg=signed-in]').innerText(), /Signed in as listener[\s\S]*Playback/);
        assert.equal(await panel.locator('textarea').count(), 0, 'saved sign-in contents are cleared');
        // Change: the ways other than the one in use; Cancel goes back
        await panel.locator('[data-sg=change]').click();
        await panel.locator('[data-sg=action]').waitFor();
        assert.equal(await panel.locator(`[data-sg-way="${c.way}"]`).count(), 0, 'the way in use is not offered again');
        await panel.locator('[data-sg=change]').click();
        await panel.locator('[data-sg=out]').click();
        await panel.locator('[data-sg=action]').waitFor();
        assert.equal(signedOut, 1);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false, 'no horizontal overflow');
        await page.unroute(endpoint);
        console.log(`ok ${c.source} ${width} ${theme}: other way, refusal, import, change, sign out`);
      }
      // Spotify: librespot is a way to play, with its credentials.json under it (once installed)
      {
        const si = structuredClone(initial.spotify); si.in_use = null; si.chosen = null;
        si.ways.forEach((w) => { w.available = true; delete w.reason; });
        let submitted = 0;
        await page.route(/\/v1\/sources\/spotify\/librespot$/, (route) => route.fulfill({ json: LIBRESPOT }));
        await page.route(/\/v1\/sources\/spotify\/signin(?:\/import|\?way=[\w-]+)?$/, (route) => {
          const req = route.request();
          if (req.method() === 'POST') {
            const body = req.postDataJSON();
            if (body.text === 'not a sign-in') return route.fulfill({ status: 400, json: { error: 'Not usable: not a credentials.json' } });
            assert.equal(body.text, CREDS); submitted++;
            si.chosen = 'librespot-file'; si.in_use = { way: 'librespot-file', label: 'librespot, from a credentials file', user: 'listener', gives: 'playback into a zone' };
          } else if (req.method() === 'DELETE') si.in_use = null;
          return route.fulfill({ json: si });
        });
        await page.goto(BASE + '/sources/spotify');
        await page.locator('#spotplay:not([hidden]) #lsfile').waitFor();
        assert.equal(await page.locator('#signin-spotify [data-sg-way]').count(), 0, 'librespot is not a sign-in way');
        await page.locator('#lsfile summary').click();
        await page.locator('#lsfilepaste textarea').fill('not a sign-in'); await page.locator('#lsfilepaste button').click();
        await page.locator('#lsfileerr:not([hidden])').waitFor();
        await page.locator('#lsfilepaste textarea').fill(CREDS); await page.locator('#lsfilepaste button').click();
        await page.locator('#lsout').waitFor();
        assert.equal(submitted, 1);
        assert.match(await page.locator('#splibrespot').innerText(), /Signed in as listener/);
        await page.locator('#lsout').click();
        await page.locator('#lsfile').waitFor();
        await page.unroute(/\/v1\/sources\/spotify\/librespot$/); await page.unroute(/\/v1\/sources\/spotify\/signin(?:\/import|\?way=[\w-]+)?$/);
        console.log(`ok spotify ${width} ${theme}: librespot's credentials.json, refusal, sign out`);
      }
      assert.deepEqual(errors, []);
      await ctx.close();
    }
  } finally { await browser.close(); }
})().catch((e) => { console.error(e); process.exitCode = 1; });
