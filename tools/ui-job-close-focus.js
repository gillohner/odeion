// The copy's popup, closed with the keyboard (desktop and tablet, 768-1280 px). Close on a finished
// copy hides its chip as well, and Dismiss does too: the focus goes on to the next control in the bar
// (Sound), and Tab goes on in the bar. Before, it was given back to the chip just as the chip was
// hidden and fell to the page, with nothing focused. A copy still running: Hide keeps
// its chip, and the focus goes back on it. With the mouse the focus is left as it was.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-job-close-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/job-close-focus-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const base = { id: 'j1', kind: 'copy', total: 25, done: 25, failed: 0, skipped: 0, bytes_total: 750e6, bytes_done: 750e6, errors: [], errors_count: 0 };
const finished = () => ({ current: null, queued: [], last: { ...base, state: 'finished', finished: true } });
const running = () => ({ current: { ...base, state: 'running', done: 9, bytes_done: 270e6, current_folder: '/music/Artist 0001' }, queued: [], last: null });

async function run(browser, width) {
  const fixtures = { '/pubky/files/jobs': { body: finished() }, '/pubky/files/jobs/last': { body: {} } };
  const { context, page, state } = await fx.create(browser, width, { fixtures });
  const focusId = () => page.evaluate(() => { const a = document.activeElement; return !a || a === document.body ? 'body' : a.id || a.tagName; });
  const shown = (sel) => page.locator(sel).isVisible();
  // A fresh page with this job, its chip shown, the popup opened with Enter on the chip
  const openWith = async (jobs, chipText, how = 'key') => {
    fixtures['/pubky/files/jobs'] = { body: jobs };
    await fx.go(page, '/library');
    await page.waitForFunction((t) => { const c = document.getElementById('jobchip'); return c && !c.hidden && c.textContent.includes(t); }, chipText, { timeout: 8000 }).catch(() => {});
    await page.waitForTimeout(300);
    if (how === 'key') { await page.locator('#jobchip').focus(); await page.keyboard.press('Enter'); } else await page.locator('#jobchip').click();
    await page.locator('#jobpop').waitFor({ timeout: 3000 }).catch(() => {});
    return shown('#jobpop');
  };
  try {
    // Close on a finished copy, with the keyboard: popup and chip gone, the focus on Sound
    let was = await openWith(finished(), 'Done');
    await page.locator('#jobpop [data-jobclose]').focus(); await page.keyboard.press('Enter'); await page.waitForTimeout(300);
    let f = await focusId();
    check(was && !await shown('#jobpop') && !await shown('#jobchip') && !await shown('#scrim') && f === 'quality',
      `${width} finished copy: Close with the keyboard hides the popup and the chip, the focus goes on to Sound (${f})`);
    await page.screenshot({ path: path.join(shots, `close-${width}.png`) });
    await page.keyboard.press('Tab'); await page.waitForTimeout(100);
    const next = await page.evaluate(() => !!document.activeElement && !!document.activeElement.closest('.player-right'));
    check(next, `${width} finished copy: then Tab goes on in the bar, not from the page's top`);

    // Dismiss on a finished copy, with the keyboard: the node forgets it, the focus on Sound
    was = await openWith(finished(), 'Done');
    fixtures['/pubky/files/jobs'] = { body: { current: null, queued: [], last: null } };
    await page.locator('#jobpop [data-jact="dismiss"]').focus(); await page.keyboard.press('Enter');
    await page.waitForFunction(() => document.getElementById('jobpop').hidden && document.getElementById('jobchip').hidden, null, { timeout: 4000 }).catch(() => {});
    await page.waitForTimeout(200);
    f = await focusId();
    check(was && state.hits.some((h) => h.v === '/pubky/files/jobs/last' && h.m === 'DELETE') && !await shown('#jobpop') && !await shown('#jobchip') && f === 'quality',
      `${width} finished copy: Dismiss with the keyboard, popup and chip gone, the focus goes on to Sound (${f})`);

    // A copy still running: Hide keeps the chip, the focus back on it. (Its popup is drawn again with
    // every look at the job, every 2 s: the key, the focus on Hide and the press go in one step, so a
    // redraw can't take Hide away between them)
    was = await openWith(running(), '9/25');
    await page.evaluate(() => { const b = document.querySelector('#jobpop [data-jobclose]'); b.focus(); b.dispatchEvent(new KeyboardEvent('keydown', { key: 'Shift', bubbles: true })); b.click(); });
    await page.waitForTimeout(300);
    f = await focusId();
    check(was && !await shown('#jobpop') && await shown('#jobchip') && f === 'jobchip', `${width} running copy: Hide with the keyboard keeps the chip, the focus back on it (${f})`);

    // With the mouse: Close hides both and moves no focus to the bar
    was = await openWith(finished(), 'Done', 'mouse');
    await page.locator('#jobpop [data-jobclose]').click(); await page.waitForTimeout(300);
    f = await focusId();
    const inBar = await page.evaluate(() => !!document.activeElement && !!document.activeElement.closest('.player-right'));
    check(was && !await shown('#jobpop') && !await shown('#jobchip') && !inBar, `${width} finished copy: Close with the mouse hides both, the focus not moved to the bar (${f})`);
    check(state.errors.length === 0, `${width}: no page errors ${state.errors.join('; ')}`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768]) await run(browser, w); }
  finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
