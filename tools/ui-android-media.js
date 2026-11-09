// The Android app's one volume and media controls, on the page's side (the app's side is
// crates/pm-desktop/src/media.rs and tools/release/android/MediaControl.kt). Real candidate assets on
// the fixture origin (tools/ui-fixture.js), the page inside the Android app: window.pmDesktop with
// platform android, and window.OdeionAndroid as the app's init script defines it (here it records
// the calls instead of posting them to the app):
//   * the page says which zone it controls (`OdeionAndroid.controlling`), at start and when the
//     zone changes, once per change;
//   * the page's own player ("This phone") has no gain of its own: its slider is a percent that
//     reads `window.pmDesktop.mediaVolume` (the script the app runs on `pmdesktop:mediavolume`),
//     `audio.volume` stays 1 (a gain set before is undone), and moving the slider calls
//     `OdeionAndroid.setMediaVolume(percent)` without touching `audio.volume`;
//   * a network zone's slider stays the zone's own (POST /zones/{id}/volume), untouched by it;
//   * a USB DAC on the phone's output: "this phone" plays through the phone's own zone, once;
//   * the web UI (no app): the page's player keeps its own dB slider (no regression).
// At 1280 px. Screenshots go to the shots folder.
//   node tools/ui-android-media.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'android-media-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const OUT = 'net:192.168.1.20:7790/cpal:Speaker or headphones';
const base = { transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, now_playing: null };
const phoneZone = { ...base, id: 'pixel-8-net', name: 'Pixel 8', output: OUT, volume: { percent: 20, muted: false } };
const linn = { ...base, id: 'linn-1', name: 'Living room', output: 'openhome:linn', volume: { percent: 12, muted: false } };
const caps = { pcm_rates: [44100, 48000], max_pcm_bits: 24, pcm_bits: [16, 24], max_channels: 2, dsd: null, hw_volume: { kind: 'percent' }, standby: false, source_switch: false, gapless: true, groupable: false, fixed_latency_ms: null, quality_ceiling: { lossless_up_to: { rate_hz: 48000, bits: 24 } } };
const outputs = (device) => [{ id: OUT, name: 'Pixel 8 · Speaker or headphones', protocol: 'net', caps, ...(device ? { device } : {}) }, { id: 'openhome:linn', name: 'Linn', protocol: 'openhome', caps }];
const KA17 = { name: 'FiiO KA17', kind: 'usb_dac', summary: 'FiiO KA17 · PCM up to 768 kHz / 32 bit', rates: [44100, 48000, 96000], bits: [16, 24, 32], max_rate: 768000, max_bits: 32, channels: 2, hardware_volume: true, mode: 'exclusive', modes: ['shared', 'bit_perfect', 'exclusive'], confirmed: false, usb: { vendor_id: 10610, product_id: 147 } };
// The scripts the app runs (crates/pm-desktop/src/media.rs `media_volume_script`, player.rs `output_script`)
const mediaVolume = (p) => `(function(p){if(!window.pmDesktop)return;window.pmDesktop.mediaVolume=p;window.dispatchEvent(new Event('pmdesktop:mediavolume'));})(${p});`;
const appOutput = (ids) => `(function(o){if(!window.pmDesktop)return;window.pmDesktop.outputs=o;window.pmDesktop.output=o.length===1?o[0]:null;window.dispatchEvent(new Event('pmdesktop:output'));})(${JSON.stringify(ids)});`;

async function open(browser, inApp) {
  const run = await fx.create(browser, 1280, { zone: 'browser', fixtures: { '/zones': { body: [phoneZone, linn] }, '/outputs': { body: outputs(null) } } });
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [phoneZone, linn] })); });
  await run.context.route('**/v1/zones/*/volume', (route) => { run.state.hits.push({ v: new URL(route.request().url()).pathname, m: 'POST', body: route.request().postData() }); return route.fulfill({ status: 204, body: '' }); });
  await run.context.addInitScript((app) => {
    if (!app) return;
    window.pmDesktop = { version: '0.3.0', platform: 'android' };
    window.__calls = [];
    window.OdeionAndroid = Object.freeze({ controlling: (z) => window.__calls.push(['controlling', String(z)]), setMediaVolume: (p) => window.__calls.push(['volume', Number(p)]) });
  }, inApp);
  return run;
}
const calls = (page) => page.evaluate(() => window.__calls.slice());
const slider = (page) => page.evaluate(() => { const s = document.querySelector('#vol'); return s ? { min: +s.min, max: +s.max, value: +s.value, text: s.getAttribute('aria-valuetext'), unknown: s.classList.contains('unknown') } : null; });
const gain = (page) => page.evaluate(() => document.querySelector('audio#browser').volume);
const setSlider = (page, v) => page.evaluate((v) => { const s = document.querySelector('#vol'); s.value = String(v); s.dispatchEvent(new Event('input', { bubbles: true })); s.dispatchEvent(new Event('change', { bubbles: true })); }, v);
const pick = async (page, id) => { await page.click('#zonebtn'); await sleep(300); await page.click(`#zonepop [data-zone="${id}"]`); await sleep(400); };

