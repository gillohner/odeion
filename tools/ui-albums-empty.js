// Albums and Artists with nothing in them. A library whose songs have no album name in their tags
// lists them under Songs and has no albums; Albums said "Add the folder with your music in Settings"
// with Open settings, a folder that is already there. Now, when the library has songs, it says they
// have no album name in their tags and Open Songs opens Songs (1 click). With no songs (or no answer
// about them) Add music opens Your library with its chooser. Artists, empty only with no songs at
// all (songs without an artist tag are its "Unknown artist" row), said only "Artists come from the
// tags of scanned files." with nothing to press; now the owner gets Add music, as on Songs and
// Albums, and a listener is told the owner adds the music. Albums' button to Songs reads "Open
// Songs" (was "Show songs"; other empty states say "Open …"). At 1280, 1024, 768 and 360 px.
//   node tools/ui-albums-empty.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'albums-empty-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const stats = (tracks) => ({ tracks, albums: 0, artists: 0, files: tracks, missing_files: 0, total_duration_ms: 0 });
const noSongs = [
  { name: 'no-songs', fixtures: { '/library/stats': { body: stats(0) } }, text: 'Add the folder with your music, or upload files', button: 'Add music', goes: 'Your library', url: '/library' },
  { name: 'stats-fail', fixtures: { '/library/stats': { body: { error: 'busy' }, status: 503 } }, text: 'Add the folder with your music, or upload files', button: 'Add music', goes: 'Your library', url: '/library' },
];
const albumCases = [
  { name: 'songs', fixtures: { '/library/stats': { body: stats(4) } }, text: 'The 4 songs in the library have no album name in their tags', button: 'Open Songs', goes: 'Songs', url: '/tracks' },
  { name: 'one-song', fixtures: { '/library/stats': { body: stats(1) } }, text: 'The song in the library has no album name in its tags', button: 'Open Songs', goes: 'Songs', url: '/tracks' },
  ...noSongs,
];
const PAGES = [
  { view: 'albums', title: 'No albums yet', cases: albumCases },
  // (the node lists songs without an artist as "Unknown artist", so no artists means no songs)
  { view: 'artists', title: 'No artists yet', cases: [{ ...noSongs[0], fixtures: {} },
    // a listener can't add music: it says who does, and offers nothing to press
    { name: 'listener-no-songs', role: 'listener', fixtures: {}, text: 'Artists show up here once the node’s owner adds music.', button: null }] },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 360]) {
      for (const pg of PAGES) for (const c0 of pg.cases) {
        const c = { ...c0, name: `${pg.view} ${c0.name}` };
        const { page, context, state } = await fx.create(browser, width, { fixtures: c.fixtures, ...(c.role ? { role: c.role } : {}) });
        try {
          await fx.go(page, `/${pg.view}`);
          const shown = await page.waitForSelector('.main .empty', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${width} ${c.name}: the page shows its empty state`);
          if (!shown) continue;
          const e = await page.evaluate(() => { const box = document.querySelector('.main .empty'); return { title: box.querySelector('h3')?.textContent, text: box.querySelector('p')?.textContent || '', buttons: [...box.querySelectorAll('button')].map((b) => b.textContent.trim()) }; });
          check(e.title === pg.title, `${width} ${c.name}: the title is "${pg.title}" (${e.title})`);
          check(e.text.includes(c.text), `${width} ${c.name}: it says "${c.text}…" (${e.text})`);
          if (c.button) check(e.buttons.length === 1 && e.buttons[0] === c.button, `${width} ${c.name}: one button, ${c.button} (${e.buttons.join(', ')})`);
          else check(!e.buttons.length, `${width} ${c.name}: no button (${e.buttons.join(', ')})`);
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${width} ${c.name}: nothing wider than the screen (${o})`);
          await page.screenshot({ path: path.join(SHOTS, `${c.name.replace(' ', '-')}-${width}.png`) });
          if (!c.button) { check(!state.errors.length, `${width} ${c.name}: no page errors (${state.errors.join(' | ')})`); continue; }
          if (!e.buttons.length) continue; // (already failed above: nothing to press)
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
