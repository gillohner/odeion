// A Zones card's level text ("40 %", "-60 dB", "Muted", "Level unknown") has one width on desktop and tablet
// widths (600-1600 px, every 8 px): every card's level box is as wide as the others, so a level
// going known/unknown or muted never moves the card's volume between beside and under the
// transport (and never moves its Mute or slider). The words are read whole: none is cut, none is
// wider than its box, and the box is no taller than the controls' line. Before, "Level unknown" made
// its box ~24 px wider than "40 %", and at some widths that one card's volume went under the
// transport while the others stayed beside it. A level that goes unknown while the page is open (a
// zone event) leaves the card's volume where it was. Phones are not changed (tools/ui-phone-zone-cards.js).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-zone-card-level-width.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'zone-card-level-width-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let ok = 0; let failed = 0;
const check = (cond, msg) => { if (cond) ok++; else { failed++; console.log(`FAIL ${msg}`); } };

const zone = (id, name, volume) => ({ id, name, output: `upnp:${id}`, transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume, now_playing: null });
// Four level words: a known level, no level yet (the output has a volume, the zone has not said
// it), muted, and the lowest level of a dB output
const ZONES = [zone('kitchen', 'Kitchen', { percent: 40 }), zone('study', 'Study', { percent: null }), zone('den', 'Den', { percent: 40, muted: true }), zone('cellar', 'Cellar', { db: -60 })];
const IDS = ZONES.map((z) => z.id);
const WORDS = { kitchen: /^40 %$/, study: /^Level unknown$/, den: /^Muted$/, cellar: /^-60 dB$/ };
const outputs = ZONES.map((z) => ({ id: z.output, name: z.name, kind: 'upnp', caps: { hw_volume: z.id === 'cellar' ? { kind: 'db', min: -60, max: 0, step: 1 } : 'percent' } }));
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

const measure = (page) => page.evaluate((ids) => [...document.querySelectorAll('#main .zone-card')].filter((c) => ids.includes(c.dataset.zcard)).map((c) => {
  const mid = (el) => { const r = el.getBoundingClientRect(); return r.top + r.height / 2; };
  const lvl = c.querySelector('.vol-level'); const slider = c.querySelector('[data-zvol]'); const mute = c.querySelector('.zc-mute');
  const lr = lvl.getBoundingClientRect(); const row = c.querySelector('[data-zt="toggle"]').getBoundingClientRect();
  return {
    id: c.dataset.zcard, text: lvl.textContent, width: Math.round(lr.width * 10) / 10, height: lr.height, rowHeight: row.height,
    whole: lvl.scrollWidth <= lvl.clientWidth && lvl.scrollHeight <= lvl.clientHeight + 1,
    beside: Math.abs(mid(slider) - mid(c.querySelector('[data-zt="toggle"]'))) < 4,
    slider: Math.round(slider.getBoundingClientRect().width), muteLeft: Math.round(mute.getBoundingClientRect().left - c.getBoundingClientRect().left),
    cardWidth: Math.round(c.getBoundingClientRect().width),
  };
}), IDS);

(async () => {
  const browser = await chromium.launch();
  const { context, page, state } = await fx.create(browser, 1280, { fixtures: { '/zones': { body: ZONES }, '/outputs': { body: outputs } } });
  try {
    let sock = null;
    await context.routeWebSocket(/\/v1\/events/, (ws) => { sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: ZONES })); });
    await fx.go(page, '/zones');
    await page.waitForFunction((ids) => ids.every((id) => document.querySelector(`#main .zone-card[data-zcard="${id}"] [data-zvol]`)), IDS, { timeout: 8000 });
    await page.waitForFunction(() => /Level unknown/.test(document.querySelector('#main .zone-card[data-zcard="study"] .vol-level')?.textContent || ''), null, { timeout: 8000 });
    let compared = 0;
    for (let w = 600; w <= 1600; w += 8) {
      await page.setViewportSize({ width: w, height: 900 });
      await frames(page);
      if (await page.evaluate(() => document.documentElement.dataset.shell === 'phone')) continue;
      const cs = await measure(page);
      check(cs.length === 4, `${w}: four cards with a volume (${cs.length})`);
      for (const c of cs) {
        const at = `${w} ${c.id}`;
        check(WORDS[c.id].test(c.text), `${at}: the level reads ${WORDS[c.id]} (${JSON.stringify(c.text)})`);
        check(c.whole, `${at}: the level's words are whole, not cut or wider than their box (${JSON.stringify(c)})`);
        check(c.height <= c.rowHeight, `${at}: the level is no taller than the controls' line (${c.height} > ${c.rowHeight})`);
      }
      // (cards of one width only: a grid's last card can be wider)
      const k = cs.find((c) => c.id === 'kitchen');
      for (const c of cs.filter((x) => x !== k && x.cardWidth === k.cardWidth)) {
        compared++;
        const at = `${w} ${c.id} (${JSON.stringify(c.text)}) vs Kitchen ("40 %")`;
        check(c.width === k.width, `${at}: the level's box is as wide (${c.width} vs ${k.width})`);
        check(c.beside === k.beside, `${at}: the volume is ${k.beside ? 'beside' : 'under'} the transport as Kitchen's is`);
        check(c.slider === k.slider && c.muteLeft === k.muteLeft, `${at}: Mute and the slider are where Kitchen's are (slider ${c.slider} vs ${k.slider}, Mute at ${c.muteLeft} vs ${k.muteLeft})`);
      }
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${w}: no sideways scroll`);
      if ([768, 1024, 1280].includes(w)) await page.screenshot({ path: path.join(SHOTS, `zones-level-${w}.png`) });
    }
    // The node says Kitchen's level is no longer known (a zone event): at a width where its volume is
    // beside the transport and the card has room for no more, it stays there, Mute and slider unmoved
    for (const w of [1000, 1144]) {
      await page.setViewportSize({ width: w, height: 900 }); await frames(page);
      const was = (await measure(page)).find((c) => c.id === 'kitchen');
      sock.send(JSON.stringify({ type: 'zone', state: { ...ZONES[0], volume: { percent: null } } }));
      const gone = await page.waitForFunction(() => document.querySelector('#main .zone-card[data-zcard="kitchen"] .vol-level')?.textContent === 'Level unknown', null, { timeout: 3000 }).then(() => true, () => false);
      check(gone, `${w}: Kitchen's level reads "Level unknown" after the zone event`);
      await frames(page);
      const now = (await measure(page)).find((c) => c.id === 'kitchen');
      check(was.beside, `${w}: Kitchen's volume was beside the transport before (the case being checked)`);
      check(now.beside === was.beside && now.slider === was.slider && now.muteLeft === was.muteLeft && now.width === was.width, `${w}: Kitchen's volume did not move when its level went unknown (${JSON.stringify(was)} -> ${JSON.stringify(now)})`);
      sock.send(JSON.stringify({ type: 'zone', state: ZONES[0] }));
      await page.waitForFunction(() => document.querySelector('#main .zone-card[data-zcard="kitchen"] .vol-level')?.textContent === '40 %', null, { timeout: 3000 }).catch(() => {});
    }
    check(compared > 50, `cards of the same width were compared at many widths (${compared})`);
    check(state.errors.length === 0, `no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); await browser.close(); }
  console.log(`${ok} ok, ${failed} failed`);
  if (failed) process.exit(1);
})().catch((e) => { console.error(e); process.exit(1); });
