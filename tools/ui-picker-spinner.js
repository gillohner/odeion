// Add to collection… from a ⋯ (a Songs row's, an album page's own): while the picker is open the ⋯
// shows no spinner (it waits on your choice, not on the node; its dots stay), and once a collection
// is picked the ⋯ spins until the node has answered the add (before, it spun the whole time the
// picker was open and stopped the moment you picked, while the add was still on its way). Also
// New collection (spins until it is made) and Cancel (the ⋯ is itself again at once). A control
// busy without a dialog still spins (Add to Bookmarks from the same ⋯, answered slowly). The
// selection bar's Add to collection… behaves the same.
// At 360 and 412 px (phone, touch) and 1280 px (mouse).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-picker-spinner.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'picker-spinner-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const road = { ...fx.summary, id: 'p-road', name: 'Road trip', items: 4, updated_at: 20 };
const SLOW = 1500; // the node's answer to an add or a new collection
const fixtures = { '/playlists/p-road/items': { body: { ok: true } } };
const delay = (v, m) => ((m === 'POST' && (v === '/playlists/p-road/items' || v === '/playlists')) || (m === 'PUT' && /^\/(bookmarks|favourites)/.test(v)) ? SLOW : 0);

// The ⋯'s state: busy, spinning, and whether its dots are drawn
const look = (page, sel) => page.evaluate((s) => {
  const b = document.querySelector(s); if (!b) return null;
  const ic = b.querySelector('.ic');
  return { busy: b.getAttribute('aria-busy') === 'true', spin: b.classList.contains('is-spinning'), dots: !!ic && getComputedStyle(ic).display !== 'none', dialog: !!document.querySelector('dialog[open]') };
}, sel);

async function openPicker(page, mobile, opener) {
  if (mobile) await page.locator(opener).first().tap(); else { await page.locator(opener).first().hover(); await page.locator(opener).first().click({ force: true }); }
  await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
  const item = page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to collection…' });
  if (mobile) await item.tap(); else await item.click();
  await page.waitForSelector('dialog[open] #pplist [data-ppi="0"]');
}
const press = (page, mobile, sel) => (mobile ? page.locator(sel).tap() : page.locator(sel).click());

