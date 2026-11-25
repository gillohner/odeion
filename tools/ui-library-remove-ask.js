// Your library › Node library: the three Remove buttons (a music folder, a WebDAV share, a cloud
// drive's folder) ask the same way as every other removal in the app: the page's own dialog, which
// names what goes, says the files stay, and has Remove drawn as a button that deletes (danger) next
// to Cancel. Before, the share and the drive folder asked with the browser's own confirm() box (on
// Android a system pop-up, unlike the rest of the app) and the music folder turned its own button
// into "Really remove?" for 3 s, so a second tap at the same spot removed it: three ways for one job.
// Cancel (and Escape) sends nothing and leaves the keyboard on that Remove; Remove sends the one
// DELETE for that item and says what went.
// Checks at 360, 360 in the Android app, 412 and 1280 px; no browser dialog, no horizontal overflow,
// no page errors.
//   node tools/ui-library-remove-ask.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'library-remove-ask-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const google = { id: 'google', name: 'Google Drive', configured: true, connected: true, user: 'me@example.com' };
const fixtures = {
  '/library/folders': { body: [['/srv/music', true]] },
  '/library/webdav': { body: [{ id: 'w1', name: 'Nextcloud', url: 'https://cloud.example.com/remote.php/dav/files/me/Music', tracks: 120 }] },
  '/library/drives': { body: { providers: [google], roots: [{ id: 'r1', name: 'Albums', provider: 'google', tracks: 40 }] } },
  '/library/hidden': { body: [] },
};
// Each Remove: its button, what the dialog says, the DELETE it sends, the toast after
const KINDS = [
  { key: 'folder', btn: '[data-folder-del="/srv/music"]', title: 'Remove this folder?', name: '/srv/music', files: /files in it stay/, del: '/v1/library/folders?path=%2Fsrv%2Fmusic', answer: { removed_tracks: 12 }, toast: /12 songs/ },
  { key: 'share', btn: '[data-wd-del="w1"]', title: 'Remove this share?', name: 'Nextcloud', files: /files on the share stay/, del: '/v1/library/webdav/w1', answer: { tracks: 120 }, toast: /120 songs/ },
  { key: 'drive', btn: '[data-dr-del="r1"]', title: 'Remove this drive folder?', name: 'Albums', files: /files on the drive stay/, del: '/v1/library/drives/roots/r1', answer: { tracks: 40 }, toast: /40 songs/ },
];

