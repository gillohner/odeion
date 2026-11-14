// On a phone a zone card's "Play to" (or its "selected" mark) sits on the card's now-playing line,
// and a zone without a volume says "Volume: set on the device" to screen readers only, so the
// card's controls are one line with a full slider and an unselected card is ~181 px instead of 227.
// Each "Play to" names its zone. Desktop keeps Play to and the note in the controls line, and the
// cards move between the two when the window crosses the phone width. Opening Zones on a phone
// scrolls the zone in use up above the mini player; redraws, Play to and Back keep the place.
// On a phone's touch screen a slider (a card's volume, Now playing's seek and volume) is a 44 px
// tall touch with nothing moved, and a swipe that starts on it scrolls without changing the volume.
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

const setup = async (browser, width, android, zone = 'linn-upnp') => {
  const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/zones': { body: zones }, '/outputs': { body: outputs }, '/zones/linn-upnp/volume': { body: {} }, '/zones/tv/volume': { body: {} } } });
  await context.addInitScript((z) => localStorage.setItem('pm.zone', z), zone);
  const sock = {};
  await context.routeWebSocket(/\/v1\/events/, (ws) => { sock.ws = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  return { context, page, sock, state };
};
// Where the zone in use is against the page's visible part (above the mini player), and the scroll
const inUse = (page) => page.evaluate(() => {
  const main = document.querySelector('#main'); const c = main.querySelector('.zone-card.on');
  const m = main.getBoundingClientRect(); const r = c.getBoundingClientRect();
  const bar = [...document.querySelectorAll('#player, .bottomnav')].filter((e) => e.getClientRects().length).map((e) => e.getBoundingClientRect().top);
  const fold = Math.min(m.bottom, ...bar);
  return { id: c.dataset.zcard, top: Math.round(r.top), bottom: Math.round(r.bottom), headBottom: Math.round(m.top), fold: Math.round(fold), scroll: Math.round(main.scrollTop), shown: r.top >= m.top - 1 && r.bottom <= fold + 1 };
});
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
        // Opened: the zone in use is all on screen, above the mini player (it was under it at 360)
        const u = await inUse(page);
        check(u.id === 'linn-upnp' && u.shown, `${at}: on opening, the zone in use is on screen above the mini player (${u.top}-${u.bottom}, fold ${u.fold}, scrolled ${u.scroll})`);
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
        const heimPlay = page.locator('[data-zcard="heimkino"] .zc-now [data-zsel="heimkino"]');
        await heimPlay.scrollIntoViewIfNeeded({ timeout: 2000 }).catch(() => {});
        const tapAt = await page.evaluate(() => document.querySelector('#main').scrollTop);
        const tapped = await heimPlay.tap({ timeout: 2000 }).then(() => true, () => false);
        check(tapped, `${at}: Heimkino's Play to is on its now-playing line, to tap`);
        await page.waitForFunction(() => localStorage.getItem('pm.zone') === 'heimkino' && document.querySelector('[data-zcard="heimkino"].on .zc-now > .pill.ok'), null, { timeout: 3000 }).catch(() => {});
        await page.waitForTimeout(200);
        const tapEnd = await page.evaluate(() => document.querySelector('#main').scrollTop);
        check(Math.abs(tapEnd - tapAt) <= 1, `${at}: Play to keeps the page where it is (scrolled ${Math.round(tapAt)} → ${Math.round(tapEnd)})`);
        const after = (await cards(page)).find((c) => c.id === 'heimkino');
        check(await page.evaluate(() => localStorage.getItem('pm.zone')) === 'heimkino' && !!after && after.on && after.pickKind === 'pill' && after.pickIn === 'zc-now', `${at}: tapping Play to picks the zone and its card says "selected" in the same place`);
        check(!!after && !!target && Math.abs(after.h - target.h) <= 1, `${at}: the card keeps its height when picked (${target && Math.round(target.h)} → ${after && Math.round(after.h)} px)`);
      } finally { await context.close(); }
    }
    // The last zone in use: opening Zones brings it up above the mini player; once there, a zone's
    // news (a redraw) or the top they scrolled back to stays put; Back to Zones keeps their place
    for (const width of [360, 412]) for (const android of [false, true]) {
      const at = `${android ? 'android' : 'browser'}-${width} (TV in use)`;
      const { context, page, sock } = await setup(browser, width, android, 'tv');
      try {
        await page.setViewportSize({ width, height: width === 360 ? 640 : 732 });
        await open(page);
        const u = await inUse(page);
        check(u.id === 'tv' && u.shown && u.scroll > 0, `${at}: on opening, the zone in use is on screen above the mini player (${u.top}-${u.bottom}, fold ${u.fold}, scrolled ${u.scroll})`);
        check(u.bottom >= u.fold - 40, `${at}: the page moves only as far as it needs (card bottom ${u.bottom}, fold ${u.fold})`);
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && document.documentElement.scrollHeight <= innerHeight + 1), `${at}: no sideways overflow, the page itself doesn't scroll`);
        await page.screenshot({ path: path.join(SHOTS, `zones-tv-${at.split(' ')[0]}.png`) });
        // A zone's news redraws the page: it stays where it is
        const tv = zones.find((z) => z.id === 'tv');
        sock.ws.send(JSON.stringify({ type: 'zone', seq: 1, state: { ...tv, transport: 'paused' } }));
        await page.waitForFunction(() => /paused/.test((document.querySelector('[data-zcard="tv"] .zc-head') || {}).textContent || ''), null, { timeout: 3000 }).catch(() => {});
        const v = await inUse(page);
        check(v.scroll === u.scroll && /paused/.test(await page.locator('[data-zcard="tv"] .zc-head').textContent()), `${at}: a redraw keeps the page where it is (scrolled ${u.scroll} → ${v.scroll})`);
        // Scrolled back to the top, the next redraw leaves it there
        await page.evaluate(() => { const m = document.querySelector('#main'); m.style.scrollBehavior = 'auto'; m.scrollTop = 0; m.style.scrollBehavior = ''; });
        sock.ws.send(JSON.stringify({ type: 'zone', seq: 2, state: { ...tv, transport: 'stopped' } }));
        await page.waitForFunction(() => /stopped/.test((document.querySelector('[data-zcard="tv"] .zc-head') || {}).textContent || ''), null, { timeout: 3000 }).catch(() => {});
        await page.waitForTimeout(150);
        check(await page.evaluate(() => document.querySelector('#main').scrollTop) === 0, `${at}: back at the top, a redraw doesn't scroll the page down again`);
        // Another page and Back: the place they left Zones at, not the zone in use
        await page.evaluate(() => { const m = document.querySelector('#main'); m.style.scrollBehavior = 'auto'; m.scrollTop = 120; m.style.scrollBehavior = ''; });
        check(await page.locator('#bottomnav [data-bnav="home"]').tap({ timeout: 2000 }).then(() => true, () => false), `${at}: Home in the bottom bar, to tap`);
        await page.waitForFunction(() => !document.querySelector('#main .zone-card'), null, { timeout: 3000 }).catch(() => {});
        await page.goBack();
        await page.waitForSelector('#main .zone-card.on', { timeout: 3000 }).catch(() => {});
        await page.waitForFunction(() => Math.round(document.querySelector('#main').scrollTop) === 120, null, { timeout: 3000 }).catch(() => {});
        await page.waitForTimeout(200); // and it stays there
        check(await page.evaluate(() => Math.round(document.querySelector('#main').scrollTop)) === 120, `${at}: Back to Zones is where they left it (${await page.evaluate(() => Math.round(document.querySelector('#main').scrollTop))})`);
        // Left at its top, Back to Zones is at its top too (not scrolled down to the zone in use)
        const toTop = () => page.evaluate(() => { const m = document.querySelector('#main'); m.style.scrollBehavior = 'auto'; m.scrollTop = 0; m.style.scrollBehavior = ''; });
        await toTop();
        await page.locator('#bottomnav [data-bnav="home"]').tap({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => !document.querySelector('#main .zone-card'), null, { timeout: 3000 }).catch(() => {});
        await page.goBack();
        await page.waitForSelector('#main .zone-card.on', { timeout: 3000 }).catch(() => {});
        await page.waitForTimeout(400);
        check(await page.evaluate(() => Math.round(document.querySelector('#main').scrollTop)) === 0, `${at}: Back to Zones left at its top is at its top (${await page.evaluate(() => Math.round(document.querySelector('#main').scrollTop))})`);
        // The everyday way in: Zones in the bottom bar from a page scrolled down (its smooth way
        // back to the top is still under way when Zones is drawn)
        await page.locator('#bottomnav [data-bnav="home"]').tap({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => !document.querySelector('#main .zone-card'), null, { timeout: 3000 }).catch(() => {});
        await page.waitForTimeout(400);
        const homeAt = await page.evaluate(() => { const m = document.querySelector('#main'); m.style.scrollBehavior = 'auto'; m.scrollTop = m.scrollHeight; m.style.scrollBehavior = ''; return Math.round(m.scrollTop); });
        await page.locator('#bottomnav [data-bnav="zones"]').tap({ timeout: 2000 }).catch(() => {});
        await page.waitForSelector('#main .zone-card.on', { timeout: 3000 }).catch(() => {});
        await page.waitForTimeout(600);
        const w = await inUse(page);
        check(homeAt > 0 && w.id === 'tv' && w.shown, `${at}: Zones in the bottom bar from Home scrolled ${homeAt} px shows the zone in use above the mini player (${w.top}-${w.bottom}, fold ${w.fold})`);
        // Zones in the bottom bar again, at its top: the same
        await toTop();
        await page.locator('#bottomnav [data-bnav="zones"]').tap({ timeout: 2000 }).catch(() => {});
        await page.waitForTimeout(600);
        const x = await inUse(page);
        check(x.shown, `${at}: Zones in the bottom bar while on Zones shows the zone in use (${x.top}-${x.bottom}, fold ${x.fold})`);
      } finally { await context.close(); }
    }
    // On a phone's touch screen a volume slider is a 44 px tall touch, not an 18 px strip, and the card is
    // drawn as before; a swipe up or down that starts on it scrolls the page and leaves the volume
    // as it was, while a tap or a drag sideways still sets it
    for (const width of [360, 412]) for (const android of [false, true]) {
      const at = `${android ? 'android' : 'browser'}-${width} (touch)`;
      const { context, page, state } = await setup(browser, width, android);
      try {
        await page.setViewportSize({ width, height: width === 360 ? 640 : 732 });
        await open(page);
        const cdp = await context.newCDPSession(page);
        const touch = async (x0, y0, x1, y1) => {
          await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: x0, y: y0 }] });
          for (let i = 1; i <= 10; i++) { await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: x0 + ((x1 - x0) * i) / 10, y: y0 + ((y1 - y0) * i) / 10 }] }); await page.waitForTimeout(16); }
          await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
          await page.waitForTimeout(400);
        };
        const sent = () => state.hits.filter((h) => h.m === 'POST' && h.v === '/zones/linn-upnp/volume').length;
        const slider = () => page.evaluate(() => {
          const e = document.querySelector('[data-zcard="linn-upnp"] input[type=range]'); e.scrollIntoView({ block: 'center', behavior: 'instant' });
          const b = e.getBoundingClientRect(); const cx = b.left + b.width / 2, cy = b.top + b.height / 2;
          const on = (x, y) => { const h = document.elementFromPoint(x, y); return !!h && (h === e || e.contains(h)); };
          let up = 0; while (up < 40 && on(cx, cy - up - 1)) up++; let dn = 0; while (dn < 40 && on(cx, cy + dn + 1)) dn++;
          const c = e.closest('.zone-card');
          return { x: b.left, w: b.width, cy, tall: up + dn + 1, value: e.value, card: c.getBoundingClientRect().height, ctl: c.querySelector('.zc-controls').getBoundingClientRect().height, coarse: matchMedia('(pointer: coarse)').matches };
        });
        const s0 = await slider();
        check(s0.coarse && s0.tall >= 44, `${at}: the volume slider takes a touch 44 px tall (${s0.tall} px)`);
        // The same card with an 18 px slider (as before): the same heights
        const was = await page.evaluate(() => {
          const st = document.createElement('style'); st.textContent = 'input[type=range] { height: 18px !important; margin: 0 !important; vertical-align: baseline !important; }'; document.head.append(st);
          const c = document.querySelector('[data-zcard="linn-upnp"]'); const r = { card: c.getBoundingClientRect().height, ctl: c.querySelector('.zc-controls').getBoundingClientRect().height, tall: c.querySelector('input[type=range]').getBoundingClientRect().height };
          st.remove(); return r;
        });
        check(was.tall === 18 && Math.abs(s0.card - was.card) < 0.5 && Math.abs(s0.ctl - was.ctl) < 0.5, `${at}: the card is drawn as before (${Math.round(was.card)} → ${Math.round(s0.card)} px, controls ${Math.round(was.ctl)} → ${Math.round(s0.ctl)} px)`);
        // A swipe up that starts on the slider, far from its knob: the page scrolls, no volume is sent
        const top0 = await page.evaluate(() => document.querySelector('#main').scrollTop);
        await touch(s0.x + s0.w * 0.9, s0.cy, s0.x + s0.w * 0.9, s0.cy - 200);
        const top1 = await page.evaluate(() => document.querySelector('#main').scrollTop);
        const v1 = await page.evaluate(() => document.querySelector('[data-zcard="linn-upnp"] input[type=range]').value);
        check(top1 > top0 + 50, `${at}: a swipe up from the slider scrolls the page (${Math.round(top0)} → ${Math.round(top1)})`);
        check(sent() === 0 && v1 === s0.value, `${at}: …and leaves the volume as it was (${sent()} sent, slider ${s0.value} → ${v1})`);
        // A tap on the slider still sets the volume, and so does a drag sideways
        const s1 = await slider();
        await touch(s1.x + s1.w * 0.7, s1.cy, s1.x + s1.w * 0.7, s1.cy);
        check(sent() === 1, `${at}: a tap on the slider sets the volume (${sent()} sent)`);
        const s2 = await slider();
        await touch(s2.x + s2.w * 0.2, s2.cy, s2.x + s2.w * 0.5, s2.cy);
        check(sent() === 2, `${at}: a drag sideways sets the volume (${sent()} sent)`);
        await page.screenshot({ path: path.join(SHOTS, `zones-touch-${android ? 'android' : 'browser'}-${width}.png`) });
        // Now playing's seek and volume take the same tall touch (the buttons just above the volume
        // keep their own reach, a few px into it)
        await page.click('#np'); await page.waitForSelector('#nowplaying:not([hidden]) .np-transport'); await page.waitForTimeout(400);
        const np = await page.evaluate(() => Object.fromEntries(['#npseek', '#npvol'].map((sel) => {
          const e = document.querySelector(sel); const b = e.getBoundingClientRect(); const cx = b.left + b.width / 2, cy = b.top + b.height / 2;
          const on = (x, y) => { const h = document.elementFromPoint(x, y); return !!h && (h === e || e.contains(h)); };
          let up = 0; while (up < 40 && on(cx, cy - up - 1)) up++; let dn = 0; while (dn < 40 && on(cx, cy + dn + 1)) dn++;
          return [sel, up + dn + 1];
        })));
        const npAt = () => page.evaluate(() => ['#npseek', '.np-times', '.np-transport', '.np-vol'].map((sel) => { const b = document.querySelector(sel).getBoundingClientRect(); return Math.round((b.top + b.height / 2) * 10) / 10; }).join(' '));
        const npNow = await npAt();
        await page.evaluate(() => { const st = document.createElement('style'); st.id = 'was18'; st.textContent = 'input[type=range] { height: 18px !important; margin: 0 !important; vertical-align: baseline !important; }'; document.head.append(st); });
        const npWas = await npAt(); await page.evaluate(() => document.querySelector('#was18').remove());
        check(npNow === npWas, `${at}: Now playing's seek, times, buttons and volume stay where they were (${npWas} → ${npNow})`);
        check(np['#npseek'] >= 44 && np['#npvol'] >= 40, `${at}: Now playing's seek and volume take a tall touch (${np['#npseek']} / ${np['#npvol']} px)`);
      } finally { await context.close(); }
    }
    // Desktop: Play to and the note stay in the controls line; crossing to a phone width moves them
    {
      const { context, page } = await setup(browser, 1280, false);
      try {
        await page.setViewportSize({ width: 1280, height: 800 });
        await open(page);
        check(await page.evaluate(() => document.querySelector('#main').scrollTop) === 0, 'desktop-1280: Zones opens at its top');
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
