// A service's own album page (Sources › SoundCloud › a set, archive.org, Bandcamp …: the
// `source-item` page) plays a song when its row is clicked, as a library album's, a playlist's and
// the Songs list's rows do. Before, only the row's small ▶ played it, and that ▶ only shows on a
// mouse hover, so on a touch screen the row had no visible way to play. Checked: a click on the
// title (and on an empty spot of the row) plays from that song, the row shows a pointer, the row's
// own buttons keep their job (+ only adds, ▶ plays once), the second click of a double click
// doesn't start the song again (a node zone gets one queue load), a drag that selects the title
// doesn't play, and a phone tap on the row plays. 1280/1024/768 px, a 768 px touch tablet, 360 px phone.
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
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/zones/z1/load': { body: Z }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
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
    check(state.errors.length === 0, `${tag}: no page errors ${JSON.stringify(state.errors)}`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) await browserRun(browser, w);
    await browserRun(browser, 768, { touch: true });
    await browserRun(browser, 360, { mobile: true });
    for (const w of [1280, 768]) await nodeRun(browser, w);
  } finally { await browser.close(); }
  console.log(`source item row: ${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
