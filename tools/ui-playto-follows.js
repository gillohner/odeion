// An open Play to menu follows the zones. Before, it was drawn once when opened: a zone that went
// stayed in it (a tap on it picked a zone no longer there), one that came was missing, and a zone
// that started playing or failed kept its old state word, until the menu was closed and opened again.
// It is drawn again only when what it shows changed (a volume change leaves it alone), keeps an error
// it shows, and keeps the keyboard's place (the first row when that row went).
// Phone widths (and the Android app) and desktop alike.
// Real candidate assets, isolated responses: node tools/ui-playto-follows.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'playto-follows-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = (id, name, transport, extra = {}) => ({ id, name, output: `upnp:${id}`, transport, queue: transport === 'stopped' ? [] : [{ id: 1, uri: 'track:1', title: 'Come On' }], queue_index: transport === 'stopped' ? null : 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: transport === 'stopped' ? null : np, ...extra });
const KITCHEN = zone('kitchen', 'Kitchen', 'playing');
const STUDY = zone('study', 'Study', 'stopped');
const GARAGE = zone('garage', 'Garage', 'stopped');
const PORCH = zone('porch', 'Porch', 'stopped');
const ZONES = [KITCHEN, STUDY, GARAGE];
const outputs = [...ZONES, PORCH].map((z) => ({ id: z.output, name: z.name, kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }));
const WHY = 'Study is not open to you';

const open = async (browser, width, android) => {
  let sock = null;
  const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/zones': { body: ZONES }, '/outputs': { body: outputs }, '/zones/kitchen/transfer': { status: 403, body: { error: WHY } } } });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  await context.addInitScript(() => { localStorage.setItem('pm.zone', 'kitchen'); });
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); sock = ws; ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: ZONES })); });
  return { context, page, state, send: (ev) => sock && sock.send(JSON.stringify(ev)) };
};

const word = (page, id) => page.evaluate((zid) => { const s = document.querySelector(`#zonepop:not([hidden]) [data-zone="${zid}"] .state`); if (!s) return null; const c = s.cloneNode(true); c.querySelectorAll('.sr-only').forEach((x) => x.remove()); return c.textContent.trim().toLowerCase(); }, id);
const rows = (page) => page.evaluate(() => [...document.querySelectorAll('#zonepop:not([hidden]) [data-zone]')].map((b) => b.dataset.zone));
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 1500 }).catch(() => {});

