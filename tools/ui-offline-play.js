// Offline playback in the Android app (crates/pm-app-player through src/native.rs; the offline
// page is pm-offline's server): the page inside the app (window.pmDesktop with nativePlayer,
// Tauri's invoke stood in for by a fake player). Real candidate assets on the fixture origin
// (tools/ui-fixture.js):
//   * offline (no node), the Downloads view's songs play through the app's own player (the
//     offline page's URL, never <audio>), and next goes on through it;
//   * online, a song downloaded to the phone plays from its copy: Now playing › Details › Output
//     says "From this phone".
//   node tools/ui-offline-play.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'offline-play-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tracks = [1, 2].map((id) => ({ ...fx.track, id, title: `Song ${id}` }));

// The app's player, as in tools/ui-native-player.js: songs of `len` ms; `fromDevice` says it
// plays the copy downloaded here
const FAKE_APP = ({ len, fromDevice }) => {
  window.pmDesktop = { version: '0.2.2', platform: 'android', nativePlayer: true };
  const calls = window.__calls = []; window.__audioPlays = [];
  const play0 = HTMLMediaElement.prototype.play;
  HTMLMediaElement.prototype.play = function () { window.__audioPlays.push(this.src); return play0.call(this); };
  const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
  const s = { url: null, transport: 'stopped', position_ms: 0, duration_ms: null, ended: 0, advanced: 0, error: null, refused: null, taken: false, broke: false, now_playing: null, from_device: false, output: 'cpal:Speaker or headphones', device: null, ahead: null };
  window.__app = s;
  const send = () => { const { ahead, ...st } = s; window.dispatchEvent(new CustomEvent('pmdesktop:native', { detail: JSON.parse(JSON.stringify(st)) })); };
  const path = { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] };
  setInterval(() => {
    if (s.transport !== 'playing') return;
    s.position_ms += 100;
    if (s.position_ms >= len) { s.ended += 1; if (s.ahead) { s.url = s.ahead; s.ahead = null; s.position_ms = 0; s.advanced += 1; } else { s.transport = 'stopped'; s.position_ms = 0; s.now_playing = null; } }
    send();
  }, 100);
  window.__TAURI_INTERNALS__ = { invoke: async (cmd, args) => {
    calls.push([cmd, args || {}]);
    if (cmd === 'native_load') { Object.assign(s, { url: args.url, ahead: args.ahead || null, transport: 'playing', position_ms: args.atMs || 0, duration_ms: len, from_device: fromDevice, now_playing: { source_format: fmt, output_format: fmt, signal_path: path, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } } }); send(); return null; }
    if (cmd === 'native_ahead') { s.ahead = args.url || null; return null; }
    if (cmd === 'native_play') { s.transport = 'playing'; send(); return null; }
    if (cmd === 'native_pause') { s.transport = 'paused'; send(); return null; }
    if (cmd === 'native_seek') { s.position_ms = args.ms; send(); return null; }
    if (cmd === 'native_stop') { Object.assign(s, { url: null, transport: 'stopped', now_playing: null }); send(); return null; }
    if (cmd === 'downloads_state') return { items: [], songs: 2, bytes: 1, track_ids: [1, 2], album_ids: [7], playlist_ids: [], favourites: false, limit: 1e10, other_bytes: 0, free: 1e10, keep_free: 1e9, progress: {} };
    return null;
  } };
};
const calls = (page, cmd) => page.evaluate((c) => window.__calls.filter((x) => !c || x[0] === c), cmd);
const now = (page) => page.evaluate(() => window.pmPlayer.now());

