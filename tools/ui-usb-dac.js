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
//     no This phone tab (it belongs to the app);
//   * every controller follows the node: `outputs_changed` on the node's event stream (no app
//     event) updates the pill (without the picker having fetched first), the picker, the open Sound
//     panel and Settings › This phone, plugged and unplugged;
//   * "bit-perfect" only for mode bit_perfect with confirmed true: unconfirmed (with a neutral
//     note in Settings) or unknown modes read as shared;
//   * answers out of order: an older GET /v1/outputs answer never restores an unplugged DAC, and
//     Settings, drawn after slow requests, never writes back the outputs it fetched before them;
//   * a change missed while disconnected: the snapshot on reconnecting fetches the outputs again;
//   * DSD: native and DoP by name, anything else just "DSD";
//   * step 2: "Bit-perfect · FiiO KA17 · 96 kHz / 24 bit" only when the device confirmed bit-perfect
//     for the format playing now and the node's path changes nothing; a confirmation for another
//     format, or a path with a conversion or DSP, reads shared (with why); a zone paused because
//     the DAC was unplugged says so in the bar.
//   * one behaviour, nothing to choose: Settings › This phone is read-only (the path and why, who
//     sets the volume: "Volume: the DAC’s own" or "Volume: set it on the DAC", nothing while that
//     is unknown), with one "Allow Exclusive USB" button while Android's permission is missing
//     (a PUT /v1/outputs/{id}/device of mode exclusive); no Preferred mode selector, no DAC-volume
//     switch, no full-level warning or question anywhere, also for what an earlier app reports.
//   * Now playing › Details has an Output line, the Sound panel's: shared with the phone's reason,
//     and bit-perfect with the rate and bits when confirmed.
// At 390 and 1280 px. Screenshots go to the shots folder.
//   node tools/ui-usb-dac.js [repo] [shots]     (ONLY=web: the web UI part alone)
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'usb-dac-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// Close Now playing whole: on a phone, Escape first closes its open sheet (More, Up next), then the page
const closeNp = async (page) => { for (let i = 0; i < 4 && await page.$('#nowplaying:not([hidden])'); i++) { await page.keyboard.press('Escape'); await sleep(300); } };

const OUT = 'net:192.168.1.20:7790/cpal:Speaker or headphones';
const src = { kind: 'pcm', rate_hz: 96000, bits: 24, channels: 2 };
const out = { kind: 'pcm', rate_hz: 48000, bits: 24, channels: 2 };
const zone = { id: 'pixel-8-net', name: 'Pixel 8 · Speaker or headphones (Android audio, shared mode)', output: OUT, volume_db: -20, transport: 'playing', queue: [{ id: 1, uri: 'track:1', title: 'Journey Through Time' }], queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 40, muted: false },
  now_playing: { item: { id: 1, uri: 'track:1', title: 'Journey Through Time', artist: 'Sefa', album: 'Journey', duration_ms: 240000 }, index: 0, position_ms: 30000, duration_ms: 240000, source_format: src, output_format: out,
    signal_path: { stages: [{ name: 'decode', in_format: src, out_format: src, quality: 'bit_perfect', note: null }, { name: 'resample', in_format: src, out_format: out, quality: 'high', note: 'the output takes 44.1 and 48 kHz' }, { name: 'output', in_format: out, out_format: out, quality: 'lossless', note: null }] },
    format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } } };
// The same zone playing 96/24 bit-perfect all the way (the phone's output takes the DAC's rates)
const bpZone = { ...zone, now_playing: { ...zone.now_playing, output_format: src, signal_path: { stages: [{ name: 'decode', in_format: src, out_format: src, quality: 'bit_perfect', note: null }, { name: 'output', in_format: src, out_format: src, quality: 'bit_perfect', note: null }] } } };
// ... with a software volume stage (the node changes the samples)
const dspZone = { ...bpZone, now_playing: { ...bpZone.now_playing, signal_path: { stages: [...bpZone.now_playing.signal_path.stages, { name: 'volume', in_format: src, out_format: src, quality: 'lossless', note: 'software volume -6.0 dB in 64-bit float' }] } } };
const setZone = async (run, z) => { run.nodeEvent({ type: 'zone', state: z }); await sleep(400); };
// What the node answers with the KA17 in (crates/pm-out-local/src/usb_dac.rs, the KA17 report)
const KA17 = { name: 'FiiO KA17', kind: 'usb_dac', summary: 'FiiO KA17 · PCM up to 768 kHz / 32 bit', rates: [44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000, 705600, 768000], bits: [16, 24, 32], max_rate: 768000, max_bits: 32, channels: 2, hardware_volume: false, mode: 'shared', modes: ['shared'], confirmed: false, usb: { vendor_id: 10610, product_id: 147 } };
const caps = { pcm_rates: [44100, 48000], max_pcm_bits: 24, pcm_bits: [16, 24], max_channels: 2, dsd: null, hw_volume: null, standby: false, source_switch: false, gapless: true, groupable: false, fixed_latency_ms: null, quality_ceiling: { lossless_up_to: { rate_hz: 48000, bits: 24 } } };
const phoneOut = (device) => ({ id: OUT, name: 'Pixel 8 · Speaker or headphones (Android audio, shared mode)', protocol: 'net', caps, ...(device ? { device } : {}) });
// The script the app runs after registering its player (crates/pm-desktop/src/player.rs, output_script)
const appScript = (ids) => `(function(o){if(!window.pmDesktop)return;window.pmDesktop.outputs=o;window.pmDesktop.output=o.length===1?o[0]:null;window.dispatchEvent(new Event('pmdesktop:output'));})(${JSON.stringify(ids)});`;

