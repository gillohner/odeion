// Toasts: an error stays readable and an Undo stays pressable when another message comes, an error
// is announced as one, a message raised while a dialog is open shows over it.
// Real candidate assets on the fixture origin (tools/ui-fixture.js), a node zone with an empty queue:
//   * a zone's error is a toast with role=alert; a plain message that comes right after it ("Added
//     to …") waits until the error has been up 2.5 s, then shows, with role=status again;
//   * a toast with a button ("The queue is empty", Shuffle my music) is not replaced for its first
//     2.5 s either; the message shows then, not only when that toast goes;
//   * with a dialog open, an error's toast is inside the dialog (over its backdrop), on screen; it
//     is back in the page once the dialog is closed, and the next toast works;
//   * a message that waits for an answer has a spinner, and every message that says it is under
//     way ("…", "Moving to …") is one, so it is never held behind an error.
//   node tools/ui-toasts.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'toasts-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const t0 = Date.now(); for (;;) { const v = await fn().catch(() => null); if (v) return v; if (Date.now() - t0 > ms) return v; await sleep(40); } };

const zone = { id: 'kitchen', name: 'Kitchen', output: 'upnp:kitchen', transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: null };
const outputs = [{ id: 'upnp:kitchen', name: 'kitchen', kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }];
const tracks = [1, 2, 3, 4].map((n) => ({ ...fx.track, id: n, title: `Song number ${n}` }));
const now = (page) => page.evaluate(() => { const t = document.getElementById('toast'); return { shown: t.classList.contains('show'), text: t.textContent.trim(), role: t.getAttribute('role'), live: t.getAttribute('aria-live'), button: t.querySelector('button') ? t.querySelector('button').textContent.trim() : null, kind: t.className }; });
// A plain message: a song bookmarked from its ⋯ menu
const bookmark = async (page, n) => {
  await page.locator('#main .track [data-act="more"]').nth(n).click();
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: /^Add to (favourites|Bookmarks)/ }).first().click();
};
const ADDED = /^Added to (favourites|Bookmarks)/;

