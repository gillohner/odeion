// Add music (an empty page's button, or the /add-music address) opens Your library with Add
// music's chooser. A page left before Your library had drawn kept the ask, so a later visit to
// Your library opened the chooser out of nowhere (one more Cancel). Now the ask belongs to that
// one drawing.
// Real UI in Chromium against fixtures (no node), at 1280, 1024, 768 and 360 px.
//   node tools/ui-add-music-ask.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'add-music-ask-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const stats = { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0, total_duration_ms: 0 };
const empty = { '/library/stats': { body: stats }, '/info': { body: { name: 'Fixture node', offline: false, library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0 } } } };
const chooserOpen = () => { const d = document.querySelector('dialog[open]'); return !!d && /Add a folder or network share/.test(d.textContent); };
// Your library's own page has drawn (its tabs), and its slow answer is in
const libraryDrawn = (page) => page.waitForFunction(() => location.pathname === '/library' && !!document.querySelector('#main [data-libtab].on'), null, { timeout: 8000 }).then(() => true, () => false);


(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 360]) {
      // Add music, waited for: the chooser opens on Your library
      {
        const T = `${width} Add music`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: empty, tracks: [], delay: (v) => (v === '/storage' ? 600 : 0) });
        try {
          await fx.go(page, '/tracks');
          await page.waitForSelector('.main .empty button', { timeout: 8000 });
          await page.locator('.main .empty button').click();
          const ok = await page.waitForFunction(chooserOpen, null, { timeout: 8000 }).then(() => true, () => false);
          check(ok, `${T}: the chooser opens (${await page.evaluate(() => location.pathname)})`);
          check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Add music, then Back before Your library has drawn: a later visit doesn't open the chooser
      {
        const T = `${width} Add music, left early`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: empty, tracks: [], delay: (v) => (v === '/storage' ? 1200 : 0) });
        try {
          await fx.go(page, '/tracks');
          await page.waitForSelector('.main .empty button', { timeout: 8000 });
          await page.locator('.main .empty button').click();
          await page.waitForFunction(() => location.pathname === '/library', null, { timeout: 8000 });
          await page.goBack();
          await page.waitForFunction(() => location.pathname === '/tracks' && !!document.querySelector('.main .empty'), null, { timeout: 8000 });
          await page.waitForTimeout(1500); // (well past the first visit's slow read: that drawing is over)
          check(!(await page.evaluate(chooserOpen)), `${T}: nothing opens on Songs`);
          // Your library from the menu (a phone's sidebar is hidden: the same button, pressed by script)
          await page.evaluate(() => document.querySelector('.nav-item[data-view="library"]').click());
          const drawn = await libraryDrawn(page);
          check(drawn, `${T}: Your library draws`);
          await page.waitForTimeout(500);
          const open = await page.evaluate(chooserOpen);
          await page.screenshot({ path: path.join(SHOTS, `library-later-${width}.png`) });
          check(!open, `${T}: a later visit to Your library doesn't open Add music's chooser`);
          // and Add music still works after that
          if (open) await page.keyboard.press('Escape');
          await page.locator('#libadd').click();
          check(await page.waitForFunction(chooserOpen, null, { timeout: 8000 }).then(() => true, () => false), `${T}: Add music on Your library still opens it`);
          check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
