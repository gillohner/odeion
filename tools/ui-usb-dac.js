// A USB DAC on the phone (the Android app's player), as the node reports it on the phone's output
// (`device` in GET /v1/outputs). Real candidate assets on the fixture origin (tools/ui-fixture.js),
// the page inside the Android app (window.pmDesktop, platform android), the phone's zone playing a
// 96/24 FLAC that Android's mixer gets at 48 kHz:
//   * before the DAC: the picker and the sound pill name no DAC;
//   * the app says its player registered again (`pmdesktop:output`, the script the app runs, with
//     this app's output id): the page fetches the outputs again, the sound pill reads
//     "… → FiiO KA17 · shared" and the zone picker's line under the phone "FiiO KA17 · shared";
//   * Settings › This phone (under This device): the DAC's name, "Shared" with why, what it takes,
//     the Android-version note when there is one, and no control (nothing to choose yet);
//   * nothing anywhere says bit-perfect for it, and nothing reads "undefined";
//   * unplugged: the same event takes it all away again;
//   * the web UI (no app): the picker and pill still name the DAC (any controller may), but there is
//     no This phone tab (it belongs to the app).
// At 390 and 1280 px. Screenshots go to the shots folder.
//   node tools/ui-usb-dac.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'usb-dac-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const OUT = 'net:192.168.1.20:7790/cpal:Speaker or headphones';
const src = { kind: 'pcm', rate_hz: 96000, bits: 24, channels: 2 };
const out = { kind: 'pcm', rate_hz: 48000, bits: 24, channels: 2 };
const zone = { id: 'pixel-8-net', name: 'Pixel 8 · Speaker or headphones (Android audio, shared mode)', output: OUT, volume_db: -20, transport: 'playing', queue: [{ id: 1, uri: 'track:1', title: 'Journey Through Time' }], queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 40, muted: false },
  now_playing: { item: { id: 1, uri: 'track:1', title: 'Journey Through Time', artist: 'Sefa', album: 'Journey', duration_ms: 240000 }, index: 0, position_ms: 30000, duration_ms: 240000, source_format: src, output_format: out,
    signal_path: { stages: [{ name: 'decode', in_format: src, out_format: src, quality: 'bit_perfect', note: null }, { name: 'resample', in_format: src, out_format: out, quality: 'high', note: 'the output takes 44.1 and 48 kHz' }, { name: 'output', in_format: out, out_format: out, quality: 'lossless', note: null }] },
    format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } } };
// What the node answers with the KA17 in (crates/pm-out-local/src/usb_dac.rs, the KA17 report)
const KA17 = { name: 'FiiO KA17', kind: 'usb_dac', summary: 'FiiO KA17 · PCM up to 768 kHz / 32 bit', rates: [44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000, 705600, 768000], bits: [16, 24, 32], max_rate: 768000, max_bits: 32, channels: 2, hardware_volume: false, mode: 'shared', modes: ['shared'], confirmed: false, usb: { vendor_id: 10610, product_id: 147 } };
const caps = { pcm_rates: [44100, 48000], max_pcm_bits: 24, pcm_bits: [16, 24], max_channels: 2, dsd: null, hw_volume: null, standby: false, source_switch: false, gapless: true, groupable: false, fixed_latency_ms: null, quality_ceiling: { lossless_up_to: { rate_hz: 48000, bits: 24 } } };
const phoneOut = (device) => ({ id: OUT, name: 'Pixel 8 · Speaker or headphones (Android audio, shared mode)', protocol: 'net', caps, ...(device ? { device } : {}) });
// The script the app runs after registering its player (crates/pm-desktop/src/player.rs, output_script)
const appScript = (ids) => `(function(o){if(!window.pmDesktop)return;window.pmDesktop.outputs=o;window.pmDesktop.output=o.length===1?o[0]:null;window.dispatchEvent(new Event('pmdesktop:output'));})(${JSON.stringify(ids)});`;

