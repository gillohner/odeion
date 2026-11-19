// A clickable row (a div the keyboard reaches: a playlist, an artist, a search result) opens with
// Enter or Space. Space held with Alt, Ctrl or Cmd is the system's (Alt+Space the window menu on
// Windows, Ctrl+Space the input language, Cmd+Space Spotlight), so it leaves the row alone and
// doesn't toggle playback either. Before, Alt+Space or Ctrl+Space on a focused row opened it.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-row-space-modifiers.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/row-space-modifiers-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function run(browser, width) {
  const r = await fx.create(browser, width, { collections: [fx.summary] });
  const { page, state } = r;
  const row = '#main [data-pl="c1"]';
  const opened = () => page.evaluate(() => location.pathname !== '/playlists');
  // Each play()/pause() the page asked of the browser's player
  await r.context.addInitScript(() => {
    window.__calls = 0;
    for (const k of ['play', 'pause']) { const f = HTMLMediaElement.prototype[k]; HTMLMediaElement.prototype[k] = function () { window.__calls++; return f.call(this); }; }
  });
  const back = async () => { await fx.go(page, '/playlists'); await page.waitForSelector(row); await page.waitForFunction((s) => document.querySelector(s).tabIndex === 0, row); };
  // A song in this browser's queue, so a Space that reached play/pause would play or pause it (with
  // nothing queued it would only say the queue is empty), and plain Space on the page shows it does
  const notice = () => page.evaluate(() => { const t = document.querySelector('#toast'); return !!t && !t.hidden && t.classList.contains('show') && /queue is empty/i.test(t.textContent); });
  await fx.go(page, '/tracks'); await page.waitForSelector('.tracks .track');
  await page.locator('.tracks .track').first().dblclick();
  await page.waitForFunction(() => /\S/.test(document.querySelector('#np .np-title')?.textContent || ''));
  await back();
  {
    const was = await page.evaluate(() => window.__calls);
    await page.evaluate(() => document.activeElement && document.activeElement.blur());
    await page.keyboard.press('Space'); await sleep(300);
    check(await page.evaluate(() => window.__calls) > was && !await notice(), `${width}: with a song queued, plain Space on the page plays or pauses (so the checks below can see a stray one)`);
  }

  for (const mod of ['Alt', 'Control', 'Meta']) {
    // Focused with the keyboard, and focused by a click on the row's blank part
    for (const how of ['keyboard', 'click']) {
      await back();
      if (how === 'keyboard') await page.focus(row);
      else { await page.evaluate((s) => { const el = document.querySelector(s); el.addEventListener('click', (e) => e.stopImmediatePropagation(), { capture: true, once: true }); }, row); await page.click(row, { position: { x: 4, y: 4 } }); }
      const focused = await page.evaluate((s) => document.activeElement === document.querySelector(s), row);
      const calls = await page.evaluate(() => window.__calls);
      const left = await page.evaluate(([s, m]) => { const e = new KeyboardEvent('keydown', { key: ' ', code: 'Space', altKey: m === 'Alt', ctrlKey: m === 'Control', metaKey: m === 'Meta', bubbles: true, cancelable: true }); document.querySelector(s).dispatchEvent(e); return !e.defaultPrevented; }, [row, mod]);
      await page.keyboard.press(`${mod}+Space`); await sleep(300);
      check(focused && !await opened() && left && await page.evaluate(() => window.__calls) === calls && !await notice(), `${width}: ${mod}+Space on a row (${how} focus) leaves it closed and the music alone (opened: ${await opened()}, left to the system: ${left}, plays or pauses: ${await page.evaluate(() => window.__calls) - calls}, empty-queue notice: ${await notice()})`);
    }
  }
  await page.screenshot({ path: path.join(shots, `row-ctrl-space-${width}.png`) });

  // Plain Space and Enter still open it
  for (const key of ['Space', 'Enter']) {
    await back(); await page.focus(row);
    await page.keyboard.press(key);
    await page.waitForFunction(() => location.pathname !== '/playlists', null, { timeout: 3000 }).catch(() => {});
    check(await opened(), `${width}: ${key} on a row opens it (${await page.evaluate(() => location.pathname)})`);
  }
  check(!state.errors.length, `${width}: no page errors (${state.errors.join('; ')})`);
  await r.context.close();
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768]) await run(browser, w); } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
