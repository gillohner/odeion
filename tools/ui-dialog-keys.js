// While a dialog is open the keyboard is the dialog's. Space, ← →, N, / and ? are the player's and
// the page's shortcuts, and they acted behind an open dialog: → on "Add music" (or on a "Delete
// this…?") went to the next song, N opened Now playing under the dialog, Space on the dialog's
// own text played or paused, ? opened a second dialog over the first.
// Real candidate assets on the fixture origin (tools/ui-fixture.js), five songs, one playing in the
// browser:
//   * with the Add music dialog open: ←, →, N, /, ? and Space (focus on the dialog, not on a
//     button) change nothing: the same song, no play or pause, Now playing closed, one dialog,
//     the focus still in it;
//   * the dialog still works: Tab moves between its buttons, Space and Enter press the focused
//     one, Escape closes it;
//   * with it closed, the shortcuts work as before.
//   node tools/ui-dialog-keys.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'dialog-keys-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tracks = Array.from({ length: 5 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}`, canonical_uri: `${fx.SONG}?n=${i + 1}` }));
const fixtures = {};
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };

(async () => {
  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const at = `${width}px`;
    const r = await fx.create(browser, width, { mobile: width < 600, tracks, fixtures });
    // (each play() and pause() the page asks its player for)
    await r.context.addInitScript(() => { window.__calls = 0; const play = HTMLMediaElement.prototype.play; const pause = HTMLMediaElement.prototype.pause; HTMLMediaElement.prototype.play = function () { window.__calls++; return play.call(this); }; HTMLMediaElement.prototype.pause = function () { window.__calls++; return pause.call(this); }; });
    const { page, state } = r;
    const seen = () => page.evaluate(() => ({ song: document.querySelector('#np').textContent.trim().slice(0, 40), calls: window.__calls, np: !document.querySelector('#nowplaying').hidden, dialogs: document.querySelectorAll('dialog[open]').length, inDialog: !!document.activeElement.closest('dialog[open]'), search: document.activeElement === document.querySelector('#search') }));
    await fx.go(page, '/tracks'); await page.waitForSelector('.tracks .track');
    // a song playing in the browser (a row's own Play on a phone, a double click on a wide page)
    if (width < 600) await page.locator('.tracks .track').nth(2).click(); else await page.locator('.tracks .track').nth(2).dblclick();
    await page.waitForFunction(() => /Song 3/.test(document.querySelector('#np').textContent)); await sleep(300);

    // The Add music dialog, opened by its button
    await page.locator('[data-addmusic]').first().click();
    await page.waitForSelector('dialog.dialog[open] .choice'); await sleep(250);
    const a = await seen();
    check(a.dialogs === 1 && a.inDialog && /Song 3/.test(a.song), `${at}: Add music is open with the focus in it, Song 3 playing (${JSON.stringify(a)})`);
    for (const key of ['ArrowRight', 'ArrowLeft', 'n', 'N', '/', '?']) {
      await page.keyboard.press(key === '?' ? 'Shift+?' : key); await sleep(250);
      const b = await seen();
      check(b.song === a.song && b.calls === a.calls && !b.np && b.dialogs === 1 && b.inDialog && !b.search, `${at}: ${key} with the dialog open changes nothing (${JSON.stringify(b)})`);
    }
    // Space with the focus on the dialog itself (as after a click on its text), not on a button
    await page.evaluate(() => { const d = document.querySelector('dialog[open]'); d.tabIndex = -1; d.focus(); });
    await page.keyboard.press('Space'); await sleep(250);
    const c = await seen();
    check(c.calls === a.calls && c.song === a.song && c.dialogs === 1, `${at}: Space on the dialog's own text neither plays nor pauses (${c.calls - a.calls} calls)`);
    await page.locator('dialog[open]').screenshot({ path: path.join(SHOTS, `dialog-${width}.png`) });

    // The dialog still has its keys: Tab to Cancel, Space presses it
    const n = await page.locator('dialog[open] .choice').count();
    await page.locator('dialog[open] .choice').first().focus();
    for (let i = 0; i < n; i++) await page.keyboard.press('Tab');
    const onCancel = await page.evaluate(() => document.activeElement.matches('dialog[open] [data-ci="-1"]'));
    await page.keyboard.press('Space'); await sleep(300);
    const d = await seen();
    check(onCancel && d.dialogs === 0 && d.calls === a.calls, `${at}: Tab reaches Cancel and Space presses it: the dialog closes, the music goes on (on Cancel ${onCancel}, ${d.dialogs} dialogs, ${d.calls - a.calls} calls)`);
    // … Enter presses the focused button too
    await page.getByRole('button', { name: 'Add music' }).first().click(); await page.waitForSelector('dialog.dialog[open] .choice'); await sleep(200);
    await page.locator('dialog[open] [data-ci="-1"]').focus(); await page.keyboard.press('Enter'); await sleep(300);
    check((await seen()).dialogs === 0, `${at}: Enter on Cancel closes the dialog`);
    // … and Escape closes it
    // (opened from Songs, the dialog is over Your library on a phone: its own Add music button there)
    await page.getByRole('button', { name: 'Add music' }).first().click(); await page.waitForSelector('dialog.dialog[open] .choice'); await sleep(200);
    await page.keyboard.press('Escape'); await sleep(300);
    check((await seen()).dialogs === 0, `${at}: Escape closes the dialog`);

    // With it closed the shortcuts are the player's again
    await page.evaluate(() => { if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur(); });
    await page.keyboard.press('ArrowRight'); await sleep(350);
    const e = await seen();
    check(/Song 4/.test(e.song), `${at}: with the dialog closed, → goes to the next song (${e.song})`);
    await page.keyboard.press('Space'); await sleep(300);
    check((await seen()).calls > e.calls, `${at}: and Space plays or pauses`);
    await page.keyboard.press('n'); await sleep(400);
    check((await seen()).np, `${at}: and N opens Now playing`);
    check(state.errors.length === 0, `${at}: no script errors (${state.errors.join(' | ')})`);
    await r.context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
