// The phone top bar keeps every control on screen (#248 review of 5545f546): with Help "?" beside
// the account button, a running scan and the "outside parts" warning can crowd a 320 px bar. Real
// candidate assets on the fixture origin (tools/ui-fixture.js): no node, no network. At 320, 360,
// 375 and 390 px, signed in, each of: nothing showing, a scan running, a part not answering (with
// two more), and both at once:
//   * the page doesn't scroll sideways;
//   * the search, "?", the warning, Install and the account button are each wholly on screen, none under
//     another;
//   * the search keeps its magnifier (44 px).
// Screenshots of the crowded bar go to the shots folder.
//   node tools/ui-topbar.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'topbar-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const info = (scanning) => ({ body: { name: 'Fixture node', offline: false, scanning, library: { tracks: 1, albums: 1, artists: 2, files: 1, missing_files: 0 } } });
const ok = [{ id: 'homeserver', name: 'Pubky homeserver', state: 'ok' }];
const bad = [
  { id: 'homeserver', name: 'Pubky homeserver', state: 'needs_sign_in', detail: 'approve new folders' },
  { id: 'musicbrainz', name: 'MusicBrainz', state: 'down', detail: 'no answer' },
  { id: 'tidal', name: 'TIDAL', state: 'down', detail: 'no answer' },
];
const STATES = [
  ['nothing showing', false, ok],
  ['a scan running', true, ok],
  ['a part not answering', false, bad],
  ['a scan and a part not answering', true, bad],
];

(async () => {
  const browser = await chromium.launch();
  for (const width of [320, 360, 375, 390]) {
    for (const [name, scanning, health] of STATES) {
      const { context, page, state } = await fx.create(browser, width, { fixtures: { '/info': info(scanning), '/health': { body: health } } });
      await fx.go(page, '/');
      await page.locator('#help button.fb-help').waitFor();
      if (scanning) await page.locator('#status .scanning').waitFor();
      if (health === bad) await page.locator('.topbar #upstreams:not([hidden])').waitFor();
      const m = await page.evaluate(() => {
        const box = (e) => { const r = e.getBoundingClientRect(); return { id: e.id || e.className.split(' ')[0], left: r.left, right: r.right, w: r.width }; };
        const shown = (e) => e && !e.hidden && e.getBoundingClientRect().width > 0 && getComputedStyle(e).display !== 'none';
        const parts = ['.topbar .search', '#help button.fb-help', '.topbar #upstreams', '#installbtn', '#me'].map((s) => document.querySelector(s)).filter(shown).map(box);
        return { scroll: document.documentElement.scrollWidth - innerWidth, inner: innerWidth, parts, search: box(document.querySelector('.topbar .search')) };
      });
      const where = `${width} px, ${name}`;
      check(m.scroll <= 0, `${where}: no sideways scroll (${m.scroll} px)`);
      const off = m.parts.filter((p) => p.left < -0.5 || p.right > m.inner + 0.5).map((p) => `${p.id} ${p.left.toFixed(0)}–${p.right.toFixed(0)}`);
      check(!off.length, `${where}: every control on screen${off.length ? ': ' + off.join(', ') : ''}`);
      const under = m.parts.slice(1).filter((p, i) => p.left < m.parts[i].right - 0.5).map((p) => p.id);
      check(!under.length, `${where}: none under another${under.length ? ': ' + under.join(', ') : ''}`);
      check(m.search.w >= 43.5, `${where}: the search keeps its magnifier (${m.search.w.toFixed(0)} px)`);
      check(!state.errors.length, `${where}: no page errors ${state.errors.join('; ')}`);
      if (scanning && health === bad) await page.screenshot({ path: path.join(SHOTS, `topbar-${width}.png`), clip: { x: 0, y: 0, width, height: 72 } });
      await context.close();
    }
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
