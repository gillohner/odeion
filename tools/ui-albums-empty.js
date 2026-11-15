// Albums with no albums. A library whose songs have no album name in their tags lists them under
// Songs and has no albums; Albums said "Add the folder with your music in Settings" with Open
// settings, a folder that is already there. Now, when the library has songs, it says they have no
// album name in their tags and Show songs opens Songs (1 click). With no songs (or no answer about
// them) it still sends you to Settings to add your music. At 1280, 1024, 768 and 360 px.
//   node tools/ui-albums-empty.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'albums-empty-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const stats = (tracks) => ({ tracks, albums: 0, artists: 0, files: tracks, missing_files: 0, total_duration_ms: 0 });
const cases = [
  { name: 'songs', fixtures: { '/library/stats': { body: stats(4) } }, text: 'The 4 songs in the library have no album name in their tags', button: 'Show songs', goes: 'Songs', url: '/tracks' },
  { name: 'one-song', fixtures: { '/library/stats': { body: stats(1) } }, text: 'The song in the library has no album name in its tags', button: 'Show songs', goes: 'Songs', url: '/tracks' },
  { name: 'no-songs', fixtures: { '/library/stats': { body: stats(0) } }, text: 'Add the folder with your music in Settings', button: 'Open settings', goes: 'Settings', url: '/settings' },
  { name: 'stats-fail', fixtures: { '/library/stats': { body: { error: 'busy' }, status: 503 } }, text: 'Add the folder with your music in Settings', button: 'Open settings', goes: 'Settings', url: '/settings' },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 360]) {
      for (const c of cases) {
        const { page, context, state } = await fx.create(browser, width, { fixtures: c.fixtures });
        try {
          await fx.go(page, '/albums');
          const shown = await page.waitForSelector('.main .empty', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${width} ${c.name}: Albums shows its empty state`);
          if (!shown) continue;
          const e = await page.evaluate(() => { const box = document.querySelector('.main .empty'); return { title: box.querySelector('h3')?.textContent, text: box.querySelector('p')?.textContent || '', buttons: [...box.querySelectorAll('button')].map((b) => b.textContent.trim()) }; });
          check(e.title === 'No albums yet', `${width} ${c.name}: the title is "No albums yet" (${e.title})`);
          check(e.text.includes(c.text), `${width} ${c.name}: it says "${c.text}…" (${e.text})`);
          check(e.buttons.length === 1 && e.buttons[0] === c.button, `${width} ${c.name}: one button, ${c.button} (${e.buttons.join(', ')})`);
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${width} ${c.name}: nothing wider than the screen (${o})`);
          await page.screenshot({ path: path.join(SHOTS, `${c.name}-${width}.png`) });
          await page.locator('.main .empty button').click();
          const opened = await page.waitForFunction(([url, want]) => location.pathname.startsWith(url) && [...document.querySelectorAll('.main h1, .main h2')].some((h) => h.offsetParent && h.textContent.trim().startsWith(want)), [c.url, c.goes], { timeout: 8000 }).then(() => true, () => false);
          check(opened, `${width} ${c.name}: ${c.button} opens ${c.goes} (${await page.evaluate(() => location.pathname)})`);
          check(!state.errors.length, `${width} ${c.name}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
