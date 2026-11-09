// The Android app's own player behind "This phone" (crates/pm-app-player, src/native.rs): the page
// inside the app (window.pmDesktop with nativePlayer, Tauri's invoke stood in for by a fake that
// plays like the app does and sends `pmdesktop:native` events). Real candidate assets on the
// fixture origin (tools/ui-fixture.js):
//   * in the app, a library song goes to the app's player (native_load with the page's absolute
//     URL), never to <audio>; the next song is handed ahead; the bar, pause, play, seek and the
//     page's Media Session follow the app's events;
//   * at the end of a song the app already plays the one handed ahead: the page moves on without
//     opening it again (gapless), and hands the next; repeat one hands the same song;
//   * Now playing › Details › Output reads the phone zone's words ("Bit-perfect · FiiO KA17 ·
//     44.1 kHz / 16 bit"), and the Sound panel's path is the app's;
//   * a stream the app refuses plays in <audio>, and Details says Android mixer for it;
//   * the node takes the phone's output: paused, and the bar says why;
//   * the phone's own zone is not a second entry in the phone's picker;
//   * a plain browser, and the app without the player, keep <audio> and call nothing.
//   node tools/ui-native-player.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'native-player-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const f16 = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, duration_ms: 240000 }));
const OUT = 'net:192.168.1.20:7790/cpal:Speaker or headphones';
const KA17 = { name: 'FiiO KA17', kind: 'usb_dac', summary: 'FiiO KA17', rates: [44100, 48000, 96000], bits: [16, 24, 32], max_rate: 96000, max_bits: 32, channels: 2, hardware_volume: false, mode: 'bit_perfect', modes: ['shared', 'bit_perfect'], confirmed: true, format: f16 };
const ownZone = { id: 'pixel-8-net', name: 'Pixel 8 · Speaker or headphones', output: OUT, transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: null, now_playing: null };

// The app's side, in the page: invoke() as Tauri's, a player that plays like the app's (each song
// `len` ms, faster than life), its status sent as the app sends it
const FAKE_APP = ({ nativePlayer, len }) => {
  window.pmDesktop = { version: '0.2.2', platform: 'android', ...(nativePlayer ? { nativePlayer: true } : {}), outputs: ['net:192.168.1.20:7790/cpal:Speaker or headphones'], output: 'net:192.168.1.20:7790/cpal:Speaker or headphones' };
  const calls = window.__calls = []; window.__audioPlays = [];
  const play0 = HTMLMediaElement.prototype.play;
  HTMLMediaElement.prototype.play = function () { window.__audioPlays.push(this.src); return play0.call(this); };
  const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
  const s = window.__app = { url: null, transport: 'stopped', position_ms: 0, duration_ms: null, ended: 0, error: null, refused: null, taken: false, now_playing: null, output: 'cpal:Speaker or headphones', device: null, ahead: null };
  const send = () => { const { ahead, ...st } = s; window.dispatchEvent(new CustomEvent('pmdesktop:native', { detail: JSON.parse(JSON.stringify(st)) })); };
  window.__send = send;
  const path = { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }, { name: 'output', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] };
  const start = (url, at) => { Object.assign(s, { url, transport: 'playing', position_ms: at, duration_ms: len, error: null, refused: null, taken: false, now_playing: { source_format: fmt, output_format: fmt, signal_path: path, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } } }); send(); };
  setInterval(() => {
    if (s.transport !== 'playing') return;
    s.position_ms += 100;
    if (s.position_ms >= len) {
      s.ended += 1;
      if (s.ahead) { const a = s.ahead; s.ahead = null; s.url = a; s.position_ms = 0; } else { s.transport = 'stopped'; s.position_ms = 0; s.now_playing = null; }
    }
    send();
  }, 100);
  const invoke = async (cmd, args) => {
    calls.push([cmd, args || {}]);
    if (cmd === 'native_load') {
      if (/refuse/.test(args.url)) throw "can't decode it here: unsupported codec";
      s.ahead = args.ahead || null; start(args.url, args.atMs || 0); return null;
    }
    if (cmd === 'native_ahead') { s.ahead = args.url || null; return null; }
    if (cmd === 'native_play') { if (s.transport !== 'playing') { s.transport = 'playing'; s.taken = false; s.error = null; send(); } return null; }
    if (cmd === 'native_pause') { if (s.transport === 'playing') { s.transport = 'paused'; send(); } return null; }
    if (cmd === 'native_seek') { s.position_ms = args.ms; send(); return null; }
    if (cmd === 'native_stop') { Object.assign(s, { url: null, transport: 'stopped', position_ms: 0, now_playing: null, ahead: null }); send(); return null; }
    throw `no command ${cmd}`;
  };
  window.__TAURI_INTERNALS__ = { invoke };
};

