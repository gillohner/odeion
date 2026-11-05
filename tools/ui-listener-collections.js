// A listener's own collections (TASK-listener-personal-library-1104, PR 2):
//   * a listener has Playlists (no longer sent to Albums); before their homeserver is connected
//     the page asks them to connect it, and a change that needs it opens the Pubky Ring dialog;
//   * connected: their own, made, opened, shared on their homeserver (the link is theirs) and
//     "Add to playlist…" in a menu; lost (revoked in Ring): "Approve this node again";
//   * none of the owner's: no imports, smart playlists, Play from orders or pins, "Save as
//     playlist" from a queue;
//   * the owner's pages are as they were.
// The node's /v1/me, /v1/me/homeserver… and /v1/playlists… are mocked here (pm-api's
// tests/listener_collections.rs covers them against a fake homeserver).
//   node tools/ui-listener-collections.js <api token> <node URL> [screenshot dir]
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const OUT = process.argv[4] || null;
const ANN = 'o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy';
const SONG = 'https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69';
const ID = '0034A0Z1Y2X3W';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const QR = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4" width="120" height="120"><rect width="4" height="4" fill="#fff"/><rect width="2" height="2" fill="#000"/></svg>';

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 820, 390]) {
      const ctx = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: 'reduce', colorScheme: 'dark' });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); HTMLMediaElement.prototype.play = async function () {}; }, TOKEN);
      const page = await ctx.newPage();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      const shot = async (name) => { if (OUT) { fs.mkdirSync(OUT, { recursive: true }); await page.screenshot({ path: path.join(OUT, `${width}-${name}.png`) }); } };
      // ---- the mocked node: Ann, a listener, and her collections ----
      let listener = true; let hs = { connected: false, lost: false, pending: 0, stale: false }; let polls = 0;
      let mine = []; const asked = [];
      const summary = (c) => ({ id: c.id, name: c.name, items: c.items.length, created_at: 1, updated_at: 2, visibility: c.public ? 'public' : 'private', synced: !hs.pending, covers: [], layout: 'list', tracks_only: true });
      const detail = (c) => ({ summary: summary(c), playlist: { name: c.name, items: c.items.map((uri) => ({ uri, added_at: 1 })), created_at: 1, updated_at: 2 }, tracks: c.items.map(() => null), entries: c.items.map((uri, index) => ({ index, uri, kind: 'track', added_at: 1 })), local_tracks: [] });
      const refused = (r) => r.fulfill({ status: 409, json: { error: 'Connect your homeserver first', needs_connect: true, lost: hs.lost } });
      await page.route(/\/v1\/me(\?.*)?$/, (r) => (listener ? r.fulfill({ json: { role: 'listener', key: ANN, zones: null } }) : r.fallback()));
      await page.route(/\/v1\/me\/homeserver(\?.*)?$/, (r) => r.fulfill({ json: hs }));
      await page.route(/\/v1\/me\/homeserver\/connect$/, (r) => { polls = 0; return r.fulfill({ json: { id: 'c1', url: 'pubkyauth:///?caps=/priv/social/v1/posts/:rw', qr_svg: QR } }); });
      await page.route(/\/v1\/me\/homeserver\/connect\/poll\?/, (r) => { polls += 1; if (polls < 2) return r.fulfill({ json: { state: 'pending' } }); hs = { connected: true, lost: false, pending: 0, stale: false }; return r.fulfill({ json: { state: 'ok' } }); });
      await page.route(/\/v1\/(playlists|collections|smart)(\/.*)?(\?.*)?$/, (r) => {
        if (!listener) return r.fallback();
        const req = r.request(); const m = req.method(); const u = new URL(req.url()).pathname.replace(/^\/v1/, '');
        asked.push(`${m} ${u}`);
        if (m === 'GET' && u === '/playlists') return r.fulfill({ json: hs.connected ? mine.map(summary) : [] });
        if (u === '/smart' || /play-from|\/order|\/source|from-queue|import/.test(u)) return r.fulfill({ status: 403, json: { error: 'Not for listeners: that is the owner’s' } });
        const c = mine.find((x) => u.startsWith(`/playlists/${x.id}`));
        if (m === 'GET') return c ? r.fulfill({ json: detail(c) }) : r.fulfill({ status: 404, json: { error: 'no collection' } });
        if (!hs.connected || hs.lost) return refused(r);
        if (m === 'POST' && u === '/playlists') { const b = req.postDataJSON(); const n = { id: ID, name: b.name, items: b.uris || [], public: false }; mine.push(n); return r.fulfill({ json: detail(n) }); }
        if (!c) return r.fulfill({ status: 404, json: { error: 'no collection' } });
        if (m === 'PUT' && u.endsWith('/visibility')) { c.public = req.postDataJSON().public; return r.fulfill({ json: detail(c) }); }
        if (m === 'PUT') { const b = req.postDataJSON(); if (b.name) c.name = b.name; return r.fulfill({ json: detail(c) }); }
        if (m === 'POST' && u.endsWith('/items')) { c.items.push(...(req.postDataJSON().uris || [])); return r.fulfill({ json: detail(c) }); }
        return r.fulfill({ json: detail(c) });
      });
      const go = async (p) => { await page.goto(BASE + p); await page.locator('#main h1, #main h2').first().waitFor({ timeout: 20000 }); await sleep(400); };
      const main = () => page.locator('#main').innerText();

      // ---- not connected: Playlists asks to connect ----
      await go('/playlists');
      assert.match(await main(), /Playlists/, 'a listener has Playlists (not sent to Albums)');
      assert.match(await page.locator('#hsnote').innerText(), /collections are kept on your own homeserver/);
      assert.doesNotMatch(await main(), /Import M3U|Smart playlists/, 'imports and smart playlists stay the owner’s');
      assert.ok(!asked.some((a) => a.includes('/smart')), 'smart playlists aren’t even asked for');
      await shot('collections-connect');
      // Creating one before connecting opens the Ring dialog; approved, it goes again
      await page.locator('#plname').fill('Road trip');
      await page.locator('#plcreate').click();
      const dlg = page.locator('dialog.dialog');
      await dlg.locator('.qr svg').waitFor();
      assert.match(await dlg.locator('h2').innerText(), /Connect your homeserver/);
      await dlg.waitFor({ state: 'detached', timeout: 10000 });
      await page.locator('[data-pl]').first().waitFor({ timeout: 5000 });
      assert.equal(mine.length, 1, 'created once connected');
      assert.equal(await page.locator('#hsnote').count(), 0, 'connected, nothing to say');
      assert.match(await page.locator('[data-pl]').first().innerText(), /Road trip[\s\S]*synced/i);
      await shot('collections-list');

      // ---- her collection's page: hers to change and share, nothing of the owner's ----
      await page.locator('[data-pl] .name').first().click();
      await page.locator('#plpubky').waitFor();
      await sleep(300);
      let txt = await main();
      assert.match(txt, /on your homeserver/);
      assert.equal(await page.locator('#plorder, #plorderopen, .pfnote, [data-psrc]').count(), 0, 'no Play from orders or pins');
      assert.ok(!asked.some((a) => /play-from/.test(a)), 'play-from (the owner’s) isn’t asked');
      await page.locator('#plshare').click();
      const share = page.locator('dialog.dialog');
      assert.match(await share.innerText(), /A public copy goes to your homeserver: the songs/);
      await share.locator('button', { hasText: 'Share' }).click();
      await page.locator('#plpubky .sharelink').waitFor();
      assert.match(await page.locator('#plpubky .sharelink').innerText(), new RegExp(`^pubky://${ANN}/pub/social/v1/posts/${ID}$`), 'the link is on her homeserver');
      await shot('collection-shared');
      // Her ⋯ menu offers Add to playlist… (no longer hidden for listeners)
      await page.locator('#plmore').click();
      await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to playlist…' }).first().waitFor();
      await page.keyboard.press('Escape'); await sleep(200);
      // Revoked in Ring: a change opens "Approve this node again"
      hs = { ...hs, lost: true };
      await page.locator('#plmore').click();
      await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Rename…' }).first().click();
      const ren = page.locator('dialog.dialog');
      await ren.locator('input').fill('Road trip 26');
      await ren.locator('button', { hasText: 'Save' }).click();
      await page.locator('dialog.dialog h2').filter({ hasText: /Approve this node again/ }).waitFor();
      await shot('collection-lost');
      await page.locator('[data-hsclose]').click(); await sleep(300);
      await go('/playlists');
      assert.match(await page.locator('#hsnote').innerText(), /no longer lets this node in/);
      hs = { connected: true, lost: false, pending: 1, stale: false };
      await go('/playlists');
      assert.match(await page.locator('#hsnote').innerText(), /1 change waits on this node/);
      assert.match(await page.locator('[data-pl]').first().innerText(), /waiting/i);
      if (width < 1100) {
        for (const sel of ['#plcreate', '[data-plplay]']) {
          const h = await page.locator(sel).first().evaluate((e) => e.getBoundingClientRect().height);
          assert.ok(h >= 40, `${sel} is 40 px to touch at ${width} (${h})`);
        }
      }
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth) <= 0, `no sideways scroll at ${width}`);

      // ---- the owner's pages are as they were ----
      listener = false;
      await go('/playlists');
      txt = await main();
      assert.match(txt, /Import M3U/); assert.match(txt, /Smart playlists/);
      assert.equal(await page.locator('#hsnote').count(), 0);
      await shot('owner-playlists');

      assert.deepEqual(errors, []);
      await ctx.close();
      console.log(`ok ${width}: listener collections (connect, create, share, lost, pending), none of the owner's, owner unchanged`);
    }
  } finally { await browser.close(); }
})().catch((e) => { console.error(e); process.exit(1); });
