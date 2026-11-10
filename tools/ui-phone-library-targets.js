// Library's section tabs and Add music entry points have comfortable phone targets.
// Real candidate assets, isolated responses: node tools/ui-phone-library-targets.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-library-targets-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };
const targets = '#libadd, [data-addmusic], .libtabs .chip';
const fixtures = {
  '/library/folders': { body: [] },
  '/library/webdav': { body: [] },
  '/library/drives': { body: { providers: [], roots: [] } },
  '/library/hidden': { body: [] },
};

async function measure(page, at) {
  const sizes = await page.locator(targets).evaluateAll((els) => els.map((el) => {
    const b = el.getBoundingClientRect();
    return { label: el.textContent.trim(), width: b.width, height: b.height };
  }));
  check(sizes.length > 0 && sizes.every((b) => b.height >= 44 && b.width >= 44), `${at}: targets at least 44 px (${sizes.map((b) => `${b.label}: ${b.width.toFixed(1)}×${b.height.toFixed(1)}`).join(', ')})`);
  await fits(page, at);
}

async function fits(page, at) {
  const layout = await page.evaluate(() => {
    const main = document.querySelector('#main');
    const els = [...document.querySelectorAll('#libadd, [data-addmusic], .libtabs .chip')];
    return {
      fits: document.documentElement.scrollWidth <= innerWidth && main.scrollWidth <= main.clientWidth,
      visible: els.every((el) => { const b = el.getBoundingClientRect(); return b.left >= 0 && b.right <= innerWidth; }),
    };
  });
  check(layout.fits && layout.visible, `${at}: no sideways overflow or clipped controls`);
}

// A real touch near the bottom edge verifies the whole target, not just its label.
async function tapEdge(page, selector) {
  const b = await page.locator(selector).boundingBox();
  await page.touchscreen.tap(b.x + b.width / 2, b.y + b.height - 2);
}

async function chosen(page, tab) {
  await page.waitForSelector(`#libtab-${tab}[aria-selected="true"]`);
  await page.waitForSelector(`#libpanel[aria-labelledby="libtab-${tab}"]`);
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) for (const theme of ['light', 'dark']) {
      const at = `${android ? 'android' : 'browser'}-${width}-${theme}`;
      const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures });
      try {
        await context.addInitScript(({ android, theme }) => {
          localStorage.setItem('pm.theme', theme);
          if (android) window.pmDesktop = { platform: 'android', version: 'fixture' };
        }, { android, theme });
        await fx.go(page, '/library');
        await chosen(page, 'mine');
        await measure(page, `${at} My music`);
        await page.screenshot({ path: path.join(SHOTS, `library-${at}.png`) });
        for (const tab of ['node', 'import', 'mine']) {
          await tapEdge(page, `#libtab-${tab}`);
          await chosen(page, tab);
          check(await page.locator('.libtabs [aria-selected="true"]').count() === 1, `${at}: one edge tap selects ${tab}`);
          await measure(page, `${at} ${tab}`);
        }
        for (const route of ['/library', '/albums', '/tracks']) {
          await fx.go(page, route);
          await page.waitForSelector('#libadd, [data-addmusic]');
          await measure(page, `${at} ${route}`);
          await tapEdge(page, '#libadd, [data-addmusic]');
          await page.waitForSelector('dialog[open] .choice');
          check(await page.locator('dialog[open]').count() === 1 && await page.locator('dialog[open] h2').textContent() === 'Add music', `${at} ${route}: one edge tap opens Add music`);
          check(await page.locator('dialog[open] .choice').count() === 3, `${at} ${route}: upload, folder and import remain available`);
          await page.keyboard.press('Escape');
          await page.waitForSelector('dialog[open]', { state: 'detached' });
        }
        // The enlarged entry control still leads to the existing upload and import flows.
        await tapEdge(page, '#libadd');
        await page.locator('dialog[open] .choice').filter({ hasText: 'Upload files' }).tap();
        await page.waitForSelector('#libupload:not([hidden])');
        check(await page.locator('#libupload').isVisible(), `${at}: Upload files opens the upload panel in two taps`);
        await fx.go(page, '/library');
        await chosen(page, 'mine');
        await tapEdge(page, '#libadd');
        await page.locator('dialog[open] .choice').filter({ hasText: 'Import' }).tap();
        await chosen(page, 'import');
        check(await page.locator('#libpanel').isVisible(), `${at}: import opens its section in two taps`);
        // Keyboard users keep the selected tab and focus as the panel is replaced.
        await page.locator('#libtab-import').focus();
        await page.keyboard.press('ArrowLeft');
        await chosen(page, 'node');
        check(await page.locator('#libtab-node').evaluate((el) => el === document.activeElement), `${at}: arrow navigation keeps focus on the selected tab`);
        // Increase the font tokens actually used by these controls; min-height must allow growth.
        await page.evaluate(() => {
          document.documentElement.style.setProperty('--fs-md', '18.2px');
          document.documentElement.style.setProperty('--fs-sm', '16.9px');
        });
        check(await page.locator('#libtab-node').evaluate((el) => parseFloat(getComputedStyle(el).fontSize) >= 16.9), `${at}: larger text is applied`);
        await measure(page, `${at} larger text`);
        await page.screenshot({ path: path.join(SHOTS, `large-text-${at}.png`) });
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // The listener's sole section gets the same target without exposing owner actions.
    for (const width of [360, 412]) {
      const { page, context } = await fx.create(browser, width, { mobile: true, role: 'listener', fixtures });
      try {
        await fx.go(page, '/library');
        await chosen(page, 'mine');
        await measure(page, `listener ${width}`);
        check(await page.locator('.libtabs .chip').count() === 1 && await page.locator('#libadd').count() === 0, `listener ${width}: only My music, no owner actions`);
      } finally { await context.close(); }
    }
    // These rules belong to the chosen phone layout, including a forced wide phone layout.
    const { page, context } = await fx.create(browser, 599, { fixtures });
    try {
      await fx.go(page, '/library');
      await chosen(page, 'mine');
      await measure(page, '599 px phone');
      await page.setViewportSize({ width: 600, height: 900 });
      await page.waitForFunction(() => document.documentElement.dataset.shell === 'desktop');
      check(await page.locator('#libtab-mine').evaluate((el) => el.getBoundingClientRect().height < 44), '600 px desktop keeps its compact tabs');
      await page.evaluate(() => document.documentElement.dataset.shell = 'phone');
      await measure(page, 'phone layout at 600 px');
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed; ${failed} failed`);
  if (failed) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
