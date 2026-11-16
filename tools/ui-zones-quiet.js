// The Zones page says only what's needed: its head is the count of cards (as Songs, Albums and
// Artists are), with no "multi-room overview" under it, and a card's state pill is there only when
// it says something the card doesn't: playing, paused or an error. A stopped zone's card already
// says "Nothing playing" (or shows the song and a Play button), so it gets no "stopped" pill.
// The head keeps its height without the pill, so a zone that stops doesn't move the cards under it.
// Phone widths (and the Android app) and desktop alike.
// Real candidate assets, isolated responses: node tools/ui-zones-quiet.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'zones-quiet-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = (id, name, transport, extra = {}) => ({ id, name, output: `upnp:${id}`, transport, queue: transport === 'stopped' ? [] : [{ id: 1, uri: 'track:1', title: 'Come On' }], queue_index: transport === 'stopped' ? null : 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: transport === 'stopped' ? null : np, ...extra });
// A playing, a paused, a quiet stopped one and one stopped by an error
const ZONES = [zone('kitchen', 'Kitchen', 'playing'), zone('study', 'Study', 'paused'), zone('bedroom', 'Bedroom', 'stopped'), zone('garage', 'Garage', 'stopped', { error: 'The device went away' })];
const outputs = ZONES.map((z) => ({ id: z.output, name: z.name, kind: 'upnp', caps: { hw_volume: 'percent' } }));

const look = (page) => page.evaluate(() => {
  const head = document.querySelector('#main .page-head h2');
  const count = head && head.querySelector('.count');
  return {
    title: head ? head.textContent.trim() : null, count: count ? count.textContent.trim() : null,
    cards: [...document.querySelectorAll('#main .zone-card')].map((c) => ({ id: c.dataset.zcard, pills: [...c.querySelectorAll('.zc-head .pill')].map((p) => p.textContent.trim().toLowerCase()), h3: c.querySelector('.zc-head h3').getBoundingClientRect().width, head: c.querySelector('.zc-head').getBoundingClientRect().width, headH: c.querySelector('.zc-head').getBoundingClientRect().height })),
    overflow: document.documentElement.scrollWidth > innerWidth,
  };
});

const run = async (browser, width, android, zones, label) => {
  const at = `${label} ${android ? 'android' : 'browser'}-${width}`;
  const { context, page } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/zones': { body: zones }, '/outputs': { body: zones.length ? outputs : [] } } });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
  try {
    await fx.go(page, '/zones');
    await page.waitForSelector('#main .zone-card', { timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(300);
    const v = await look(page);
    const n = v.cards.length;
    check(n === zones.length + 1, `${at}: ${zones.length + 1} zone cards, this device's too (${n})`);
    check(v.count === String(n), `${at}: the head's count is the number of cards only ("${v.count}")`);
    check(!/overview/i.test(v.title || ''), `${at}: no "multi-room overview" in the head ("${v.title}")`);
    const byId = Object.fromEntries(v.cards.map((c) => [c.id, c]));
    const here = v.cards.find((c) => !zones.some((z) => z.id === c.id));
    check(!!here && !here.pills.includes('stopped') && here.pills.length === 0, `${at}: this device's card, stopped with nothing playing, has no state pill (${here && here.pills.join(',')})`);
    check(v.cards.every((c) => !c.pills.includes('stopped')), `${at}: no card says "stopped" (${v.cards.map((c) => `${c.id}:${c.pills.join('+') || '-'}`).join(' ')})`);
    if (zones.length) {
      check(byId.kitchen && byId.kitchen.pills.join() === 'playing', `${at}: a playing zone still says "playing" (${byId.kitchen && byId.kitchen.pills})`);
      check(byId.study && byId.study.pills.join() === 'paused', `${at}: a paused zone still says "paused" (${byId.study && byId.study.pills})`);
      check(byId.garage && byId.garage.pills.join() === 'error', `${at}: a zone stopped by an error still says "error" (${byId.garage && byId.garage.pills})`);
      check(byId.bedroom && byId.bedroom.pills.length === 0, `${at}: a quiet stopped zone has no pill (${byId.bedroom && byId.bedroom.pills})`);
    }
    // (a zone that stops loses its pill: its head keeps its height, so nothing under it moves)
    const hs = v.cards.map((c) => c.headH);
    check(Math.max(...hs) - Math.min(...hs) < 0.5, `${at}: every card's head is the same height, with or without a pill (${hs.map((h) => h.toFixed(1)).join('/')} px)`);
    check(!!here && here.h3 >= here.head - 40, `${at}: without the pill the name has the card's width (${here && Math.round(here.h3)} of ${here && Math.round(here.head)} px)`);
    check(!v.overflow, `${at}: no sideways overflow`);
    await page.screenshot({ path: path.join(SHOTS, `zones-${label}-${android ? 'android-' : ''}${width}.png`), fullPage: width >= 600 });
  } finally { await context.close(); }
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, true], [360, false], [412, false], [1280, false]]) {
      await run(browser, width, android, [], 'alone');
      await run(browser, width, android, ZONES, 'four');
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
