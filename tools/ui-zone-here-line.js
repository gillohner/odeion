// The page's own player's line under "Nothing playing" on the Zones page fits a phone. Real
// candidate assets on the fixture origin (tools/ui-fixture.js), touch, at 360 and 412 px, a node
// zone present, three ways: the Android app (window.pmDesktop.platform 'android'), the desktop app
// ('linux') and the plain web UI. With the page's own player chosen (its card shows "selected") and
// with the node zone chosen (its card shows "Play to"):
//   * the card's line says what plays it in a few words ("Plays in this browser" / "Plays in the
//     app") and is not cut off by its ellipsis;
//   * the Play to list's row for it says the same on one line (that line wraps rather than cuts);
//   * no horizontal overflow.
//   node tools/ui-zone-here-line.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'zone-here-line-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const outputs = [{ id: 'airplay:kitchen', name: 'Kitchen', kind: 'airplay', caps: { hw_volume: 'percent' } }];
const zone = { id: 'kitchen', name: 'Kitchen', output: 'airplay:kitchen', transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 40, muted: false }, now_playing: null };
const fixtures = { '/zones': { body: [zone] }, '/outputs': { body: outputs } };
const CASES = [['android', { platform: 'android', version: '0.0.0' }, 'This phone', 'Plays in the app'], ['desktop', { platform: 'linux', version: '0.0.0' }, 'This computer', 'Plays in the app'], ['web', null, 'This browser', 'Plays in this browser']];
// The line's text and whether its ellipsis cuts it (the box clips what is wider than it)
const line = (page, sel) => page.$eval(sel, (el) => { const r = document.createRange(); r.selectNodeContents(el); const need = r.getBoundingClientRect().width; const has = el.getBoundingClientRect().width; return { text: el.textContent.trim(), cut: el.scrollWidth > el.clientWidth || need > has + 0.5, w: Math.round(has), need: Math.round(need) }; });

(async () => {
  const browser = await chromium.launch();
  for (const [tag, desktop, name, words] of CASES) {
    for (const w of [360, 412]) {
      for (const chosen of ['browser', 'kitchen']) {
        const at = `${tag} ${w} px, ${chosen === 'browser' ? name : 'Kitchen'} chosen`;
        const { page, context, state } = await fx.create(browser, w, { touch: true, fixtures, zone: chosen });
        await page.setViewportSize({ width: w, height: 800 });
        await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
        if (desktop) await context.addInitScript((d) => { window.pmDesktop = d; }, desktop);
        await fx.go(page, '/zones');
        await page.waitForSelector('[data-zcard="browser"] .zc-text .sub', { timeout: 7000 }).catch(() => {});
        await sleep(400);
        const card = await line(page, '[data-zcard="browser"] .zc-text .sub').catch(() => null);
        check(!!card, `${at}: the Zones page has ${name}'s card`);
        if (card) {
          check(card.text === words, `${at}: the card says "${words}" (${card.text})`);
          check(!card.cut, `${at}: the card's line is not cut off (needs ${card.need} px, has ${card.w})`);
        }
        const pick = await page.$eval('[data-zcard="browser"] .zc-now', (el) => (el.querySelector('.pill.ok') ? 'selected' : el.querySelector('[data-zsel]') ? 'Play to' : 'none')).catch(() => 'none');
        check(pick === (chosen === 'browser' ? 'selected' : 'Play to'), `${at}: the card shows ${chosen === 'browser' ? '"selected"' : 'Play to'} beside it (${pick})`);
        await page.screenshot({ path: path.join(SHOTS, `zones-${tag}-${w}-${chosen}.png`) });
        await page.$eval('#zonebtn', (b) => b.click()); await page.waitForSelector('#zonepop:not([hidden])').catch(() => {}); await sleep(200);
        const row = await line(page, '#zonepop [data-zone="browser"] .sub').catch(() => null);
        check(!!row && row.text === words, `${at}: the Play to list's row says "${words}" (${row && row.text})`);
        const rowLines = await page.$eval('#zonepop [data-zone="browser"] .sub', (el) => { const r = document.createRange(); r.selectNodeContents(el); return new Set([...r.getClientRects()].map((q) => Math.round(q.top))).size; }).catch(() => 0);
        check(rowLines === 1, `${at}: the Play to list's row is one line (${rowLines})`);
        if (chosen === 'browser') await page.screenshot({ path: path.join(SHOTS, `playto-${tag}-${w}.png`) });
        const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
        check(over <= 0, `${at}: no horizontal overflow (${over})`);
        check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
        await context.close();
      }
    }
  }
  await browser.close();
  console.log(failed ? `${failed} FAILED` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
