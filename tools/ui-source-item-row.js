// A service's own album page (Sources › SoundCloud › a set, archive.org, Bandcamp …: the
// `source-item` page) plays a song when its row is clicked, as a library album's, a playlist's and
// the Songs list's rows do. Before, only the row's small ▶ played it, and that ▶ only shows on a
// mouse hover, so on a touch screen the row had no visible way to play. Checked: a click on the
// title (and on an empty spot of the row) plays from that song, the row shows a pointer, the row's
// own buttons keep their job (+ only adds, ▶ plays once), the second click of a double click
// doesn't start the song again (a node zone gets one queue load), a drag that selects the title
// doesn't play, a phone tap on the row plays, and on a touch screen or a phone the row shows only its ⋯
// (Play next, Add to queue, Add to playlist…: each checked), on one line, and a swipe right plays it next. 1280/1024/768 px, a 768 px touch tablet, 360/412 px phones.
// In phone mode (no collections) a song's ⋯ has no Add to playlist… and a swipe left adds to no playlist.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-source-item-row.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'source-item-row-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const url = 'https://soundcloud.com/sefa/sets/journey';
// The songs' own addresses play straight in this browser (as an archive.org file does): a 20 s
// mono WAV on the fixture's origin, served with ranges
const WAV = (() => { const rate = 8000, n = rate * 20, b = Buffer.alloc(44 + n * 2); b.write('RIFF'); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); return b; })();
const serveWav = (r) => {
  const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || '');
  const a = m ? Number(m[1]) : 0, z = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1;
  return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
};
const tracks = ['First', 'Second', 'Third'].map((title, i) => ({ url: `${fx.ORIGIN}/audio/${title.toLowerCase()}.wav`, title, track_no: i + 1, duration_ms: 200000, format: 'mp3' }));
const item = { title: 'Journey', creator: 'Sefa', year: 2020, url, image: null, tracks };
const fixtures = {
  '/sources': { body: [{ id: 'soundcloud', name: 'SoundCloud', enabled: true, status: 'signed in' }] },
  '/sources/soundcloud/likes': { body: { tracks: [], playlists: [{ id: 'abc', title: 'Journey', creator: 'Sefa' }] } },
  '/sources/soundcloud/playlists': { body: [] },
  '/sources/soundcloud/items/abc': { body: item },
  // (a song's ⋯ › Add to playlist…: one playlist to pick)
  '/playlists': { body: [{ id: 'p1', name: 'Road trip', items: 4, covers: [], visibility: 'private', synced: false, layout: 'list', tracks_only: true }] },
  '/playlists/p1/items': { body: {} },
};

const onBar = (page) => page.evaluate(() => (document.querySelector('.player .np-title') || {}).textContent || '');
const row = (page, i) => page.locator('#main .tracks .track').nth(i);
const openItem = async (page) => {
  await fx.go(page, '/sources/soundcloud');
  await page.click('[data-sitem="abc"]');
  await page.waitForSelector('#main .tracks .track');
};
const waitBar = (page, re) => page.waitForFunction((s) => new RegExp(s).test((document.querySelector('.player .np-title') || {}).textContent || ''), re.source, { timeout: 3000 }).catch(() => {});
// The browser's queue as the app keeps it (its saved copy), as titles
const queueTitles = async (page, want) => {
  await page.waitForFunction((w) => { try { return (JSON.parse(localStorage.getItem('pm.browserQueue') || '[]')).map((x) => x.title).join() === w; } catch { return false; } }, want, { timeout: 3000 }).catch(() => {});
  return page.evaluate(() => { try { return (JSON.parse(localStorage.getItem('pm.browserQueue') || '[]')).map((x) => x.title); } catch { return null; } });
};

