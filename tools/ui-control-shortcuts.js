// /, N and ? with the focus on a slider or a switch (desktop and tablet, 1280, 1024 and 768 px). These
// controls take no typed characters, so the three keys are the shortcuts there too: Search, Now
// playing and the list of shortcuts. Before, any input counted as a text field: with the focus on the
// volume popover's slider (where opening it puts the focus), the bar's volume slider or a Settings
// switch, the keys did nothing and the popover stayed open. The arrows still move the slider, and a
// text field (the search box) still types the characters.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-control-shortcuts.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/control-shortcuts-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const FIXTURES = { '/node/settings': { body: { contact: '', discovery: false, token_replaceable: true, prefer_full: true, dev_outputs: false, first_steps_hidden: false } } };

async function run(browser, width) {
  const { context, page, state } = await fx.create(browser, width, { fixtures: FIXTURES });
  try {
    const shown = (sel) => page.locator(sel).isVisible();
    const help = () => page.evaluate(() => [...document.querySelectorAll('dialog[open]')].some((d) => /Keyboard shortcuts/.test(d.textContent)));
    const focusId = () => page.evaluate(() => { const a = document.activeElement; return !a || a === document.body ? 'body' : a.id || a.tagName; });
    const reset = async () => {
      for (let k = 0; k < 4 && (await shown('#scrim') || await page.locator('dialog[open]').count() || await shown('#nowplaying')); k++) {
        await page.keyboard.press('Escape'); await page.waitForTimeout(300);
      }
      if (await page.evaluate(() => !!(history.state && history.state.transient))) { await page.goBack(); await page.waitForTimeout(400); }
      if (await page.evaluate(() => document.activeElement && document.activeElement.id === 'search')) { await page.keyboard.press('Escape'); await page.waitForTimeout(200); }
    };
    await fx.go(page, '/library');
    await page.locator('#zonebtn').waitFor();
    await page.waitForTimeout(800);

    // Where the bar has no room for its slider (1024, 768): the volume button opens the popover with the
    // focus on its slider. Where it has (1280): the bar's own slider, focused.
    const pop = await shown('#volbtn');
    const focusSlider = async () => {
      await reset();
      if (pop) { await page.locator('#volbtn').click(); await page.locator('#volpop:not([hidden]) #volpopvol').waitFor({ timeout: 3000 }).catch(() => {}); }
      else await page.locator('#vol').focus();
      return focusId();
    };
    const slider = pop ? 'volpopvol' : 'vol';
    const where = pop ? 'the volume popover\'s slider' : 'the bar\'s volume slider';
    check(pop === (width < 1280), `${width}: ${pop ? 'the volume button (no room for the slider)' : 'the bar\'s slider'} is shown`);

    // N: Now playing opens (the popover closed first, its scrim too)
    let f = await focusSlider();
    await page.keyboard.press('n'); await page.waitForTimeout(500);
    check(f === slider && await shown('#nowplaying') && !await shown('#volpop') && !await shown('#scrim'), `${width} ${where}: N opens Now playing${pop ? ' and closes the popover' : ''} (focus was ${f})`);
    await page.screenshot({ path: path.join(shots, `slider-n-${width}.png`) });
    // /: the search field has the focus
    f = await focusSlider();
    await page.keyboard.press('/'); await page.waitForTimeout(300);
    check(f === slider && await focusId() === 'search' && !await shown('#volpop') && !await shown('#scrim'), `${width} ${where}: / goes to the search field${pop ? ' and closes the popover' : ''} (${await focusId()})`);
    // ?: the list of shortcuts
    f = await focusSlider();
    await page.keyboard.press('?'); await page.waitForTimeout(500);
    check(f === slider && await help() && !await shown('#volpop'), `${width} ${where}: ? opens the list of shortcuts${pop ? ' and closes the popover' : ''}`);
    await page.screenshot({ path: path.join(shots, `slider-help-${width}.png`) });
    // The arrows are still the slider's: they move it, and the popover stays
    f = await focusSlider();
    const [before, low] = await page.$eval(`#${slider}`, (e) => [Number(e.value), Number(e.min)]);
    await page.keyboard.press(before > low ? 'ArrowLeft' : 'ArrowRight'); await page.waitForTimeout(300);
    const after = await page.$eval(`#${slider}`, (e) => Number(e.value));
    check(f === slider && after !== before && await focusId() === slider && (!pop || await shown('#volpop')) && !await shown('#nowplaying'), `${width} ${where}: an arrow still moves the slider (${before} → ${after})`);

    // A Settings switch: / and N are the shortcuts, and the switch stays as it was
    await reset();
    await fx.go(page, '/settings/playback'); await page.locator('#preferfull').waitFor({ timeout: 5000 });
    await page.locator('#preferfull').focus();
    const was = await page.$eval('#preferfull', (e) => e.checked);
    await page.keyboard.press('/'); await page.waitForTimeout(300);
    check(await focusId() === 'search' && await page.$eval('#preferfull', (e) => e.checked) === was, `${width} Settings switch (Prefer full tracks): / goes to the search field, the switch unchanged (${await focusId()})`);
    await reset();
    await page.locator('#preferfull').focus();
    await page.keyboard.press('n'); await page.waitForTimeout(500);
    check(await shown('#nowplaying') && await page.$eval('#preferfull', (e) => e.checked) === was, `${width} Settings switch (Prefer full tracks): N opens Now playing, the switch unchanged`);
    await page.screenshot({ path: path.join(shots, `switch-n-${width}.png`) });

    // A text field still types the characters: n, / and ? go into the search box
    await reset();
    await page.locator('#search').focus();
    await page.keyboard.type('n/?'); await page.waitForTimeout(300);
    const typed = await page.$eval('#search', (e) => e.value);
    check(/n\/\?$/.test(typed) && !await shown('#nowplaying') && !await help(), `${width} search field: n, / and ? are typed into it (${JSON.stringify(typed)})`);
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