(async () => {
  const browser = await chromium.launch();
  {
    const run = await open(browser, true);
    const { page, state } = run;
    await fx.go(page, '/'); await page.waitForSelector('#vol'); await sleep(600);
    let c = await calls(page);
    check(c.some(([k, z]) => k === 'controlling' && z === 'browser'), `the page told the app it controls This phone (${JSON.stringify(c)})`);
    const before = await slider(page);
    check(before && before.min === 0 && before.max === 100 && before.unknown, `before the app's reading: a percent slider, level unknown (${JSON.stringify(before)})`);
    // A gain set some other way is undone by the app's reading
    await page.evaluate(() => { document.querySelector('audio#browser').volume = 0.5; });
    await page.evaluate(mediaVolume(20)); await sleep(200);
    const s = await slider(page);
    check(s && s.value === 20 && s.text === '20 %' && !s.unknown, `the slider reads Android's media volume (${JSON.stringify(s)})`);
    check(await gain(page) === 1, `audio.volume stays 1: no gain of the app's own (${await gain(page)})`);
    await page.screenshot({ path: path.join(SHOTS, 'this-phone-media-volume.png') });
    // A key press on the phone: the app says so, the slider follows
    await page.evaluate(mediaVolume(24)); await sleep(200);
    check((await slider(page)).value === 24, `a volume key on the phone moves the slider (${(await slider(page)).value})`);
    // The slider sets Android's media volume through the app, never the page's gain
    await setSlider(page, 35); await sleep(200);
    c = await calls(page);
    check(c.some(([k, p]) => k === 'volume' && p === 35), `moving the slider calls OdeionAndroid.setMediaVolume(35) (${JSON.stringify(c)})`);
    check(await gain(page) === 1, `and leaves audio.volume at 1 (${await gain(page)})`);
    check(!state.hits.some((h) => /\/zones\/.*\/volume/.test(h.v || '')), 'and asks the node nothing');
    // Another zone: the app hears it once, and that zone's slider is the zone's own
    const n = (await calls(page)).filter(([k]) => k === 'controlling').length;
    await pick(page, 'linn-1');
    c = await calls(page);
    check(c.filter(([k]) => k === 'controlling').length === n + 1 && c.at(-1)[1] === 'linn-1', `choosing Living room tells the app once (${JSON.stringify(c.slice(-3))})`);
    const ls = await slider(page);
    check(ls && ls.value === 12, `Living room's slider is its own level (${JSON.stringify(ls)})`);
    await setSlider(page, 14); await sleep(300);
    check(state.hits.some((h) => h.v === '/v1/zones/linn-1/volume' && /"percent":14/.test(h.body || '')), 'and sets the zone itself (POST /zones/linn-1/volume)');
    check(!(await calls(page)).some(([k, p]) => k === 'volume' && p === 14), 'not Android\'s media volume');
    // Back to This phone, then a USB DAC on the phone's output: the phone's own zone plays
    await pick(page, 'browser');
    run.state.fixtures['/outputs'] = { body: outputs(KA17) };
    await page.evaluate(appOutput([OUT])); await sleep(700);
    const zid = await page.evaluate(() => localStorage.getItem('pm.zone'));
    check(zid === 'pixel-8-net', `with the KA17 on the phone, "this phone" plays through its own zone (${zid})`);
    const toast = await page.evaluate(() => [...document.querySelectorAll('.toast')].map((t) => t.textContent).join(' | '));
    check(/FiiO KA17/.test(toast), `and says so ("${toast}")`);
    c = await calls(page);
    check(c.at(-1)[0] === 'controlling' && c.at(-1)[1] === 'pixel-8-net', `the app hears the new zone (${JSON.stringify(c.at(-1))})`);
    // Once per DAC: back to This phone by hand stays there
    await pick(page, 'browser');
    await page.evaluate(appOutput([OUT])); await sleep(600);
    check(await page.evaluate(() => localStorage.getItem('pm.zone')) === 'browser', 'chosen again by hand, This phone stays (once per DAC)');
    await page.screenshot({ path: path.join(SHOTS, 'dac-phone-zone.png') });
    await fx.cleanCopy(page).then(() => check(true, 'no "undefined", no overflow'), (e) => check(false, e.message));
    check(state.errors.length === 0, `no page errors (${state.errors.join('; ')})`);
    await run.context.close();
  }
  {
    // The web UI: nothing changes for the browser's own player
    const run = await open(browser, false);
    const { page, state } = run;
    await fx.go(page, '/'); await page.waitForSelector('#vol'); await sleep(500);
    const s = await slider(page);
    check(s && s.min === -60 && s.max === 0, `the web UI's own player keeps its dB slider (${JSON.stringify(s)})`);
    check(state.errors.length === 0, `no page errors (${state.errors.join('; ')})`);
    await run.context.close();
  }
  await browser.close();
  if (failed) { console.log(`${failed} check(s) failed`); process.exit(1); }
  console.log('PASS');
})().catch((e) => { console.error(e); process.exit(1); });