async function open(browser, w, app) {
  const run = await fx.create(browser, w, { tracks, fixtures: { '/zones': { body: [ownZone] }, '/outputs': { body: [{ id: OUT, name: ownZone.name, protocol: 'net', caps: { pcm_rates: [44100, 48000] } }] } } });
  if (app) await run.context.addInitScript(FAKE_APP, app);
  else await run.context.addInitScript(() => { window.__calls = []; window.__audioPlays = []; const p0 = HTMLMediaElement.prototype.play; HTMLMediaElement.prototype.play = function () { window.__audioPlays.push(this.src); return p0.call(this); }; });
  await fx.go(run.page, '/');
  await run.page.waitForFunction(() => !!window.pmPlayer);
  await sleep(400);
  return run;
}
const calls = (page, cmd) => page.evaluate((c) => window.__calls.filter((x) => !c || x[0] === c), cmd);
const now = (page) => page.evaluate(() => window.pmPlayer.now());
const queue = (n) => tracks.slice(0, n).map((t) => ({ uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 240000 }));
// Now playing › Details › Output (the Details tab on wide screens; always there on phones)
const detailsOutput = async (page) => {
  await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden])', { timeout: 5000 }); await sleep(400);
  if (await page.evaluate(() => { const b = !document.querySelector('#nowplaying .np-output') && document.querySelector('#nowplaying [data-npsheet="details"]'); if (b) b.click(); return !!b; })) await sleep(400);
  return page.evaluate(() => { const d = document.querySelector('#nowplaying .np-output'); return d ? d.textContent : null; });
};
const fileUrl = (id) => `${fx.ORIGIN}/v1/library/tracks/${id}/file?token=fixture-owner`;

