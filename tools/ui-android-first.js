// The Android app on a phone (TASK-ui-motion-polish-1106 section 4, part a; the owner's screen
// recording, 2026-11-06): every screen panned sideways and clipped its left edge, Play to opened
// as a narrow panel beside the page, and the status chip said "API token not answering" while
// nothing was down. Real candidate assets on the fixture origin (tools/ui-fixture.js) with a
// phone's viewport rules (a page wider than the screen is zoomed out, as the WebView does), the
// app's bridge (window.pmDesktop, Android), the owner's six zones and the API token warning, at
// 360x800, 390x844 and 412x915:
//   * on every main page and every open panel the page is exactly the screen's width: not zoomed
//     out, no horizontal scroll, and no element past either edge (outside a row that scrolls
//     sideways by design);
//   * Play to, Sleep timer and the status panel are bottom sheets the width of the screen; Back
//     closes the top one and the page stays; a swipe down closes Play to, a short pull doesn't;
//   * the chip says what is wrong ("Change the API token") and its panel is "External resources";
//   * the last zone card scrolls fully above the mini player.
//   node tools/ui-android-first.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'android-first-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
// The owner's zones, as Zones and Play to listed them in the recording
const NAMES = [['heimkino', 'Heimkino (AirPlay)', 'airplay'], ['linn-air', 'Linn Klimax DS3 (AirPlay)', 'airplay'], ['linn-upnp', 'Linn Klimax DS3:UPnP AV', 'upnp'], ['mbp', 'MacBook Pro ELo (AirPlay)', 'airplay'], ['tv', '[TV] The Frame 65 Lohner', 'upnp']];
const outputs = NAMES.map(([id, name, kind]) => ({ id: `${kind}:${id}`, name, kind, caps: kind === 'upnp' ? { hw_volume: 'percent' } : {} }));
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zones = NAMES.map(([id, name, kind], i) => ({ id, name, output: `${kind}:${id}`, transport: i === 2 ? 'playing' : 'stopped', queue: i === 2 ? [{ id: 1, uri: 'track:1', title: 'Come On' }] : [], queue_index: i === 2 ? 0 : null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: kind === 'upnp' ? { percent: 20, muted: false } : null, now_playing: i === 2 ? np : null }));
// The node's health as the owner's showed it: the homeserver and Pinakes answer, the API token needs a change
const health = [{ id: 'homeserver', name: 'Your homeserver', state: 'ok', ms: 134 }, { id: 'pinakes', name: 'Pinakes (Home, links, tags)', state: 'ok', ms: 57 }, { id: 'api-token', name: 'API token', state: 'change', detail: 'An older version kept this token in your homeserver\'s node record: change [api] token (or PM_TOKEN) in the node\'s config and restart it', ms: null }];
const fixtures = { '/zones': { body: zones }, '/outputs': { body: outputs }, '/health': { body: health } };

// Wider than the screen: the page itself, or an element past an edge that isn't inside a row
// made to scroll sideways (the Library chips) or a clipped box
const measure = (page) => page.evaluate(() => {
  const vw = document.documentElement.clientWidth; const out = [];
  const name = (e) => (e.id ? '#' + e.id : e.tagName.toLowerCase() + (typeof e.className === 'string' && e.className.trim() ? '.' + e.className.trim().split(/\s+/).join('.') : ''));
  const clippedBy = (e) => { for (let a = e.parentElement; a && a !== document.body; a = a.parentElement) { const ox = getComputedStyle(a).overflowX; if (ox !== 'visible') { const b = a.getBoundingClientRect(); if (b.left >= -0.5 && b.right <= vw + 0.5) return true; } } return false; };
  for (const e of document.querySelectorAll('body *')) {
    if (e.closest('svg defs, svg symbol') || !e.getClientRects().length) continue;
    const b = e.getBoundingClientRect(); if (b.width === 0 || b.height === 0) continue;
    if ((b.right > vw + 0.5 || b.left < -0.5) && !clippedBy(e)) {
      const p = e.parentElement && e.parentElement.getBoundingClientRect();
      if (p && (p.right > vw + 0.5 || p.left < -0.5) && !clippedBy(e.parentElement)) continue; // only the outermost
      out.push(`${name(e)} ${Math.round(b.left)}..${Math.round(b.right)}`);
    }
  }
  return { vw, iw: innerWidth, sw: document.documentElement.scrollWidth, out };
});
const fits = async (page, at) => {
  await sleep(350);
  const m = await measure(page);
  check(m.iw === m.vw && m.sw <= m.vw && !m.out.length, `${at}: the page is the screen's width (window ${m.iw}, page ${m.sw}, screen ${m.vw}${m.out.length ? `; past the edge: ${m.out.slice(0, 4).join(', ')}` : ''})`);
};
// A bottom sheet the width of the screen
const sheet = async (page, sel, at) => {
  const r = await page.$eval(sel, (el) => { const b = el.getBoundingClientRect(); return { l: Math.round(b.left), r: Math.round(b.right), b: Math.round(b.bottom), vw: document.documentElement.clientWidth, vh: innerHeight }; });
  check(r.l === 0 && r.r === r.vw && Math.abs(r.b - r.vh) <= 1, `${at}: a bottom sheet the width of the screen (${r.l}..${r.r} of ${r.vw}, bottom ${r.b} of ${r.vh})`);
};
// A one-finger pull down on an element, as touch events
const pull = (page, sel, dy) => page.$eval(sel, (el, dy) => {
  const b = el.getBoundingClientRect(); const x = b.left + b.width / 2; const y = b.top + 12;
  const t = (yy) => new Touch({ identifier: 1, target: el, clientX: x, clientY: yy });
  el.dispatchEvent(new TouchEvent('touchstart', { touches: [t(y)], changedTouches: [t(y)], bubbles: true }));
  for (let i = 1; i <= 6; i++) el.dispatchEvent(new TouchEvent('touchmove', { touches: [t(y + (dy * i) / 6)], changedTouches: [t(y + (dy * i) / 6)], bubbles: true }));
  el.dispatchEvent(new TouchEvent('touchend', { touches: [], changedTouches: [t(y + dy)], bubbles: true }));
}, dy);

