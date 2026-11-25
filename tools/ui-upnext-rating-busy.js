// Two presses that start a request now show it and fire once (owner, 2026-11-04: "every click shows
// loader and fires once"): Now playing's Up next rows and the item page's rating stars. Before, their
// handlers started the request without returning it, so the pressed row or star showed no busy mark,
// and every quick second press ran the handler again (api() shared the request on its way, but Up next
// was drawn again once per press). Real candidate assets on the fixture origin (tools/ui-fixture.js), a
// zone "z1" playing a queue of 8, its transport and the reviews answered 700 ms late, at 1280, 1024 and
// 768 px and on a 412 px phone (Up next as a sheet, 44 px stars):
//   * Up next: a row pressed three times quickly is busy at once (aria-busy, aria-disabled, focus
//     kept), its spinner in its length's place (the length doesn't move), sends one transport play
//     for its index, and when the zone answers Now playing shows the song it started, no row is left
//     busy and the list was drawn again once for the press (the zone's answer once, the press once);
//     Now playing closed before the zone answers is not drawn again (shut, it fetched lyrics for nothing);
//   * the stars: a half star pressed three times quickly is busy at once, its spinner on its star's
//     middle (a left half and a right half), sends one PUT, then the page says "3½ of 5"; pressed
//     again (the same value), one DELETE clears it;
//   * a star stays busy until the page is drawn again with its rating: pressed again while that
//     drawing still reads the reviews, it sends nothing (before, it sent the same rating again), and
//     pressed once more afterwards it clears the rating.
// ("One play" and "one PUT" are guards: api() already shared an identical request on its way.)
//   node tools/ui-upnext-rating-busy.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'upnext-rating-busy-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const end = Date.now() + ms; for (;;) { try { if (await fn()) return true; } catch { /* not yet */ } if (Date.now() > end) return false; await sleep(50); } };
const json = (r, body, status = 200) => r.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
const SLOW = 700;
// (the star's spinner, from its star's middle, both ways: a left half's spinner is centred on its
// right edge, a right half's on its left; its box is the width plus its borders, as ::after is not border-box)
const spinAt = (page, sel) => page.evaluate((sel) => {
  const e = document.querySelector(sel); const a = getComputedStyle(e, '::after'); const hb = e.getBoundingClientRect(); const sb = e.parentElement.getBoundingClientRect();
  const px = (k) => parseFloat(a[k]) || 0; const bw = px('width') + (a.boxSizing === 'border-box' ? 0 : px('borderLeftWidth') + px('borderRightWidth')); const bh = px('height') + (a.boxSizing === 'border-box' ? 0 : px('borderTopWidth') + px('borderBottomWidth'));
  return { spinning: e.classList.contains('is-spinning'), pos: a.position, x: Math.round(hb.left + px('left') + px('marginLeft') + bw / 2 - (sb.left + sb.width / 2)), y: Math.round(hb.top + px('top') + px('marginTop') + bh / 2 - (sb.top + sb.height / 2)) };
}, sel);

