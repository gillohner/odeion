// Two presses that start a request now show it and fire once (owner, 2026-11-04: "every click shows
// loader and fires once"): Now playing's Up next rows and the item page's rating stars. Before, their
// handlers started the request without returning it, so the pressed row or star showed no busy mark,
// and every quick second press ran the handler again (api() shared the request on its way, but Up next
// was drawn again once per press). Real candidate assets on the fixture origin (tools/ui-fixture.js), a
// zone "z1" playing a queue of 8, its transport and the reviews answered 700 ms late, at 1280, 1024 and
// 768 px:
//   * Up next: a row pressed three times quickly is busy at once (aria-busy, aria-disabled, focus
//     kept), sends one transport play for its index, and when the zone answers Now playing shows the
//     song it started, no row is left busy and the list was drawn again once for the press;
//   * the stars: a half star pressed three times quickly is busy at once, sends one PUT, then the page
//     says "3½ of 5"; pressed again (the same value), one DELETE clears it.
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
    for (const w of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [1280, 1024, 768])) {
      const T = `${w}:`;
      const Z = zoneOf(); const sent = []; let store = null;
      const fixtures = { '/zones': { body: [Z] }, '/zones/z1': { get body() { return Z; } }, '/zones/z1/queue': { get body() { return Z.queue; } }, '/outputs': { body: [{ id: 'upnp:sp', name: 'Speaker', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
      for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };
      fixtures['/library/tracks'] = { body: { tracks, next: null } };
      const { page, context, state } = await fx.create(browser, w, { fixtures, touch: w < 1025 });
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
        if (m === 'GET') return json(r, { reviews: store ? [store] : [] });
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
      sent.length = 0;
      // (each new Up next list drawn after the press)
      await page.evaluate(() => { window.__drawn = 0; new MutationObserver((ms) => { for (const m of ms) for (const n of m.addedNodes) if (n.nodeType === 1 && (n.matches('.np-queue') || n.querySelector('.np-queue'))) window.__drawn++; }).observe(document.querySelector('#nowplaying'), { childList: true, subtree: true }); });
      await press3(page, row);
      const a = await attrs(page, row);
      check(!!a && a.busy === 'true' && a.dis === 'true' && /is-busy/.test(a.cls), `${T} a pressed Up next row is busy at once (${JSON.stringify(a)})`);
      check(!!a && a.focus, `${T} …and keeps the focus`);
      await sleep(250); await page.screenshot({ path: path.join(SHOTS, `${w}-upnext-busy.png`) });
      check(await until(async () => page.evaluate(() => { const t = document.querySelector('#nowplaying:not([hidden]) .np-title, #nowplaying:not([hidden]) h1, #nowplaying:not([hidden]) h2'); return !!t && /Song 4/.test(t.textContent); }), 4000), `${T} when the zone answers, Now playing shows Song 4`);
      await sleep(400);
      const plays = sent.filter((b) => b && b.action === 'play');
      check(plays.length === 1 && plays[0].index === 3, `${T} three quick presses sent one play (${JSON.stringify(plays)})`);
      check(!(await page.locator('#nowplaying [data-npqi][aria-busy], #nowplaying [data-npqi].is-busy').count()), `${T} …and no Up next row is left busy`);
      const drawn = await page.evaluate(() => window.__drawn);
      // (the zone's answer draws it once, the press once after it: before, once more per press)
      check(drawn <= 2, `${T} …and Up next is drawn again once for the press, not once per press (${drawn} draws)`);
      await page.keyboard.press('Escape'); await sleep(300);
      if (await page.locator('#nowplaying:not([hidden])').count()) { await page.keyboard.press('Escape'); await sleep(300); }

      // ---- The item page's rating stars ----
      sent.length = 0; store = null;
      await fx.go(page, '/album/lib-7'); await page.waitForSelector('#main .item-hero'); await page.waitForSelector('#stars', { timeout: 8000 }).catch(() => {}); await sleep(400);
      const star = '#stars [data-rate="7"]';
      check(await page.locator(star).count() === 1, `${T} the album page has the rating stars`);
      await press3(page, star);
      const s = await attrs(page, star);
      check(!!s && s.busy === 'true' && s.dis === 'true' && /is-busy/.test(s.cls), `${T} a pressed star is busy at once (${JSON.stringify(s)})`);
      await sleep(200);
      // (the spinner's middle is the star's middle, both ways: a left half's spinner is centred on its
      // right edge; its box is the width plus its borders, as ::after is not border-box)
      const spin = await page.evaluate((sel) => {
        const e = document.querySelector(sel); const a = getComputedStyle(e, '::after'); const hb = e.getBoundingClientRect(); const sb = e.parentElement.getBoundingClientRect();
        const px = (k) => parseFloat(a[k]) || 0; const bw = px('width') + (a.boxSizing === 'border-box' ? 0 : px('borderLeftWidth') + px('borderRightWidth')); const bh = px('height') + (a.boxSizing === 'border-box' ? 0 : px('borderTopWidth') + px('borderBottomWidth'));
        return { spinning: e.classList.contains('is-spinning'), pos: a.position, x: Math.round(hb.left + px('left') + px('marginLeft') + bw / 2 - (sb.left + sb.width / 2)), y: Math.round(hb.top + px('top') + px('marginTop') + bh / 2 - (sb.top + sb.height / 2)) };
      }, star);
      check(spin.spinning && spin.pos === 'absolute' && Math.abs(spin.x) <= 1 && Math.abs(spin.y) <= 1, `${T} …its spinner sits on the middle of its star (${JSON.stringify(spin)})`);
      await page.locator('.reviewrow').screenshot({ path: path.join(SHOTS, `${w}-star-busy.png`) });
      check(await until(async () => /3½ of 5/.test(await page.locator('#ratingval').textContent()), 4000), `${T} when it is saved the page says 3½ of 5`);
      await sleep(300);
      const puts = sent.filter((x) => x.review === 'PUT');
      check(puts.length === 1 && JSON.parse(puts[0].body).rating === 7, `${T} three quick presses sent one PUT (${puts.length})`);
      // The same star again clears it, once
      sent.length = 0;
      await page.waitForSelector(`${star}[aria-pressed="true"]`);
      await press3(page, star);
      check(await until(async () => (await page.locator('#ratingval').textContent()).trim() === '', 4000), `${T} the same star again clears the rating`);
      await sleep(300);
      check(sent.filter((x) => x.review === 'DELETE').length === 1 && !sent.some((x) => x.review === 'PUT'), `${T} …with one DELETE (${JSON.stringify(sent.map((x) => x.review))})`);
      await page.screenshot({ path: path.join(SHOTS, `${w}-item.png`) });
      check(!state.errors.length, `${T} no page errors (${state.errors.join(' | ')})`);
      await context.close();
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
