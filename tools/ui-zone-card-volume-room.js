// A Zones card's volume has room on desktop and tablet widths (600-1600 px, every 8 px): its volume
// (Mute, slider, level) and its Play to (or "selected", or "Volume: set on the device") stay together
// on one line, beside the transport only while the slider gets at least 80 px there, else under it,
// where the slider takes the line. Before, from ~790 px (and again at ~930, ~1100, ~1300, ~1500 px)
// the volume stayed beside the transport with a 56-79 px slider and Play to went under it alone.
// A group's Ungroup comes after them. Nothing wider than its card or the window. Phones are not changed (tools/ui-phone-zone-cards.js).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-zone-card-volume-room.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'zone-card-volume-room-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let ok = 0; let failed = 0;
const check = (cond, msg) => { if (cond) ok++; else { failed++; console.log(`FAIL ${msg}`); } };

const zone = (id, name) => ({ id, name, output: `upnp:${id}`, transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 }, now_playing: null });
// (Kitchen and Study have a volume; the Porch's is set on the device: no slider, its note instead)
// (and a group of two outputs: its Ungroup comes after them, on a line of its own if need be)
const ZONES = [zone('kitchen', 'Kitchen'), zone('study', 'Study'), { ...zone('porch', 'Porch'), volume: null }, { ...zone('group:g1', 'Kitchen + Study'), output: 'upnp:kitchen' }];
const outputs = ZONES.filter((z) => !z.id.startsWith('group:')).map((z) => ({ id: z.output, name: z.name, kind: 'upnp', caps: z.volume ? { hw_volume: 'percent' } : {} }));

// Each card: the slider's width, and the lines its transport, volume and Play to / selected / note are on
const measure = (page) => page.evaluate(() => [...document.querySelectorAll('#main .zone-card')].map((c) => {
  const mid = (el) => { if (!el || !el.getClientRects().length) return null; const r = el.getBoundingClientRect(); return r.top + r.height / 2; };
  const slider = c.querySelector('.zc-controls input[type=range]');
  const pick = c.querySelector('.zc-controls [data-zsel], .zc-controls .pill.ok');
  const note = [...c.querySelectorAll('.zc-controls span')].find((s) => /set on the device/.test(s.textContent));
  const vol = slider || note; const t = mid(c.querySelector('[data-zt="toggle"]'));
  const cb = c.getBoundingClientRect();
  const out = [...c.querySelectorAll('.zc-controls *')].filter((e) => e.getClientRects().length).some((e) => { const r = e.getBoundingClientRect(); return r.right > cb.right + 0.5 || r.left < cb.left - 0.5; });
  return {
    id: c.dataset.zcard, slider: slider ? slider.getBoundingClientRect().width : null,
    besideTransport: vol ? Math.abs(mid(vol) - t) < 4 : null,
    pickWithVolume: pick && vol ? Math.abs(mid(pick) - mid(vol)) < 4 : null,
    pickKind: pick ? (pick.matches('[data-zsel]') ? 'Play to' : 'selected') : null, out,
  };
}));

(async () => {
  const browser = await chromium.launch();
  const { context, page, state } = await fx.create(browser, 1280, { fixtures: { '/zones': { body: ZONES }, '/outputs': { body: outputs } } });
  try {
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: ZONES })); });
    await fx.go(page, '/zones');
    await page.waitForSelector('#main .zone-card[data-zcard="porch"]', { timeout: 8000 });
    await page.waitForTimeout(400);
    const seen = new Set();
    for (let w = 600; w <= 1600; w += 8) {
      await page.setViewportSize({ width: w, height: 900 });
      await page.waitForTimeout(40);
      if (await page.evaluate(() => document.documentElement.dataset.shell === 'phone')) continue;
      const cs = await measure(page);
      check(cs.length === 5, `${w}: five cards (${cs.length})`);
      for (const c of cs) {
        const at = `${w} ${c.id}`;
        check(c.pickWithVolume === true, `${at}: its ${c.pickKind} is on the volume's line, not alone (${JSON.stringify(c)})`);
        if (c.slider != null && c.besideTransport) check(c.slider >= 80, `${at}: a volume beside the transport has a slider of at least 80 px (${Math.round(c.slider)})`);
        if (c.slider != null) check(c.slider >= 64, `${at}: the slider is at least 64 px (${Math.round(c.slider)})`);
        check(!c.out, `${at}: nothing in its controls is wider than the card`);
        seen.add(`${c.id}:${c.besideTransport ? 'beside' : 'under'}`);
      }
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${w}: no sideways scroll`);
      if ([800, 1024, 1304].includes(w)) await page.screenshot({ path: path.join(SHOTS, `zones-${w}.png`) });
    }
    // (both layouts were reached, so the checks above saw each)
    for (const k of ['browser:beside', 'browser:under', 'kitchen:beside', 'kitchen:under', 'porch:beside', 'porch:under']) check(seen.has(k), `some width draws ${k.replace(':', '’s volume ')} the transport (${[...seen].join(', ')})`);
    check(state.errors.length === 0, `no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); await browser.close(); }
  console.log(`${ok} ok, ${failed} failed`);
  if (failed) process.exit(1);
})().catch((e) => { console.error(e); process.exit(1); });
