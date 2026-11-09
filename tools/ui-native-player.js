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
const MIRROR = fs.readFileSync(path.join(path.resolve(process.argv[2] || '.'), 'crates/pm-desktop/src/media.rs'), 'utf8').match(/pub const BRIDGE_JS: &str = r#"([\s\S]*?)"#;/)[1];
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
const FAKE_APP = ({ nativePlayer, len, slow }) => {
  window.pmDesktop = { version: '0.2.2', platform: 'android', ...(nativePlayer ? { nativePlayer: true } : {}), outputs: ['net:192.168.1.20:7790/cpal:Speaker or headphones'], output: 'net:192.168.1.20:7790/cpal:Speaker or headphones' };
  const calls = window.__calls = []; window.__audioPlays = [];
  const play0 = HTMLMediaElement.prototype.play;
  HTMLMediaElement.prototype.play = function () { window.__audioPlays.push(this.src); return play0.call(this); };
  const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
  const s = window.__app = { url: null, transport: 'stopped', position_ms: 0, duration_ms: null, ended: 0, advanced: 0, error: null, refused: null, taken: false, broke: false, now_playing: null, output: 'cpal:Speaker or headphones', device: null, ahead: null };
  const send = () => { const { ahead, ...st } = s; window.dispatchEvent(new CustomEvent('pmdesktop:native', { detail: JSON.parse(JSON.stringify(st)) })); };
  window.__send = send;
  const path = { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }, { name: 'output', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] };
  const start = (url, at) => { Object.assign(s, { url, transport: 'playing', position_ms: at, duration_ms: len, error: null, refused: null, taken: false, now_playing: { source_format: fmt, output_format: fmt, signal_path: path, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } } }); send(); };
  setInterval(() => {
    if (s.transport !== 'playing') return;
    s.position_ms += 100;
    if (s.position_ms >= len) {
      s.ended += 1;
      if (s.ahead) { const a = s.ahead; s.ahead = null; s.url = a; s.position_ms = 0; s.advanced += 1; } else { s.transport = 'stopped'; s.position_ms = 0; s.now_playing = null; }
    }
    send();
  }, 100);
  const invoke = async (cmd, args) => {
    calls.push([cmd, args || {}]);
    if (cmd === 'native_load') {
      if (/refuse/.test(args.url)) throw { refused: true, message: "the app can't play this itself: unsupported codec" };
      if (/busy/.test(args.url)) throw { refused: false, message: 'HTTP 503' };
      const gen = s.gen = (s.gen || 0) + 1;
      s.loading = true;
      if (slow) await new Promise((r) => setTimeout(r, slow));
      if (gen === s.gen) s.loading = false;
      // A later load overtook this one (as the app's generation does)
      if (gen !== s.gen) throw { refused: false, overtaken: true, message: 'another song or a stop came first' };
      if (window.__failLoad) { Object.assign(s, { url: null }); send(); throw { refused: false, overtaken: false, message: 'the node did not answer' }; }
      s.ahead = args.ahead || null; start(args.url, args.atMs || 0); return null;
    }
    if (cmd === 'native_ahead') { s.ahead = args.url || null; return null; }
    if (cmd === 'native_play') { if (s.loading) return null; if (window.__failPlay) throw { refused: false, overtaken: false, message: 'HTTP 503' }; if (s.transport !== 'playing') { s.transport = 'playing'; s.taken = false; s.broke = false; s.error = null; send(); } return null; }
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
  if (app && app.mirror) {
    await run.context.addInitScript(() => { window.__media = []; window.OdeionAndroidPort = { postMessage: (m) => window.__media.push(JSON.parse(m)) }; });
    await run.context.addInitScript(MIRROR);
  }
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
    {
      const r = await open(browser, w, { nativePlayer: true, len: 60000, mirror: true });
      const p = r.page;
      await p.evaluate((q) => window.pmPlayer.play(q), queue(3)); await sleep(500);
      let mirror = await p.evaluate(() => window.__media.filter((m) => m.t === 'session').at(-1));
      check(await p.evaluate(() => document.querySelector('#browser').paused) && mirror && !mirror.paused && mirror.native && mirror.state === 'playing' && mirror.pos > 0 && mirror.dur === 60, `${at}: the actual bridge mirrors native timing while DOM audio stays paused (${JSON.stringify(mirror)})`);
      await p.evaluate(() => window.dispatchEvent(new CustomEvent('pmdesktop:media', { detail: { action: 'seekto', seekTime: 12 } }))); await sleep(200);
      check((await calls(p, 'native_seek')).some((c) => c[1].ms === 12000), `${at}: the session seek reaches the native engine`);
      await p.evaluate(() => window.dispatchEvent(new CustomEvent('pmdesktop:media', { detail: { action: 'pause' } }))); await sleep(200);
      mirror = await p.evaluate(() => window.__media.filter((m) => m.t === 'session').at(-1));
      check(mirror.paused && mirror.state === 'paused' && (await calls(p, 'native_pause')).length > 0, `${at}: session pause reaches the engine and mirror`);
      await p.evaluate(() => window.dispatchEvent(new CustomEvent('pmdesktop:media', { detail: { action: 'nexttrack' } }))); await sleep(300);
      check((await now(p)).index === 1 && (await calls(p, 'native_load')).at(-1)[1].url === fileUrl(2), `${at}: session Next retains the page queue and native decoder`);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(150);
      r.state.fixtures['/outputs'] = { body: [{ id: OUT, name: ownZone.name, protocol: 'net', caps: { pcm_rates: [44100,48000] }, device: KA17 }] };
      await p.evaluate(() => window.dispatchEvent(new Event('pmdesktop:output'))); await sleep(400);
      check(await p.evaluate(() => (localStorage.getItem('pm.zone') || 'browser') === 'browser'), `${at}: paused native DAC arrival keeps This phone selected`);
      await p.evaluate(() => { window.pmDesktop.phoneVolume = { kind: 'db', min: -60, max: 0, step: 1, value: -24, muted: false }; window.dispatchEvent(new Event('pmdesktop:mediavolume')); });
      const volume = await p.evaluate(() => { const s = document.querySelector('#vol'); return { min: +s.min, max: +s.max, value: +s.value }; });
      check(volume.min === -60 && volume.max === 0 && volume.value === -24, `${at}: native slider reads the DAC dB level (${JSON.stringify(volume)})`);
      await p.evaluate(() => { const s = document.querySelector('#vol'); s.value = '-30'; s.dispatchEvent(new Event('change', { bubbles: true })); });
      check(await p.evaluate(() => window.__media.some((m) => m.t === 'volume' && m.native && m.db === -30) && document.querySelector('#browser').volume === 1), `${at}: native slider sends actual DAC volume, with no PCM gain`);
      await p.evaluate(() => { window.pmDesktop.phoneVolume = null; window.dispatchEvent(new Event('pmdesktop:mediavolume')); });
      check(await p.evaluate(() => document.querySelector('.player').classList.contains('novol')), `${at}: fixed native output has no ineffective STREAM_MUSIC slider`);
      check(!r.state.hits.some((h) => /\/zones\/.*\/volume/.test(h.v || '')), `${at}: native volume does not set a node zone`);
      const beforeRetry = (await calls(p, 'native_load')).length;
      await p.evaluate((url) => {
        window.dispatchEvent(new CustomEvent('pmdesktop:native-command', { detail: { id: 1, action: 'play' } }));
        Object.assign(window.__app, { url: null, transport: 'stopped' }); window.__send();
        window.dispatchEvent(new CustomEvent('pmdesktop:native-failed', { detail: { id: 1, url, at: 12, error: { refused: false, overtaken: false, message: 'fixture reopen failed' } } }));
      }, fileUrl(2));
      await sleep(2400);
      const retry = (await calls(p, 'native_load')).at(-1);
      check((await calls(p, 'native_load')).length === beforeRetry + 1 && retry[1].atMs === 12000, `${at}: failed direct session Play reaches the existing one-retry path at its saved position`);
      const afterRetry = (await calls(p, 'native_load')).length;
      await p.evaluate((url) => {
        window.dispatchEvent(new CustomEvent('pmdesktop:native-command', { detail: { id: 2, action: 'play' } }));
        Object.assign(window.__app, { url: null, transport: 'stopped' }); window.__send();
        window.dispatchEvent(new CustomEvent('pmdesktop:native-command', { detail: { id: 3, action: 'pause' } }));
        window.dispatchEvent(new CustomEvent('pmdesktop:native-failed', { detail: { id: 2, url, at: 12, error: { refused: true, overtaken: false, message: 'late refusal' } } }));
      }, fileUrl(2));
      await sleep(2200);
      check((await calls(p, 'native_load')).length === afterRetry && await p.evaluate(() => window.__audioPlays.length === 0) && (await now(p)).paused, `${at}: a later system pause cancels stale retry and refused-stream fallback`);
      await p.click('#toggle'); await sleep(250);
      check((await calls(p, 'native_load')).at(-1)[1].atMs >= 12000, `${at}: Play after a cancelled reopen failure keeps its saved position`);
      check(!r.state.errors.length, `${at}: combined media/native bridge has no page errors (${r.state.errors.join(' | ')})`);
      await p.screenshot({ path: path.join(SHOTS, `native-media-${w}.png`) });
      await r.context.close();
    }
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

    // A DAC-refused resume may be reported before or after the command reply. It stays paused
    // with a Play button and never retries onto the speaker or the WebView.
    for (const order of ['before', 'after']) {
      const r = await open(browser, w, { nativePlayer: true, len: 60000 });
      const p = r.page;
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(500);
      await p.click('#toggle'); await sleep(200);
      const loads = (await calls(p, 'native_load')).length;
      await p.evaluate((order) => {
        const invoke = window.__TAURI_INTERNALS__.invoke;
        window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
          if (cmd !== 'native_play') return invoke(cmd, args);
          window.__calls.push([cmd, args || {}]);
          const pause = () => { Object.assign(window.__app, { transport: 'paused', error: 'FiiO KA17 is no longer where this phone plays: paused', taken: false, broke: false }); window.__send(); };
          if (order === 'before') pause(); else setTimeout(pause, 30);
          return null;
        };
      }, order);
      await p.click('#toggle'); await sleep(2200);
      check((await now(p)).paused && await p.getAttribute('#toggle', 'aria-label') === 'Play', `${at}: refused resume ${order} reply leaves the page paused with Play`);
      check(await p.evaluate(() => navigator.mediaSession.playbackState === 'paused' && /no longer where/.test(document.querySelector('#np').innerText)), `${at}: refused resume ${order} reply reports the reason and paused media state`);
      check((await calls(p, 'native_load')).length === loads && await p.evaluate(() => window.__audioPlays.length === 0 && window.__app.transport === 'paused'), `${at}: refused resume ${order} reply never retries on another output`);
      await r.context.close();
    }

    // Review round 1: a busy node isn't a refusal; a restored position is where it opens; a pause
    // while it opens holds; repeat one goes on without opening the song again
    {
      const r = await open(browser, w, { nativePlayer: true, len: 1500, slow: 400 });
      const p = r.page;
      await p.evaluate(() => window.pmPlayer.play([{ uri: 'http://odeion-fixture.test/v1/sources/x/stream/busy', title: 'Busy' }]));
      await sleep(1200);
      const ap = await p.evaluate(() => window.__audioPlays);
      const nn = await p.evaluate(() => JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').some((x) => x.nativeNo));
      check(ap.length === 0 && !nn, `${at}: a busy node is no refusal: still the app's, nothing marked (${JSON.stringify(ap)})`);
      await sleep(2200);
      const tries = (await calls(p, 'native_load')).filter((c) => /busy/.test(c[1].url)).length;
      const said = await p.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).join(' | '));
      check(tries === 2 && /can't play here: HTTP 503/.test(said), `${at}: it is tried once more, then said (${tries} loads; ${said})`);
      // Play pressed twice while a song opens: the page still hears the app, and pause stops it
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(60);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(30);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(1200);
      const n2 = await now(p);
      check(!n2.paused && n2.time > 0, `${at}: play pressed twice while it opens: the bar follows the app (${JSON.stringify(n2)})`);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(400);
      check((await p.evaluate(() => window.__app.transport)) === 'paused', `${at}: and pause reaches the app`);
      // Pause pressed while the song opens
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(80);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(900);
      const pauses = (await calls(p, 'native_pause')).length;
      check(pauses >= 1 && (await now(p)).paused, `${at}: a pause while it opens holds (${pauses} pauses)`);
      // Repeat one: the app goes on with the same song; the page doesn't open it again
      await p.evaluate(() => localStorage.setItem('pm.repeat', 'one'));
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1));
      await sleep(700);
      const loads0 = (await calls(p, 'native_load')).length;
      const ah = (await calls(p, 'native_load')).pop();
      check(ah && ah[1].ahead === fileUrl(1), `${at}: repeat one hands the same song ahead at once (${JSON.stringify(ah && ah[1].ahead)})`);
      await sleep(2200);
      check((await calls(p, 'native_load')).length === loads0 && !(await now(p)).paused, `${at}: and goes on with it without opening it again`);
      // A break at the very start: the song's one retry (a new load)
      await p.evaluate(() => localStorage.setItem('pm.repeat', 'off'));
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(700);
      let b0 = (await calls(p, 'native_load')).length;
      await p.evaluate(() => { Object.assign(window.__app, { transport: 'paused', broke: true, position_ms: 0, error: 'decode: the stream broke off' }); window.__send(); }); await sleep(2600);
      check((await calls(p, 'native_load')).length > b0, `${at}: a break at the very start is tried once more`);
      await p.evaluate(() => { Object.assign(window.__app, { broke: false, error: null }); });
      // The same song reopened, pause and play while it opens, and the open fails: said, and the
      // bar doesn't stay "playing"
      await p.evaluate(() => localStorage.setItem('pm.repeat', 'off'));
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(700);
      await p.evaluate(() => { window.__failLoad = true; });
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(60);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(120);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(900);
      const fl = await p.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).join(' | '));
      check(/the node did not answer/.test(fl) && (await now(p)).paused, `${at}: a reopen that fails after pause and play is said, not left "playing" (${fl}; ${JSON.stringify(await now(p))}; ${JSON.stringify((await calls(p)).slice(-6).map((c) => c[0]))}; ${JSON.stringify(r.state.errors)})`);
      await p.evaluate(() => { window.__failLoad = false; }); await sleep(2500);

      await p.evaluate(() => localStorage.setItem('pm.repeat', 'off'));
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(700);
      let before = (await calls(p, 'native_load')).length;
      await p.evaluate(() => { Object.assign(window.__app, { transport: 'paused', error: 'FiiO KA17 was unplugged: paused' }); window.__send(); }); await sleep(2200);
      const bar1 = await p.evaluate(() => document.querySelector('#np').innerText);
      const after = (await calls(p, 'native_load')).length;
      check(after === before && /unplugged/.test(bar1), `${at}: a DAC gone: paused with the reason, not reopened (${before} -> ${after} loads; ${JSON.stringify(bar1.slice(0, 120))})`);
      // A break mid-song: opened again where it broke (the page's mid-song retry)
      await p.evaluate((q) => window.pmPlayer.play(q), queue(1)); await sleep(700);
      await p.evaluate(() => { Object.assign(window.__app, { transport: 'playing', position_ms: 800 }); window.__send(); }); await sleep(150);
      // (the zone's stop reaches the page first, then the break: still the mid-song retry)
      await p.evaluate(() => { Object.assign(window.__app, { transport: 'stopped', position_ms: 0 }); window.__send(); }); await sleep(100);
      await p.evaluate(() => { Object.assign(window.__app, { transport: 'paused', broke: true, position_ms: 800, error: 'decode: the stream broke off' }); window.__send(); }); await sleep(2200);
      const again = (await calls(p, 'native_load')).pop();
      check(again && again[1].atMs >= 700, `${at}: a break mid-song is opened again where it broke (${JSON.stringify(again && again[1].atMs)}; ${JSON.stringify((await calls(p)).slice(-8).map((c) => c[0] + (c[1].atMs != null ? ':' + c[1].atMs : '')))})`);
      // Play after the node took the output, the node out of reach: said, and tried once more
      await p.evaluate(() => { Object.assign(window.__app, { transport: 'paused', taken: true, broke: false, error: "the node plays on this device's zone now" }); window.__send(); }); await sleep(300);
      before = (await calls(p, 'native_load')).length;
      await p.evaluate(() => { window.__failPlay = true; document.querySelector('#toggle').click(); }); await sleep(500);
      const t2 = await p.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).join(' | '));
      check(/did not start: HTTP 503/.test(t2) && (await now(p)).paused, `${at}: play failing after the node took it: said, not shown playing (${t2})`);
      await p.evaluate(() => { window.__failPlay = false; }); await sleep(2200);
      check((await calls(p, 'native_load')).length > before, `${at}: and tried once more`);
      // A mid-song break whose retry fails after pause and play: said too (not only browserPlay's)
      await p.evaluate(() => { window.__failLoad = false; });
      await p.evaluate((q) => window.pmPlayer.play(q), queue(2)); await sleep(100); await p.evaluate(() => document.querySelector('#next').click()); await sleep(800);
      await p.evaluate(() => { Object.assign(window.__app, { transport: 'playing', position_ms: 900 }); window.__send(); }); await sleep(150);
      await p.evaluate(() => { window.__failLoad = true; Object.assign(window.__app, { transport: 'paused', broke: true, position_ms: 900, error: 'decode: the stream broke off' }); window.__send(); }); await sleep(1700);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(120);
      await p.evaluate(() => document.querySelector('#toggle').click()); await sleep(1500);
      const fl2 = await p.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).join(' | '));
      check(/the node did not answer/.test(fl2) && (await now(p)).paused, `${at}: a failed mid-song retry after pause and play is said, not left "playing" (${fl2})`);
      await p.evaluate(() => { window.__failLoad = false; Object.assign(window.__app, { broke: false, error: null }); }); await sleep(2500);
      check(!r.state.errors.length, `${at}: review checks: no page errors (${r.state.errors.join(' | ')})`);
      await r.context.close();
      // A restored song opens where it was
      const r2 = await fx.create(browser, w, { tracks });
      await r2.context.addInitScript(FAKE_APP, { nativePlayer: true, len: 60000 });
      await r2.context.addInitScript((u) => { localStorage.setItem('pm.browserQueue', JSON.stringify([{ id: 1, uri: 'track:1', track_id: 1, title: 'Song 1' }])); localStorage.setItem('pm.browserAt', JSON.stringify({ index: 0, pos: 42 })); }, null);
      await fx.go(r2.page, '/'); await r2.page.waitForFunction(() => !!window.pmPlayer); await sleep(700);
      await r2.page.evaluate(() => document.querySelector('#toggle').click()); await sleep(600);
      const l = (await calls(r2.page, 'native_load')).pop();
      check(l && l[1].atMs === 42000, `${at}: a restored song opens where it was (${JSON.stringify(l && l[1].atMs)})`);
      await r2.context.close();
    }

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
