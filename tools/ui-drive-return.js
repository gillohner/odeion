// Back from a cloud drive's sign-in (drives.rs callback: /settings/library?drive=ok|error).
// Before, the page toasted "Drive signed in: add a folder" before anything was drawn and left you
// at the top of Node library; on a phone the drive's Add a folder sat about two screens down,
// under Music folders and WebDAV. A failure's reason was only in that 6 s toast.
// Now, once Node library is drawn: a drive with no folder yet opens its folder picker; one that
// already has folders is scrolled into view; a failure scrolls to the drive's row, which says
// "Last sign-in failed: <why>" until the next Sign in there. Opened on another page (a listener),
// the old message shows.
// Real UI in Chromium against fixtures (no node), phone widths in the browser and the Android app.
//   node tools/ui-drive-return.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/drive-return-shots');
fs.mkdirSync(shots, { recursive: true });

const google = (connected) => ({ id: 'google', name: 'Google Drive', configured: true, connected, user: connected ? 'me@example.com' : null });
const onedrive = (connected) => ({ id: 'onedrive', name: 'OneDrive', configured: true, connected, user: null });
const root = { id: 'r1', provider: 'google', name: 'Music', tracks: 12 };
const base = (drives) => ({
  '/library/folders': { body: [['/srv/music', true]] },
  '/library/webdav': { body: [{ id: 'w1', name: 'NAS', url: 'https://nas.example/dav/Music/', tracks: 40 }] },
  '/library/hidden': { body: [] },
  '/library/drives': { body: drives },
  '/library/drives/google/folders': { body: [{ id: 'f1', name: 'Albums' }, { id: 'f2', name: 'Live' }] },
  '/library/drives/google/login': { body: { url: 'http://odeion-fixture.test/fake-google-login' } },
});
const shown = (page) => page.evaluate(() => { const t = document.querySelector('#toast'); return t.classList.contains('show') ? t.textContent : ''; });
// Fully on screen between the top bar and the mini player
const onScreen = (page, sel) => page.evaluate((s) => {
  const el = document.querySelector(s); if (!el) return 'missing';
  const b = el.getBoundingClientRect();
  const m = document.querySelector('#main').getBoundingClientRect(); // (#main: under the top bar, above the player)
  const mini = document.querySelector('footer.player'); const bottom = Math.min(m.bottom, mini && mini.getBoundingClientRect().top > 0 ? mini.getBoundingClientRect().top : innerHeight);
  // (the library's sticky section chips sit on #main's top)
  const chips = document.querySelector('.libnav, .lib-top, .libtop'); const top = Math.max(m.top, chips && getComputedStyle(chips).position === 'sticky' ? chips.getBoundingClientRect().bottom : m.top);
  return b.top >= top && b.bottom <= bottom ? true : `y ${Math.round(b.top)}–${Math.round(b.bottom)} not within ${Math.round(top)}–${Math.round(bottom)}`;
}, sel);

