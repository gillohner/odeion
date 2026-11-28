// Sources › Your library: each "Open Settings › X" button opens Settings at the tab named X. Its
// Play from row said "Which source plays a song first when several have it" but opened Settings ›
// Zones and outputs, where the source order isn't (it has had its own tab, Settings › Play from,
// since Play from became a personal setting), so changing it took a second press on the right tab.
// Now the button opens Settings › Play from with the order in sight, and Back returns to Sources.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-sources-settings-links.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/sources-settings-links-shots');
fs.mkdirSync(shots, { recursive: true });

let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

// [width, fixture options, name]: desktop and tablet with a mouse, a phone (each tab its own page)
const SIZES = [[1280, {}, '1280'], [1024, {}, '1024'], [768, {}, '768'], [412, { mobile: true }, '412 phone']];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [w, opts, name] of SIZES) {
      const { page, context, state } = await fx.create(browser, w, opts);
      try {
        const open = async () => {
          await fx.go(page, '/sources/library');
          await page.waitForSelector('#main [data-goto-settings]', { state: 'visible' });
        };
        await open();
        const links = await page.$$eval('#main [data-goto-settings]', (bs) => bs.map((b) => ({ tab: b.dataset.gotoSettings, text: b.textContent.trim(), row: (b.closest('.set-row')?.querySelector('.set-name') || {}).textContent || '' })));
        check(links.some((l) => l.row === 'Play from'), `${name}: Your library has its Play from row with a Settings button (${JSON.stringify(links)})`);
        for (const l of links) {
          const what = `${name} "${l.row}" ${JSON.stringify(l.text)}`;
          await open();
          const label = (l.text.match(/^Open Settings › (.+)$/) || [])[1];
          check(!!label, `${what}: says which Settings tab it opens`);
          await page.click(`#main [data-goto-settings="${l.tab}"]`);
          // Settings has drawn the tab (its panel; Sources' own panels share the class)
          await page.waitForFunction((t) => location.pathname === `/settings/${t}` && !!document.querySelector(`#main .set-panel[data-panel="${t}"]:not([hidden])`), l.tab);
          const got = await page.evaluate(() => {
            const pnl = [...document.querySelectorAll('#main .set-panel')].find((p) => !p.hidden);
            return { path: location.pathname, title: (pnl.querySelector('.set-title') || {}).textContent || '', groups: [...pnl.querySelectorAll('.set-h')].map((h) => h.textContent) };
          });
          check(got.title === label, `${what}: opens the tab it names (${got.path}, "${got.title}")`);
          if (l.row === 'Play from') {
            check(got.path === '/settings/playfrom', `${what}: opens Settings › Play from (${got.path})`);
            // the order's rows, not the loading placeholder #gorder holds before they come
            const order = await page.waitForSelector('#main #gorder .porder-row', { state: 'visible', timeout: 5000 }).then(() => true, () => false);
            check(order && got.groups.includes('Play from'), `${what}: the source order is on the page it opens (${JSON.stringify(got.groups)})`);
            await page.screenshot({ path: path.join(shots, `play-from-${w}.png`) });
          }
          await page.goBack();
          const back = await page.waitForFunction(() => location.pathname === '/sources/library' && !!document.querySelector('#main [data-goto-settings]'), null, { timeout: 4000 }).then(() => true, () => false);
          check(back, `${what}: Back returns to Sources › Your library`);
        }
        await fx.cleanCopy(page);
        check(!state.errors.length, `${name}: no page errors ${state.errors.join('; ')}`);
      } catch (e) { failed++; console.log(`FAIL ${name}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})();