(async () => {
  const browser = await chromium.launch();
  for (const [w, h] of [[360, 800], [390, 844], [412, 915]]) {
    const { page, context, state } = await fx.create(browser, w, { mobile: true, fixtures });
    await page.setViewportSize({ width: w, height: h });
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
    await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn-upnp'); window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
    for (const p of ['/', '/search', '/library', '/albums', '/zones', '/settings']) {
      await fx.go(page, p); await page.waitForSelector('#main'); await page.waitForSelector('#upstreams:not([hidden])', { timeout: 5000 }).catch(() => {});
      await fits(page, `${w} ${p}`);
      if (p === '/') await page.screenshot({ path: path.join(SHOTS, `home-${w}.png`) });
    }
    // The chip and its panel
    await fx.go(page, '/'); await page.waitForSelector('#upstreams:not([hidden])');
    const chip = (await page.textContent('#upstreams .up-text')).trim();
    check(chip === 'Change the API token', `${w}: the chip says what is wrong (${chip})`);
    await page.click('#upstreams'); await page.waitForSelector('.popover.menu:not([hidden]) .pop-title');
    const title = (await page.textContent('.popover.menu:not([hidden]) .pop-title')).trim();
    check(title === 'External resources', `${w}: the panel is "External resources" (${title})`);
    await fits(page, `${w} status panel`); await sheet(page, '.popover.menu:not([hidden])', `${w} status panel`);
    await page.goBack(); await sleep(400);
    check(await page.$eval('.popover.menu', (m) => m.hidden) && new URL(page.url()).pathname === '/', `${w}: Back closes the status panel and the page stays`);
    // Play to, then Sleep timer from it
    await page.click('#zonebtn'); await page.waitForSelector('#zonepop:not([hidden])');
    await fits(page, `${w} Play to`); await sheet(page, '#zonepop', `${w} Play to`);
    await page.screenshot({ path: path.join(SHOTS, `playto-${w}.png`) });
    await pull(page, '#zonepop', 40); await sleep(300);
    check(!(await page.$eval('#zonepop', (e) => e.hidden)), `${w}: a short pull leaves Play to open`);
    await pull(page, '#zonepop', 260); await sleep(300);
    check(await page.$eval('#zonepop', (e) => e.hidden) && await page.$eval('#scrim', (e) => e.hidden), `${w}: a swipe down closes Play to and its scrim`);
    if (!(await page.$eval('#zonepop', (e) => e.hidden))) await page.keyboard.press('Escape'); // (so the rest still runs where the swipe failed)
    await page.click('#zonebtn'); await page.waitForSelector('#zonepop:not([hidden]) #zpsleep');
    await page.click('#zpsleep'); await page.waitForSelector('.popover.menu:not([hidden]) .pop-title');
    await fits(page, `${w} Sleep timer`); await sheet(page, '.popover.menu:not([hidden])', `${w} Sleep timer`);
    await page.goBack(); await sleep(400);
    check(await page.$eval('.popover.menu', (m) => m.hidden) && await page.$eval('#zonepop', (e) => e.hidden) && new URL(page.url()).pathname === '/', `${w}: Back closes Sleep timer and the page stays`);
    // Now playing
    await page.click('#np'); await page.waitForSelector('#nowplaying:not([hidden])', { timeout: 5000 }).catch(() => {});
    await fits(page, `${w} Now playing`);
    await page.screenshot({ path: path.join(SHOTS, `nowplaying-${w}.png`) });
    await page.goBack(); await sleep(400);
    // The last zone card above the mini player
    await fx.go(page, '/zones'); await page.waitForSelector('.zone-card');
    await page.$eval('#main', (m) => { m.style.scrollBehavior = 'auto'; m.scrollTop = m.scrollHeight; }); await sleep(300);
    const z = await page.evaluate(() => { const cards = [...document.querySelectorAll('.zone-card')]; const last = cards[cards.length - 1].getBoundingClientRect(); const bar = document.querySelector('.player').getBoundingClientRect(); return { last: Math.round(last.bottom), bar: Math.round(bar.top), n: cards.length }; });
    check(z.last <= z.bar, `${w}: the last of ${z.n} zone cards ends above the mini player (${z.last} <= ${z.bar})`);
    check(!state.errors.length, `${w}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