(async () => {
  const browser = await chromium.launch();
  for (const w of [390, 1280]) {
    const at = `${w}px`;
    // The app with its player; each song lasts 1.5 s here
    const run = await open(browser, w, { nativePlayer: true, len: 1500 });
    const { page, state } = run;
    await page.evaluate((q) => window.pmPlayer.play(q), queue(3));
    await sleep(400);
    const loads = await calls(page, 'native_load');
    check(loads.length === 1 && loads[0][1].url === fileUrl(1), `${at}: the app's player opens the page's absolute URL (${JSON.stringify(loads[0] && loads[0][1].url)})`);
    check(loads[0] && loads[0][1].ahead === fileUrl(2), `${at}: and the next song is handed ahead (${JSON.stringify(loads[0] && loads[0][1].ahead)})`);
    check((await page.evaluate(() => window.__audioPlays.length)) === 0 && !(await page.evaluate(() => document.querySelector('#browser').getAttribute('src'))), `${at}: <audio> plays nothing`);
    let n = await now(page);
    check(!n.paused && n.index === 0, `${at}: the bar says it plays (${JSON.stringify(n)})`);
    const ms = await page.evaluate(() => navigator.mediaSession && navigator.mediaSession.playbackState);
    check(ms === 'playing', `${at}: the Media Session says playing (${ms})`);
    // Pause and play: the app's commands; the bar follows its events
    await page.click('#toggle'); await sleep(300);
    n = await now(page);
    check(n.paused && (await calls(page, 'native_pause')).length === 1, `${at}: pause reaches the app (${JSON.stringify(n)})`);
    check((await page.evaluate(() => navigator.mediaSession.playbackState)) === 'paused', `${at}: and the Media Session says paused`);
    await page.click('#toggle'); await sleep(300);
    check(!(await now(page)).paused && (await calls(page, 'native_play')).length === 1, `${at}: play goes on in the app (no new load)`);
    // The end of song 1: the app already plays song 2; the page moves on without opening it
    await page.waitForFunction(() => window.pmPlayer.now().index === 1, null, { timeout: 5000 }).catch(() => {});
    n = await now(page);
    check(n.index === 1 && !n.paused, `${at}: at the end the page is on song 2 (${JSON.stringify(n)})`);
    check((await calls(page, 'native_load')).length === 1, `${at}: gapless: song 2 was not opened again (${(await calls(page, 'native_load')).length} loads)`);
    await sleep(700);
    const aheads = await calls(page, 'native_ahead');
    check(aheads.some((a) => a[1].url === fileUrl(3)), `${at}: and song 3 is handed ahead (${JSON.stringify(aheads.map((a) => a[1].url))})`);
    // Details › Output: the phone zone's words for what the app plays to
    await page.evaluate((d) => { window.__app.device = d; window.__send(); }, KA17); await sleep(300);
    const outLine = await detailsOutput(page);
    check(outLine === 'Bit-perfect · FiiO KA17 · 44.1 kHz / 16 bit', `${at}: Details › Output reads the DAC line ("${outLine}")`);
    await page.screenshot({ path: path.join(SHOTS, `details-native-${w}.png`) });
    await page.keyboard.press('Escape'); await sleep(300);
    // Repeat one: the same song is handed ahead
    await page.evaluate(() => { localStorage.setItem('pm.repeat', 'one'); }); await sleep(700);
    const last = (await calls(page, 'native_ahead')).pop(); const cur = (await now(page)).src;
    check(last && last[1].url === cur, `${at}: repeat one hands the same song ahead (${JSON.stringify(last && last[1].url)}, playing ${cur})`);
    await page.evaluate(() => localStorage.setItem('pm.repeat', 'off'));
    // The node takes the phone's output: paused here, saying why
    await page.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(400);
    await page.evaluate(() => { Object.assign(window.__app, { transport: 'paused', taken: true, error: "the node plays on this device's zone now" }); window.__send(); }); await sleep(500);
    n = await now(page);
    const bar = await page.evaluate(() => document.querySelector('#np').innerText);
    check(n.paused && /the node plays on this device/.test(bar), `${at}: taken by the node: paused, and the bar says why`);
    // The phone's own zone is not a second entry
    await page.click('#zonebtn'); await sleep(300);
    const entries = await page.evaluate(() => [...document.querySelectorAll('#zonepop [data-zone]')].map((b) => b.dataset.zone));
    check(entries.includes('browser') && !entries.includes(ownZone.id), `${at}: the phone's picker has one This phone (${entries.join(', ')})`);
    await page.keyboard.press('Escape'); await sleep(200);
    // A stream the app can't play: <audio> plays it, and Details says Android mixer
    await page.evaluate(() => window.pmPlayer.play([{ uri: 'http://odeion-fixture.test/v1/sources/x/stream/refuse', title: 'A playlist stream' }]));
    await sleep(800);
    const plays = await page.evaluate(() => window.__audioPlays);
    check(plays.some((u) => /refuse/.test(u)), `${at}: refused by the app, <audio> plays it (${JSON.stringify(plays)})`);
    // (the fixture's <audio> plays nothing: it says it does)
    await page.evaluate(() => document.querySelector('#browser').dispatchEvent(new Event('playing'))); await sleep(300);
    await page.screenshot({ path: path.join(SHOTS, `refused-${w}.png`) });
    const outLine2 = await detailsOutput(page).catch((e) => `(${e.message.split('\n')[0]})`);
    check(!!outLine2 && /^Android mixer/.test(outLine2) && /can’t play this stream itself/.test(outLine2), `${at}: Details says Android mixer for it ("${outLine2}")`);
    await page.keyboard.press('Escape');
    check(!state.errors.length, `${at}: no page errors (${state.errors.join(' | ')})`);
    await run.context.close();

    // A plain browser, and the app without the player: <audio>, nothing called
    for (const [what, app] of [['a plain browser', null], ['the app without its player', { nativePlayer: false, len: 1500 }]]) {
      const r = await open(browser, w, app);
      await r.page.evaluate((q) => window.pmPlayer.play(q), queue(2)); await sleep(500);
      const c = await calls(r.page);
      const ap = await r.page.evaluate(() => window.__audioPlays);
      check(c.length === 0 && ap.length === 1 && /\/v1\/library\/tracks\/1\/file/.test(ap[0]), `${at}: ${what} plays in <audio> and calls nothing (${c.length} calls, ${JSON.stringify(ap)})`);
      check(!r.state.errors.length, `${at}: ${what}: no page errors (${r.state.errors.join(' | ')})`);
      await r.context.close();
    }
  }
  await browser.close();
  if (failed) { console.error(`${failed} check(s) failed`); process.exit(1); }
  console.log('the app’s own player drives This phone');
})();