const N = 8;
const tracks = Array.from({ length: N }, (_, k) => ({ id: k + 1, title: `Song ${k + 1}`, artists: [{ id: 11, name: 'Band' }], album: { id: 7, title: 'Album' }, duration_ms: 200000 }));
const zoneOf = () => {
  const queue = tracks.map((t, k) => ({ id: k + 1, uri: `track:${t.id}`, title: t.title, artist: 'Band', album: 'Album', duration_ms: t.duration_ms }));
  return { id: 'z1', name: 'Living room', output: 'upnp:sp', transport: 'playing', now_playing: { item: queue[0], index: 0, position_ms: 30000, duration_ms: 200000 }, queue, queue_index: 0, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false } };
};
// Three presses on the centre of what the selector finds, 60 ms apart
const press3 = async (page, sel) => { const b = await page.locator(sel).first().boundingBox(); const x = b.x + b.width / 2; const y = b.y + b.height / 2; for (let i = 0; i < 3; i++) { await page.mouse.click(x, y); if (i < 2) await sleep(60); } };
const attrs = (page, sel) => page.evaluate((s) => { const e = document.querySelector(s); return e ? { busy: e.getAttribute('aria-busy'), dis: e.getAttribute('aria-disabled'), cls: e.className, focus: document.activeElement === e } : null; }, sel);

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [1280, 1024, 768, 412])) {
      const T = `${w}:`;
      const Z = zoneOf(); const sent = []; let store = null; let slowGet = false;
      const fixtures = { '/zones': { body: [Z] }, '/zones/z1': { get body() { return Z; } }, '/zones/z1/queue': { get body() { return Z.queue; } }, '/outputs': { body: [{ id: 'upnp:sp', name: 'Speaker', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
      for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };
      fixtures['/library/tracks'] = { body: { tracks, next: null } };
      const { page, context, state } = await fx.create(browser, w, { fixtures, touch: w < 1025, mobile: w < 600 });
      await page.setViewportSize({ width: w, height: 900 });
      let seq = 0; let sock = null;
      const snapshot = () => sock && sock.send(JSON.stringify({ type: 'snapshot', seq: seq++, boot: 'fixture-boot', library_rev: 0, zones: [Z] }));
      await context.routeWebSocket(/\/v1\/events/, (ws) => { sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); snapshot(); });
      await page.route(/\/v1\/zones\/z1\/transport$/, async (r) => {
        const body = JSON.parse(r.request().postData() || 'null'); sent.push(body);
        await sleep(SLOW);
        if (body && body.action === 'play' && Number.isInteger(body.index)) { Z.queue_index = body.index; Z.now_playing = { item: Z.queue[body.index], index: body.index, position_ms: 0, duration_ms: 200000 }; Z.transport = 'playing'; }
        return json(r, Z);
      });
      await page.route(/\/v1\/reviews(\?.*)?$/, async (r) => {
        const m = r.request().method();
        if (m === 'GET') { if (slowGet) await sleep(2 * SLOW); return json(r, { reviews: store ? [store] : [] }); }
        sent.push({ review: m, body: r.request().postData() }); await sleep(SLOW);
        if (m === 'PUT') { const b = JSON.parse(r.request().postData()); store = { rating: b.rating, text: b.text || '' }; }
        else store = null;
        return json(r, {});
      });
      await context.addInitScript(() => localStorage.setItem('pm.zone', 'z1'));

      // ---- Now playing › Up next ----
      await fx.go(page, '/albums'); await page.waitForSelector('#np'); await sleep(400);
      await page.click(await page.locator('#npart').isVisible() ? '#npart' : '#np'); await page.waitForSelector('#nowplaying:not([hidden])'); await sleep(300);
      const dq = page.locator('#nowplaying:not([hidden]) .np-dock [data-npsheet="queue"]');
      if (await dq.count() && await dq.isVisible() && !(await page.locator('#nowplaying:not([hidden]) [data-npqi]').count())) { await dq.click(); await sleep(300); }
      const row = '#nowplaying:not([hidden]) [data-npqi="3"]';
      check(await until(async () => (await page.locator(row).count()) === 1, 3000), `${T} Up next lists the queue's fourth song`);
      const lenAt = () => page.evaluate((s) => { const r = document.querySelector(s); const l = r && r.lastElementChild; if (!l) return null; const b = l.getBoundingClientRect(); const a = getComputedStyle(l, '::after'); const ra = getComputedStyle(r, '::after'); return { right: Math.round(b.right), mid: Math.round(b.top + b.height / 2), text: l.textContent, color: getComputedStyle(l).color, spin: a.content !== 'none' && a.position === 'absolute', rowSpin: ra.content !== 'none' && ra.content !== 'normal', spinMid: Math.round(b.top + (parseFloat(a.top) || 0) + (parseFloat(a.marginTop) || 0) + 8), spinX: Math.round(b.left + (parseFloat(a.left) || 0) + (parseFloat(a.marginLeft) || 0) + 8 - (b.left + b.width / 2)) }; }, row);
      const len0 = await lenAt();
      sent.length = 0;
      // (each new Up next list drawn after the press)
      await page.evaluate(() => { window.__drawn = 0; new MutationObserver((ms) => { for (const m of ms) for (const n of m.addedNodes) if (n.nodeType === 1 && (n.matches('.np-queue') || n.querySelector('.np-queue'))) window.__drawn++; }).observe(document.querySelector('#nowplaying'), { childList: true, subtree: true }); });
      await press3(page, row);
      const a = await attrs(page, row);
      check(!!a && a.busy === 'true' && a.dis === 'true' && /is-busy/.test(a.cls), `${T} a pressed Up next row is busy at once (${JSON.stringify(a)})`);
      check(!!a && a.focus, `${T} …and keeps the focus`);
      await sleep(250);
      const len1 = await lenAt();
      check(!!len0 && !!len1 && len1.right === len0.right && len1.text === len0.text, `${T} …its length stays where it was while it is busy (${JSON.stringify(len0)} -> ${JSON.stringify(len1)})`);
      check(!!len1 && len1.spin && !len1.rowSpin && Math.abs(len1.spinMid - len1.mid) <= 1 && Math.abs(len1.spinX) <= 1, `${T} …its spinner takes the length's place, in its middle (${JSON.stringify(len1)})`);
      check(!!len1 && len1.color === 'rgba(0, 0, 0, 0)', `${T} …and the length's words are not drawn under it (${len1 && len1.color})`); await page.screenshot({ path: path.join(SHOTS, `${w}-upnext-busy.png`) });
      check(await until(async () => page.evaluate(() => { const t = document.querySelector('#nowplaying:not([hidden]) .np-title, #nowplaying:not([hidden]) h1, #nowplaying:not([hidden]) h2'); return !!t && /Song 4/.test(t.textContent); }), 4000), `${T} when the zone answers, Now playing shows Song 4`);
      // (past the redraw 500 ms after the zone answers)
      await sleep(900);
      const plays = sent.filter((b) => b && b.action === 'play');
      check(plays.length === 1 && plays[0].index === 3, `${T} three quick presses sent one play (guard) (${JSON.stringify(plays)})`);
      check(!(await page.locator('#nowplaying [data-npqi][aria-busy], #nowplaying [data-npqi].is-busy').count()), `${T} …and no Up next row is left busy`);
      const drawn = await page.evaluate(() => window.__drawn);
      // (the zone's answer draws it once, the press once after it: before, once more per press)
      check(drawn === 2, `${T} …and Up next is drawn again once for the press, not once per press (${drawn} draws)`);
      await page.keyboard.press('Escape'); await sleep(300);
      if (await page.locator('#nowplaying:not([hidden])').count()) { await page.keyboard.press('Escape'); await sleep(300); }
      // Now playing closed before the zone answers: the closed panel is not drawn again afterwards
      await page.click(await page.locator('#npart').isVisible() ? '#npart' : '#np'); await page.waitForSelector('#nowplaying:not([hidden])'); await sleep(300);
      if (await dq.count() && await dq.isVisible() && !(await page.locator('#nowplaying:not([hidden]) [data-npqi]').count())) { await dq.click(); await sleep(300); }
      const row6 = '#nowplaying:not([hidden]) [data-npqi="5"]';
      check(await until(async () => (await page.locator(row6).count()) === 1, 3000), `${T} Up next lists the queue's sixth song`);
      await page.locator(row6).click();
      await page.keyboard.press('Escape'); await sleep(100);
      if (await page.locator('#nowplaying:not([hidden])').count()) { await page.keyboard.press('Escape'); await sleep(100); }
      check(!(await page.locator('#nowplaying:not([hidden])').count()), `${T} Now playing closed while the zone is still answering`);
      await page.evaluate(() => { window.__drawn = 0; });
      await sleep(SLOW + 900);
      check(Z.queue_index === 5, `${T} …the zone played the sixth song`);
      const late = await page.evaluate(() => window.__drawn);
      // (a guard on a phone: closing it there already left nothing to draw)
      check(late === 0, `${T} …and the closed Now playing was not drawn again after the zone answered${w < 600 ? ' (guard)' : ''} (${late} draws)`);

      // ---- The item page's rating stars ----
      sent.length = 0; store = null;
      await fx.go(page, '/album/lib-7'); await page.waitForSelector('#main .item-hero'); await page.waitForSelector('#stars', { timeout: 8000 }).catch(() => {}); await sleep(400);
      const star = '#stars [data-rate="7"]';
      check(await page.locator(star).count() === 1, `${T} the album page has the rating stars`);
      await press3(page, star);
      const s = await attrs(page, star);
      check(!!s && s.busy === 'true' && s.dis === 'true' && /is-busy/.test(s.cls), `${T} a pressed star is busy at once (${JSON.stringify(s)})`);
      await sleep(200);
      const spin = await spinAt(page, star);
      check(spin.spinning && spin.pos === 'absolute' && Math.abs(spin.x) <= 1 && Math.abs(spin.y) <= 1, `${T} …its spinner sits on the middle of its star (${JSON.stringify(spin)})`);
      await page.locator('.reviewrow').screenshot({ path: path.join(SHOTS, `${w}-star-busy.png`) });
      check(await until(async () => /3½ of 5/.test(await page.locator('#ratingval').textContent()), 4000), `${T} when it is saved the page says 3½ of 5`);
      await sleep(300);
      const puts = sent.filter((x) => x.review === 'PUT');
      check(puts.length === 1 && JSON.parse(puts[0].body).rating === 7, `${T} three quick presses sent one PUT (guard) (${puts.length})`);
      // The same star again clears it, once
      sent.length = 0;
      await page.waitForSelector(`${star}[aria-pressed="true"]`);
      await press3(page, star);
      check(await until(async () => (await page.locator('#ratingval').textContent()).trim() === '', 4000), `${T} the same star again clears the rating`);
      await sleep(300);
      check(sent.filter((x) => x.review === 'DELETE').length === 1 && !sent.some((x) => x.review === 'PUT'), `${T} …with one DELETE (${JSON.stringify(sent.map((x) => x.review))})`);
      await page.screenshot({ path: path.join(SHOTS, `${w}-item.png`) });

      // A right half: its spinner on its star's middle as well
      const star8 = '#stars [data-rate="8"]';
      await page.waitForSelector(`${star8}[aria-pressed="false"]`);
      sent.length = 0; slowGet = true;
      await page.locator(star8).click();
      await sleep(250);
      const spin8 = await spinAt(page, star8);
      check(spin8.spinning && spin8.pos === 'absolute' && Math.abs(spin8.x) <= 1 && Math.abs(spin8.y) <= 1, `${T} a right half's spinner sits on the middle of its star too (${JSON.stringify(spin8)})`);
      // Saved (the PUT answered), the page is drawn again, its reviews read slowly: the same star
      // pressed meanwhile is still busy and sends nothing
      check(await until(async () => sent.some((x) => x.review === 'PUT'), 3000), `${T} the 4 stars are being saved`);
      await sleep(SLOW + 250);
      const mid = await attrs(page, star8);
      check(!!mid && mid.busy === 'true', `${T} …saved, while the page is drawn again the star is still busy (${JSON.stringify(mid)})`);
      await page.locator(star8).click({ force: true });
      check(await until(async () => /^4 of 5$/.test((await page.locator('#ratingval').textContent()).trim()), 4000), `${T} …then the page says 4 of 5`);
      await sleep(300);
      check(sent.filter((x) => x.review === 'PUT').length === 1 && !sent.some((x) => x.review === 'DELETE'), `${T} …and the press meanwhile sent nothing (${JSON.stringify(sent.map((x) => x.review))})`);
      // Pressed once more, on the star drawn with the rating: cleared
      sent.length = 0;
      await page.locator(star8).click();
      check(await until(async () => (await page.locator('#ratingval').textContent()).trim() === '', 4000), `${T} …and pressed once more it clears the rating`);
      await sleep(SLOW + 300);
      check(sent.filter((x) => x.review === 'DELETE').length === 1 && !sent.some((x) => x.review === 'PUT'), `${T} …with one DELETE (${JSON.stringify(sent.map((x) => x.review))})`);
      slowGet = false;
      check(!state.errors.length, `${T} no page errors (${state.errors.join(' | ')})`);
      await context.close();
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
