// Phone dialog actions fit inside their sheet and have at least 44 px touch targets; so do the
// items of a row's ⋯ sheet and the rows of the collection picker.
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

// A stack of flush rows (a menu sheet's items, the collection picker's rows): each is at least
// 44 px tall, and nothing (such as a neighbour's tap reach) covers its top or bottom edge. Each row
// is scrolled into its own list first (on a short screen the sheet scrolls).
async function measureRows(page, selector, label) {
  await page.locator(selector).first().waitFor();
  const m = await page.evaluate((selector) => {
    const rows = [...document.querySelectorAll(selector)].filter((r) => r.offsetParent);
    const out = rows.map((r) => {
      r.scrollIntoView({ block: 'nearest' });
      const b = r.getBoundingClientRect();
      return { h: b.height, hit: [b.top + 1, b.bottom - 1].every((y) => r.contains(document.elementFromPoint(b.left + b.width / 2, y))) };
    });
    return {
      sizes: out.map((o) => o.h.toFixed(1)).join(', '),
      tall: out.length > 0 && out.every((o) => o.h >= 44),
      hit: out.length > 0 && out.every((o) => o.hit),
      overflow: document.documentElement.scrollWidth > innerWidth,
    };
  }, selector);
  check(m.tall, `${label}: every row at least 44 px tall (${m.sizes})`);
  check(m.hit, `${label}: each row's top and bottom edge are its own`);
  check(!m.overflow, `${label}: no horizontal overflow`);
}