async function open(browser, w, inApp) {
  const run = await fx.create(browser, w, { fixtures: { '/zones': { body: [zone] }, '/outputs': { body: [phoneOut(null)] } } });
  await run.page.setViewportSize({ width: w, height: 860 });
  // The node's event stream: kept, so a test can send what the node sends (numbered per socket)
  run.sockets = [];
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { const sock = { ws, seq: 0 }; run.sockets.push(sock); ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  // GET /v1/outputs answers what the fixture held when asked, after a delay the test may queue
  run.outQueue = [];
  await run.context.route('**/v1/outputs', async (route) => {
    if (route.request().method() !== 'GET') return route.fallback();
    const body = JSON.stringify(run.state.fixtures['/outputs'].body); run.state.hits.push({ v: '/outputs', m: 'GET', at: Date.now() });
    const d = run.outQueue.shift() || 0; if (d) await sleep(d);
    return route.fulfill({ status: 200, contentType: 'application/json', body });
  });
  // PUT /v1/outputs/{id}/device: the node answers the output, as listed
  await run.context.route('**/v1/outputs/*/device', async (route) => {
    const req = route.request(); const u = new URL(req.url());
    run.state.hits.push({ v: u.pathname.slice(3), m: req.method(), body: req.postData(), at: Date.now() });
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(run.state.fixtures['/outputs'].body[0]) });
  });
  // Settings' other requests, slowed down on demand
  run.slowSettings = 0;
  await run.context.route('**/v1/node/settings', async (route) => { if (run.slowSettings) await sleep(run.slowSettings); return route.fallback(); });
  run.nodeEvent = (ev) => { for (const s of run.sockets) { try { s.seq += 1; s.ws.send(JSON.stringify({ ...ev, seq: s.seq })); } catch { /* closed */ } } };
  await run.context.addInitScript(([z, app]) => { localStorage.setItem('pm.zone', z); if (app) window.pmDesktop = { version: '0.2.2', platform: 'android' }; }, [zone.id, inApp]);
  return run;
}
const pill = (page) => page.evaluate(() => document.querySelector('#quality .qtext').textContent);
const pickerLine = async (page) => { await page.$eval('#zonebtn', (b) => b.click()); await sleep(300); const t = await page.evaluate((id) => { const b = document.querySelector(`#zonepop [data-zone="${id}"] .sub`); return b ? [...b.childNodes].filter((n) => !(n.classList && n.classList.contains('sr-only'))).map((n) => n.textContent).join('') : null; }, zone.id); return t; };
const closePicker = (page) => page.keyboard.press('Escape');
const fetches = (st) => st.hits.filter((h) => h.v === '/outputs' && h.m === 'GET').length;
// The app's own event (this phone), or the node's (every controller)
const plug = async (run, device, ids = [OUT]) => { run.state.fixtures['/outputs'] = { body: [phoneOut(device)] }; await run.page.evaluate(appScript(ids)); await sleep(400); };
const nodePlug = async (run, device) => { run.state.fixtures['/outputs'] = { body: [phoneOut(device)] }; run.nodeEvent({ type: 'outputs_changed', output: OUT }); await sleep(500); };
const panelLine = (page) => page.evaluate(() => { const p = document.querySelector('#qpanel'); const s = p && !p.hidden && p.querySelector('.qp-zone .sub'); return s ? s.textContent : null; });
const dacTab = (page) => page.evaluate(() => { const p = document.querySelector('#set-dac'); return p ? p.innerText : null; });

