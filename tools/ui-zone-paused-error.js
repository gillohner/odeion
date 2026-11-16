// A zone's error always reaches the UI, wherever the zone is listed, and each list says a zone's
// state the same way. A zone paused by an error (the app's own player when its USB DAC went away,
// or a device that refused to go on) says why on its Zones card, where the artist is (as the mini
// player says it), and keeps its "paused" pill. Settings › Playback's zone list follows the Zones
// page's rule: playing, paused or "error" only, with no "stopped" pill for a quiet zone.
// Phone widths (and the Android app) and desktop alike.
// Real candidate assets, isolated responses: node tools/ui-zone-paused-error.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'zone-paused-error-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = (id, name, transport, extra = {}) => ({ id, name, output: `upnp:${id}`, transport, queue: transport === 'stopped' ? [] : [{ id: 1, uri: 'track:1', title: 'Come On' }], queue_index: transport === 'stopped' ? null : 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: transport === 'stopped' ? null : np, ...extra });
const WHY = 'The USB DAC went away';
const ZONES = [zone('kitchen', 'Kitchen', 'playing'), zone('study', 'Study', 'paused'), zone('porch', 'Porch', 'paused', { error: WHY }), zone('bedroom', 'Bedroom', 'stopped'), zone('garage', 'Garage', 'stopped', { error: 'The device went away' })];
const outputs = ZONES.map((z) => ({ id: z.output, name: z.name, kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }));

const open = async (browser, width, android) => {
  const { context, page } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/zones': { body: ZONES }, '/outputs': { body: outputs } } });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: ZONES })); });
  return { context, page };
};

