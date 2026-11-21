// Phone mode's own songs ("My songs"): with the `songs` and `add-songs` capabilities the phone's
// backend lists the songs added to it (/v1/offline, the offline page's shape) and takes new ones
// through the node's upload routes (/v1/library/uploads). At 360 and 1280 px:
//   * the Library tab (phone) or the sidebar's My songs opens "My songs", with the phone's song and
//     an "Add songs" button; nothing says Downloads;
//   * files picked in the system's picker (the page's file input) go up with the node's upload
//     protocol (POST, then PUT from offset 0), each once, and the list is read again after;
//   * Shuffle plays the phone's file (/v1/library/tracks/<id>/file).
//   node tools/ui-phone-songs.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-songs-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const INFO = { mode: 'phone', capabilities: ['search', 'items', 'sources', 'songs', 'add-songs'], name: 'This phone', api: 'v1', version: '0.2.4', offline: false,
  library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0, total_duration_ms: 0 }, zones: 0, outputs: 0, scanning: false };
const SONG = { id: 1, title: 'Pocket Song', artists: [{ id: 7, name: 'Phone Band' }], album: { id: 9, title: 'Phone Album', year: 2026 }, duration_ms: 61000, track_no: 1, disc_no: 1,
  file: { container: 'flac', format: { kind: 'pcm', bits: 16, rate_hz: 44100, channels: 2 } }, offline: { bytes: 1234, format: 'original' } };
const LIST = { offline: false, device_id: null, synced_at: null, bytes: 1234, queued: 0, tracks: [SONG], albums: [{ id: 9, title: 'Phone Album', year: 2026, track_ids: [1], cover: false }], playlists: [], favourites: { track_ids: [] } };

const run = async (browser, w) => {
  const r = await fx.create(browser, w, { mobile: w < 600 });
  r.reqs = []; r.offline = 0;
  await r.context.route((u) => u.origin === fx.ORIGIN && u.pathname.startsWith('/v1/'), async (route) => {
    const req = route.request(); const u = new URL(req.url()); const v = u.pathname.slice(3);
    r.reqs.push(`${req.method()} ${v}${u.search}`);
    const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    if (v === '/info') return json(INFO);
    if (v === '/me') return json({ role: 'owner', zones: [] });
    if (v === '/zones') return json([]);
    if (v === '/sources') return json([]);
    if (v === '/play-order') return json({ order: [], services: [] });
    if (v === '/offline') { r.offline++; return json(LIST); }
    if (v === '/library/uploads' && req.method() === 'POST') return json({ id: 'up1', offset: 0, chunk_size: 4194304, already: false });
    if (v === '/library/uploads/up1' && req.method() === 'PUT') return json({ offset: Number(req.headers()['content-length'] || 4), state: 'stored' });
    if (/^\/library\/tracks\/1\/file/.test(v)) return route.fulfill({ status: 404, body: '' });
    return json({ error: 'Not available on this phone' }, 503);
  });
  await r.context.addInitScript(() => {
    window.__TAURI__ = { core: { invoke: () => new Promise(() => {}) } };
    window.__audioPlays = [];
    const play0 = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () { window.__audioPlays.push(this.src); return play0.call(this).catch(() => {}); };
  });
  return r;
};
const shown = (page, sel) => page.evaluate((s) => { const el = document.querySelector(s); return !!el && getComputedStyle(el).display !== 'none' && !el.closest('[hidden]'); }, sel);

(async () => {
  const browser = await chromium.launch();
  for (const w of [360, 1280]) {
    const at = `${w}px`;
    const r = await run(browser, w); const p = r.page;
    await fx.go(p, '/');
    await p.waitForFunction(() => document.body.classList.contains('phone'), null, { timeout: 5000 }).catch(() => {});
    await sleep(500);
    if (w < 600) {
      check(await shown(p, '[data-bnav="library"]'), `${at}: the Library tab is offered (the phone's songs)`);
      await p.click('[data-bnav="library"]');
    } else {
      check(await shown(p, '.nav-item.nav-offline'), `${at}: "My songs" is in the sidebar`);
      check((await p.locator('.nav-item.nav-offline').innerText()).trim() === 'My songs', `${at}: it is called My songs`);
      await p.click('.nav-item.nav-offline');
    }
    await p.waitForFunction(() => /Pocket Song/.test((document.querySelector('#main') || {}).innerText || ''), null, { timeout: 5000 }).catch(() => {});
    const main = await p.locator('#main').innerText();
    check(/My songs/.test(main) && /Pocket Song/.test(main), `${at}: My songs lists the phone's song`);
    check(!/Downloads|Offline:/.test(main), `${at}: nothing says Downloads or Offline`);
    check(await shown(p, '#phoneadd'), `${at}: "Add songs" is there`);
    await p.screenshot({ path: path.join(SHOTS, `my-songs-${w}.png`) });
    // Picked in the system's picker: sent with the node's upload protocol, once
    const before = r.offline;
    await p.setInputFiles('#phoneaddin', { name: 'New Song.flac', mimeType: 'audio/flac', buffer: Buffer.from('fLaCdata') });
    await p.waitForFunction(() => /added/i.test(document.body.innerText), null, { timeout: 5000 }).catch(() => {});
    await sleep(500);
    const ups = r.reqs.filter((x) => x.includes('/library/uploads'));
    check(ups.filter((x) => x.startsWith('POST /library/uploads')).length === 1, `${at}: the picked file starts one upload (${ups.join(', ')})`);
    check(ups.some((x) => x.startsWith('PUT /library/uploads/up1?offset=0')), `${at}: and its bytes go up from the start`);
    check(r.offline > before, `${at}: the list is read again after`);
    // Shuffle plays the phone's own file
    await p.click('[data-dlplay="all"]').catch(() => {});
    await sleep(800);
    const plays = await p.evaluate(() => window.__audioPlays);
    check(plays.some((u) => /\/v1\/library\/tracks\/1\/file/.test(u)), `${at}: Shuffle plays the phone's file (${plays.join(', ')})`);
    check(!r.state.errors.length, `${at}: no page errors (${r.state.errors.join(' | ')})`);
    await r.context.close();
  }
  await browser.close();
  if (failed) { console.error(`${failed} check(s) failed`); process.exit(1); }
  console.log('my songs on the phone: listed, added through the upload routes, played from the phone');
})();