const run = async (browser, width, android) => {
  const at = `${android ? 'android' : 'browser'}-${width}`;
  const { context, page, send } = await open(browser, width, android);
  let seq = 0;
  try {
    await fx.go(page, '/');
    await page.waitForTimeout(500);
    await page.evaluate(() => document.querySelector('#zonebtn').click()); // (a phone opens it from Now playing's zone line, the same button)
    await page.waitForSelector('#zonepop:not([hidden]) [data-zone="study"]', { timeout: 3000 }).catch(() => {});
    check((await rows(page)).includes('study'), `${at}: Play to lists Study (${await rows(page)})`);
    await page.screenshot({ path: path.join(SHOTS, `open-${at}.png`) });

    // A volume change shows nothing new: the menu is not drawn again
    await page.evaluate(() => { const t = document.querySelector('#zonepop .pop-title'); if (t) t.dataset.mark = '1'; });
    send({ type: 'zone', seq: ++seq, state: { ...GARAGE, volume: { percent: 40, muted: false } } });
    await page.waitForTimeout(300);
    check(await page.evaluate(() => !!document.querySelector('#zonepop .pop-title[data-mark]')), `${at}: a zone event that changes nothing it shows leaves the menu as drawn`);

    // The keyboard is on Study's row; Study starts playing
    await page.evaluate(() => { const b = document.querySelector('#zonepop [data-zone="study"]'); if (b) b.focus(); });
    send({ type: 'zone', seq: ++seq, state: zone('study', 'Study', 'playing') });
    await until(page, () => /playing/.test((document.querySelector('#zonepop [data-zone="study"] .state') || {}).textContent || ''));
    const playing = await word(page, 'study');
    check(playing === 'playing', `${at}: the open menu says "playing" as soon as Study starts (${JSON.stringify(playing)})`);
    check(await page.evaluate(() => (document.activeElement && document.activeElement.dataset.zone) === 'study'), `${at}: the keyboard stays on Study's row`);
    // Study fails
    send({ type: 'zone', seq: ++seq, state: { ...STUDY, error: 'The device went away' } });
    await until(page, () => /error/.test((document.querySelector('#zonepop [data-zone="study"] .state') || {}).textContent || ''));
    const err = await word(page, 'study');
    check(err === 'error', `${at}: … and "error" as soon as it fails (${JSON.stringify(err)})`);

    // A zone comes
    send({ type: 'zone', seq: ++seq, state: PORCH });
    await until(page, () => !!document.querySelector('#zonepop [data-zone="porch"]'));
    check((await rows(page)).includes('porch'), `${at}: a zone that came is in the open menu (${await rows(page)})`);

    // The focused zone goes: its row goes, the keyboard stays in the menu
    send({ type: 'zone_removed', seq: ++seq, zone: 'study' });
    await until(page, () => !document.querySelector('#zonepop [data-zone="study"]'));
    check(!(await rows(page)).includes('study'), `${at}: a zone that went is gone from the open menu (${await rows(page)})`);
    const focus = await page.evaluate(() => { const a = document.activeElement; return { inMenu: !!a && !!a.closest('#zonepop'), item: !!a && a.matches('.pop-item') }; });
    check(focus.inMenu && focus.item, `${at}: the keyboard, on the row that went, moves to a row of the menu (${JSON.stringify(focus)})`);
    await page.screenshot({ path: path.join(SHOTS, `followed-${at}.png`) });

    // An error it shows stays when a zone event draws it again
    await page.evaluate(() => { const b = document.querySelector('#zonepop [data-move="garage"]'); if (b) b.click(); });
    await until(page, () => !!document.querySelector('#zonepop .pop-err'));
    const shown = await page.evaluate(() => (document.querySelector('#zonepop .pop-err') || {}).textContent || '');
    send({ type: 'zone', seq: ++seq, state: zone('garage', 'Garage', 'playing') });
    await until(page, () => /playing/.test((document.querySelector('#zonepop [data-zone="garage"] .state') || {}).textContent || ''));
    const kept = await page.evaluate(() => ({ err: (document.querySelector('#zonepop .pop-err') || {}).textContent || '', word: ((document.querySelector('#zonepop [data-zone="garage"] .state') || {}).textContent || '').trim() }));
    check(shown.includes(WHY) && kept.err === shown && /playing/.test(kept.word), `${at}: Move here's error stays when the menu follows a zone (${JSON.stringify({ shown, ...kept })})`);
    // … and when a fresh snapshot comes (a reconnect), which also asks the outputs again
    send({ type: 'snapshot', seq: ++seq, boot: 'fixture-boot', library_rev: 0, zones: [KITCHEN, zone('garage', 'Garage', 'paused'), PORCH] });
    await until(page, () => /paused/.test((document.querySelector('#zonepop [data-zone="garage"] .state') || {}).textContent || ''));
    await page.waitForTimeout(400);
    const snap = await page.evaluate(() => ({ err: (document.querySelector('#zonepop .pop-err') || {}).textContent || '', word: ((document.querySelector('#zonepop [data-zone="garage"] .state') || {}).textContent || '').trim() }));
    check(snap.err === shown && /paused/.test(snap.word), `${at}: … and after a fresh snapshot and the outputs asked again (${JSON.stringify(snap)})`);
    await page.screenshot({ path: path.join(SHOTS, `error-kept-${at}.png`) });

    // A tap on a row picks a zone that is there
    await page.evaluate(() => { const b = document.querySelector('#zonepop [data-zone="porch"]'); if (b) b.click(); });
    await page.waitForTimeout(200);
    check(await page.evaluate(() => localStorage.getItem('pm.zone') === 'porch' && document.querySelector('#zonepop').hidden), `${at}: a tap on the zone that came plays to it and closes the menu`);
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at}: no sideways overflow`);
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