const run = async (browser, width, android) => {
  const at = `${android ? 'android' : 'browser'}-${width}`;
  const { context, page } = await open(browser, width, android);
  try {
    await fx.go(page, '/zones');
    await page.waitForSelector('#main .zone-card[data-zcard="porch"]', { timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(300);
    const cards = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll('#main .zone-card')].map((c) => {
      const alert = c.querySelector('.zc-text .warn-text');
      const r = alert && alert.getBoundingClientRect();
      return [c.dataset.zcard, { pills: [...c.querySelectorAll('.zc-head .pill')].map((p) => p.textContent.trim().toLowerCase()), text: c.querySelector('.zc-text').innerText, alert: alert ? alert.textContent.trim() : null, loud: !!(alert && alert.closest('[role="alert"]')), seen: !!r && r.width > 0 && r.height > 0 && r.right <= innerWidth + 0.5 }];
    })));
    const porch = cards.porch || {}; const study = cards.study || {};
    check(!!porch.alert && porch.alert.includes(WHY), `${at}: a zone paused by an error says why on its card ("${porch.alert}")`);
    check(!!porch.seen, `${at}: the error is on screen, within the width`);
    check(!porch.loud, `${at}: the paused zone's error is not an alert (the page is drawn again on each play/pause; the mini player announces it)`);
    check((porch.pills || []).join() === 'paused', `${at}: it keeps its "paused" pill (${porch.pills})`);
    check(/Come On/.test(porch.text || '') && !/Dr\. Peacock/.test(porch.text || ''), `${at}: the error is where the artist is, under the song (as the mini player has it)`);
    check(!study.alert && /Dr\. Peacock/.test(study.text || ''), `${at}: a zone paused without an error still shows its artist, no error line`);
    check(cards.garage && /Couldn’t play: The device went away/.test(cards.garage.alert || ''), `${at}: a zone stopped by an error still says so (${cards.garage && cards.garage.alert})`);
    check(cards.garage && !cards.garage.loud, `${at}: the stopped zone's "Couldn’t play" is not an alert either (read out again on every redraw)`);
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at}: Zones: no sideways overflow`);
    await page.screenshot({ path: path.join(SHOTS, `zones-${at}.png`), fullPage: width >= 600 });

    // This device's card is drawn again when its player plays or pauses (the app's player pausing
    // on a lost DAC is a pause): before, only a node zone's change drew the page again
    const wiped = await page.evaluate(() => { const here = [...document.querySelectorAll('#main .zone-card')].find((c) => !['kitchen', 'study', 'porch', 'bedroom', 'garage'].includes(c.dataset.zcard)); if (!here) return false; here.dataset.stale = '1'; document.querySelector('#browser').dispatchEvent(new Event('pause')); return true; });
    await page.waitForFunction(() => !document.querySelector('#main .zone-card[data-stale]'), null, { timeout: 2000 }).catch(() => {});
    check(wiped && await page.evaluate(() => !document.querySelector('#main .zone-card[data-stale]') && document.querySelectorAll('#main .zone-card').length === 6), `${at}: a pause of this device's player draws its Zones card again`);
    // A keyboard user's Play on that card keeps the focus through the new drawing
    await page.evaluate(() => { const here = [...document.querySelectorAll('#main .zone-card')].find((c) => !['kitchen', 'study', 'porch', 'bedroom', 'garage'].includes(c.dataset.zcard)); const t = here && here.querySelector('[data-zt="toggle"]'); if (t) { t.focus(); here.dataset.stale = '1'; } document.querySelector('#browser').dispatchEvent(new Event('pause')); });
    await page.waitForFunction(() => !document.querySelector('#main .zone-card[data-stale]'), null, { timeout: 2000 }).catch(() => {});
    await page.waitForTimeout(100);
    const kept = await page.evaluate(() => { const a = document.activeElement; const c = a && a.closest('#main .zone-card'); return !!c && a.dataset.zt === 'toggle' && !['kitchen', 'study', 'porch', 'bedroom', 'garage'].includes(c.dataset.zcard); });
    check(kept, `${at}: the focus stays on this device's Play through that redraw`);
    // A long reason is read whole: it wraps inside the card, not cut with "…"
    const long = await page.evaluate(() => { const s = document.querySelector('#main .zone-card[data-zcard="porch"] .zc-text .sub'); const w = s && s.querySelector('.warn-text'); if (!w) return null; w.textContent = 'The USB DAC went away: plug it in again, or pick another output for this phone'; return { cut: s.scrollWidth > s.clientWidth + 1, lines: Math.round(w.getBoundingClientRect().height / parseFloat(getComputedStyle(w).lineHeight || '16')), over: document.documentElement.scrollWidth > innerWidth }; });
    check(!!long && !long.cut && !long.over, `${at}: a long error wraps inside the card, nothing cut off (${JSON.stringify(long)})`);

    await fx.go(page, '/settings/playback');
    await page.waitForFunction(() => [...document.querySelectorAll('#main .set-name')].some((n) => n.textContent.trim() === 'Porch'), null, { timeout: 5000 }).catch(() => {});
    const rows = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll('#main .set-row')].filter((r) => r.querySelector('.set-name') && /^(Kitchen|Study|Porch|Bedroom|Garage)$/.test(r.querySelector('.set-name').textContent.trim())).map((r) => [r.querySelector('.set-name').textContent.trim(), [...r.querySelectorAll('.pill')].map((p) => p.textContent.trim().toLowerCase())])));
    check(Object.keys(rows).length === 5, `${at}: Settings › Playback lists the five zones (${Object.keys(rows).join(',')})`);
    check(Object.values(rows).every((p) => !p.includes('stopped')), `${at}: Settings' zone list has no "stopped" pill (${Object.entries(rows).map(([k, p]) => `${k}:${p.join('+') || '-'}`).join(' ')})`);
    check((rows.Bedroom || ['x']).length === 0, `${at}: a quiet stopped zone has no pill in Settings (${rows.Bedroom})`);
    check((rows.Garage || []).join() === 'error', `${at}: a zone stopped by an error says "error" in Settings, as on its card (${rows.Garage})`);
    check((rows.Kitchen || []).join() === 'playing' && (rows.Study || []).join() === 'paused', `${at}: playing and paused zones keep their pills in Settings (${rows.Kitchen}/${rows.Study})`);
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at}: Settings: no sideways overflow`);
    const zh = await page.$('#main .set-row .set-name >> text=Kitchen');
    if (zh) await zh.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(SHOTS, `settings-${at}.png`) });
  } finally { await context.close(); }
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, true], [360, false], [412, false], [1280, false]]) await run(browser, width, android);
  } finally { await browser.close(); }
  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
