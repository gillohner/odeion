// Favourites with nothing hearted. Its empty state said "Press the heart on a song, album or artist
// to keep it here" with nothing to press, a dead end; the hearts are on Songs, Albums and Artists.
// Now, with songs in the library, Open Songs opens Songs (1 click, owner or listener, also with
// the listener's own homeserver connected). With an empty library the owner gets Add music (Your
// library), as on the other empty pages, and a listener gets no button and is told the hearts
// come once the node's owner adds music (not to press a heart with nothing to press). A listener whose
// homeserver isn't connected still sees only Connect your homeserver (no empty state). At 1280,
// 1024 and 768 px. Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-favourites-empty.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'favourites-empty-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

const fav = (o) => ({ '/favourites': { body: { uris: [], albums: [], artists: [], tracks: [], other: [], ...o } } });
const cases = [
  { name: 'owner-songs', opts: { fixtures: fav({}) }, button: 'Open Songs', goes: 'Songs', url: '/tracks' },
  { name: 'listener-songs', opts: { role: 'listener', fixtures: fav({}) }, button: 'Open Songs', goes: 'Songs', url: '/tracks' },
  { name: 'listener-homeserver', opts: { role: 'listener', fixtures: fav({ homeserver: { connected: true } }) }, text: 'on your own homeserver', button: 'Open Songs', goes: 'Songs', url: '/tracks' },
  { name: 'owner-no-songs', opts: { tracks: [], fixtures: fav({}) }, button: 'Add music', goes: 'Your library', url: '/library' },
  { name: 'listener-no-songs', opts: { role: 'listener', tracks: [], fixtures: fav({}) }, text: 'Once the node’s owner adds music, press the heart on a song, album or artist to keep it here.', exact: true, button: null },
  { name: 'listener-homeserver-no-songs', opts: { role: 'listener', tracks: [], fixtures: fav({ homeserver: { connected: true } }) }, text: 'Once the node’s owner adds music, press the heart on a song, album or artist to keep it here, on your own homeserver.', exact: true, button: null },
  { name: 'listener-homeserver-off', opts: { role: 'listener', fixtures: fav({ homeserver: { connected: false } }) }, noEmpty: true },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      for (const c of cases) {
        const { page, context, state } = await fx.create(browser, width, c.opts);
        try {
          await fx.go(page, '/favourites');
          if (c.noEmpty) {
            const note = await page.waitForSelector('.main #hsnote [data-hsconnect]', { timeout: 8000 }).then(() => true, () => false);
            check(note, `${width} ${c.name}: Connect your homeserver is shown`);
            check(!(await page.$('.main .empty')), `${width} ${c.name}: and no empty state under it`);
            continue;
          }
          const shown = await page.waitForSelector('.main .empty', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${width} ${c.name}: the page shows its empty state`);
          if (!shown) continue;
          const e = await page.evaluate(() => { const box = document.querySelector('.main .empty'); return { title: box.querySelector('h3')?.textContent, text: box.querySelector('p')?.textContent || '', buttons: [...box.querySelectorAll('button')].map((b) => b.textContent.trim()) }; });
          check(e.title === 'Nothing hearted yet', `${width} ${c.name}: the title is "Nothing hearted yet" (${e.title})`);
          if (c.exact) check(e.text === c.text, `${width} ${c.name}: it says "${c.text}" (${e.text})`);
          else check(e.text.includes(c.text || 'Press the heart on a song, album or artist'), `${width} ${c.name}: it says how to heart something (${e.text})`);
          if (c.button) check(e.buttons.length === 1 && e.buttons[0] === c.button, `${width} ${c.name}: one button, ${c.button} (${e.buttons.join(', ')})`);
          else check(!e.buttons.length, `${width} ${c.name}: no button (${e.buttons.join(', ')})`);
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${width} ${c.name}: nothing wider than the screen (${o})`);
          await page.screenshot({ path: path.join(SHOTS, `${c.name}-${width}.png`) });
          if (!c.button || !e.buttons.length) { check(!state.errors.length, `${width} ${c.name}: no page errors (${state.errors.join(' | ')})`); continue; }
          await page.locator('.main .empty button').click();
          const opened = await page.waitForFunction(([url, want]) => location.pathname.startsWith(url) && [...document.querySelectorAll('.main h1, .main h2')].some((h) => h.offsetParent && h.textContent.trim().startsWith(want)), [c.url, c.goes], { timeout: 8000 }).then(() => true, () => false);
          check(opened, `${width} ${c.name}: ${c.button} opens ${c.goes} (${await page.evaluate(() => location.pathname)})`);
          check(!state.errors.length, `${width} ${c.name}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${passed} ok, ${failed} FAIL` : `${passed} ok, all passed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