async function open(browser, w, inApp) {
  const run = await fx.create(browser, w, { fixtures: { '/zones': { body: [zone] }, '/outputs': { body: [phoneOut(null)] } } });
  await run.page.setViewportSize({ width: w, height: 860 });
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  await run.context.addInitScript(([z, app]) => { localStorage.setItem('pm.zone', z); if (app) window.pmDesktop = { version: '0.2.2', platform: 'android' }; }, [zone.id, inApp]);
  return run;
}
const pill = (page) => page.evaluate(() => document.querySelector('#quality .qtext').textContent);
const pickerLine = async (page) => { await page.click('#zonebtn'); await sleep(300); const t = await page.evaluate((id) => { const b = document.querySelector(`#zonepop [data-zone="${id}"] .sub`); return b ? [...b.childNodes].filter((n) => !(n.classList && n.classList.contains('sr-only'))).map((n) => n.textContent).join('') : null; }, zone.id); return t; };
const closePicker = (page) => page.keyboard.press('Escape');
const plug = async (run, device, ids = [OUT]) => { run.state.fixtures['/outputs'] = { body: [phoneOut(device)] }; await run.page.evaluate(appScript(ids)); await sleep(400); };

(async () => {
  const browser = await chromium.launch();
  for (const w of [390, 1280]) {
    const at = `${w}px`;
    const run = await open(browser, w, true);
    const { page, state } = run;
    await fx.go(page, '/'); await page.waitForFunction(() => { const q = document.querySelector('#quality .qtext'); return q && q.textContent !== '—'; }); await sleep(500);
    check(!/FiiO/.test(await pill(page)), `${at}: before the DAC the sound pill names none ("${await pill(page)}")`);
    const before = await pickerLine(page); await closePicker(page);
    check(before !== null && !/FiiO/.test(before), `${at}: and the picker line under the phone neither ("${before}")`);

    await plug(run, KA17);
    const p = await pill(page);
    check(/^FLAC .* → FiiO KA17 · shared$/.test(p), `${at}: plugged in, the pill reads "… → FiiO KA17 · shared" ("${p}")`);
    check(state.hits.filter((h) => h.v === '/outputs' && h.m === 'GET').length >= 1, `${at}: the page fetched the outputs again on the app's event`);
    const line = await pickerLine(page);
    check(line === 'FiiO KA17 · shared', `${at}: the picker's line under the phone ("${line}")`);
    await page.screenshot({ path: path.join(SHOTS, `picker-${w}.png`) });
    await closePicker(page);
    const id = await page.evaluate(() => window.pmDesktop.output);
    check(id === OUT, `${at}: the page knows this app's output (${id})`);

    // Settings › This phone
    // The app's script comes after the page drew Settings (on_page_load can be late): it draws again
    await fx.go(page, '/settings/dac'); await page.waitForSelector('[data-stab="appearance"]', { state: 'attached' }); await sleep(300);
    check(await page.evaluate(() => !document.querySelector('[data-stab="dac"]')), `${at}: before the app says which output is its own, no This phone`);
    await page.evaluate(appScript([OUT])); await page.waitForSelector('#set-dac'); await sleep(300);
    const tab = await page.evaluate(() => { const t = document.querySelector('[data-stab="dac"]'); const p = document.querySelector('#set-dac'); return { tab: t && t.textContent.trim(), shown: !!p && !p.hidden, text: p ? p.innerText : '', controls: p ? p.querySelectorAll('input,select,textarea,button:not([data-tabs-back])').length : -1, group: t ? (t.closest('[aria-labelledby]') || t).getAttribute('aria-labelledby') : null }; });
    check(/This phone/.test(tab.tab || '') && tab.shown, `${at}: Settings has This phone, shown at /settings/dac ("${tab.tab}")`);
    check(/FiiO KA17/i.test(tab.text) && /Shared\. Android plays it through its own mixer/.test(tab.text), `${at}: it names the DAC and says it plays shared, and why`);
    check(/PCM up to 768 kHz · 16, 24, 32 bit · 2 channels/.test(tab.text), `${at}: what it takes ("${(tab.text.match(/PCM[^\n]*/) || [''])[0]}")`);
    check(tab.controls === 0, `${at}: and offers nothing to choose yet (${tab.controls} controls)`);
    const grp = await page.evaluate(() => { const t = document.querySelector('[data-stab="dac"]'); let e = t; while (e && e.previousElementSibling) { e = e.previousElementSibling; if (e.classList.contains('set-group-h')) return e.textContent; } const g = t && t.closest('[role="group"]'); return g ? g.getAttribute('aria-label') || g.textContent : ''; });
    check(/This device/.test(grp || ''), `${at}: under This device ("${(grp || '').slice(0, 40)}")`);
    await page.screenshot({ path: path.join(SHOTS, `settings-${w}.png`), fullPage: true });
    check(!/bit.?perfect/i.test(tab.text) && !/bit.?perfect/i.test(p) && !/bit.?perfect/i.test(line), `${at}: nothing about the DAC says bit-perfect`);
    try { await fx.cleanCopy(page); check(true, `${at}: no "undefined", no overflow`); } catch (e) { check(false, `${at}: ${e.message}`); }

    // An Android 13 phone: the note says why not bit-perfect
    await plug(run, { ...KA17, note: 'Bit-perfect playback needs Android 14 or later; this phone has Android 13.' });
    await fx.go(page, '/settings/dac'); await page.evaluate(appScript([OUT])); await page.waitForSelector('#set-dac'); await sleep(300);
    const note = await page.evaluate(() => { const n = document.querySelector('#set-dac .notice'); return n ? n.textContent : null; });
    check(note === 'Bit-perfect playback needs Android 14 or later; this phone has Android 13.', `${at}: the note is shown ("${note}")`);

    // Unplugged
    await fx.go(page, '/'); await page.evaluate(appScript([OUT])); await page.waitForFunction(() => { const q = document.querySelector('#quality .qtext'); return q && q.textContent !== '—'; }); await sleep(300);
    await plug(run, null);
    check(!/FiiO/.test(await pill(page)), `${at}: unplugged, the pill names no DAC ("${await pill(page)}")`);
    const gone = await pickerLine(page); await closePicker(page);
    check(!/FiiO/.test(gone || ''), `${at}: nor the picker ("${gone}")`);
    await fx.go(page, '/settings'); await page.evaluate(appScript([OUT])); await sleep(800);
    check(await page.evaluate(() => !!document.querySelector('[data-stab="appearance"]') && !document.querySelector('[data-stab="dac"]')), `${at}: and Settings has no This phone`);
    check(state.errors.length === 0, `${at}: no page errors ${JSON.stringify(state.errors)}`);
    await run.context.close();
  }
  // The web UI: any controller sees the device on the phone's zone; the phone's settings are the app's
  {
    const run = await open(browser, 1280, false);
    run.state.fixtures['/outputs'] = { body: [phoneOut(KA17)] };
    const { page } = run;
    await fx.go(page, '/'); await page.waitForFunction(() => { const q = document.querySelector('#quality .qtext'); return q && q.textContent !== '—'; }); await sleep(500);
    const line = await pickerLine(page); await closePicker(page);
    check(line === 'FiiO KA17 · shared', `web UI: the picker names the DAC on the phone's zone ("${line}")`);
    check(/→ FiiO KA17 · shared$/.test(await pill(page)), `web UI: the pill too ("${await pill(page)}")`);
    await fx.go(page, '/settings'); await sleep(800);
    check(await page.evaluate(() => !!document.querySelector('[data-stab="appearance"]') && !document.querySelector('[data-stab="dac"]')), 'web UI: no This phone tab');
    check(run.state.errors.length === 0, `web UI: no page errors ${JSON.stringify(run.state.errors)}`);
    await run.context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAILED` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