// Read geometry as it is. In particular, no locator action may scroll the target
// between a resize/reflow and this measurement (that would conceal a clipped focus).
const focusedAction = (page) => page.locator('dialog[open]').evaluate((dialog) => {
  const action = document.activeElement, b = action.getBoundingClientRect(), d = dialog.getBoundingClientRect();
  const scale = d.height / dialog.offsetHeight;
  const top = Math.max(0, d.top + dialog.clientTop * scale), bottom = Math.min(innerHeight, d.bottom - dialog.clientTop * scale);
  return {
    action: ['save', 'close', 'undo'].find((key) => action.hasAttribute(`data-${key}`)),
    visible: b.top >= Math.max(0, top) && b.bottom <= Math.min(innerHeight, bottom)
      && b.left >= d.left && b.right <= d.right,
    outline: b.top - 4 * scale >= top && b.bottom + 4 * scale <= bottom,
    hit: action.contains(document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2)),
    scroll: dialog.scrollTop, mainScroll: document.querySelector('#main').scrollTop,
    overflow: document.documentElement.scrollWidth > innerWidth || dialog.scrollWidth > dialog.clientWidth,
    rect: `${b.top.toFixed(1)}–${b.bottom.toFixed(1)} in ${top.toFixed(1)}–${bottom.toFixed(1)}`,
  };
});
const settle = (page) => page.waitForTimeout(200);
async function focusAction(page, action) {
  // Start at Edit tags' initial Title focus and use the actual keyboard tab order.
  for (let i = 0; i < { undo: 4, close: 5, save: 6 }[action]; i++) await page.keyboard.press('Tab');
  check((await focusedAction(page)).action === action, `Tab reaches ${action}`);
}
function visibleFocus(m, action, label) {
  check(m.action === action && m.visible && m.hit, `${label}: ${action} keeps visible, hit-testable focus (${m.rect})`);
  check(!m.overflow, `${label}: no horizontal overflow`);
}
async function auditFocusedReflow(browser) {
  for (const android of [false, true]) {
    const { page, context, state } = await fx.create(browser, 412, { mobile: true, fixtures });
    try {
      await context.addInitScript((android) => {
        localStorage.setItem('pm.theme', android ? 'dark' : 'light');
        if (android) window.pmDesktop = { platform: 'android', version: 'fixture' };
      }, android);
      await fx.go(page, '/tracks');
      const mode = android ? 'android' : 'browser';
      for (const [from, to] of [
        [[412, 480], [360, 480]], [[599, 480], [412, 480]],
        [[360, 900], [360, 480]], [[360, 900], [412, 480]],
        [[600, 480], [599, 480]], [[360, 480], [320, 480]],
      ]) {
        const label = `${mode} ${from.join('×')} → ${to.join('×')}`;
        await page.setViewportSize({ width: from[0], height: from[1] });
        await openTags(page); await page.locator('dialog [data-k="title"]').fill(label);
        await focusAction(page, 'save');
        const before = await focusedAction(page);
        visibleFocus(before, 'save', label + ' before');
        await page.setViewportSize({ width: to[0], height: to[1] }); await settle(page);
        const after = await focusedAction(page);
        visibleFocus(after, 'save', label + ' after');
        check(after.mainScroll === before.mainScroll, `${label}: the page behind the dialog stays put`);
        await page.screenshot({ path: path.join(SHOTS, `focus-${mode}-${from.join('x')}-to-${to.join('x')}.png`) });
        const previous = writes(state).length;
        await page.keyboard.press('Enter'); await closed(page);
        check(writes(state).length === previous + 1 && JSON.parse(writes(state).at(-1).body).title === label, `${label}: Enter saves once`);
      }
      for (const action of ['save', 'close', 'undo']) {
        await page.setViewportSize({ width: 412, height: 480 }); await openTags(page);
        await focusAction(page, action);
        await page.setViewportSize({ width: 360, height: 480 }); await settle(page);
        visibleFocus(await focusedAction(page), action, `${mode} ${action} after resize`);
        for (const font of ['18.2px', '28px']) {
          await page.evaluate((font) => document.documentElement.style.setProperty('--fs-md', font), font);
          await settle(page);
          visibleFocus(await focusedAction(page), action, `${mode} ${action} after text ${font}`);
        }
        // A person can scroll back to the fields without an unrelated redraw snapping back.
        const manual = await page.locator('dialog').evaluate((d) => { d.scrollTop = 0; d.querySelector('h2').dataset.audit = 'unrelated'; return d.scrollTop; });
        await settle(page);
        check((await focusedAction(page)).scroll === manual, `${mode} ${action}: unrelated updates preserve manual scrolling`);
        // Move focus out of the footer, then reflow: the observer must leave the field alone.
        for (let i = 0; i < { undo: 4, close: 5, save: 6 }[action]; i++) await page.keyboard.press('Shift+Tab');
        await page.evaluate(() => document.documentElement.style.removeProperty('--fs-md')); await settle(page);
        check(await page.locator('dialog [data-k="title"]').evaluate((e) => e === document.activeElement), `${mode}: text input keeps focus after reflow`);
        const previous = writes(state).length;
        await page.keyboard.press('Escape'); await closed(page);
        check(writes(state).length === previous, `${mode}: Escape after reflow changes no tags`);
      }
      // Closing with a pending resize must not act on a stale button or steal focus.
      await openTags(page); await focusAction(page, 'save');
      await page.evaluate(() => { dispatchEvent(new Event('resize')); document.querySelector('dialog [data-close]').click(); });
      await closed(page); await settle(page);

      // Pause the real opening animation so this edge cannot pass just because a slow
      // runner has already finished animating. Measure both during and after its scale.
      await page.emulateMedia({ reducedMotion: 'no-preference' });
      const pause = await page.addStyleTag({ content: '.dialog[open] { animation-play-state: paused; }' });
      await page.setViewportSize({ width: 412, height: 480 }); await openTags(page);
      await focusAction(page, 'save');
      check(await page.locator('dialog').evaluate((d) => getComputedStyle(d).transform !== 'none'), `${mode}: opening animation is active`);
      await page.setViewportSize({ width: 360, height: 480 }); await settle(page);
      const during = await focusedAction(page);
      check(during.action === 'save' && during.visible && during.outline && during.hit, `${mode}: resize during opening keeps the entire focused Save and its outline visible`);
      await page.locator('dialog').evaluate((d) => d.getAnimations().forEach((a) => a.finish())); await settle(page);
      const after = await focusedAction(page);
      check(after.action === 'save' && after.visible && after.outline && after.hit, `${mode}: Save and its focus outline stay visible when the opening animation ends`);
      await page.screenshot({ path: path.join(SHOTS, `focus-${mode}-after-opening.png`) });
      await page.keyboard.press('Escape'); await closed(page); await pause.evaluate((e) => e.remove());
      check(!state.errors.length, `${mode}: repeated reflows and close have no page errors (${state.errors.join('; ')})`);
    } finally { await context.close(); }
  }
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
        // At the default text size: the row's ⋯ sheet, then the collection picker it opens
        // (a fresh install's picker has one row, New playlist)
        await page.locator('.track [data-act="more"]').first().tap();
        await measureRows(page, '.popover.menu.sheet:not([hidden]) .pop-item', `${at} row menu sheet`);
        await page.locator('.popover.menu .pop-item').filter({ hasText: /^Add to (playlist|collection)/ }).tap();
        await measureRows(page, 'dialog[open] #pplist .list-row', `${at} collection picker rows`);
        await page.screenshot({ path: path.join(SHOTS, `picker-${at}.png`) });
        await page.locator('dialog[open] [data-ppclose]').tap();
        await closed(page);

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
        // This layout check explicitly scrolls the footer; focus/reflow checks below never do.
        await page.locator('dialog[open] .dialog-actions').scrollIntoViewIfNeeded();
        await measure(page, `${at} short viewport`);
        await tapEdge(page, 'dialog[open] [data-close]');
        await closed(page);
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    await auditFocusedReflow(browser);
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
    // A short phone screen: the row's ⋯ sheet scrolls, and every item is still 44 px and its own
    for (const android of [false, true]) {
      const short = await fx.create(browser, 360, { mobile: true });
      try {
        await short.context.addInitScript((a) => { if (a) window.pmDesktop = { platform: 'android', version: 'fixture' }; }, android);
        await short.page.setViewportSize({ width: 360, height: 568 });
        await fx.go(short.page, '/tracks');
        await short.page.locator('.track [data-act="more"]').first().tap();
        const label = `${android ? 'android' : 'browser'}-360x568 row menu sheet`;
        await measureRows(short.page, '.popover.menu.sheet:not([hidden]) .pop-item', label);
        check(await short.page.locator('.popover.menu.sheet:not([hidden])').evaluate((s) => s.scrollHeight > s.clientHeight && s.scrollTop > 0), `${label}: scrolls to reach its last item`);
      } finally { await short.context.close(); }
    }
    // A mouse window keeps its compact menu and picker rows
    const desk = await fx.create(browser, 1280, { collections: [fx.summary] });
    try {
      await fx.go(desk.page, '/tracks');
      await desk.page.locator('.track').first().hover();
      await desk.page.locator('.track [data-act="more"]').first().click();
      const item = desk.page.locator('.popover.menu:not([hidden]) .pop-item').first();
      await item.waitFor();
      check(await item.evaluate((b) => !b.closest('.popover').classList.contains('sheet') && b.getBoundingClientRect().height < 44), '1280 px desktop keeps its compact menu items');
      await desk.page.locator('.popover.menu .pop-item').filter({ hasText: /^Add to (playlist|collection)/ }).click();
      check(await desk.page.locator('dialog[open] #pplist .list-row').first().evaluate((r) => r.getBoundingClientRect().height < 44), '1280 px desktop keeps its compact picker rows');
    } finally { await desk.context.close(); }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed; ${failed} failed`);
  if (failed) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
