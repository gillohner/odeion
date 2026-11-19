// The keyboard shortcuts Space, ← → and N are single keys: held with Alt, Ctrl or Cmd they belong to
// the browser or the system and leave the music alone. Before, Alt+← and Alt+→ (the browser's Back
// and Forward on Windows and Linux) also went to the previous or next song, so going back a page
// changed the song; Alt+Space and Ctrl+Space played or paused, and Alt+N opened Now playing.
// Alt+Space or Ctrl+Space on a button a click left focused doesn't press that button again either.
// The plain keys still do their job, and / and ? still work when the keyboard needs AltGr for them.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-shortcut-modifiers.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/shortcut-modifiers-shots');
fs.mkdirSync(shots, { recursive: true });
const tracks = Array.from({ length: 5 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, canonical_uri: `${fx.SONG}?n=${i + 1}` }));
const fixtures = {};
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function run(browser, width) {
  const r = await fx.create(browser, width, { tracks, fixtures });
  // Which song the app's player was given (by track id), and each play()/pause() it asked for
  await r.context.addInitScript(() => {
    window.__starts = []; window.__calls = [];
    const d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
    Object.defineProperty(HTMLMediaElement.prototype, 'src', { configurable: true, get() { return d.get.call(this); }, set(v) { const m = /\/library\/tracks\/(\d+)\/file/.exec(v); if (m) window.__starts.push(m[1]); d.set.call(this, v); } });
    const play = HTMLMediaElement.prototype.play; const pause = HTMLMediaElement.prototype.pause;
    HTMLMediaElement.prototype.play = function () { window.__calls.push('play'); return play.call(this); };
    HTMLMediaElement.prototype.pause = function () { window.__calls.push('pause'); return pause.call(this); };
  });
  const { page } = r;
  const title = () => page.$eval('#np', (e) => e.textContent.trim());
  const calls = () => page.evaluate(() => window.__calls.length);
  const npOpen = () => page.$eval('#nowplaying', (e) => !e.hidden);
  // Focus on the page itself (no control), as after a click on blank space
  const blank = () => page.evaluate(() => { if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur(); });
  await fx.go(page, '/tracks'); await page.waitForSelector('.tracks .track');
  await page.locator('.tracks .track').nth(2).dblclick();
  await page.waitForFunction(() => /Song 3/.test(document.querySelector('#np').textContent));
  await blank();

  // The plain arrows still skip
  await page.keyboard.press('ArrowRight'); await sleep(300);
  check(/Song 4/.test(await title()), `${width}: → goes to the next song (${await title()})`);
  await page.keyboard.press('ArrowLeft'); await sleep(300);
  check(/Song 3/.test(await title()), `${width}: ← goes back to the song before (${await title()})`);

  // With Alt, Ctrl or Cmd the arrows are the browser's
  for (const mod of ['Alt', 'Control', 'Meta']) {
    for (const key of ['ArrowLeft', 'ArrowRight']) {
      const was = await page.evaluate(() => window.__starts.length); const before = await title();
      await page.keyboard.press(`${mod}+${key}`); await sleep(300);
      const now = await title();
      check(now === before && await page.evaluate(() => window.__starts.length) === was, `${width}: ${mod}+${key} leaves the song alone (${now})`);
    }
  }
  await page.screenshot({ path: path.join(shots, `alt-arrows-${width}.png`) });

  // With Alt or Ctrl, Space neither plays nor pauses, and the page doesn't swallow it
  for (const mod of ['Alt', 'Control']) {
    const was = await calls();
    const kept = await page.evaluate((m) => { const e = new KeyboardEvent('keydown', { key: ' ', code: 'Space', altKey: m === 'Alt', ctrlKey: m === 'Control', bubbles: true, cancelable: true }); document.body.dispatchEvent(e); return !e.defaultPrevented; }, mod);
    await page.keyboard.press(`${mod}+Space`); await sleep(300);
    check(kept && await calls() === was, `${width}: ${mod}+Space neither plays nor pauses (${await calls() - was} calls, left to the browser: ${kept})`);
  }
  {
    const was = await calls();
    await page.keyboard.press('Space'); await sleep(300);
    check(await calls() > was, `${width}: Space still plays or pauses`);
  }

  // A button a click left focused isn't pressed again by Alt+Space or Ctrl+Space (Shuffle would
  // shuffle again, an armed "Really delete?" would delete), and the music stays as it was
  await page.evaluate(() => {
    const b = document.createElement('button'); b.id = 'press-count'; b.textContent = 'Count'; b.dataset.n = '0';
    b.style.cssText = 'position:fixed;top:120px;left:300px;z-index:99999';
    b.addEventListener('click', () => { b.dataset.n = String(+b.dataset.n + 1); });
    document.body.append(b);
  });
  const presses = () => page.$eval('#press-count', (b) => +b.dataset.n);
  await page.click('#press-count');
  check(await presses() === 1 && await page.evaluate(() => document.activeElement.id === 'press-count'), `${width}: the clicked button is pressed once and keeps focus`);
  for (const mod of ['Control', 'Alt']) {
    const was = await calls(); const n = await presses();
    await page.keyboard.press(`${mod}+Space`); await sleep(300);
    check(await presses() === n && await calls() === was, `${width}: ${mod}+Space on the clicked button presses it again: ${await presses() - n}, plays or pauses: ${await calls() - was}`);
  }
  await page.evaluate(() => document.getElementById('press-count').remove());
  await blank();

  // Alt+N doesn't open Now playing; N does, and N again closes it
  await page.keyboard.press('Alt+KeyN'); await sleep(300);
  check(!await npOpen(), `${width}: Alt+N leaves Now playing closed`);
  if (await npOpen()) { await page.keyboard.press('Escape'); await sleep(300); }
  await page.keyboard.press('KeyN'); await sleep(300);
  check(await npOpen(), `${width}: N opens Now playing`);
  await page.keyboard.press('KeyN'); await sleep(300);
  check(!await npOpen(), `${width}: N again closes it`);

  // / typed with AltGr (Ctrl+Alt on Windows) still goes to the search box
  await blank();
  await page.evaluate(() => { document.body.dispatchEvent(new KeyboardEvent('keydown', { key: '/', code: 'Digit7', ctrlKey: true, altKey: true, bubbles: true, cancelable: true })); });
  await sleep(200);
  check(await page.evaluate(() => document.activeElement && document.activeElement.id === 'search'), `${width}: / typed with AltGr goes to the search box`);
  await page.keyboard.press('Escape'); await blank();
  // ? typed with AltGr still opens the list of shortcuts
  await page.evaluate(() => { document.body.dispatchEvent(new KeyboardEvent('keydown', { key: '?', code: 'Minus', ctrlKey: true, altKey: true, bubbles: true, cancelable: true })); });
  await sleep(300);
  check(await page.evaluate(() => [...document.querySelectorAll('dialog[open]')].some((d) => /Keyboard shortcuts/.test(d.textContent))), `${width}: ? typed with AltGr opens the list of shortcuts`);
  await page.keyboard.press('Escape');

  check(!r.state.errors.length, `${width}: no page errors (${r.state.errors.join('; ')})`);
  await r.context.close();
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 768]) await run(browser, w); } finally { await browser.close(); }
  console.log(`\n${ok} ok / ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
