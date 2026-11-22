// A Zones card's volume is the bar's volume popover's: Mute, the slider, the level. Before, the card
// had only the slider and the level (no speaker, so it read like a seek bar), and a 1280 px window had
// no Mute anywhere (its bar shows a slider, not the popover). Mute sits just before the slider, names
// its zone, says whether it is pressed, and mutes that zone (a node zone by POST, this browser's own
// player by its audio) without choosing it; pressed again it unmutes. A refused mute puts the button
// back, and one muted or unmuted elsewhere (the node's zone event) follows. Enter on it works; Zones
// drawn again gives a focused Mute its focus back. A zone without a volume has none. Mute, slider and
// level stay on one line together; cards that had one line keep it (1024, ~1000, ~1150 px), except this
// browser's card at ~1000 and ~1150 px, whose volume goes under the transport there with "selected"
// (tools/ui-zone-card-volume-room.js) rather than keep a 66-70 px slider beside it. On a
// touch tablet Next and Mute share the 6 px between them, and where the volume goes under the
// transport a press just under Previous is Previous's, not Mute's (the split checked by the half
// pixel: 4 / 4.5 px on this browser's card, 5.5 / 6.5 px on a node's). Three quick presses while the zone
// answers: busy at once, one request. With another zone in use, only the card's own zone changes.
// Phones are as before: no Mute, one line.
//   node tools/ui-zone-card-mute.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'zone-card-mute-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };

const zone = (id, name, kind, volume) => ({ id, name, output: `${kind}:${id}`, transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume });
const zones = [
  zone('linn', 'Linn Klimax DS3', 'upnp', { percent: 20, muted: false }),
  zone('kitchen', 'Kitchen', 'upnp', { percent: 35, muted: true }),
  zone('frame', 'The Frame', 'upnp', { percent: 10, muted: false }),
  zone('heimkino', 'Heimkino', 'airplay', null),
];
const outputs = zones.map((z) => ({ id: z.output, name: z.name, kind: z.output.split(':')[0], caps: z.volume ? { hw_volume: 'percent' } : {} }));

