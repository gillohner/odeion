// On a phone a zone card's "Play to" (or its "selected" mark) sits on the card's now-playing line,
// and a zone without a volume says "Volume: set on the device" to screen readers only, so the
// card's controls are one line with a full slider and an unselected card is ~181 px instead of 227.
// Each "Play to" names its zone. Desktop keeps Play to and the note in the controls line, and the
// cards move between the two when the window crosses the phone width.
// Real candidate assets, isolated responses: node tools/ui-phone-zone-cards.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-zone-cards-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// The owner's five zones (as in ui-android-first.js): the Linn seen two ways is one card with chips,
// the UPnP ones have a volume, the AirPlay ones don't
const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const NAMES = [['heimkino', 'Heimkino (AirPlay)', 'airplay'], ['linn-air', 'Linn Klimax DS3 (AirPlay)', 'airplay'], ['linn-upnp', 'Linn Klimax DS3:UPnP AV', 'upnp'], ['mbp', 'MacBook Pro ELo (AirPlay)', 'airplay'], ['tv', '[TV] The Frame 65 Lohner', 'upnp']];
const outputs = NAMES.map(([id, name, kind]) => ({ id: `${kind}:${id}`, name, kind, caps: kind === 'upnp' ? { hw_volume: 'percent' } : {} }));
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zones = NAMES.map(([id, name, kind], i) => ({ id, name, output: `${kind}:${id}`, transport: i === 2 ? 'playing' : 'stopped', queue: i === 2 ? [{ id: 1, uri: 'track:1', title: 'Come On' }] : [], queue_index: i === 2 ? 0 : null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: kind === 'upnp' ? { percent: 20, muted: false } : null, now_playing: i === 2 ? np : null }));

// Each card: where its Play to / selected mark is, its controls line, slider, note and height
const cards = (page) => page.evaluate(() => [...document.querySelectorAll('#main .zone-card')].map((c) => {
  const box = (el) => { if (!el || !el.getClientRects().length) return null; const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, right: r.right, bottom: r.bottom }; };
  const pick = c.querySelector(':scope > .zc-now > [data-zsel], :scope > .zc-now > .pill.ok, :scope > .zc-controls > [data-zsel], :scope > .zc-controls > .pill.ok');
  const ctl = c.querySelector('.zc-controls');
  const note = [...c.querySelectorAll('.zc-controls span')].find((s) => /set on the device/.test(s.textContent));
  const pb = box(pick);
  let hits = null;
  if (pick && pick.matches('[data-zsel]')) {
    pick.scrollIntoView({ block: 'center', behavior: 'instant' });
    const b = pick.getBoundingClientRect();
    const on = (x, y) => { const h = document.elementFromPoint(x, y); return !!h && (h === pick || pick.contains(h)); };
    hits = { mid: on(b.left + b.width / 2, b.top + b.height / 2), above: on(b.left + b.width / 2, b.top - 2), below: on(b.left + b.width / 2, b.bottom + 2) };
  }
  return {
    id: c.dataset.zcard, on: c.classList.contains('on'), h: c.getBoundingClientRect().height,
    pickIn: pick ? pick.parentElement.className : null, pickKind: pick ? (pick.matches('[data-zsel]') ? 'button' : 'pill') : null,
    pickLabel: pick && pick.getAttribute('aria-label'), pick: pb, hits,
    ctlH: ctl.getBoundingClientRect().height,
    slider: box(c.querySelector('.zc-controls input[type=range]')),
    note: note ? { text: note.textContent.trim(), shown: !!box(note) && note.getBoundingClientRect().width > 1 } : null,
    name: c.querySelector('h3').textContent.trim(),
  };
}));

