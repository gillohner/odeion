// Log out has a time limit, and says the node's reason with one end mark.
// 1. A node that answers nothing (neither the owner's renewal question, asked first, nor the
//    session end): after 15 s in all the page says "The node didn’t answer. You are still logged
//    in." with Try again, which asks again with the same limit (before, Log out waited for good).
// 2. A reason that ends in ?, ! or … gets no full stop after it ("Is the disk full? You are …").
// 3. A node that never answers the renewal question but ends the session: after 5 s Log out goes
//    on without the code, and the page starts over at Log in.
// The waits are skipped with Playwright's clock. Checks at 360, 360 in the Android app, 412 and
// 1280 px; no browser dialog, no page errors.
//   node tools/ui-logout-limit.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'logout-limit-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

(async () => {
  const browser = await chromium.launch();
  for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
    const at = `${width}${android ? '-android' : ''}`;
    const phone = width < 600;
    // what the node does: 'hang' (answers nothing), 'renewal' (never answers the renewal
    // question, then ends the session), or a reason it refuses the session end with
    let mode = 'hang'; const deletes = []; const renewals = [];
    const { context, page, state } = await fx.create(browser, width, { mobile: phone });
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    const native = []; page.on('dialog', (d) => { native.push(d.message()); d.dismiss().catch(() => {}); });
    await page.route(/\/v1\/me\/token$/, (r) => {
      if (r.request().method() !== 'DELETE') return r.fallback();
      deletes.push(mode);
      if (mode === 'hang') return; // never answered
      if (mode === 'renewal') return r.fulfill({ status: 204, body: '' });
      return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: mode }) });
    });
    await page.route(/\/v1\/auth\/pubky\/renewal$/, (r) => {
      renewals.push(mode);
      if (mode === 'renewal' || mode === 'hang') return; // never answered
      return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ renewal: null }) });
    });
    await page.clock.install();
    await fx.go(page, '/');
    const ready = await page.waitForSelector('#me', { state: 'visible', timeout: 10000 }).then(() => true).catch(() => false);
    check(ready, `${at}: the account button is there`);
    if (!ready) { await context.close(); continue; }
    await page.evaluate(() => { window.__sameLoad = true; });
    const dlg = page.locator('dialog.dialog[open]');
    const logOut = async () => {
      await page.click('#me');
      await page.locator('.popover.menu .pop-item', { hasText: 'Log out' }).click();
    };
    const asked = (ms = 1500) => dlg.waitFor({ timeout: ms }).then(() => true).catch(() => false);
    const stillIn = async () => !(await page.isVisible('#loginpage')) && await page.evaluate(() => !!localStorage.getItem('pm.token') && window.__sameLoad === true);

    const until = async (fn) => { for (let i = 0; i < 30 && !fn(); i++) await page.waitForTimeout(100); return fn(); };
    // 1. The node answers nothing: 15 s in all (the renewal question's 5, then the session end's)
    const noAnswer = async (what) => {
      const asks = deletes.length;
      await page.clock.fastForward(4000); await page.waitForTimeout(200);
      check(deletes.length === asks, `${at}: ${what}: 4 s on it still waits for the renewal code (${deletes.length - asks} session ends asked)`);
      await page.clock.fastForward(1500);
      check(await until(() => deletes.length === asks + 1), `${at}: ${what}: after 5 s it goes on without the code and asks the node to end the session (${deletes.length - asks})`);
      await page.clock.fastForward(8500);
      check(!(await asked(300)), `${at}: ${what}: 14 s on it still waits for the node`);
      await page.clock.fastForward(1500);
      const late = await asked(3000);
      check(late, `${at}: ${what}: after 15 s with no answer it asks in the page's dialog`);
      if (!late) return false;
      const d = await dlg.evaluate((e) => ({ title: e.querySelector('h2').textContent, text: (e.querySelector('p') || {}).textContent || '', go: [...e.querySelectorAll('.choice')].map((b) => b.textContent.trim()) }));
      check(d.title === 'Couldn’t log out' && d.text === 'The node didn’t answer. You are still logged in.' && d.go.join() === 'Try again', `${at}: ${what}: "Couldn’t log out", that the node didn't answer, still logged in, Try again (${d.title} | ${d.text} | ${d.go})`);
      return true;
    };
    await logOut();
    check(await until(() => renewals.length === 1), `${at}: the owner's Log out asks for the renewal code first (${renewals.length} asks)`);
    if (await noAnswer('Log out')) {
      await page.screenshot({ path: path.join(SHOTS, `${at}-no-answer.png`) });
      // Try again: the same limit once more
      const r0 = renewals.length;
      await dlg.locator('.choice').click();
      check(await until(() => renewals.length === r0 + 1) && await dlg.count() === 0, `${at}: Try again asks the node again (${renewals.length - r0} renewal asks)`);
      if (await noAnswer('Try again')) {
        await page.keyboard.press('Escape'); await page.waitForTimeout(200);
        check(await dlg.count() === 0 && await stillIn(), `${at}: Escape leaves the person logged in, on the same page`);
      }
    }

    // 2. A reason ending in its own mark gets no full stop after it
    for (const [why, want] of [['is the disk full?', 'Is the disk full? You are still logged in.'], ['Not allowed!', 'Not allowed! You are still logged in.'], ['still saving…', 'Still saving… You are still logged in.'], ['database is locked...', 'Database is locked. You are still logged in.']]) {
      mode = why;
      await logOut();
      if (await asked(3000)) {
        const text = await dlg.locator('p').textContent();
        check(text === want, `${at}: "${why}" reads "${want}" (${text})`);
        if (why.endsWith('?')) await page.screenshot({ path: path.join(SHOTS, `${at}-question.png`) });
        await page.keyboard.press('Escape'); await page.waitForTimeout(200);
      } else check(false, `${at}: a refusal ("${why}") asks in the page's dialog`);
    }

    // 3. The renewal question is never answered: after 5 s Log out goes on, and ends the session
    mode = 'renewal'; const before = deletes.length;
    await logOut();
    check(await until(() => renewals.includes('renewal')) && deletes.length === before, `${at}: the renewal question is asked, the session end not yet (${deletes.length - before} ends)`);
    const nav = page.waitForEvent('load', { timeout: 6000 }).then(() => true).catch(() => false);
    await page.clock.fastForward(5500);
    check(await nav && deletes.length === before + 1, `${at}: after 5 s with no renewal answer it ends the session, and the page starts over at Log in (${deletes.length - before} ends)`);

    check(native.length === 0, `${at}: never a browser box (${JSON.stringify(native)})`);
    check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
