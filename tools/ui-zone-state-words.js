// Every zone list says a zone's state with the same words as the Zones page: "playing", "paused",
// "error" for a zone stopped by an error, nothing for a quiet stopped zone. Before, the Play to
// picker said only "playing" (a paused zone or one that failed looked idle, so you could pick a
// broken speaker without a hint), and the Playing on panel had no word for an error.
// Phone widths (and the Android app) and desktop alike.
// Real candidate assets, isolated responses: node tools/ui-zone-state-words.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'zone-state-words-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = (id, name, transport, extra = {}) => ({ id, name, output: `upnp:${id}`, transport, queue: transport === 'stopped' ? [] : [{ id: 1, uri: 'track:1', title: 'Come On' }], queue_index: transport === 'stopped' ? null : 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: transport === 'stopped' ? null : np, ...extra });
const WHY = 'The device went away';
const ZONES = [zone('kitchen', 'Kitchen', 'playing'), zone('study', 'Study', 'paused'), zone('porch', 'Porch', 'paused', { error: 'The USB DAC went away' }), zone('bedroom', 'Bedroom', 'stopped'), zone('garage', 'Garage', 'stopped', { error: WHY })];
const outputs = ZONES.map((z) => ({ id: z.output, name: z.name, kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }));
// The Zones page's pills: what every other list must say
const WANT = { kitchen: 'playing', study: 'paused', porch: 'paused', bedroom: '', garage: 'error' };

const open = async (browser, width, android) => {
  let sock = null;
  const { context, page } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/zones': { body: ZONES }, '/outputs': { body: outputs } } });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); sock = ws; ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: ZONES })); });
  return { context, page, send: (ev) => sock && sock.send(JSON.stringify(ev)) };
};