// In this browser: a click on the row plays from it
async function browserRun(browser, width, opts = {}) {
  const tag = `${width} px${opts.touch ? ' touch' : ''}${opts.mobile ? ' phone' : ''}`;
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true, mobile: !!opts.mobile, touch: !!opts.touch });
  await page.route('**/audio/*.wav', serveWav);
  try {
    await openItem(page);
    if (width === 1280 && !opts.touch) await page.screenshot({ path: path.join(SHOTS, `page-${width}.png`) });
    const cursor = await row(page, 1).evaluate((r) => getComputedStyle(r).cursor);
    if (!opts.mobile && !opts.touch) check(cursor === 'pointer', `${tag}: a song row shows a pointer, as a library song row does (${cursor})`);

    // A click (a tap) on the second song's title plays from there
    const title = row(page, 1).locator('.title');
    if (opts.mobile || opts.touch) await title.tap(); else await title.click();
    await waitBar(page, /Second/);
    check(/Second/.test(await onBar(page)), `${tag}: ${opts.mobile || opts.touch ? 'a tap' : 'a click'} on song 2's row plays song 2 ("${await onBar(page)}")`);
    await page.waitForFunction(() => { const r = document.querySelectorAll('#main .tracks .track'); return r[1] && r[1].classList.contains('cur'); }, null, { timeout: 3000 }).catch(() => {});
    check(await row(page, 1).evaluate((r) => r.classList.contains('cur')), `${tag}: and song 2's row is marked as playing`);
    const q = await queueTitles(page, 'Second,Third');
    check(q && q.join() === 'Second,Third', `${tag}: the queue is the album from song 2 on (${JSON.stringify(q)})`);
    if (width === 1280 && !opts.touch) await page.screenshot({ path: path.join(SHOTS, `played-${width}.png`) });

    if (opts.mobile || opts.touch) {
      // A touch screen / phone hides a song row's buttons other than its ⋯ (they are in it). These rows
      // have a ⋯ now (Play next, Add to queue, Add to playlist…), so they show only that, as every
      // other song row does. Before #755 the rows had no way to queue a song on a phone; until this
      // check they had only + (no Play next, no Add to playlist for one song). The format's empty
      // cell takes no column and nothing wraps to a second, empty grid line under the row
      const rows = await page.evaluate(() => [...document.querySelectorAll('#main .src-tracks > .track')].map((r) => {
        const b = r.querySelector('[data-amore1]'), a = r.querySelector('[data-aadd1]'), rb = r.getBoundingClientRect(); const bb = b ? b.getBoundingClientRect() : null;
        const shown = [...r.children].filter((c) => getComputedStyle(c).display !== 'none');
        return { more: !!bb && getComputedStyle(b).display !== 'none' && Math.round(bb.width) >= 40 && Math.round(bb.height) >= 40 && bb.left >= rb.left && bb.right <= rb.right + 0.5 && bb.top >= rb.top - 0.5 && bb.bottom <= rb.bottom + 0.5,
          plus: !!a && a.getClientRects().length > 0,
          lines: getComputedStyle(r).gridTemplateRows.split(' ').length, empty: shown.filter((c) => !c.innerText.trim() && ![...c.querySelectorAll('button')].some((x) => x.getClientRects().length)).length };
      }));
      check(rows.length === 3 && rows.every((r) => r.more), `${tag}: every song row shows its ⋯, a 40 px tap inside the row ${JSON.stringify(rows.map((r) => r.more))}`);
      check(rows.every((r) => !r.plus), `${tag}: and not its + (it is in the ⋯), as a library song row ${JSON.stringify(rows.map((r) => r.plus))}`);
      check(rows.every((r) => r.lines === 1 && r.empty === 0), `${tag}: each row is one line with no empty cell ${JSON.stringify(rows.map((r) => [r.lines, r.empty]))}`);
      const menuOf = async (i) => {
        const more = row(page, i).locator('[data-amore1]');
        if (!(await more.isVisible())) return null;
        await more.tap();
        await page.waitForSelector('.popover.menu:not([hidden]) .pop-item', { timeout: 2000 }).catch(() => {});
        return page.evaluate(() => { const m = document.querySelector('.popover.menu:not([hidden])'); return m ? { title: (m.querySelector('.pop-title') || {}).textContent || '', items: [...m.querySelectorAll('.pop-item')].map((b) => b.textContent.trim()) } : null; });
      };
      const pick = async (label) => { await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: label }).first().tap(); };
      const m1 = await menuOf(0);
      check(!!m1 && m1.title === 'First' && m1.items.join('|') === 'Play next|Add to queue|Add to collection…', `${tag}: song 1's ⋯ is its menu: Play next, Add to queue, Add to collection… ${JSON.stringify(m1)}`);
      if (width < 600) await page.screenshot({ path: path.join(SHOTS, `phone-menu-${width}.png`) });
      if (m1) {
        await pick('Add to queue');
        const q2 = await queueTitles(page, 'Second,Third,First');
        check(q2 && q2.join() === 'Second,Third,First', `${tag}: its Add to queue adds song 1 to the end of the queue (${JSON.stringify(q2)})`);
        await page.waitForTimeout(150);
        check(/Second/.test(await onBar(page)), `${tag}: and doesn't play it ("${await onBar(page)}")`);
      }
      if (await menuOf(2)) {
        await pick('Play next');
        const q3 = await queueTitles(page, 'Second,Third,Third,First');
        check(q3 && q3.join() === 'Second,Third,Third,First', `${tag}: song 3's ⋯ › Play next puts it right after the song playing (${JSON.stringify(q3)})`);
      } else check(false, `${tag}: song 3's ⋯ › Play next (no ⋯)`);
      if (await menuOf(1)) {
        await pick('Add to collection');
        await page.waitForSelector('dialog [data-ppi]', { timeout: 2000 }).catch(() => {});
        const n0 = state.hits.length;
        const pl = page.locator('dialog [data-ppi]', { hasText: 'Road trip' });
        if (await pl.count()) await pl.first().tap();
        await page.waitForFunction(() => !document.querySelector('dialog[open]'), null, { timeout: 2000 }).catch(() => {});
        for (let k = 0; k < 40 && !state.hits.slice(n0).some((h) => h.m === 'POST' && h.v === '/playlists/p1/items'); k++) await page.waitForTimeout(50);
        const post = state.hits.slice(n0).find((h) => h.m === 'POST' && h.v === '/playlists/p1/items');
        const body = post ? JSON.parse(post.body || '{}') : {};
        check(!!post && JSON.stringify(body.uris) === JSON.stringify([tracks[1].url]), `${tag}: song 2's ⋯ › Add to collection… › Road trip adds that one song by its address (${post ? post.body : 'no POST'})`);
      } else check(false, `${tag}: song 2's ⋯ › Add to collection… (no ⋯)`);
      // A swipe right on a row is Play next, as on a library song row (the browser's own touch input)
      const cdp = await context.newCDPSession(page);
      const rb = await row(page, 0).boundingBox();
      const y = rb.y + rb.height / 2, x0 = rb.x + 40;
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: x0, y }] });
      for (let k = 1; k <= 8; k++) { await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: x0 + k * 15, y }] }); await page.waitForTimeout(16); }
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
      const q4 = await queueTitles(page, 'Second,First,Third,Third,First');
      check(q4 && q4.join() === 'Second,First,Third,Third,First', `${tag}: a swipe right on song 1's row plays it next (${JSON.stringify(q4)})`);
      await page.waitForTimeout(150);
      check(/Second/.test(await onBar(page)), `${tag}: and the swipe doesn't play the row ("${await onBar(page)}")`);
      if (width < 600) await page.screenshot({ path: path.join(SHOTS, `phone-${width}.png`) });
    }

    if (!opts.mobile && !opts.touch) {
      // An empty spot of the third row (between the title and the format) plays from song 3
      const box = await row(page, 2).boundingBox();
      const fmtBox = await row(page, 2).locator('.fmt').boundingBox();
      await page.mouse.click(fmtBox.x - 6, box.y + box.height / 2);
      await waitBar(page, /Third/);
      check(/Third/.test(await onBar(page)), `${tag}: a click on an empty spot of song 3's row plays song 3 ("${await onBar(page)}")`);

      // The row's + only adds (no new start), and a drag over the first title selects it without playing
      await row(page, 0).hover();
      await row(page, 0).locator('[data-aadd1]').click();
      await page.waitForTimeout(150);
      check(/Third/.test(await onBar(page)), `${tag}: the row's + adds, it doesn't play song 1 ("${await onBar(page)}")`);
      const tb = await row(page, 0).locator('.title').boundingBox();
      await page.mouse.move(tb.x + 2, tb.y + tb.height / 2);
      await page.mouse.down();
      await page.mouse.move(tb.x + Math.min(tb.width, 40), tb.y + tb.height / 2, { steps: 4 });
      await page.mouse.up();
      await page.waitForTimeout(150);
      const picked = await page.evaluate(() => String(getSelection()));
      check(picked !== '' && /Third/.test(await onBar(page)), `${tag}: a drag selecting song 1's title ("${picked}") doesn't play it ("${await onBar(page)}")`);
      await page.evaluate(() => getSelection().removeAllRanges());
    }
    if (opts.mobile) {
      const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      check(over <= 0, `${tag}: no sideways scroll (${over} px over)`);
    }
    check(state.errors.length === 0, `${tag}: no page errors ${JSON.stringify(state.errors)}`);
  } finally { await context.close(); }
}

