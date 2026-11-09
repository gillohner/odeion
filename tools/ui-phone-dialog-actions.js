// Phone dialog actions fit inside their sheet and have at least 44 px touch targets.
// node tools/ui-phone-dialog-actions.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-dialog-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let passed = 0, failed = 0;
const check = (ok, label) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${label}`); ok ? passed++ : failed++; };
const fixtures = { '/library/tracks/1/edit': { body: { file: { title: fx.track.title, artist: 'Sefa', album: 'Journey' }, edit: {} } } };
const writes = (state) => state.hits.filter((h) => h.v === '/library/tracks/1/edit' && h.m !== 'GET');
const closed = (page) => page.locator('dialog[open]').waitFor({ state: 'detached' });

async function openTags(page) {
  await page.locator('.track [data-act="more"]').first().tap();
  await page.locator('.popover.menu .pop-item').filter({ hasText: 'Edit tags' }).tap();
  await page.locator('dialog[open] [data-save]').waitFor();
}

async function tapEdge(page, selector) {
  const button = page.locator(selector);
  await button.scrollIntoViewIfNeeded();
  const b = await button.boundingBox();
  await page.touchscreen.tap(b.x + b.width / 2, b.y + b.height - 2);
}

async function measure(page, label) {
  await page.locator('dialog[open] .dialog-actions').scrollIntoViewIfNeeded();
  const m = await page.locator('dialog[open]').evaluate((dialog) => {
    const footer = dialog.querySelector('.dialog-actions').getBoundingClientRect();
    const buttons = [...dialog.querySelectorAll('.dialog-actions .btn')];
    const boxes = buttons.map((b) => b.getBoundingClientRect());
    return {
      sizes: boxes.map((b) => `${b.width.toFixed(1)}×${b.height.toFixed(1)}`).join(', '),
      targets: boxes.length > 0 && boxes.every((b) => b.width >= 44 && b.height >= 44),
      contained: boxes.every((b) => b.left >= footer.left - 0.5 && b.right <= footer.right + 0.5 && b.top >= 0 && b.bottom <= innerHeight),
      separate: boxes.every((a, i) => boxes.slice(i + 1).every((b) => a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top)),
      reachable: buttons.every((button, i) => {
        const b = boxes[i];
        return [b.left + 2, b.right - 2].every((x) => button.contains(document.elementFromPoint(x, b.y + b.height / 2)));
      }),
      overflow: document.documentElement.scrollWidth > innerWidth || dialog.scrollWidth > dialog.clientWidth,
    };
  });
  check(m.targets, `${label}: targets at least 44 px (${m.sizes})`);
  check(m.contained && m.separate && m.reachable, `${label}: every action fits, without clipping, overlap or blocked edges`);
  check(!m.overflow, `${label}: no horizontal overflow`);
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
        await page.locator('#libadd').tap();
        await measure(page, `${at} Add music`);
        await page.screenshot({ path: path.join(SHOTS, `add-music-${at}.png`) });
        await tapEdge(page, 'dialog[open] [data-ci="-1"]');
        await closed(page);
        check(new URL(page.url()).pathname === '/library', `${at}: one edge tap cancels Add music on the same page`);

        await fx.go(page, '/tracks');
        await openTags(page);
        await measure(page, `${at} Edit tags`);
        await page.screenshot({ path: path.join(SHOTS, `edit-tags-${at}.png`) });
        await page.locator('dialog [data-k="title"]').fill('An unsaved title');
        await tapEdge(page, 'dialog[open] [data-close]');
        await closed(page);
        check(writes(state).length === 0, `${at}: Cancel leaves tags unchanged`);

        await openTags(page);
        await page.locator('dialog [data-k="title"]').fill('A saved title');
        await Promise.all([
          page.waitForResponse((r) => r.url().endsWith('/library/tracks/1/edit') && r.request().method() === 'PUT'),
          tapEdge(page, 'dialog[open] [data-save]'),
        ]);
        await closed(page);
        check(writes(state).length === 1 && JSON.parse(writes(state)[0].body).title === 'A saved title', `${at}: one edge tap saves once`);

        await openTags(page);
        await Promise.all([
          page.waitForResponse((r) => r.url().endsWith('/library/tracks/1/edit') && r.request().method() === 'DELETE'),
          tapEdge(page, 'dialog[open] [data-undo]'),
        ]);
        await closed(page);
        check(writes(state).length === 2 && writes(state)[1].m === 'DELETE', `${at}: one edge tap restores the file's tags once`);

        // Existing text tokens, increased by 30%; wrapping keeps all three actions in view.
        await page.evaluate(() => document.documentElement.style.setProperty('--fs-md', '18.2px'));
        await openTags(page);
        check(await page.locator('dialog [data-save]').evaluate((b) => parseFloat(getComputedStyle(b).fontSize) >= 18.2), `${at}: larger text applied`);
        await measure(page, `${at} larger text`);
        await page.screenshot({ path: path.join(SHOTS, `large-text-${at}.png`) });
        await page.locator('dialog [data-close]').focus();
        await page.keyboard.press('Enter');
        await closed(page);
        check(writes(state).length === 2, `${at}: keyboard Cancel leaves tags unchanged`);
        await openTags(page);
        await page.keyboard.press('Escape');
        await closed(page);
        check(writes(state).length === 2, `${at}: Escape leaves tags unchanged`);

        await page.locator('.track [data-act="more"]').first().tap();
        await page.locator('.popover.menu .pop-item').filter({ hasText: /^Add to (playlist|collection)/ }).tap();
        await measure(page, `${at} collection picker`);
        await tapEdge(page, 'dialog[open] [data-ppclose]');
        await closed(page);
        check(!state.hits.some((h) => h.v.startsWith('/playlists') && h.m !== 'GET'), `${at}: cancelling the picker changes no collections`);

        // A reduced viewport stands in for the space left above a phone keyboard.
        await page.setViewportSize({ width, height: 480 });
        await openTags(page);
        await measure(page, `${at} short viewport`);
        await tapEdge(page, 'dialog[open] [data-close]');
        await closed(page);
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    const { page, context } = await fx.create(browser, 599, { mobile: true });
    try {
      await fx.go(page, '/library');
      await page.locator('#libadd').tap();
      await measure(page, '599 px phone');
      await page.setViewportSize({ width: 600, height: 900 });
      await page.waitForFunction(() => document.documentElement.dataset.shell === 'desktop');
      check(await page.locator('dialog .dialog-actions .btn').evaluate((b) => b.getBoundingClientRect().height < 44), '600 px desktop keeps its compact dialog actions');
      await page.evaluate(() => document.documentElement.dataset.shell = 'phone');
      await measure(page, 'forced phone layout');
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed; ${failed} failed`);
  if (failed) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
