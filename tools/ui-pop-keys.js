// /, N and ? with the zone picker, the Sound panel or the volume popover open (desktop and tablet,
// 768-1280 px): it closes first, with its grey scrim, and the shortcut runs. Before, it stayed open
// over Now playing (greyed under its scrim) and behind Help, and over the page while the search field
// had the focus. / leaves no history entry of it; after N or ?, Back (or Esc) closes Now playing or Help
// and the page is still there, with no entry left.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-pop-keys.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/pop-keys-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const POPS = [['zone picker', '#zonebtn', '#zonepop'], ['Sound panel', '#quality', '#qpanel'], ['volume', '#volbtn', '#volpop']];

async function run(browser, width) {
  const { context, page, state } = await fx.create(browser, width, {});
  try {
    await fx.go(page, '/library');
    await page.locator('#zonebtn').waitFor();
    await page.waitForTimeout(800);
    const shown = (sel) => page.locator(sel).isVisible();
    const help = () => page.evaluate(() => [...document.querySelectorAll('dialog[open]')].some((d) => /Keyboard shortcuts/.test(d.textContent)));
    const entry = () => page.evaluate(() => !!(history.state && history.state.transient));
    const atLibrary = () => new URL(page.url()).pathname === '/library';
    // (what a failed step left open goes first: Esc, then Back for Now playing, so one failure
    // doesn't hide the next checks)
    const reset = async () => {
      for (let k = 0; k < 4 && (await shown('#scrim') || await page.locator('dialog[open]').count() || await shown('#nowplaying')); k++) {
        await page.keyboard.press('Escape'); await page.waitForTimeout(300);
        if (await shown('#nowplaying') && !await shown('#scrim')) { await page.keyboard.press('n'); await page.waitForTimeout(300); }
      }
      if (await entry()) { await page.goBack(); await page.waitForTimeout(400); }
    };
    for (const [name, btn, pop] of POPS) {
      if (!await shown(btn)) { check(width >= 1280 && btn === '#volbtn', `${width}: ${name} button shown (only the wide bar has a slider instead)`); continue; }
      const open = async () => {
        await reset();
        await page.locator(btn).click(); await page.locator(pop).waitFor({ timeout: 3000 }).catch(() => {});
        // (the key goes to the button that opened it. The volume popover puts the focus on its slider,
        // which owns the keys as before: there, the popover closes on these keys only once the focus left it)
        await page.locator(btn).focus();
        return shown(pop);
      };
      // /: closed, its scrim too, the search field has the focus, no entry of it left in the history
      let was = await open();
      await page.keyboard.press('/');
      await page.waitForFunction(() => !(history.state && history.state.transient), null, { timeout: 2000 }).catch(() => {}); await page.waitForTimeout(200);
      check(was && !await shown(pop) && !await shown('#scrim') && await page.evaluate(() => document.activeElement && document.activeElement.id === 'search') && !await entry(),
        `${width} ${name}: / closes it (and the scrim) and goes to the search field, no entry left in the history`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(200);
      // N: closed, Now playing open and not under a scrim; Back closes Now playing, the Library stays
      was = await open();
      await page.keyboard.press('n'); await page.waitForTimeout(500);
      const np = await shown('#nowplaying');
      check(was && !await shown(pop) && !await shown('#scrim') && np, `${width} ${name}: N closes it (and the scrim) and opens Now playing`);
      await page.screenshot({ path: path.join(shots, `${btn.slice(1)}-np-${width}.png`) });
      await page.goBack(); await page.waitForTimeout(600);
      check(!await shown('#nowplaying') && !await shown(pop) && atLibrary() && !await entry(), `${width} ${name}: then Back closes Now playing, the Library still there, no entry left (${new URL(page.url()).pathname})`);
      // ?: closed, Help open; Back closes Help, the Library stays; again, then Esc: no entry left
      was = await open();
      await page.keyboard.press('?'); await page.waitForTimeout(500);
      check(was && !await shown(pop) && !await shown('#scrim') && await help(), `${width} ${name}: ? closes it (and the scrim) and opens the list of shortcuts`);
      await page.screenshot({ path: path.join(shots, `${btn.slice(1)}-help-${width}.png`) });
      await page.goBack(); await page.waitForTimeout(600);
      check(!await help() && !await shown(pop) && atLibrary() && !await entry(), `${width} ${name}: then Back closes it, the Library still there, no entry left (${new URL(page.url()).pathname})`);
      was = await open();
      await page.keyboard.press('?'); await page.waitForTimeout(500);
      await page.keyboard.press('Escape'); await page.waitForTimeout(600);
      check(was && !await help() && !await shown(pop) && atLibrary() && !await entry(), `${width} ${name}: ? then Esc: both closed, no entry left in the history`);
    }
    // Opened over Now playing (the bar's zone picker, beside it at 1024 px and wider): / closes the
    // picker and Now playing (the search field is under it); N closes both too; no entry left
    if (width >= 1024) {
      for (const key of ['/', 'n']) {
        await reset();
        await page.keyboard.press('n'); await page.locator('#nowplaying').waitFor({ timeout: 3000 }).catch(() => {});
        await page.locator('#zonebtn').click(); await page.locator('#zonepop').waitFor({ timeout: 3000 }).catch(() => {}); await page.locator('#zonebtn').focus();
        const was = await shown('#nowplaying') && await shown('#zonepop');
        await page.keyboard.press(key);
        await page.waitForFunction(() => !(history.state && history.state.transient), null, { timeout: 2000 }).catch(() => {}); await page.waitForTimeout(200);
        const focus = await page.evaluate(() => document.activeElement && document.activeElement.id);
        check(was && !await shown('#zonepop') && !await shown('#scrim') && !await shown('#nowplaying') && atLibrary() && !await entry() && (key !== '/' || focus === 'search'),
          `${width} zone picker over Now playing: ${key} closes both${key === '/' ? ' and goes to the search field' : ''}, the Library still there, no entry left (${focus})`);
        await page.keyboard.press('Escape'); await page.waitForTimeout(200);
      }
    }
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
