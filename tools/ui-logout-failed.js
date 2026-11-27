// A Log out the node didn't confirm (it answered with an error, or not at all) asks in the page's
// own dialog, as every other question in the app: "Couldn’t log out", why, "You are still logged
// in.", Try again (the keyboard on it) next to Cancel. Before, it was the browser's own confirm()
// box (on Android a system pop-up, unlike the rest of the app), the last one left.
// Cancel and Escape leave the person logged in, on the page (asked from the keyboard: the
// keyboard back on the account button); Try again asks the node again, and once it ends the
// session the page starts over at Log in.
// Checks at 360, 360 in the Android app, 412 and 1280 px; no browser dialog, no horizontal
// overflow, no page errors.
//   node tools/ui-logout-failed.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'logout-failed-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

(async () => {
  // It was the last browser box: the app asks every question in its own dialog
  const ROOT = path.resolve(process.argv[2] || path.join(__dirname, '..'));
  const src = String(require('./ui-src').read(path.join(ROOT, 'crates/pm-web/static'), 'app.js'));
  const boxes = src.split('\n').filter((l) => /(^|[^\w.])(window\.)?(confirm|alert|prompt)\(/.test(l.replace(/\/\/.*$/, ''))).map((l) => l.trim().slice(0, 80));
  check(boxes.length === 0, `app.js calls no confirm(), alert() or prompt() (${JSON.stringify(boxes)})`);
  const browser = await chromium.launch();
  for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
    const at = `${width}${android ? '-android' : ''}`;
    const phone = width < 600;
    const { context, page, state } = await fx.create(browser, width, { mobile: phone });
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    const native = []; page.on('dialog', (d) => { native.push(d.message()); d.dismiss().catch(() => {}); });
    // The node refuses four times, then ends the session
    const deletes = []; let refusals = 4;
    await page.route(/\/v1\/me\/token$/, (r) => {
      if (r.request().method() !== 'DELETE') return r.fallback();
      deletes.push(Date.now());
      // (the second refusal's reason is a whole sentence, as many of the node's are)
      const error = deletes.length === 2 ? 'Something went wrong on the node. Try again.' : 'database is locked';
      return refusals-- > 0 ? r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error }) }) : r.fulfill({ status: 204, body: '' });
    });
    await page.route(/\/v1\/auth\/pubky\/renewal$/, (r) => r.fulfill({ contentType: 'application/json', body: JSON.stringify({ renewal: null }) }));
    await fx.go(page, '/');
    const ready = await page.waitForSelector('#me', { state: 'visible', timeout: 10000 }).then(() => true).catch(() => false);
    check(ready, `${at}: the account button is there`);
    if (!ready) { await context.close(); continue; }
    await page.evaluate(() => { window.__sameLoad = true; });
    const dlg = page.locator('dialog.dialog[open]');
    // By mouse, or by keyboard (Enter on the account button, then on its Log out)
    const logOut = async (keys) => {
      if (keys) { await page.focus('#me'); await page.keyboard.press('Enter'); await page.locator('.popover.menu .pop-item', { hasText: 'Log out' }).focus(); await page.keyboard.press('Enter'); } else {
        await page.click('#me');
        await page.locator('.popover.menu .pop-item', { hasText: 'Log out' }).click();
      }
      return dlg.waitFor({ timeout: 4000 }).then(() => true).catch(() => false);
    };
    const stillIn = async () => !(await page.isVisible('#loginpage')) && await page.evaluate(() => !!localStorage.getItem('pm.token') && window.__sameLoad === true);

    // 1. The question (asked from the keyboard), then Cancel
    let opened = await logOut(true);
    await page.waitForTimeout(150);
    check(native.length === 0, `${at}: no browser confirm box (${JSON.stringify(native)})`);
    check(opened, `${at}: a failed Log out asks in the page's dialog`);
    if (opened) {
      const d = await dlg.evaluate((e) => {
        const go = [...e.querySelectorAll('.choice')]; const cancel = e.querySelector('[data-ci="-1"]');
        const box = (b) => { const r = b.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, h: r.height }; };
        return { title: e.querySelector('h2').textContent, text: (e.querySelector('p') || {}).textContent || '', go: go.map((b) => ({ label: b.textContent.trim(), primary: b.classList.contains('primary'), danger: b.classList.contains('danger'), ...box(b) })), cancel: cancel && { label: cancel.textContent.trim(), ...box(cancel) }, focus: document.activeElement === go[0], vh: innerHeight };
      });
      check(d.title === 'Couldn’t log out', `${at}: it says "Couldn’t log out" (${d.title})`);
      check(d.text === 'Database is locked. You are still logged in.', `${at}: why, and that the person is still logged in (${d.text})`);
      check(d.go.length === 1 && d.go[0].label === 'Try again' && d.go[0].primary && !d.go[0].danger, `${at}: one choice, Try again, the main one, not drawn as deleting (${JSON.stringify(d.go.map((g) => [g.label, g.primary, g.danger]))})`);
      check(d.cancel && d.cancel.label === 'Cancel', `${at}: with Cancel`);
      check(d.focus, `${at}: the keyboard is on Try again`);
      check(d.go.concat(d.cancel || []).every((b) => b.top >= 0 && b.bottom <= d.vh), `${at}: Try again and Cancel are on screen`);
      if (phone) check(d.go.concat(d.cancel || []).every((b) => b.h >= 44), `${at}: Try again and Cancel are 44 px tall or more (${d.go.concat(d.cancel || []).map((b) => b.h).join(', ')})`);
      await page.screenshot({ path: path.join(SHOTS, `${at}-ask.png`) });
      await dlg.locator('[data-ci="-1"]').focus(); await page.keyboard.press('Enter');
      await page.waitForTimeout(250);
      check(await dlg.count() === 0 && deletes.length === 1, `${at}: Cancel closes it and asks the node nothing more (${deletes.length} asks)`);
      check(await stillIn(), `${at}: and the person is still logged in, on the same page`);
      check(await page.evaluate(() => document.activeElement === document.querySelector('#me')), `${at}: and the keyboard is back on the account button (${await page.evaluate(() => document.activeElement && (document.activeElement.id || document.activeElement.tagName))})`);
    }

    // 2. Escape: the same as Cancel
    opened = await logOut();
    check(opened, `${at}: a second failed Log out asks again`);
    if (opened) {
      const text = await dlg.locator('p').textContent();
      check(text === 'Something went wrong on the node. Try again. You are still logged in.', `${at}: a reason that is a sentence keeps one full stop (${text})`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(250);
      check(await dlg.count() === 0 && deletes.length === 2, `${at}: Escape closes it and asks the node nothing more (${deletes.length} asks)`);
      check(await stillIn(), `${at}: and the person is still logged in`);
    }

    // 3. Try again: refused once more (asks again), then the node ends the session: Log in
    opened = await logOut();
    if (opened) {
      await dlg.locator('.choice').click();
      const again = await page.waitForFunction(() => document.querySelector('dialog.dialog[open]'), null, { timeout: 4000 }).then(() => true).catch(() => false);
      check(again && deletes.length === 4 && await page.evaluate(() => window.__sameLoad === true), `${at}: Try again asks the node again, and a refusal asks once more (${deletes.length} asks)`);
      const nav = page.waitForEvent('load', { timeout: 6000 }).then(() => true).catch(() => false);
      await dlg.locator('.choice').click();
      check(await nav && deletes.length === 5, `${at}: once the node ends the session, the page starts over at Log in (${deletes.length} asks)`);
    } else check(false, `${at}: a third failed Log out asks again`);
    check(native.length === 0, `${at}: never a browser box (${native.length})`);
    await page.waitForTimeout(300);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth).catch(() => 0);
    check(overflow <= 0, `${at}: no horizontal overflow (${overflow})`);
    check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
