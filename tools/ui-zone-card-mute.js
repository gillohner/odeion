// A Zones card's volume is the bar's volume popover's: Mute, the slider, the level. Before, the card
// had only the slider and the level (no speaker, so it read like a seek bar), and a 1280 px window had
// no Mute anywhere (its bar shows a slider, not the popover). Mute sits just before the slider, names
// its zone, says whether it is pressed, and mutes that zone (a node zone by POST, this browser's own
// player by its audio) without choosing it; pressed again it unmutes. A refused mute puts the button
// back, and one muted or unmuted elsewhere (the node's zone event) follows. Enter on it works; Zones
// drawn again gives a focused Mute its focus back. A zone without a volume has none. Mute, slider and
// level stay on one line together; cards that had one line keep it (1024, ~1000, ~1150 px). On a
// touch tablet Next and Mute share the 6 px between them. Phones are as before: no Mute, one line.
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

const setup = async (browser, width, touch) => {
  const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, touch, fixtures: { '/zones': { body: zones }, '/outputs': { body: outputs }, '/zones/linn/volume': { body: {} }, '/zones/kitchen/volume': { body: {} }, '/zones/frame/volume': { body: { error: 'The TV did not answer' }, status: 502 } } });
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
        for (const id of ['browser', 'linn']) { const l = await lineOf(page, id); check(l.lines === 1 || l.spread <= 4, `${w}: ${id}'s controls stay on one line (${JSON.stringify(l)})`); }
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