const setup = async (browser, width, touch, inUse) => {
  const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, touch, zone: inUse, fixtures: { '/zones': { body: zones }, '/outputs': { body: outputs }, '/zones/linn/volume': { body: {} }, '/zones/kitchen/volume': { body: {} }, '/zones/frame/volume': { body: { error: 'The TV did not answer' }, status: 502 } } });
  const sock = {};
  await context.routeWebSocket(/\/v1\/events/, (ws) => { sock.ws = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
  await fx.go(page, '/zones');
  await page.waitForSelector('[data-zcard="frame"] [data-zvol]');
  await page.waitForTimeout(400);
  return { context, page, state, sock };
};
const card = (id) => `[data-zcard="${id}"]`;
const mute = (id) => `${card(id)} [data-volmute]`;
const press = async (page, sel, touch) => { if (touch) await page.tap(sel); else await page.click(sel); await page.waitForTimeout(300); };
const posts = (state, id) => state.hits.filter((h) => h.m === 'POST' && h.v === `/zones/${id}/volume`).map((h) => JSON.parse(h.body || '{}').volume);
const lineOf = (page, id) => page.$eval(card(id), (c) => { const kids = [...c.querySelector('.zc-controls').children].filter((e) => e.getClientRects().length); const tops = new Set(kids.map((e) => Math.round(e.getBoundingClientRect().top + e.getBoundingClientRect().height / 2))); return { lines: tops.size, spread: Math.max(...tops) - Math.min(...tops) }; });

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      const { context, page, state, sock } = await setup(browser, width, false);
      if (!await page.$(mute('linn'))) { check(false, `${width}: a card with a volume shows Mute`); await context.close(); continue; }
      const info = await page.$eval(mute('linn'), (b) => ({ label: b.getAttribute('aria-label'), pressed: b.getAttribute('aria-pressed'), title: b.title, next: b.nextElementSibling && b.nextElementSibling.dataset.zvol, icon: !!b.querySelector('svg use[href="#i-volume"]'), shown: !!b.getClientRects().length }));
      check(info.shown && info.icon, `${width}: a card with a volume shows Mute, the speaker icon`);
      check(info.label === 'Mute Linn Klimax DS3', `${width}: Mute names its zone (${info.label})`);
      check(info.next === 'linn', `${width}: Mute sits just before the card's slider, as in the volume popover`);
      check(info.pressed === 'false' && info.title === 'Mute', `${width}: a zone that isn't muted: not pressed, "Mute" (${info.pressed}, ${info.title})`);
      check(await page.$eval(mute('kitchen'), (b) => b.getAttribute('aria-pressed') === 'true' && b.title === 'Unmute'), `${width}: a muted zone's Mute is pressed and says Unmute`);
      check(await page.$(mute('heimkino')) === null, `${width}: a zone without a volume has no Mute`);
      check(await page.$(mute('browser')) !== null, `${width}: this browser's own card has Mute`);
      await page.screenshot({ path: path.join(SHOTS, `zones-${width}.png`) });

      // Mute a zone that isn't the one in use: that zone, nothing chosen
      await press(page, mute('linn'));
      check(JSON.stringify(posts(state, 'linn')) === '[{"mute":true}]', `${width}: Mute sends that zone's mute (${JSON.stringify(posts(state, 'linn'))})`);
      check(await page.$eval(mute('linn'), (b) => b.getAttribute('aria-pressed') === 'true' && b.title === 'Unmute'), `${width}: and is pressed`);
      check(/Muted/.test(await page.textContent(`${card('linn')} .vol-level`)), `${width}: the card's level says Muted`);
      check(await page.evaluate(() => localStorage.getItem('pm.zone')) === 'browser', `${width}: the zone in use stays this browser`);
      await page.screenshot({ path: path.join(SHOTS, `zones-muted-${width}.png`) });
      await press(page, mute('linn'));
      check(JSON.stringify(posts(state, 'linn')) === '[{"mute":true},{"mute":false}]', `${width}: pressed again it unmutes (${JSON.stringify(posts(state, 'linn'))})`);
      check(await page.$eval(mute('linn'), (b) => b.getAttribute('aria-pressed') === 'false'), `${width}: and is no longer pressed`);
      // A refused mute puts the button back
      await press(page, mute('frame'));
      check(posts(state, 'frame').length === 1 && await page.$eval(mute('frame'), (b) => b.getAttribute('aria-pressed') === 'false'), `${width}: a mute the zone refused leaves it unpressed`);
      // This browser's own player
      await press(page, mute('browser'));
      check(await page.evaluate(() => document.querySelector('audio') ? document.querySelector('audio').muted : null) === true && await page.$eval(mute('browser'), (b) => b.getAttribute('aria-pressed') === 'true'), `${width}: this browser's Mute mutes its player`);
      await press(page, mute('browser'));
      check(await page.evaluate(() => document.querySelector('audio').muted) === false, `${width}: and unmutes it`);
      // The node says a zone was unmuted elsewhere (its own remote, another window): the card follows
      sock.ws.send(JSON.stringify({ type: 'zone', state: { ...zones[1], volume: { percent: 35, muted: false } } }));
      check(await page.waitForFunction(() => { const b = document.querySelector('[data-zcard="kitchen"] [data-volmute]'); return b && b.getAttribute('aria-pressed') === 'false' && b.title === 'Mute'; }, null, { timeout: 3000 }).then(() => true, () => false), `${width}: a zone unmuted elsewhere: its Mute follows (not pressed, "Mute")`);
      // The keyboard: Mute takes Enter, and keeps the focus
      await page.focus(mute('kitchen')); await page.keyboard.press('Enter'); await page.waitForTimeout(300);
      check(JSON.stringify(posts(state, 'kitchen')) === '[{"mute":true}]' && await page.evaluate(() => document.activeElement && document.activeElement.matches('[data-zcard="kitchen"] [data-volmute]')), `${width}: Enter on a focused Mute mutes, focus stays on it (${JSON.stringify(posts(state, 'kitchen'))})`);
      // The page drawn again (this browser's player paused) while Mute has the focus: it has it again
      await page.focus(mute('browser'));
      await page.evaluate(() => document.querySelector('audio').dispatchEvent(new Event('pause')));
      await page.waitForTimeout(600);
      check(await page.evaluate(() => document.activeElement && document.activeElement.matches('[data-zcard="browser"] [data-volmute]')), `${width}: after Zones is drawn again, the focused Mute keeps the focus (${await page.evaluate(() => document.activeElement && (document.activeElement.getAttribute('aria-label') || document.activeElement.tagName))})`);
      if (width === 1024) for (const id of ['browser', 'linn']) { const l = await lineOf(page, id); check(l.lines === 1 || l.spread <= 4, `1024: ${id}'s controls stay on one line (${JSON.stringify(l)})`); }
      check(!state.errors.length, `${width}: no page errors (${state.errors.join('; ')})`);
      await context.close();
    }

    // A touch tablet (landscape; narrower, the volume goes under the transport): a tap in the 6 px between Next and Mute goes to the nearer one
    {
      const { context, page } = await setup(browser, 1024, true);
      const hit = await page.$eval(card('linn'), (c) => {
        const n = c.querySelector('[data-zt="next"]'); const m = c.querySelector('[data-volmute]'); if (!m) return { mute: null };
        n.scrollIntoView({ block: 'center', behavior: 'instant' });
        const a = n.getBoundingClientRect(); const b = m.getBoundingClientRect(); const y = a.top + a.height / 2;
        const at = (x) => { const h = document.elementFromPoint(x, y); return h && (n.contains(h) ? 'next' : m.contains(h) ? 'mute' : 'other'); };
        return { gap: Math.round(b.left - a.right), nearNext: at(a.right + 1), nearMute: at(b.left - 1) };
      });
      check(hit.gap === 6 && hit.nearNext === 'next' && hit.nearMute === 'mute', `1024 touch: Next and Mute share the gap between them (${JSON.stringify(hit)})`);
      await context.close();
    }

    // A touch tablet where a card's volume goes under the transport (cards ~304-320 px wide: windows
    // 744-776, 1064-1096, 1224-1272 px): Mute sits 8-10 px under Previous, and the gap is shared, as
    // the one between Next and Mute is (Mute, drawn later, took it all: a tap 4 px under Previous muted)
    for (const width of [760, 1080, 1248]) {
      const { context, page, state } = await setup(browser, width, true);
      const scan = [];
      for (const id of await page.$$eval('#main [data-zcard]', (cs) => cs.map((c) => c.dataset.zcard))) {
        const r = await page.$eval(card(id), (c) => {
          const p = c.querySelector('[data-zt="previous"]'); const m = c.querySelector('[data-volmute]'); if (!p || !m) return null;
          p.scrollIntoView({ block: 'center', behavior: 'instant' });
          const a = p.getBoundingClientRect(); const b = m.getBoundingClientRect(); if (b.top < a.bottom) return null; // one line
          const x = a.left + a.width / 2;
          const at = (y) => { const h = document.elementFromPoint(x, y); return h && (p.contains(h) ? 'previous' : m.contains(h) ? 'mute' : 'other'); };
          const gap = Math.round(b.top - a.bottom); const own = [];
          for (let d = 1; d < gap; d++) own.push(at(a.bottom + d));
          // (and every half pixel down the gap: where Previous's reach ends and Mute's starts)
          const half = []; for (let d = 0.5; d < gap; d += 0.5) half.push([d, at(a.bottom + d)]);
          return { gap, x: Math.round(x), y4: Math.round(a.bottom + 4), under4: at(a.bottom + 4), aboveMute: at(b.top - 1), own, half };
        });
        if (r) scan.push({ id, ...r });
      }
      check(scan.length >= 2, `${width} touch: cards whose volume went under the transport (${scan.map((r) => r.id).join(',') || 'none'})`);
      for (const r of scan) {
        // (what the reaches promise: Previous's first 4 px down are Previous's, the 2 px next to Mute are Mute's)
        const fair = r.own.every((o, i) => { const d = i + 1; return d <= 4 ? o === 'previous' : d >= r.gap - 2 ? o === 'mute' : true; });
        check(r.under4 === 'previous' && r.aboveMute === 'mute' && fair, `${width} touch, ${r.id}: a tap 4 px under Previous is Previous's, 1 px over Mute is Mute's, the ${r.gap} px between them shared (${r.own.join(' ')})`);
        // The split itself, measured by the half pixel: this browser's card (Mute 8 px under Previous)
        // gives 4 px to Previous and 4.5 px on to Mute; a node's card (10 px) gives up to 5.5 px to
        // Previous, 6 px to the card, 6.5 px on to Mute. Previous's reach (6 px down) or Mute's
        // (3 px up) grown or shrunk by half a pixel moves it.
        const [prevTo, muteFrom] = r.id === 'browser' ? [4, 4.5] : [5.5, 6.5];
        const split = r.half.every(([d, o]) => (d <= prevTo ? o === 'previous' : d >= muteFrom ? o === 'mute' : o === 'other'));
        check(split, `${width} touch, ${r.id}: Previous to ${prevTo} px under it, Mute from ${muteFrom} px (${r.half.map(([d, o]) => `${d}:${o}`).join(' ')})`);
      }
      // Pressed for real 4 px under a node zone's Previous: never Mute. A click at that exact point (a
      // stylus, a mouse on a touch screen) is what the reach decides: Previous. A finger's tap also
      // goes through Chromium's touch adjustment, which may give it to the card instead: not Mute
      const linn = scan.find((r) => r.id === 'linn');
      if (linn) {
        for (const how of ['tap', 'click']) {
          await page.evaluate(() => { window.__pressed = []; window.__seen = (e) => { const b = e.target.closest && e.target.closest('button'); window.__pressed.push(b ? b.dataset.zt || (b.dataset.volmute != null ? 'mute' : b.title) : e.target.tagName); }; document.addEventListener('click', window.__seen, true); });
          const at = await page.$eval(card('linn'), (c) => { const p = c.querySelector('[data-zt="previous"]'); p.scrollIntoView({ block: 'center', behavior: 'instant' }); const a = p.getBoundingClientRect(); return { x: a.left + a.width / 2, y: a.bottom + 4.5 }; });
          if (how === 'click') await page.mouse.click(at.x, at.y); else await page.touchscreen.tap(at.x, at.y);
          await page.waitForTimeout(300);
          const pressed = await page.evaluate(() => { document.removeEventListener('click', window.__seen, true); return window.__pressed; });
          check((how === 'click' ? JSON.stringify(pressed) === '["previous"]' : !pressed.includes('mute')) && !posts(state, 'linn').length && await page.$eval(mute('linn'), (b) => b.getAttribute('aria-pressed') === 'false'), `${width} touch: a ${how} 4 px under Linn's Previous ${how === 'click' ? 'presses Previous' : 'doesn\'t press Mute'} (${JSON.stringify(pressed)}, ${JSON.stringify(posts(state, 'linn'))})`);
        }
      } else check(false, `${width} touch: Linn's volume went under the transport`);
      if (width === 1080) await page.screenshot({ path: path.join(SHOTS, 'zones-1080-touch.png') });
      check(!state.errors.length, `${width} touch: no page errors (${state.errors.join('; ')})`);
      await context.close();
    }

    // Mute pressed three times quickly while the zone hasn't answered: busy at once (aria-busy, the
    // spinner), one request, and itself again once the answer is in
    {
      const { context, page, state } = await setup(browser, 1280, false);
      state.delay = (v, m) => (v === '/zones/linn/volume' && m === 'POST' ? 1500 : 0);
      for (let i = 0; i < 3; i++) await page.click(mute('linn'), { force: true });
      const now = await page.$eval(mute('linn'), (b) => ({ busy: b.getAttribute('aria-busy'), off: b.getAttribute('aria-disabled'), pressed: b.getAttribute('aria-pressed') }));
      check(now.busy === 'true' && now.off === 'true' && now.pressed === 'true', `1280: Mute pressed three times while the zone answers: busy at once, pressed (${JSON.stringify(now)})`);
      await page.waitForTimeout(400);
      check(await page.$eval(mute('linn'), (b) => b.classList.contains('is-spinning')), '1280: …and shows the spinner');
      await page.waitForTimeout(1600);
      check(JSON.stringify(posts(state, 'linn')) === '[{"mute":true}]', `1280: …one request (${JSON.stringify(posts(state, 'linn'))})`);
      const after = await page.$eval(mute('linn'), (b) => ({ busy: b.getAttribute('aria-busy'), off: b.getAttribute('aria-disabled'), spin: b.classList.contains('is-spinning') || b.classList.contains('is-busy'), pressed: b.getAttribute('aria-pressed') }));
      check(after.busy == null && after.off == null && !after.spin && after.pressed === 'true', `1280: …and after the answer it is itself again, pressed (${JSON.stringify(after)})`);
      check(!state.errors.length, `1280: no page errors (${state.errors.join('; ')})`);
      await context.close();
    }

    // Another zone in use (Kitchen): muting Linn from its card touches only Linn, Kitchen stays the zone
    {
      const { context, page, state } = await setup(browser, 1280, false, 'kitchen');
      const was = await page.$eval(mute('kitchen'), (b) => b.getAttribute('aria-pressed'));
      await press(page, mute('linn'));
      check(JSON.stringify(posts(state, 'linn')) === '[{"mute":true}]', `1280, Kitchen in use: Mute on Linn's card mutes Linn (${JSON.stringify(posts(state, 'linn'))})`);
      check(!state.hits.some((h) => h.v === '/zones/kitchen/volume'), `1280, Kitchen in use: Kitchen gets no volume request (${state.hits.filter((h) => h.v.startsWith('/zones/kitchen')).map((h) => `${h.m} ${h.v}`).join(', ') || 'none'})`);
      check(await page.$eval(mute('kitchen'), (b) => b.getAttribute('aria-pressed')) === was, '1280, Kitchen in use: Kitchen\'s Mute is unchanged');
      check(await page.evaluate(() => localStorage.getItem('pm.zone')) === 'kitchen', '1280, Kitchen in use: Kitchen stays the zone in use');
      check(/Kitchen/.test(await page.evaluate(() => (document.querySelector('#zonename') || document.querySelector('.player')).textContent)), '1280, Kitchen in use: the player bar still names Kitchen');
      check(!state.errors.length, `1280, Kitchen in use: no page errors (${state.errors.join('; ')})`);
      await context.close();
    }

    // Window widths between tablet and wide: the level stays with its slider (Mute, slider, level go
    // to a second line as one, not the level alone), and where the cards had one line they keep it
    // (no Mute: 984-1000 and 1144-1160 px lost it with the selected pill)
    {
      const { context, page } = await setup(browser, 1280, false);
      for (let w = 760; w <= 1280; w += 24) {
        await page.setViewportSize({ width: w, height: 900 }); await page.waitForTimeout(120);
        const bad = await page.$$eval('#main .zone-card', (cs) => cs.flatMap((c) => { const r = c.querySelector('[data-zvol]'); const l = c.querySelector('.vol-level'); if (!r || !l) return []; const mid = (e) => { const b = e.getBoundingClientRect(); return (b.top + b.bottom) / 2; }; return Math.abs(mid(r) - mid(l)) > 4 ? [c.dataset.zcard] : []; }));
        check(!bad.length, `${w}: each card's level is on its slider's line (${bad.join(',') || 'all'})`);
      }
      for (const w of [992, 1000, 1152, 1160]) {
        await page.setViewportSize({ width: w, height: 900 }); await page.waitForTimeout(150);
        { const l = await lineOf(page, 'linn'); check(l.lines === 1 || l.spread <= 4, `${w}: linn's controls stay on one line (${JSON.stringify(l)})`); }
        // (this browser's card, whose "selected" and dB level are wider: one line, or its volume and
        // "selected" together under the transport, not a slider under 80 px beside it)
        const l = await lineOf(page, 'browser'); const sw = await page.$eval(`${card('browser')} [data-zvol]`, (r) => r.getBoundingClientRect().width);
        check(l.lines === 1 || l.spread <= 4 ? sw >= 80 : l.lines === 2 && sw >= 80, `${w}: browser's controls stay on one line, or its volume goes under the transport with room (${JSON.stringify(l)}, slider ${Math.round(sw)} px)`);
      }
      await context.close();
    }

    // Phones: as before, no Mute, controls on one line
    for (const width of [360, 412]) {
      const { context, page } = await setup(browser, width, true);
      check(await page.$$eval('#main [data-volmute]', (bs) => bs.filter((b) => b.getClientRects().length).length) === 0, `${width}: a phone's cards show no Mute`);
      for (const id of ['browser', 'linn']) { const l = await lineOf(page, id); check(l.lines === 1 || l.spread <= 8, `${width}: ${id}'s controls stay on one line (${JSON.stringify(l)})`); }
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${width}: no sideways scroll`);
      await page.screenshot({ path: path.join(SHOTS, `zones-${width}.png`) });
      await context.close();
    }
  } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