async function run(browser, width) {
  const mobile = width < 600;
  const { page, state, context } = await fx.create(browser, width, { mobile, fixtures, collections: [road], delay });
  const at = `${width}`;
  try {
    for (const [where, url, ready, opener] of [
      ['Songs row ⋯', '/tracks', '#tracklist .track', '#tracklist .track:first-child [data-act="more"]'],
      ["album page's own ⋯", '/album/lib-7', '#imore', '#imore'],
    ]) {
      await fx.go(page, url); await page.waitForSelector(ready);
      const sel = opener;
      // 1. The picker open: no spinner on the ⋯, its dots drawn
      await openPicker(page, mobile, opener);
      await page.waitForTimeout(600);
      const open = await look(page, sel);
      check(open && open.dialog && !open.spin && open.dots, `${at} ${where}: no spinner while the picker is open (${JSON.stringify(open)})`);
      if (where === 'Songs row ⋯') await page.screenshot({ path: path.join(SHOTS, `picker-open-${width}.png`) });
      // 2. A collection picked: the ⋯ spins until the add is answered, then is itself again
      const before = state.hits.length;
      await press(page, mobile, 'dialog[open] #pplist [data-ppi="0"]');
      await page.waitForTimeout(500);
      const adding = await look(page, sel);
      check(adding && !adding.dialog && adding.busy && adding.spin && !adding.dots, `${at} ${where}: the ⋯ spins while the add is on its way (${JSON.stringify(adding)})`);
      if (where === 'Songs row ⋯') await page.screenshot({ path: path.join(SHOTS, `adding-${width}.png`) });
      await page.waitForFunction(() => /Added to “Road trip”/.test(document.querySelector('#toast').textContent), null, { timeout: SLOW + 3000 });
      await page.waitForTimeout(100);
      const after = await look(page, sel);
      check(after && !after.busy && !after.spin && after.dots, `${at} ${where}: the ⋯ is itself again once the add is answered (${JSON.stringify(after)})`);
      check(state.hits.slice(before).filter((h) => h.m === 'POST' && h.v === '/playlists/p-road/items').length === 1, `${at} ${where}: one add sent`);
      // 3. Cancel: the ⋯ is itself again at once
      await openPicker(page, mobile, opener);
      await press(page, mobile, 'dialog[open] [data-ppclose]');
      await page.waitForTimeout(50);
      const cancelled = await look(page, sel);
      check(cancelled && !cancelled.dialog && !cancelled.busy && !cancelled.spin && cancelled.dots, `${at} ${where}: Cancel leaves the ⋯ as it was (${JSON.stringify(cancelled)})`);
    }
    // 4. New collection: the ⋯ spins until it is made
    await fx.go(page, '/tracks'); await page.waitForSelector('#tracklist .track');
    const sel = '#tracklist .track:first-child [data-act="more"]';
    await openPicker(page, mobile, sel);
    const n0 = state.hits.length;
    await press(page, mobile, 'dialog[open] #pplist [data-ppnew]');
    await page.waitForTimeout(500);
    const making = await look(page, sel);
    check(making && !making.dialog && making.busy && making.spin, `${at} New collection: the ⋯ spins while it is made (${JSON.stringify(making)})`);
    await page.waitForFunction(() => /^Created/.test(document.querySelector('#toast').textContent.trim()), null, { timeout: SLOW + 3000 });
    await page.waitForTimeout(100);
    const made = await look(page, sel);
    check(made && !made.busy && !made.spin, `${at} New collection: the ⋯ is itself again once it is made (${JSON.stringify(made)})`);
    check(state.hits.slice(n0).filter((h) => h.m === 'POST' && h.v === '/playlists').length === 1, `${at} New collection: one made`);
    // 5. Busy with no dialog still spins: the same ⋯'s Add to Bookmarks, answered slowly
    if (mobile) await page.locator(sel).tap(); else { await page.locator(sel).hover(); await page.locator(sel).click({ force: true }); }
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
    const fav = page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to Bookmarks' });
    if (mobile) await fav.tap(); else await fav.click();
    await page.waitForTimeout(500);
    const saving = await look(page, sel);
    check(saving && !saving.dialog && saving.busy && saving.spin, `${at} Add to Bookmarks (no dialog): the ⋯ still spins while it is saved (${JSON.stringify(saving)})`);
    await page.waitForTimeout(SLOW + 500);
    // 6. The selection bar's Add to collection… (an album page's songs): no spinner while the picker
    // is open, then it spins until the add is answered
    state.collections = [road]; // (the fixture's New collection put its own in the list)
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs [data-selstart]');
    await press(page, mobile, '#isongs [data-selstart]');
    await page.locator('#isongs .selbox input').first().check();
    const bar = '.selbar [data-sel="playlist"]';
    await press(page, mobile, bar);
    await page.waitForSelector('dialog[open] #pplist [data-ppi="0"]');
    await page.waitForTimeout(600);
    const barOpen = await look(page, bar);
    check(barOpen && barOpen.dialog && !barOpen.spin, `${at} selection bar: no spinner while the picker is open (${JSON.stringify(barOpen)})`);
    const b0 = state.hits.length;
    await press(page, mobile, 'dialog[open] #pplist [data-ppi="0"]');
    await page.waitForTimeout(500);
    const barAdding = await look(page, bar);
    check(barAdding && !barAdding.dialog && barAdding.busy && barAdding.spin, `${at} selection bar: it spins while the add is on its way (${JSON.stringify(barAdding)})`);
    await page.waitForFunction(() => /added to “Road trip”/.test(document.querySelector('#toast').textContent), null, { timeout: SLOW + 3000 });
    check(state.hits.slice(b0).filter((h) => h.m === 'POST' && h.v === '/playlists/p-road/items').length === 1, `${at} selection bar: one add sent`);
    check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
  } catch (e) {
    check(false, `${at}: ${e.message.split('\n')[0]}`);
  } finally {
    await context.close();
  }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [360, 412, 1280]) await run(browser, w); } finally { await browser.close(); }
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})();