const setup = async (browser, width, android) => {
  const { context, page } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/zones': { body: zones }, '/outputs': { body: outputs } } });
  await context.addInitScript(() => localStorage.setItem('pm.zone', 'linn-upnp'));
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  return { context, page };
};
const open = async (page) => { await fx.go(page, '/zones'); await page.waitForSelector('#main .zone-card [data-zsel]'); await page.waitForTimeout(300); };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      const { context, page } = await setup(browser, width, android);
      try {
        await page.setViewportSize({ width, height: width === 360 ? 640 : 732 });
        await open(page);
        const cs = await cards(page);
        check(cs.length === 5, `${at}: five zone cards (${cs.length})`);
        const sel = cs.find((c) => c.on);
        check(!!sel && sel.pickKind === 'pill' && sel.pickIn === 'zc-now', `${at}: the selected card says "selected" on its now-playing line (${sel && sel.pickIn})`);
        const others = cs.filter((c) => !c.on);
        check(others.length === 4 && others.every((c) => c.pickKind === 'button' && c.pickIn === 'zc-now'), `${at}: every other card's Play to is on its now-playing line (${others.map((c) => c.pickIn).join(', ')})`);
        check(others.every((c) => c.pickLabel === `Play to ${(zones.find((z) => z.id === c.id) || { name: c.name }).name}`), `${at}: each Play to names its zone (${others.map((c) => c.pickLabel).join(' | ')})`);
        check(others.every((c) => c.hits && c.hits.mid && c.hits.above && c.hits.below && c.pick.right <= width), `${at}: each Play to takes a tap at its middle and just above and below, inside the screen`);
        check(cs.every((c) => c.ctlH <= 48), `${at}: every card's controls are one line (${cs.map((c) => Math.round(c.ctlH)).join('/')} px)`);
        const sliders = cs.filter((c) => c.slider);
        check(sliders.length === 3 && sliders.every((c) => c.slider.w >= 100), `${at}: the volume sliders keep their full width (${sliders.map((c) => Math.round(c.slider.w)).join('/')} px)`);
        const noVol = cs.filter((c) => !c.slider);
        check(noVol.length === 2 && noVol.every((c) => c.note && c.note.text === 'Volume: set on the device' && !c.note.shown), `${at}: a card without a volume says so to screen readers only`);
        check(others.every((c) => c.h <= 190), `${at}: unselected cards are compact (${others.map((c) => Math.round(c.h)).join('/')} px)`);
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && document.documentElement.scrollHeight <= innerHeight + 1), `${at}: no sideways overflow, the page itself doesn't scroll`);
        await page.screenshot({ path: path.join(SHOTS, `zones-${at}.png`) });
        // Play to on a phone: one tap picks the zone, and the card's "selected" takes the button's place
        const target = others.find((c) => c.id === 'heimkino');
        const tapped = await page.locator('[data-zcard="heimkino"] .zc-now [data-zsel="heimkino"]').tap({ timeout: 2000 }).then(() => true, () => false);
        check(tapped, `${at}: Heimkino's Play to is on its now-playing line, to tap`);
        await page.waitForFunction(() => localStorage.getItem('pm.zone') === 'heimkino' && document.querySelector('[data-zcard="heimkino"].on .zc-now > .pill.ok'), null, { timeout: 3000 }).catch(() => {});
        const after = (await cards(page)).find((c) => c.id === 'heimkino');
        check(await page.evaluate(() => localStorage.getItem('pm.zone')) === 'heimkino' && !!after && after.on && after.pickKind === 'pill' && after.pickIn === 'zc-now', `${at}: tapping Play to picks the zone and its card says "selected" in the same place`);
        check(!!after && !!target && Math.abs(after.h - target.h) <= 1, `${at}: the card keeps its height when picked (${target && Math.round(target.h)} → ${after && Math.round(after.h)} px)`);
      } finally { await context.close(); }
    }
    // Desktop: Play to and the note stay in the controls line; crossing to a phone width moves them
    {
      const { context, page } = await setup(browser, 1280, false);
      try {
        await page.setViewportSize({ width: 1280, height: 800 });
        await open(page);
        let cs = await cards(page);
        check(cs.filter((c) => !c.on).every((c) => c.pickIn === 'zc-controls') && cs.find((c) => c.on).pickIn === 'zc-controls', 'desktop-1280: Play to and "selected" stay in the controls line');
        check(cs.filter((c) => !c.slider).every((c) => c.note && c.note.shown), 'desktop-1280: "Volume: set on the device" is shown');
        await page.screenshot({ path: path.join(SHOTS, 'zones-desktop-1280.png') });
        await page.setViewportSize({ width: 360, height: 640 });
        await page.waitForFunction(() => document.documentElement.dataset.shell === 'phone' && document.querySelector('#main .zone-card .zc-now > [data-zsel]'), null, { timeout: 3000 }).catch(() => {});
        cs = await cards(page);
        check(cs.filter((c) => !c.on).every((c) => c.pickIn === 'zc-now') && cs.every((c) => c.ctlH <= 48), 'desktop → 360: the cards take the phone layout');
        await page.setViewportSize({ width: 1280, height: 800 });
        await page.waitForFunction(() => document.documentElement.dataset.shell === 'desktop' && document.querySelector('#main .zone-card .zc-controls > [data-zsel]'), null, { timeout: 3000 }).catch(() => {});
        cs = await cards(page);
        check(cs.filter((c) => !c.on).every((c) => c.pickIn === 'zc-controls') && cs.filter((c) => !c.slider).every((c) => c.note && c.note.shown), '360 → desktop: Play to and the note are back in the controls line');
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