(async () => {
  const browser = await chromium.launch();
  // ONLY=web runs the web UI part alone
  for (const w of process.env.ONLY === 'web' ? [] : [390, 1280]) {
    const at = `${w}px`;
    const run = await open(browser, w, true);
    const { page, state } = run;
    await fx.go(page, '/'); await page.waitForFunction(() => { const q = document.querySelector('#quality .qtext'); return q && q.textContent !== '—'; }); await sleep(500);
    check(!/FiiO/.test(await pill(page)), `${at}: before the DAC the sound pill names none ("${await pill(page)}")`);
    const before = await pickerLine(page); await closePicker(page);
    check(before !== null && !/FiiO/.test(before), `${at}: and the picker line under the phone neither ("${before}")`);

    const n0 = fetches(state);
    await plug(run, KA17);
    const p = await pill(page);
    check(/^FLAC .* → FiiO KA17 · shared$/.test(p), `${at}: plugged in, the pill reads "… → FiiO KA17 · shared" ("${p}")`);
    check(fetches(state) > n0, `${at}: the page fetched the outputs again on the app's event (${n0} → ${fetches(state)})`);
    const line = await pickerLine(page);
    check(line === 'FiiO KA17 · shared', `${at}: the picker's line under the phone ("${line}")`);
    await page.screenshot({ path: path.join(SHOTS, `picker-${w}.png`) });
    await closePicker(page);
    const id = await page.evaluate(() => window.pmDesktop.output);
    check(id === OUT, `${at}: the page knows this app's output (${id})`);

    // The open Sound panel follows the node: unplugged and plugged while it is open
    await page.$eval('#quality', (q) => q.click()); await page.waitForSelector('#qpanel:not([hidden])'); await sleep(300);
    const pl = await panelLine(page);
    check(pl === 'Shared · FiiO KA17', `${at}: the open Sound panel's Playing on line ("${pl}")`);
    await nodePlug(run, null);
    const pl2 = await panelLine(page);
    check(pl2 !== null && !/FiiO/.test(pl2), `${at}: unplugged (the node's event), the open panel names no DAC ("${pl2}")`);
    await nodePlug(run, KA17);
    check(await panelLine(page) === 'Shared · FiiO KA17', `${at}: plugged again, it does ("${await panelLine(page)}")`);
    // Bit-perfect confirmed for 96/24, and 96/24 plays bit-perfect all the way: the full line
    const BP = { ...KA17, mode: 'bit_perfect', modes: ['shared', 'bit_perfect'], confirmed: true, format: src, settings: { mode: 'auto', dac_volume: true } };
    await setZone(run, bpZone); await nodePlug(run, BP);
    check(await panelLine(page) === 'Bit-perfect · FiiO KA17 · 96 kHz / 24 bit', `${at}: confirmed for what plays: "${await panelLine(page)}"`);
    await page.screenshot({ path: path.join(SHOTS, `panel-bit-perfect-${w}.png`) });
    // Confirmed for another format (the confirmation of this open not in yet): not claimed
    await nodePlug(run, { ...BP, format: { ...src, rate_hz: 44100 } });
    check(await panelLine(page) === 'Shared · FiiO KA17', `${at}: confirmed for another format: shared ("${await panelLine(page)}")`);
    // The node turns it down itself: not bit-perfect, and why
    await setZone(run, dspZone); await nodePlug(run, BP);
    check(await panelLine(page) === 'Shared · FiiO KA17 · the node changes the samples (volume, DSP or a conversion)', `${at}: with the node's volume: "${await panelLine(page)}"`);
    // Shared with Android's reason
    await setZone(run, bpZone); await nodePlug(run, { ...BP, mode: 'shared', confirmed: false, note: 'Android refused bit-perfect for this format' });
    check(await panelLine(page) === 'Shared · FiiO KA17 · Android refused bit-perfect for this format', `${at}: refused: "${await panelLine(page)}"`);
    // ... and nothing else claims bit-perfect: the whole pill, its label and the panel's headline
    {
      const all = await page.evaluate(() => ({ pill: document.querySelector('#quality .qtext').textContent, aria: document.querySelector('#quality').getAttribute('aria-label'), cls: document.querySelector('#quality').className, head: (document.querySelector('#qpanel .path-head') || {}).textContent || '', rows: [...document.querySelectorAll('#qpanel .path-row .pn')].map((e) => e.textContent) }));
      check(!/bit.?perfect/i.test(all.pill + all.aria + all.head) && !/q-bit_perfect/.test(all.cls), `${at}: refused: no bit-perfect claim anywhere (pill "${all.pill}", headline "${all.head.trim()}", class "${all.cls}")`);
      check(all.rows.includes('Android mixer'), `${at}: refused: the path shows Android's mixer (${all.rows.join(', ')})`);
    }
    // Confirmed for what plays: the pill and the panel say bit-perfect all the way
    await nodePlug(run, BP);
    {
      const all = await page.evaluate(() => ({ pill: document.querySelector('#quality .qtext').textContent, head: (document.querySelector('#qpanel .path-head') || {}).textContent || '' }));
      check(all.pill === 'FLAC 24/96 → 24/96 · Bit-perfect → FiiO KA17 · bit-perfect' && /^Bit-perfect/.test(all.head.trim()), `${at}: confirmed: pill "${all.pill}", headline "${all.head.trim()}"`);
    }
    // Bit-perfect at any media volume, the DAC's own volume in the path: the Sound panel keeps its
    // line and adds no warning, also for what an earlier app reports (its agreement to full level)
    await nodePlug(run, { ...BP, own_volume: true, media_volume: 40, settings: { mode: 'bit_perfect', dac_volume: true, full_level_ok: true } });
    {
      const zl = await page.evaluate(() => { const z = document.querySelector('#qpanel .qp-zone'); return z ? { warn: (z.querySelector('.warn-text') || {}).textContent || '', text: z.textContent } : null; });
      check(zl && zl.warn === '' && !/full level|does not apply/.test(zl.text), `${at}: the Sound panel has no full-level warning (${JSON.stringify(zl)})`);
      check(await panelLine(page) === 'Bit-perfect · FiiO KA17 · 96 kHz / 24 bit', `${at}: and its line stays ("${await panelLine(page)}")`);
    }
    // The node no longer knows the DAC (unplugged), the node's path unchanged: no bit-perfect claim
    await nodePlug(run, null);
    {
      const all = await page.evaluate(() => ({ pill: document.querySelector('#quality .qtext').textContent, cls: document.querySelector('#quality').className, head: (document.querySelector('#qpanel .path-head') || {}).textContent || '', rows: [...document.querySelectorAll('#qpanel .path-row .pn')].map((e) => e.textContent) }));
      check(!/bit.?perfect/i.test(all.pill + all.head) && !/q-bit_perfect/.test(all.cls) && all.rows.includes('system mixer'), `${at}: no DAC known: the system's mixer, never bit-perfect (pill "${all.pill}", headline "${all.head.trim()}", rows ${all.rows.join(', ')})`);
    }
    await setZone(run, zone); await nodePlug(run, KA17);
    await page.keyboard.press('Escape'); await sleep(200);

    // Now playing › Details: the Output line, with Android's reason when it is shared
    {
      const VOL = 'Android 13: bit-perfect needs Android 14. Exclusive USB needs Android\'s permission for the DAC: tap Allow Exclusive USB in Settings > This phone';
      const detailsOutput = async () => {
        await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden])', { timeout: 5000 }); await sleep(400);
        // the Details tab on wide screens, or More on a phone (where Details lives in its sheet)
        if (await page.evaluate(() => { const b = !document.querySelector('#nowplaying .np-output') && document.querySelector('#nowplaying .np-tabs [data-npsheet="details"], #nowplaying .np-dock [data-npsheet="more"]'); if (b) b.click(); return !!b; })) await sleep(400);
        const t = await page.evaluate(() => { const d = document.querySelector('#nowplaying .np-output'); return d ? d.textContent : null; });
        return t;
      };
      await setZone(run, bpZone); await nodePlug(run, { ...BP, mode: 'shared', confirmed: false, note: VOL });
      const o1 = await detailsOutput();
      check(o1 === `Shared · FiiO KA17 · ${VOL}`, `${at}: Details' Output line, shared with the phone's reason: "${o1}"`);
      await page.screenshot({ path: path.join(SHOTS, `details-output-shared-${w}.png`) });
      await closeNp(page);
      await nodePlug(run, BP);
      const o2 = await detailsOutput();
      check(o2 === 'Bit-perfect · FiiO KA17 · 96 kHz / 24 bit', `${at}: Details' Output line, confirmed: "${o2}"`);
      // Keep Details open: a changed route must not leave an old bit-perfect claim on screen.
      await nodePlug(run, { ...BP, mode: 'shared', confirmed: false, note: VOL });
      await page.waitForFunction((reason) => document.querySelector('#nowplaying .np-output')?.textContent === `Shared · FiiO KA17 · ${reason}`, VOL);
      check(true, `${at}: open Details follows the same song from bit-perfect to shared`);
      await nodePlug(run, BP);
      await page.waitForFunction(() => document.querySelector('#nowplaying .np-output')?.textContent === 'Bit-perfect · FiiO KA17 · 96 kHz / 24 bit');
      await nodePlug(run, null);
      await page.waitForFunction(() => !document.querySelector('#nowplaying .np-output'));
      check(true, `${at}: open Details drops the DAC claim when the output disappears`);
      await closeNp(page);
      await setZone(run, zone); await nodePlug(run, KA17);
    }

    // Bit-perfect only when confirmed: unconfirmed and unknown modes read as shared
    for (const [dev, word, what] of [[{ ...KA17, mode: 'bit_perfect', modes: ['shared', 'bit_perfect'], confirmed: false }, 'shared', 'bit_perfect, not confirmed'], [{ ...KA17, mode: 'unknown', confirmed: true }, 'shared', 'an unknown mode, "confirmed"'], [{ ...KA17, mode: 'bit_perfect', modes: ['shared', 'bit_perfect'], confirmed: true }, 'shared', 'bit_perfect, confirmed for no format the path plays as is'], [{ ...KA17, mode: 'bit_perfect', modes: ['shared', 'bit_perfect'], confirmed: true, format: src }, 'bit-perfect', 'bit_perfect, confirmed for 96/24 playing bit-perfect']]) {
      if (word === 'bit-perfect') await setZone(run, bpZone);
      await plug(run, dev);
      const pp = await pill(page); const pk = await pickerLine(page); await closePicker(page);
      check(pp.endsWith(`→ FiiO KA17 · ${word}`) && pk === `FiiO KA17 · ${word}`, `${at}: ${what} → "${word}" (pill "${pp}", picker "${pk}")`);
      if (word === 'shared') check(!/bit.?perfect/i.test(pp + pk), `${at}: ${what}: the pill and picker never say bit-perfect`);
    }
    await setZone(run, zone); await nodePlug(run, KA17);

    // Settings › This phone
    // The app's script comes after the page drew Settings (on_page_load can be late): it draws again
    await fx.go(page, '/settings/dac'); await page.waitForSelector('[data-stab="appearance"]', { state: 'attached' }); await sleep(300);
    check(await page.evaluate(() => !document.querySelector('[data-stab="dac"]')), `${at}: before the app says which output is its own, no This phone`);
    await page.evaluate(appScript([OUT])); await page.waitForSelector('#set-dac'); await sleep(300);
    const tab = await page.evaluate(() => { const t = document.querySelector('[data-stab="dac"]'); const p = document.querySelector('#set-dac'); return { tab: t && t.textContent.trim(), shown: !!p && !p.hidden, text: p ? p.innerText : '', controls: p ? p.querySelectorAll('input,select,textarea,button:not([data-tabs-back])').length : -1, group: t ? (t.closest('[aria-labelledby]') || t).getAttribute('aria-labelledby') : null }; });
    check(/This phone/.test(tab.tab || '') && tab.shown, `${at}: Settings has This phone, shown at /settings/dac ("${tab.tab}")`);
    check(/FiiO KA17/i.test(tab.text) && /Shared · FiiO KA17\. Android plays it through its own mixer/.test(tab.text), `${at}: it names the DAC and says it plays shared, and why`);
    check(/PCM up to 768 kHz · 16, 24, 32 bit · 2 channels/.test(tab.text), `${at}: what it takes ("${(tab.text.match(/PCM[^\n]*/) || [''])[0]}")`);
    check(tab.controls === 0, `${at}: an app that takes no settings: nothing to choose (${tab.controls} controls)`);
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

    // Unconfirmed in Settings: shared, with a neutral note; confirmed: bit-perfect
    await nodePlug(run, { ...KA17, mode: 'bit_perfect', confirmed: false });
    await page.waitForFunction(() => /not confirmed/.test((document.querySelector('#set-dac') || {}).innerText || ''));
    const unc = await dacTab(page);
    check(/Playing through it\s+Shared · FiiO KA17/.test(unc) && /Android has not confirmed bit-perfect for this format\./.test(unc), `${at}: Settings, unconfirmed: shared and the note`);
    await setZone(run, bpZone);
    await nodePlug(run, { ...KA17, mode: 'bit_perfect', modes: ['shared', 'bit_perfect'], confirmed: true, format: src, settings: { mode: 'auto', dac_volume: true } });
    await page.waitForFunction(() => /Bit-perfect · /.test((document.querySelector('#set-dac') || {}).innerText || '')).catch(() => {});
    const con = await dacTab(page);
    check(/Playing through it\s+Bit-perfect · FiiO KA17 · 96 kHz \/ 24 bit\./.test(con) && !/not confirmed/.test(con), `${at}: Settings, confirmed for what plays: "Bit-perfect · FiiO KA17 · 96 kHz / 24 bit."`);
    // One behaviour, nothing to choose: no Preferred mode selector, no DAC-volume switch, no
    // question; who sets the volume, read-only
    let dialogs = 0; const onDialog = (d) => { dialogs++; d.dismiss(); };
    page.on('dialog', onDialog);
    const ctl = await page.evaluate(() => ({ mode: !!document.querySelector('#dvmode'), vol: !!document.querySelector('#dvvol'), selects: document.querySelectorAll('#set-dac select').length, allow: !!document.querySelector('#dvallow') }));
    check(!ctl.mode && !ctl.vol && ctl.selects === 0, `${at}: Settings has no Preferred mode and no DAC-volume switch (${JSON.stringify(ctl)})`);
    check(!ctl.allow, `${at}: no Allow button while Android's permission isn't known to be missing`);
    const help = await dacTab(page);
    check(!/Preferred mode|full level|Use the DAC’s volume/.test(help), `${at}: Settings says nothing of a mode to pick or of full level`);
    check(!/Volume: /.test(help), `${at}: whether it has a volume of its own unknown: nothing claimed`);
    // Its own volume (from its descriptors): said
    await nodePlug(run, { ...KA17, own_volume: true, usb_permission: true, mode: 'bit_perfect', modes: ['shared', 'bit_perfect', 'exclusive'], confirmed: true, format: src });
    await page.waitForFunction(() => /Volume: the DAC’s own/.test((document.querySelector('#set-dac') || {}).innerText || '')).catch(() => {});
    {
      const t = await dacTab(page);
      check(/Volume: the DAC’s own/.test(t) && /volume keys/.test(t), `${at}: "Volume: the DAC’s own" (${(t.match(/Volume:[^\n]*/) || [''])[0]})`);
      check(!(await page.evaluate(() => !!document.querySelector('#dvallow'))), `${at}: the permission given: no Allow button`);
    }
    // Android's bit-perfect path, whether the DAC has a volume of its own not known (no
    // descriptors): the note says what to do if its level doesn't follow; no volume words
    const FOLLOW = 'If the DAC\'s own level doesn\'t follow the volume, set it on the DAC.';
    await nodePlug(run, { ...KA17, usb_permission: false, mode: 'bit_perfect', modes: ['shared', 'bit_perfect', 'exclusive'], confirmed: true, format: src, note: FOLLOW });
    await page.waitForFunction((f) => ((document.querySelector('#set-dac') || {}).innerText || '').includes(f), FOLLOW).catch(() => {});
    {
      const t = await dacTab(page);
      check(t.includes(FOLLOW) && !/Volume: /.test(t), `${at}: Android bit-perfect, own volume unknown: "${FOLLOW}", no volume words`);
    }
    // Played shared (Android's mixer applies the volume): no words about the DAC's own volume
    await nodePlug(run, { ...KA17, own_volume: true, usb_permission: true, mode: 'shared', confirmed: false, format: src, note: 'Android refused bit-perfect for this format' });
    await page.waitForFunction(() => /Android refused/.test((document.querySelector('#set-dac') || {}).innerText || '')).catch(() => {});
    check(!/Volume: |stays bit-perfect|stays untouched/.test(await dacTab(page)), `${at}: shared: no bit-perfect volume words`);
    // What an earlier app reports (a mode chosen, the agreement to full level, its media volume):
    // still no selector, no warning
    await nodePlug(run, { ...KA17, mode: 'bit_perfect', modes: ['shared', 'bit_perfect'], confirmed: true, format: src, media_volume: 40, settings: { mode: 'bit_perfect', dac_volume: true, full_level_ok: true } });
    await sleep(200);
    {
      const t = await dacTab(page);
      check(!/full level|does not apply/.test(t) && !(await page.evaluate(() => !!document.querySelector('#dvmode'))), `${at}: an earlier app's agreement to full level: no warning, no selector`);
    }
    // Exclusive USB (the app's own driver), confirmed for what plays: said as such, with its format
    const EX = { ...KA17, hardware_volume: true, own_volume: true, usb_permission: true, mode: 'exclusive', modes: ['shared', 'exclusive'], confirmed: true, format: src };
    await setZone(run, bpZone); await nodePlug(run, EX);
    await page.waitForFunction(() => /Exclusive USB · /.test((document.querySelector('#set-dac') || {}).innerText || '')).catch(() => {});
    {
      const t = await dacTab(page);
      check(/Playing through it\s+Exclusive USB · FiiO KA17 · 96 kHz \/ 24 bit\./.test(t), `${at}: Exclusive USB confirmed: "Exclusive USB · FiiO KA17 · 96 kHz / 24 bit." (${(t.match(/Playing through it[^\n]*\n?[^\n]*/) || [''])[0]})`);
      check(/Volume: the DAC’s own/.test(t) && !/full level|does not apply/.test(t), `${at}: Exclusive USB: the DAC's own volume, no warning`);
      check(!(await page.evaluate(() => !!document.querySelector('#dvvol') || !!document.querySelector('#dvmode'))), `${at}: Exclusive USB: no switch, no selector`);
      const pp = await pill(page);
      check(pp.endsWith('→ FiiO KA17 · exclusive USB'), `${at}: the pill: "… → FiiO KA17 · exclusive USB" ("${pp}")`);
    }
    // Not confirmed (the clock not read back yet, or another rate): never bit-perfect anywhere
    await nodePlug(run, { ...EX, confirmed: false });
    await page.waitForFunction(() => /not confirmed/.test((document.querySelector('#set-dac') || {}).innerText || '')).catch(() => {});
    {
      const t = await dacTab(page); const pp = await pill(page);
      check(/Playing through it\s+Exclusive USB · FiiO KA17 · not confirmed\./.test(t) && /Exclusive USB mode is not confirmed for this format\./.test(t), `${at}: Exclusive USB unconfirmed: said so (${(t.match(/Playing through it[^\n]*\n?[^\n]*/) || [''])[0]})`);
      check(pp.endsWith('→ FiiO KA17 · shared') && !/bit.?perfect|exclusive/i.test(pp), `${at}: unconfirmed, the pill claims nothing ("${pp}")`);
    }
    // A DAC without a volume of its own: bit-perfect all the same; the volume is set on the DAC
    const NOVOL = 'This DAC has no volume of its own: set the volume on the DAC or amplifier.';
    await nodePlug(run, { ...KA17, own_volume: false, usb_permission: true, mode: 'bit_perfect', modes: ['shared', 'bit_perfect', 'exclusive'], confirmed: true, format: src, note: NOVOL });
    await page.waitForFunction(() => /Volume: set it on the DAC/.test((document.querySelector('#set-dac') || {}).innerText || '')).catch(() => {});
    {
      const t = await dacTab(page);
      check(/Playing through it\s+Bit-perfect · FiiO KA17 · 96 kHz \/ 24 bit\./.test(t), `${at}: no volume of its own: still bit-perfect (${(t.match(/Playing through it[^\n]*/) || [''])[0]})`);
      check(/Volume: set it on the DAC/.test(t) && t.includes(NOVOL), `${at}: "Volume: set it on the DAC", and the note says it plainly`);
      await page.screenshot({ path: path.join(SHOTS, `settings-no-volume-${w}.png`), fullPage: true });
    }
    // Android's permission for Exclusive USB missing (Android 13): one button asks again, a PUT of
    // mode exclusive; nothing else to press
    await setZone(run, zone);
    const WHY13 = 'Android 13: bit-perfect needs Android 14. Exclusive USB needs Android\'s permission for the DAC: tap Allow Exclusive USB in Settings > This phone';
    await nodePlug(run, { ...KA17, usb_permission: false, modes: ['shared', 'exclusive'], note: WHY13 });
    await page.waitForSelector('#dvallow', { timeout: 5000 }).catch(() => {});
    {
      const t = await dacTab(page);
      const b = await page.evaluate(() => { const x = document.querySelector('#dvallow'); return x ? x.textContent.trim() : null; });
      check(b === 'Allow Exclusive USB', `${at}: permission missing: an "Allow Exclusive USB" button ("${b}")`);
      check(t.includes(WHY13), `${at}: and why it plays shared`);
      const n = await page.evaluate(() => document.querySelectorAll('#set-dac input, #set-dac select, #set-dac textarea, #set-dac button:not([data-tabs-back])').length);
      check(n === 1, `${at}: the one control on This phone (${n})`);
      await page.screenshot({ path: path.join(SHOTS, `settings-allow-${w}.png`), fullPage: true });
      const h0 = state.hits.length;
      if (b) { await page.click('#dvallow'); await sleep(400); }
      const puts = state.hits.slice(h0).filter((h) => h.m === 'PUT' && h.v.endsWith('/device')).map((h) => `${decodeURIComponent(h.v)} ${h.body}`);
      check(puts.length === 1 && puts[0] === `/outputs/${OUT}/device {"mode":"exclusive"}`, `${at}: Allow is one PUT of mode exclusive (${JSON.stringify(puts)})`);
    }
    page.off('dialog', onDialog);
    check(dialogs === 0, `${at}: no question asked anywhere (${dialogs} dialogs)`);
    await setZone(run, zone);
    // Unplugged with the node's event only (no app event): This phone goes
    await nodePlug(run, null);
    await page.waitForFunction(() => !document.querySelector('[data-stab="dac"]')).catch(() => {});
    check(await page.evaluate(() => !!document.querySelector('[data-stab="appearance"]') && !document.querySelector('[data-stab="dac"]')), `${at}: unplugged (the node's event), Settings has no This phone`);
    await nodePlug(run, KA17);
    await page.waitForFunction(() => !!document.querySelector('[data-stab="dac"]')).catch(() => {});
    check(await page.evaluate(() => !!document.querySelector('[data-stab="dac"]')), `${at}: plugged again, This phone is back`);
    // DSD by name, an unknown way just "DSD"
    for (const [dsd, words] of [['native', 'DSD (native)'], ['dop', 'DSD (DoP)'], ['unknown', 'DSD']]) {
      await plug(run, { ...KA17, dsd });
      await page.waitForFunction(() => /DSD/.test((document.querySelector('#set-dac') || {}).innerText || '')).catch(() => {});
      const txt = (await dacTab(page)) || '';
      const shown = (txt.match(/DSD[^·\n]*/) || [''])[0].trim();
      check(shown === words, `${at}: dsd "${dsd}" reads "${words}" ("${shown}")`);
    }
    await plug(run, KA17);
    if (w === 1280) {
      // Settings drawn after slow requests: the DAC unplugged meanwhile stays unplugged
      run.slowSettings = 1500;
      const n1 = fetches(state);
      await fx.go(page, '/settings/dac'); await page.evaluate(appScript([OUT]));
      for (let i = 0; i < 100 && fetches(state) <= n1; i++) await sleep(50); // Settings has its outputs
      await sleep(300);
      run.state.fixtures['/outputs'] = { body: [phoneOut(null)] }; run.nodeEvent({ type: 'outputs_changed', output: OUT });
      await sleep(2500); run.slowSettings = 0;
      check(await page.evaluate(() => !!document.querySelector('[data-stab="appearance"]') && !document.querySelector('[data-stab="dac"]')), `${at}: unplugged while Settings loaded: no This phone (it did not write back its older outputs)`);
      check(!/FiiO/.test(await pill(page)), `${at}: nor in the pill ("${await pill(page)}")`);
      await nodePlug(run, KA17);
    }

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
    // The pill first: it must not wait for the picker to have fetched the outputs
    check(/→ FiiO KA17 · shared$/.test(await pill(page)), `web UI: the pill names the DAC without the picker opened first ("${await pill(page)}")`);
    const line = await pickerLine(page); await closePicker(page);
    check(line === 'FiiO KA17 · shared', `web UI: the picker names the DAC on the phone's zone ("${line}")`);
    // Unplugged and plugged on the phone: this controller hears it from the node
    const n0 = fetches(run.state);
    await nodePlug(run, null);
    check(fetches(run.state) > n0, 'web UI: outputs_changed fetches the outputs again');
    check(!/FiiO/.test(await pill(page)), `web UI: unplugged, the pill names no DAC ("${await pill(page)}")`);
    const gone = await pickerLine(page); await closePicker(page);
    check(!/FiiO/.test(gone || ''), `web UI: nor the picker ("${gone}")`);
    await nodePlug(run, KA17);
    check(/→ FiiO KA17 · shared$/.test(await pill(page)), `web UI: plugged again, the pill ("${await pill(page)}")`);
    const back = await pickerLine(page); await closePicker(page);
    check(back === 'FiiO KA17 · shared', `web UI: and the picker ("${back}")`);
    // Plugged, then unplugged at once: the first answer is slow and comes last
    run.outQueue.push(900); run.nodeEvent({ type: 'outputs_changed', output: OUT });
    await sleep(100);
    run.state.fixtures['/outputs'] = { body: [phoneOut(null)] }; run.nodeEvent({ type: 'outputs_changed', output: OUT });
    await sleep(1800);
    check(!/FiiO/.test(await pill(page)), `web UI: an older answer arriving last does not bring the DAC back ("${await pill(page)}")`);
    // Missed while disconnected: plugged, the link drops, it is unplugged, the link comes back
    await nodePlug(run, KA17);
    check(/FiiO KA17/.test(await pill(page)), 'web UI: plugged again before the link drops');
    run.state.fixtures['/outputs'] = { body: [phoneOut(null)] };
    for (const s of run.sockets) { try { s.ws.close(); } catch { /* gone */ } }
    await page.waitForFunction(() => !/FiiO/.test(document.querySelector('#quality .qtext').textContent), null, { timeout: 8000 }).catch(() => {});
    check(!/FiiO/.test(await pill(page)), `web UI: reconnected, the change missed meanwhile shows ("${await pill(page)}")`);
    // Unplugged mid-track: the zone is paused, and the bar says why
    await setZone(run, { ...zone, transport: 'paused', error: 'FiiO KA17 was unplugged: paused' });
    const sub = await page.evaluate(() => (document.querySelector('#np .np-sub') || {}).textContent || '');
    check(sub === 'FiiO KA17 was unplugged: paused', `web UI: a zone paused because its DAC went says so in the bar ("${sub}")`);
    await setZone(run, zone);
    await fx.go(page, '/settings'); await sleep(800);
    check(await page.evaluate(() => !!document.querySelector('[data-stab="appearance"]') && !document.querySelector('[data-stab="dac"]')), 'web UI: no This phone tab');
    check(run.state.errors.length === 0, `web UI: no page errors ${JSON.stringify(run.state.errors)}`);
    await run.context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAILED` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