(async () => {
  const browser = await chromium.launch();
  for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
    const tag = `${width}${android ? '-android' : ''}`;
    const phone = width < 600;
    for (const k of KINDS) {
      const { context, page, state } = await fx.create(browser, width, { mobile: phone, fixtures });
      if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
      const native = []; page.on('dialog', (d) => { native.push(d.message()); d.dismiss().catch(() => {}); });
      const deletes = [];
      await page.route(/\/v1\/library\/(folders|webdav|drives\/roots)(\/|\?|$)/, (r) => {
        if (r.request().method() !== 'DELETE') return r.fallback();
        const u = new URL(r.request().url()); deletes.push(u.pathname + u.search);
        return r.fulfill({ contentType: 'application/json', body: JSON.stringify(k.answer) });
      });
      await fx.go(page, '/library/node');
      const at = `${tag} ${k.key}`;
      const ready = await page.waitForSelector(k.btn, { timeout: 10000 }).then(() => true).catch(() => false);
      check(ready, `${at}: Node library shows its Remove`);
      if (!ready) { await context.close(); continue; }
      await page.locator(k.btn).scrollIntoViewIfNeeded();
      await page.locator(k.btn).click();
      const label = await page.locator(k.btn).textContent().then((t) => t.trim());
      const dlg = page.locator('dialog.dialog[open]');
      const opened = await dlg.waitFor({ timeout: 3000 }).then(() => true).catch(() => false);
      await page.waitForTimeout(150);
      check(native.length === 0, `${at}: no browser confirm box (${JSON.stringify(native)})`);
      check(opened, `${at}: Remove asks in the page's dialog`);
      check(deletes.length === 0, `${at}: nothing is removed before the answer (${deletes.join(', ')})`);
      check(label === 'Remove', `${at}: the row's button still says Remove (${label})`);
      if (opened) {
        const d = await dlg.evaluate((e) => {
          const go = [...e.querySelectorAll('.choice')]; const cancel = e.querySelector('[data-ci="-1"]');
          const box = (b) => { const r = b.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, h: r.height, w: r.width }; };
          return { title: e.querySelector('h2').textContent, text: (e.querySelector('p') || {}).textContent || '', go: go.map((b) => ({ label: b.textContent.trim(), danger: b.classList.contains('danger'), ...box(b) })), cancel: cancel && { label: cancel.textContent.trim(), ...box(cancel) }, focus: document.activeElement === go[0], vh: innerHeight };
        });
        check(d.title === k.title, `${at}: the dialog asks "${k.title}" (${d.title})`);
        check(d.text.includes(`\u201c${k.name}\u201d`), `${at}: and names it (${d.text})`);
        check(k.files.test(d.text), `${at}: and says the files stay (${d.text})`);
        check(d.go.length === 1 && d.go[0].label === 'Remove' && d.go[0].danger, `${at}: one choice, Remove, drawn as deleting (${JSON.stringify(d.go.map((g) => [g.label, g.danger]))})`);
        check(d.cancel && d.cancel.label === 'Cancel', `${at}: with Cancel`);
        check(d.focus, `${at}: the keyboard is on Remove`);
        check(d.go.concat(d.cancel || []).every((b) => b.top >= 0 && b.bottom <= d.vh), `${at}: Remove and Cancel are on screen`);
        if (phone) check(d.go.concat(d.cancel || []).every((b) => b.h >= 44), `${at}: Remove and Cancel are 44 px tall or more (${d.go.concat(d.cancel || []).map((b) => b.h).join(', ')})`);
        await page.screenshot({ path: path.join(SHOTS, `${tag}-${k.key}-ask.png`) });
        // Cancel: nothing sent, the keyboard back on the row's Remove
        await dlg.locator('[data-ci="-1"]').click();
        await page.waitForTimeout(200);
        check(await dlg.count() === 0 && deletes.length === 0, `${at}: Cancel closes it and removes nothing`);
        check(await page.evaluate((s) => document.activeElement === document.querySelector(s), k.btn), `${at}: and the keyboard is on that Remove again`);
        // Escape: the same
        await page.locator(k.btn).click(); await dlg.waitFor({ timeout: 3000 }).catch(() => {});
        await page.keyboard.press('Escape'); await page.waitForTimeout(200);
        check(await dlg.count() === 0 && deletes.length === 0, `${at}: Escape closes it and removes nothing`);
        check(await page.evaluate((s) => document.activeElement === document.querySelector(s), k.btn), `${at}: and the keyboard is on that Remove again`);
        // Remove: the one DELETE, then the toast
        await page.locator(k.btn).click(); await dlg.waitFor({ timeout: 3000 }).catch(() => {});
        await dlg.locator('.choice').click();
        const said = await page.waitForFunction((re) => [...document.querySelectorAll('.toast')].some((t) => new RegExp(re).test(t.textContent)), k.toast.source, { timeout: 3000 }).then(() => true).catch(() => false);
        check(deletes.length === 1 && deletes[0] === k.del, `${at}: Remove sends the one DELETE (${deletes.join(', ')})`);
        check(said, `${at}: and says what went`);
      }
      check(native.length === 0, `${at}: no browser dialog all along`);
      const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      check(over <= 0, `${at}: no horizontal overflow (${over})`);
      check(state.errors.length === 0, `${at}: no page errors (${state.errors.join('; ')})`);
      await context.close();
    }
  }
  // A long path or share name (one word, no spaces) wraps inside the dialog
  {
    const LONG = '/srv/media/storage/Music_Collection_Electronic_Ambient_Downtempo_2019_FLAC_Remasters';
    const SHARE = 'Nextcloud_Family_Archive_Music_Collection_Lossless_Backup';
    const long = { ...fixtures, '/library/folders': { body: [[LONG, true]] }, '/library/webdav': { body: [{ id: 'w2', name: SHARE, url: 'https://cloud.example.com/dav', tracks: 3 }] } };
    for (const [key, btn] of [['folder', `[data-folder-del="${LONG}"]`], ['share', '[data-wd-del="w2"]']]) {
      const { context, page, state } = await fx.create(browser, 360, { mobile: true, fixtures: long });
      await fx.go(page, '/library/node');
      await page.waitForSelector(btn, { timeout: 10000 });
      await page.locator(btn).scrollIntoViewIfNeeded(); await page.locator(btn).click();
      const dlg = page.locator('dialog.dialog[open]'); await dlg.waitFor({ timeout: 3000 }).catch(() => {});
      const m = await dlg.evaluate((e) => ({ d: e.scrollWidth - e.clientWidth, h: e.querySelector('h2').scrollWidth - e.querySelector('h2').clientWidth, p: e.querySelector('p').scrollWidth - e.querySelector('p').clientWidth, text: e.querySelector('p').textContent })).catch(() => null);
      check(m && m.d <= 0 && m.h <= 0 && m.p <= 0, `360 long ${key}: its name wraps inside the dialog (${JSON.stringify(m)})`);
      check(m && m.text.includes(key === 'folder' ? LONG : SHARE), `360 long ${key}: the whole name is said`);
      await page.screenshot({ path: path.join(SHOTS, `360-long-${key}-ask.png`) });
      check(state.errors.length === 0, `360 long ${key}: no page errors (${state.errors.join('; ')})`);
      await context.close();
    }
  }
  // The page draws again while the dialog is open (every 3 s while a scan runs): Cancel puts the
  // keyboard on the same Remove in the new rows; Remove still removes that folder
  for (const how of ['cancel', 'remove']) {
    const scanning = { ...fixtures, '/info': { body: { name: 'Fixture node', offline: false, scanning: true, library: { tracks: 5, albums: 1, artists: 1, files: 5, missing_files: 0 } } } };
    const { context, page, state } = await fx.create(browser, 360, { mobile: true, fixtures: scanning });
    const deletes = [];
    await page.route(/\/v1\/library\/folders\?/, (r) => { if (r.request().method() !== 'DELETE') return r.fallback(); const u = new URL(r.request().url()); deletes.push(u.pathname + u.search); return r.fulfill({ contentType: 'application/json', body: '{"removed_tracks":12}' }); });
    await fx.go(page, '/library/node');
    const btn = KINDS[0].btn;
    await page.waitForSelector(btn, { timeout: 10000 });
    await page.evaluate((s) => { window.oldRemove = document.querySelector(s); }, btn);
    await page.locator(btn).scrollIntoViewIfNeeded(); await page.locator(btn).click();
    const dlg = page.locator('dialog.dialog[open]'); await dlg.waitFor({ timeout: 3000 }).catch(() => {});
    const redrawn = await page.waitForFunction(() => !window.oldRemove.isConnected, null, { timeout: 8000 }).then(() => true).catch(() => false);
    check(redrawn && await dlg.count() === 1, `360 scanning ${how}: the page drew again under the open dialog`);
    if (how === 'cancel') {
      await dlg.locator('[data-ci="-1"]').click(); await page.waitForTimeout(200);
      check(deletes.length === 0 && await page.evaluate((s) => document.activeElement === document.querySelector(s), btn), `360 scanning: Cancel puts the keyboard on the new rows' Remove (${await page.evaluate(() => document.activeElement && document.activeElement.outerHTML.slice(0, 60))})`);
    } else {
      await dlg.locator('.choice').click(); await page.waitForTimeout(300);
      check(deletes.length === 1 && deletes[0] === KINDS[0].del, `360 scanning: Remove still removes that folder (${deletes.join(', ')})`);
    }
    check(state.errors.length === 0, `360 scanning ${how}: no page errors (${state.errors.join('; ')})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