(async () => {
  const browser = await chromium.launch();
  for (const w of [390, 1280]) {
    const at = `${w}px`;
    // Offline: the app's local page, the songs kept here
    const run = await fx.create(browser, w, { offline: true, tracks });
    await run.context.addInitScript(FAKE_APP, { len: 1500, fromDevice: true });
    await fx.go(run.page, '/downloads');
    await run.page.waitForFunction(() => !!window.pmPlayer); await sleep(800);
    const p = run.page;
    await p.click('[data-dlplay="all"]').catch(() => {}); await sleep(600);
    const loads = await calls(p, 'native_load');
    check(loads.length === 1 && /\/v1\/library\/tracks\/[12]\/file/.test(loads[0][1].url) && loads[0][1].url.startsWith(fx.ORIGIN), `${at}: offline, a kept song plays through the app's player from the offline page (${JSON.stringify(loads.map((l) => l[1].url))})`);
    check((await p.evaluate(() => window.__audioPlays.length)) === 0, `${at}: never through <audio>`);
    check(!!(loads[0] && loads[0][1].ahead), `${at}: the next kept song is handed ahead (${JSON.stringify(loads[0] && loads[0][1].ahead)})`);
    await p.waitForFunction(() => window.pmPlayer.now().index === 1, null, { timeout: 5000 }).catch(() => {});
    const n = await now(p);
    check(n.index === 1 && !n.paused && (await calls(p, 'native_load')).length === 1, `${at}: and next goes on through it (${JSON.stringify(n)})`);
    await p.screenshot({ path: path.join(SHOTS, `offline-playing-${w}.png`) });
    check(!run.state.errors.length, `${at}: offline: no page errors (${run.state.errors.join(' | ')})`);
    await run.context.close();

    // Online, a song downloaded here: Details says it plays from this phone
    const on = await fx.create(browser, w, { tracks });
    await on.context.addInitScript(FAKE_APP, { len: 60000, fromDevice: true });
    await fx.go(on.page, '/'); await on.page.waitForFunction(() => !!window.pmPlayer); await sleep(500);
    await on.page.evaluate(() => window.pmPlayer.play([{ uri: 'track:1', track_id: 1, title: 'Song 1' }])); await sleep(600);
    await on.page.click('#npart'); await on.page.waitForSelector('#nowplaying:not([hidden])', { timeout: 5000 }); await sleep(400);
    if (await on.page.evaluate(() => { const b = !document.querySelector('#nowplaying .np-output') && document.querySelector('#nowplaying [data-npsheet="details"]'); if (b) b.click(); return !!b; })) await sleep(400);
    const o = await on.page.evaluate(() => { const d = document.querySelector('#nowplaying .np-output'); return d ? d.textContent : null; });
    check(!!o && /From this phone/.test(o), `${at}: Details › Output says it plays from this phone ("${o}")`);
    await on.page.screenshot({ path: path.join(SHOTS, `from-this-phone-${w}.png`) });
    check(!on.state.errors.length, `${at}: online: no page errors (${on.state.errors.join(' | ')})`);
    await on.context.close();
    // A streamed song: no "From this phone"
    const st2 = await fx.create(browser, w, { tracks });
    await st2.context.addInitScript(FAKE_APP, { len: 60000, fromDevice: false });
    await fx.go(st2.page, '/'); await st2.page.waitForFunction(() => !!window.pmPlayer); await sleep(500);
    await st2.page.evaluate(() => window.pmPlayer.play([{ uri: 'track:1', track_id: 1, title: 'Song 1' }])); await sleep(600);
    await st2.page.click('#npart'); await st2.page.waitForSelector('#nowplaying:not([hidden])', { timeout: 5000 }); await sleep(400);
    if (await st2.page.evaluate(() => { const b = !document.querySelector('#nowplaying .np-output') && document.querySelector('#nowplaying [data-npsheet="details"]'); if (b) b.click(); return !!b; })) await sleep(400);
    const o2 = await st2.page.evaluate(() => { const d = document.querySelector('#nowplaying .np-output'); return d ? d.textContent : null; });
    check(!!o2 && !/From this phone/.test(o2), `${at}: a streamed song's Output line doesn't say "From this phone" ("${o2}")`);
    await st2.context.close();
  }
  await browser.close();
  if (failed) { console.error(`${failed} check(s) failed`); process.exit(1); }
  console.log('downloaded songs play in the app, offline too');
})();