// A node zone: one click is one queue load (POST /zones/z1/load, in order from that song); a double click on a row is one too (its second click
// doesn't start the song again), and a click on the row's ▶ then a quick second one on the row the same
async function nodeRun(browser, width) {
  const tag = `${width} px, node zone`;
  const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'stopped', now_playing: null, queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/load': { body: Z }, '/zones/z1/queue': { body: Z }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
  const { page, context, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1' });
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
  const loads = () => state.hits.filter((h) => h.m === 'POST' && h.v === '/zones/z1/load');
  // (up to 3 s for the load a click sends, then 400 ms more for a second one that shouldn't come)
  const settle = async (n0) => { for (let k = 0; k < 60 && loads().length <= n0; k++) await page.waitForTimeout(50); await page.waitForTimeout(400); };
  try {
    await openItem(page);
    let n0 = loads().length;
    await row(page, 1).locator('.title').click();
    await settle(n0);
    let got = loads().slice(n0);
    const first = (got[0] && JSON.parse(got[0].body || '{}')) || {};
    check(got.length === 1 && first.items && first.items[0] && first.items[0].title === 'Second' && first.index === 0 && first.shuffle === false, `${tag}: a click on song 2's row loads Kitchen's queue once, from song 2 (${got.length} loads, first ${first.items ? first.items[0].title : '-'})`);

    n0 = loads().length;
    await row(page, 2).locator('.title').dblclick();
    await settle(n0);
    got = loads().slice(n0);
    const dbl = (got[0] && JSON.parse(got[0].body || '{}')) || {};
    check(got.length === 1 && dbl.items && dbl.items[0] && dbl.items[0].title === 'Third', `${tag}: a double click on song 3's row loads the queue once, not twice (${got.length} loads)`);
    const picked = await page.evaluate(() => String(getSelection()));
    check(picked === '', `${tag}: and selects no word of the row ("${picked}")`);

    n0 = loads().length;
    await row(page, 0).hover();
    const pb = await row(page, 0).locator('[data-aplay]').boundingBox();
    await page.mouse.click(pb.x + pb.width / 2, pb.y + pb.height / 2, { clickCount: 1 });
    const tb = await row(page, 0).locator('.title').boundingBox();
    // (the second click of a double click: the browser counts it as 2 when it comes this quick)
    await page.mouse.move(tb.x + 10, tb.y + tb.height / 2);
    await page.mouse.down({ clickCount: 2 });
    await page.mouse.up({ clickCount: 2 });
    await settle(n0);
    got = loads().slice(n0);
    check(got.length === 1, `${tag}: ▶ then a quick second click on the same row is one start (${got.length} loads)`);
    // The row's ⋯ › Play next sends that one song to play next in Kitchen's queue (a mouse: the ⋯ shows on hover)
    const n1 = state.hits.length;
    await row(page, 1).hover();
    if (await row(page, 1).locator('[data-amore1]').count()) await row(page, 1).locator('[data-amore1]').click();
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item', { timeout: 2000 }).catch(() => {});
    const items = await page.evaluate(() => [...document.querySelectorAll('.popover.menu:not([hidden]) .pop-item')].map((b) => b.textContent.trim()));
    check(items.join('|') === 'Play next|Add to queue|Add to collection…', `${tag}: song 2's ⋯ is the service song's menu, not a library song's (${JSON.stringify(items)})`);
    if (items.length) await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Play next' }).click();
    for (let k = 0; k < 40 && !state.hits.slice(n1).some((h) => h.m === 'POST' && h.v === '/zones/z1/queue'); k++) await page.waitForTimeout(50);
    const qp = state.hits.slice(n1).find((h) => h.m === 'POST' && h.v === '/zones/z1/queue'); const qb = qp ? JSON.parse(qp.body || '{}') : {};
    check(!!qp && qb.at === 'next' && qb.items && qb.items.length === 1 && qb.items[0].title === 'Second' && qb.items[0].uri === tracks[1].url && !qb.replace, `${tag}: its Play next queues song 2 next in Kitchen (${qp ? qp.body : 'no POST'})`);
    check(state.errors.length === 0, `${tag}: no page errors ${JSON.stringify(state.errors)}`);
  } finally { await context.close(); }
}

// Phone mode (the Android app without a node): no 'collections' in /info's capabilities and /playlists
// answers 503. A song's ⋯ then has Play next and Add to queue only (as the shared song menus), and a
// swipe left adds to no playlist: no dialog, no /playlists request, only "isn't available for this row".
async function phoneModeRun(browser, width) {
  const tag = `${width} px phone mode (no collections)`;
  const pf = { ...fixtures,
    '/info': { body: { name: 'This phone', offline: false, capabilities: ['library', 'sources', 'bookmarks'], library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0 } } },
    '/playlists': { status: 503, body: { error: 'This needs an Odeion node' } } };
  const { page, context, state } = await fx.create(browser, width, { fixtures: pf, realAudio: true, mobile: true });
  await page.route('**/audio/*.wav', serveWav);
  await context.addInitScript(() => { window.pmDesktop = Object.assign(window.pmDesktop || {}, { platform: 'android' }); });
  try {
    await openItem(page);
    const more = row(page, 0).locator('[data-amore1]');
    if (await more.isVisible()) await more.tap();
    await page.waitForSelector('.popover.menu:not([hidden]) .pop-item', { timeout: 2000 }).catch(() => {});
    const items = await page.evaluate(() => [...document.querySelectorAll('.popover.menu:not([hidden]) .pop-item')].map((b) => b.textContent.trim()));
    check(items.join('|') === 'Play next|Add to queue', `${tag}: song 1's ⋯ has Play next and Add to queue only (${JSON.stringify(items)})`);
    if (width < 400) await page.screenshot({ path: path.join(SHOTS, `phone-mode-menu-${width}.png`) });
    await page.keyboard.press('Escape');
    await page.evaluate(() => { const m = document.querySelector('.popover.menu'); if (m) m.hidden = true; });
    // A swipe left on song 2's row: nothing is added and no playlist dialog opens
    const n0 = state.hits.length;
    const cdp = await context.newCDPSession(page);
    const rb = await row(page, 1).boundingBox();
    const y = rb.y + rb.height / 2, x0 = rb.x + rb.width - 40;
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: x0, y }] });
    for (let k = 1; k <= 8; k++) { await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: x0 - k * 15, y }] }); await page.waitForTimeout(16); }
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await page.waitForTimeout(600);
    const after = await page.evaluate(() => ({ dialog: !!document.querySelector('dialog[open]'), toasts: [...document.querySelectorAll('.toast')].map((t) => t.textContent.trim()) }));
    const pl = state.hits.slice(n0).filter((h) => h.v.startsWith('/playlists'));
    check(!after.dialog && pl.length === 0, `${tag}: a swipe left on song 2 opens no collection dialog and asks no /playlists (${after.dialog ? 'dialog open' : 'no dialog'}, ${pl.map((h) => h.m + ' ' + h.v).join(', ') || 'no request'})`);
    // (the swipe did reach the row's ⋯: it says so, as a shared song row in phone mode does)
    check(after.toasts.some((t) => /Add to collection isn.t available for this row/.test(t)), `${tag}: and says "Add to collection isn't available for this row" (${JSON.stringify(after.toasts)})`);
    check(state.errors.length === 0, `${tag}: no page errors ${JSON.stringify(state.errors)}`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) await browserRun(browser, w);
    await browserRun(browser, 768, { touch: true });
    for (const w of [360, 412]) await browserRun(browser, w, { mobile: true });
    for (const w of [1280, 768]) await nodeRun(browser, w);
    for (const w of [360, 412]) await phoneModeRun(browser, w);
  } finally { await browser.close(); }
  console.log(`source item row: ${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