(async () => {
  const browser = await chromium.launch();
  let fails = 0; let oks = 0;
  const check = async (name, fn) => { try { await fn(); oks++; console.log('ok  ', name); } catch (e) { fails++; console.log('FAIL', name, '-', e.message.split('\n')[0]); } };
  const cases = [[360, false], [412, false], [360, true], [412, true], [1280, false]];
  for (const [width, android] of cases) {
    const tag = `${width}${android ? ' android' : ''}`;
    const file = (n) => path.join(shots, `${n}-${width}${android ? '-android' : ''}.png`);
    const open = async (drives, opts = {}) => {
      const o = await fx.create(browser, width, { mobile: width < 600, fixtures: base(drives), ...opts });
      if (android) await o.context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
      return o;
    };
    const back = async (page, query, provider) => {
      if (provider) await page.addInitScript((p) => { if (!sessionStorage.getItem('pm.seeded')) { sessionStorage.setItem('pm.seeded', '1'); sessionStorage.setItem('pm.driveLogin', p); } }, provider);
      await fx.go(page, '/settings/library?' + query);
      await page.locator('[data-dr-add], [data-dr-login]').first().waitFor();
    };

    await check(`signed in, no folder yet: its folder picker opens (${tag})`, async () => {
      const { page, context, state } = await open({ providers: [google(true), onedrive(false)], roots: [] });
      try {
        await back(page, 'drive=ok', 'google');
        const dlg = page.locator('dialog.dialog[open]');
        await dlg.waitFor({ timeout: 3000 });
        assert.match(await dlg.locator('h2').textContent(), /A folder on Google Drive/);
        await page.locator('[data-dfopen]').first().waitFor();
        await page.screenshot({ path: file('ok-picker') });
        assert.equal(new URL(page.url()).search, '', 'the answer is gone from the address');
        // Picking a folder adds it, as the button's own dialog does
        await page.locator('[data-dfopen]').first().click();
        await page.locator('#dfadd:not([disabled])').waitFor();
        assert.deepEqual(state.errors, [], 'no page errors');
        await page.keyboard.press('Escape');
        await dlg.waitFor({ state: 'detached' });
        assert.equal(await onScreen(page, '[data-dr-add="google"]'), true, "the drive's Add a folder is in view after the picker closes");
        assert.equal(await page.evaluate(() => document.activeElement?.dataset.drAdd), 'google', 'the focus is back on its Add a folder');
      } finally { await context.close(); }
    });

    await check(`signed in again, it has a folder: no picker, its row in view, "Google Drive signed in" (${tag})`, async () => {
      const { page, context, state } = await open({ providers: [google(true)], roots: [root] });
      try {
        await back(page, 'drive=ok', 'google');
        await page.waitForFunction(() => /Google Drive signed in/.test(document.querySelector('#toast.show')?.textContent || ''));
        assert.equal(await page.locator('dialog.dialog[open]').count(), 0, 'no picker');
        assert.equal(await onScreen(page, '[data-dr-add="google"]'), true, 'Add a folder in view');
        assert.equal(await page.evaluate(() => document.activeElement?.dataset.drAdd), 'google', 'the focus is on Add a folder');
        await page.screenshot({ path: file('ok-has-folder') });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });

    await check(`no provider kept (storage off, another tab): the only signed-in drive is used (${tag})`, async () => {
      const { page, context } = await open({ providers: [google(true), onedrive(false)], roots: [] });
      try {
        await back(page, 'drive=ok', null);
        await page.locator('dialog.dialog[open]').waitFor({ timeout: 3000 });
        assert.match(await page.locator('dialog.dialog[open] h2').textContent(), /Google Drive/);
      } finally { await context.close(); }
    });

    await check(`a Sign in left half-way on another drive: the drive in the answer's state is used (${tag})`, async () => {
      const { page, context } = await open({ providers: [google(true), onedrive(false)], roots: [] });
      try {
        await back(page, 'drive=ok', 'onedrive'); // (OneDrive is still signed out: not the one that came back)
        await page.locator('dialog.dialog[open]').waitFor({ timeout: 3000 });
        assert.match(await page.locator('dialog.dialog[open] h2').textContent(), /Google Drive/);
      } finally { await context.close(); }
    });

    await check(`failed with the node's own words: the drive's name once (${tag})`, async () => {
      const { page, context } = await open({ providers: [google(false), onedrive(false)], roots: [] });
      try {
        await back(page, 'drive=error&message=' + encodeURIComponent('Google Drive refused the sign-in: access_denied'), 'google');
        await page.waitForFunction(() => /refused/.test(document.querySelector('#toast.show')?.textContent || ''));
        assert.equal(await shown(page), 'Google Drive refused the sign-in: access_denied');
        assert.equal(await page.locator('.set-row:has([data-dr-login="google"]) [data-drv-failed]').textContent(), 'Last sign-in failed: Google Drive refused the sign-in: access_denied');
      } finally { await context.close(); }
    });

    await check(`failed: the drive's row says why and is in view; the message names the drive (${tag})`, async () => {
      const { page, context, state } = await open({ providers: [google(false), onedrive(false)], roots: [] });
      try {
        await back(page, 'drive=error&message=' + encodeURIComponent('Google said: access_denied'), 'google');
        await page.waitForFunction(() => /Google Drive: Google said: access_denied/.test(document.querySelector('#toast.show')?.textContent || ''));
        const line = page.locator('.set-row:has([data-dr-login="google"]) [data-drv-failed]');
        assert.equal(await line.textContent(), 'Last sign-in failed: Google said: access_denied');
        assert.equal(await page.locator('.set-row:has([data-dr-login="onedrive"]) [data-drv-failed]').count(), 0, 'not on the other drive');
        assert.equal(await onScreen(page, '.set-row:has([data-dr-login="google"]) [data-drv-failed]'), true, 'the reason is in view');
        assert.equal(await onScreen(page, '[data-dr-login="google"]'), true, 'Sign in is in view');
        assert.equal(await page.evaluate(() => document.activeElement?.dataset.drLogin), 'google', 'the focus is on its Sign in');
        assert.equal(await page.locator('dialog.dialog[open]').count(), 0, 'no picker');
        await page.screenshot({ path: file('failed') });
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });

    await check(`failed: the line stays on a redraw; a new Sign in keeps which drive for its return (${tag})`, async () => {
      const { page, context, state } = await open({ providers: [google(false)], roots: [] });
      try {
        await back(page, 'drive=error', 'google');
        const line = page.locator('.set-row:has([data-dr-login="google"]) [data-drv-failed]');
        await line.waitFor();
        assert.equal(await line.textContent(), 'Last sign-in failed: The drive gave no reason');
        assert.match(await shown(page), /Google Drive: The drive gave no reason/);
        // Redraw Node library inside the app (another library tab and back)
        await page.locator('[data-libtab="mine"]').first().click();
        await page.waitForFunction(() => !document.querySelector('[data-dr-login]'));
        await page.locator('[data-libtab="node"]').first().click();
        await page.locator('[data-dr-login="google"]').waitFor();
        assert.equal(await line.count(), 1, 'kept on a redraw');
        // A new try clears it (the fixture's sign-in page is our own origin's 404)
        await page.locator('[data-dr-login="google"]').click();
        await page.waitForURL(/fake-google-login/);
        assert.equal(await page.evaluate(() => sessionStorage.getItem('pm.driveLogin')), 'google', 'the drive whose Sign in was pressed is kept for the return');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });

    await check(`opened on another page (a listener): the old message, nothing else (${tag})`, async () => {
      const { page, context } = await open({ providers: [google(true)], roots: [] }, { role: 'listener' });
      try {
        await fx.go(page, '/settings/library?drive=ok');
        await page.waitForFunction(() => /Drive signed in: add a folder/.test(document.querySelector('#toast.show')?.textContent || ''));
        assert.equal(await page.locator('dialog.dialog[open]').count(), 0, 'no picker');
      } finally { await context.close(); }
    });
  }
  await browser.close();
  console.log(`${oks} ok, ${fails} failed`);
  process.exit(fails ? 1 : 0);
})();