(async () => {
  const browser = await chromium.launch();
  // A progress message is the answer to a press and shows at once, also during an error's 2.5 s:
  // every toast whose words end in "…" says it waits ('wait'), or is an error. (Not "… Trying
  // again…": the page says that by itself after a start failed, and a retry that works says nothing
  // more, so a 'wait' toast, which stays until the next one, would stay on.)
  {
    const src = fs.readFileSync(path.join(path.resolve(process.argv[2] || path.join(__dirname, '..')), 'crates/pm-web/static/app.js'), 'utf8');
    const plain = [...src.matchAll(/\btoast\((['"`])((?:(?!\1)[^\\]|\\.)*…)\1\s*\)/g)].map((m) => m[2]).filter((w) => !/ Trying again…$/.test(w));
    const said = [...src.matchAll(/said = `Moving to [^`]*…`; toast\(said(, '[a-z]+')?\)/g)].map((m) => m[1] || '');
    check(!plain.length, `a toast that says it is under way ("…") is shown as one that waits (${plain.length ? plain.join(' | ') : 'all are'})`);
    check(said.length === 1 && said[0] === ", 'wait'", `"Moving to …" is shown at once, also right after an error (${JSON.stringify(said)})`);
  }
  for (const width of [390, 1280]) {
    const at = `${width}px`;
    const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, zone: 'kitchen', tracks, fixtures: { '/zones': { body: [zone] }, '/outputs': { body: outputs }, '/zones/kitchen/queue': { body: [] } } });
    const sock = { ws: null, seq: 0 };
    const snapshot = (ws) => ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] }));
    await context.routeWebSocket(/\/v1\/events/, (ws) => { sock.ws = ws; sock.opened = (sock.opened || 0) + 1; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); snapshot(ws); });
    const zoneError = (words) => sock.ws.send(JSON.stringify({ type: 'zone_event', seq: ++sock.seq, zone: 'kitchen', event: { type: 'error', message: words } }));
    await fx.go(page, '/tracks'); await page.waitForSelector('#main .track'); await until(async () => !!sock.ws); await sleep(300);

    // ---- an error, then a plain message at once
    zoneError('The device refused');
    const err = await until(async () => { const t = await now(page); return t.shown && /The device refused/.test(t.text) ? t : null; });
    check(!!err && err.role === 'alert' && err.live === 'assertive' && /\berror\b/.test(err.kind), `${at}: a zone's error is a toast announced as an alert (${JSON.stringify(err)})`);
    const t0 = Date.now();
    await bookmark(page, 0); await sleep(350);
    const still = await now(page);
    check(still.shown && /The device refused/.test(still.text), `${at}: a message that comes ${Date.now() - t0} ms later does not take the error's place ("${still.text}")`);
    if (width === 1280) await page.screenshot({ path: path.join(SHOTS, 'error-kept-1280.png') });
    const after = await until(async () => { const t = await now(page); return t.shown && ADDED.test(t.text) ? t : null; }, 4000);
    check(!!after && after.role === 'status' && after.live === 'polite' && Date.now() - t0 < 3600, `${at}: it shows once the error has been up 2.5 s, announced as a status again (${after ? `"${after.text}", ${Date.now() - t0} ms after it came` : 'never shown'})`);
    await until(async () => !(await now(page)).shown, 6000);

    // ---- a toast with a button, then a plain message
    await page.click('#toggle');
    const act = await until(async () => { const t = await now(page); return t.shown && t.button ? t : null; });
    check(!!act && /queue is empty/i.test(act.text) && !!act.button, `${at}: Play on an empty queue says so, with a button (${act ? `"${act.text}"` : 'no toast'})`);
    if (act) {
      const t1 = Date.now();
      await bookmark(page, 1); await sleep(900);
      const kept = await now(page);
      check(kept.shown && kept.button === act.button, `${at}: a message that comes ${Date.now() - t1} ms later does not take the button away ("${kept.text}")`);
      const later = await until(async () => { const t = await now(page); return t.shown && ADDED.test(t.text) && !t.button ? t : null; }, 4000);
      check(!!later && Date.now() - t1 < 3600, `${at}: it shows once the button has been up 2.5 s, not only when that toast goes (${later ? `${Date.now() - t1} ms after it came` : `still "${(await now(page)).text}"`})`);
    }
    await until(async () => !(await now(page)).shown, 6000);

    // ---- a dialog open: the toast is over it
    await page.keyboard.press('?');
    const dlg = await until(() => page.evaluate(() => !!document.querySelector('dialog[open]')));
    if (!dlg) { await page.evaluate(() => { const d = document.createElement('dialog'); d.className = 'dialog'; d.innerHTML = '<h3>A dialog</h3><form method="dialog"><button class="btn">Close</button></form>'; document.body.appendChild(d); d.showModal(); }); }
    zoneError('Lost the speaker');
    const inDlg = await until(async () => { const t = await now(page); return t.shown && /Lost the speaker/.test(t.text) ? t : null; });
    const where = await page.evaluate(() => { const t = document.getElementById('toast'); const r = t.getBoundingClientRect(); const d = t.closest('dialog'); return { inDialog: !!d && d.open, modal: !!d && d.matches(':modal'), onScreen: r.width > 40 && r.height > 10 && r.top >= 0 && r.bottom <= innerHeight && r.left >= 0 && r.right <= innerWidth, bottomHalf: r.top > innerHeight / 2, opacity: getComputedStyle(t).opacity }; });
    check(!!inDlg && where.inDialog && where.modal && where.onScreen && where.bottomHalf, `${at}: with a dialog open, the error's toast is inside the dialog, on screen, where toasts are (${JSON.stringify(where)})`);
    await page.screenshot({ path: path.join(SHOTS, `toast-over-dialog-${width}.png`) });
    await page.keyboard.press('Escape'); await page.evaluate(() => { for (const d of document.querySelectorAll('dialog[open]')) d.close(); }); await sleep(300);
    const home = await page.evaluate(() => { const t = document.getElementById('toast'); return { there: !!t, inBody: !!t && t.parentElement === document.body, ones: document.querySelectorAll('#toast').length }; });
    check(home.there && home.inBody && home.ones === 1, `${at}: with the dialog closed the toast is back in the page (${JSON.stringify(home)})`);
    zoneError('And again');
    const again = await until(async () => { const t = await now(page); return t.shown && /And again/.test(t.text) ? t : null; });
    check(!!again, `${at}: and the next toast shows`);

    // ---- a message that waits for an answer shows that it does
    const spin = await page.evaluate(() => { const d = document.createElement('div'); d.className = 'toast wait show'; d.textContent = 'Bringing the songs back…'; document.body.appendChild(d); const b = getComputedStyle(d, '::before'); const out = { name: b.animationName, shape: b.borderTopStyle, w: b.width }; d.remove(); return out; });
    check(spin.shape === 'solid' && spin.w === '14px' && (spin.name === 'spin' || spin.name === 'none'), `${at}: a waiting message has a spinner (${JSON.stringify(spin)}; the fixture asks for less motion)`);
    check(!state.errors.length, `${at}: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