const run = async (browser, width, android) => {
  const at = `${android ? 'android' : 'browser'}-${width}`;
  const { context, page, send } = await open(browser, width, android);
  try {
    await fx.go(page, '/zones');
    await page.waitForSelector('#main .zone-card[data-zcard="garage"]', { timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(300);
    const pills = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll('#main .zone-card')].map((c) => [c.dataset.zcard, [...c.querySelectorAll('.zc-head .pill')].map((p) => p.textContent.trim().toLowerCase()).join(' ')])));
    check(Object.entries(WANT).every(([id, w]) => pills[id] === w), `${at}: the Zones page's pills, the reference (${JSON.stringify(pills)})`);

    // Play to: the zone picker
    await page.evaluate(() => document.querySelector('#zonebtn').click()); // (a phone opens it from Now playing's zone line, the same button)
    await page.waitForSelector('#zonepop:not([hidden]) [data-zone="garage"]', { timeout: 3000 }).catch(() => {});
    const pick = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll('#zonepop [data-zone]')].map((b) => { const s = b.querySelector('.state'); if (!s) return [b.dataset.zone, null]; const c = s.cloneNode(true); c.querySelectorAll('.sr-only').forEach((x) => x.remove()); return [b.dataset.zone, c.textContent.trim().toLowerCase()]; })));
    for (const [id, w] of Object.entries(WANT)) check(pick[id] === w, `${at}: Play to says "${w}" for ${id}, as its Zones card (${JSON.stringify(pick[id])})`);
    const err = await page.evaluate(() => { const s = document.querySelector('#zonepop [data-zone="garage"] .state .warn-text'); return s ? { title: s.title, colour: getComputedStyle(s).color, muted: getComputedStyle(s.parentElement).color, name: s.closest('button').textContent.replace(/\s+/g, ' ').trim() } : null; });
    check(!!err && err.title.includes(WHY) && err.colour !== err.muted && err.name.includes(WHY), `${at}: Play to's "error" is in the warning colour, and its reason is in the tooltip and the button's name (${JSON.stringify(err)})`);
    const fit = await page.evaluate(() => [...document.querySelectorAll('#zonepop .pop-row')].every((r) => { const a = r.getBoundingClientRect(); const s = r.querySelector('.state'); const b = s && s.getBoundingClientRect(); return !b || !b.width || (b.right <= a.right + 0.5 && b.left >= a.left - 0.5); }) && document.documentElement.scrollWidth <= innerWidth);
    check(fit, `${at}: Play to: every state word inside its row, no sideways overflow`);
    await page.screenshot({ path: path.join(SHOTS, `playto-${at}.png`) });
    await page.keyboard.press('Escape');
    await page.waitForTimeout(150);

    // Playing on: the panel behind the quality pill (Sound in Now playing on a phone), for each zone in use
    const said = {};
    for (const id of Object.keys(WANT)) {
      await page.evaluate((zid) => { localStorage.setItem('pm.zone', zid); }, id);
      await page.evaluate(() => document.querySelector('#zonebtn').click()); // (a phone opens it from Now playing's zone line, the same button)
      await page.waitForSelector(`#zonepop:not([hidden]) [data-zone="${id}"]`, { timeout: 3000 }).catch(() => {});
      await page.evaluate((zid) => { const b = document.querySelector(`#zonepop [data-zone="${zid}"]`); if (b) b.click(); }, id);
      await page.waitForTimeout(150);
      await page.evaluate(() => document.querySelector('#quality').click());
      await page.waitForSelector('#qpanel:not([hidden]) .qp-zone', { timeout: 3000 }).catch(() => {});
      said[id] = await page.evaluate(() => { const z = document.querySelector('#qpanel:not([hidden]) .qp-zone'); const st = z && z.querySelector('.state'); if (!st) return null; const c = st.cloneNode(true); c.querySelectorAll('.sr-only').forEach((x) => x.remove()); return { name: z.querySelector('.name').textContent.trim(), state: c.textContent.trim().toLowerCase(), heard: st.textContent.trim(), warn: !!st.querySelector('.warn-text') }; });
      if (id === 'garage') await page.screenshot({ path: path.join(SHOTS, `playingon-garage-${at}.png`) });
      if (id === 'kitchen') await page.screenshot({ path: path.join(SHOTS, `playingon-kitchen-${at}.png`) });
      await page.keyboard.press('Escape');
      await page.waitForTimeout(150);
    }
    const NAMES = Object.fromEntries(ZONES.map((z) => [z.id, z.name]));
    check(Object.keys(WANT).every((id) => said[id] && said[id].name === NAMES[id]), `${at}: Playing on shows each picked zone (${Object.values(said).map((x) => x && x.name).join(',')})`);
    for (const [id, w] of Object.entries(WANT)) check(!!said[id] && said[id].state === w, `${at}: Playing on says "${w}" for ${id}, as its Zones card (${JSON.stringify(said[id])})`);
    check(!!said.garage && said.garage.warn && said.garage.heard.includes(WHY), `${at}: Playing on's "error" is in the warning colour, its reason read out (${said.garage && said.garage.heard})`);
    // The open panel follows a zone that fails while it is open (it stays stopped; only its error changes), and back
    const panelWord = () => page.evaluate(() => { const st = document.querySelector('#qpanel:not([hidden]) .qp-zone .state'); if (!st) return null; const c = st.cloneNode(true); c.querySelectorAll('.sr-only').forEach((x) => x.remove()); return c.textContent.trim().toLowerCase(); });
    await page.evaluate(() => document.querySelector('#zonebtn').click());
    await page.waitForSelector('#zonepop:not([hidden]) [data-zone="bedroom"]', { timeout: 3000 }).catch(() => {});
    await page.evaluate(() => { const b = document.querySelector('#zonepop [data-zone="bedroom"]'); if (b) b.click(); });
    await page.waitForTimeout(150);
    await page.evaluate(() => document.querySelector('#quality').click());
    await page.waitForSelector('#qpanel:not([hidden]) .qp-zone', { timeout: 3000 }).catch(() => {});
    const quiet = await panelWord();
    const bedroom = ZONES.find((z) => z.id === 'bedroom');
    send({ type: 'zone', seq: 1, state: { ...bedroom, error: WHY } });
    await page.waitForFunction(() => /error/.test((document.querySelector('#qpanel:not([hidden]) .qp-zone .state') || {}).textContent || ''), null, { timeout: 2000 }).catch(() => {});
    const failed = await panelWord();
    send({ type: 'zone', seq: 2, state: { ...bedroom, error: null } });
    await page.waitForFunction(() => !/error/.test((document.querySelector('#qpanel:not([hidden]) .qp-zone .state') || {}).textContent || 'error'), null, { timeout: 2000 }).catch(() => {});
    const cleared = await panelWord();
    check(quiet === '' && failed === 'error' && cleared === '', `${at}: the open Playing on panel says "error" as soon as its stopped zone fails, and drops it when the error clears ("${quiet}" → "${failed}" → "${cleared}")`);
    await page.keyboard.press('Escape');
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
