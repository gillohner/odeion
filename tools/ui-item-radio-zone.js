// Start radio from a page's ⋯ (itemMenu: a song row on an album page, a song page's ⋯) goes on for
// the zone whose queue the song replaced, as a Songs row's Start radio (trackMenu) does. Switch to
// another zone while the song is on its way: before, the zone switched to got the radio and the one
// now playing the song had none, without a word. Now the first zone's radio goes on and the toast
// names it. Without a switch: that zone, and the same "Radio from “…”" toast as a Songs row.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-item-radio-zone.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'item-radio-zone-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };

const song = { no: 1, title: fx.track.title, artist: 'Sefa', artist_id: 'lib-11', album_id: 'lib-7', duration_ms: 240000, uri: 'track:1', track_id: 1, id: 'lib-1', page: 'lib-1' };
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: {}, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [song], total_ms: 240000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
const Z = (id, name) => ({ id, name, output: 'upnp:' + id, transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } });
const z1 = Z('z1', 'Kitchen'); const z2 = Z('z2', 'Living room');
const fixtures = {
  '/item/album/lib-7': { body: albumPage },
  '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
  '/zones': { body: [z1, z2] }, '/zones/z1': { body: z1 }, '/zones/z2': { body: z2 }, '/zones/z1/queue': { body: [] }, '/zones/z2/queue': { body: [] },
  '/outputs': { body: [{ id: 'upnp:z1', name: 'Kitchen', kind: 'upnp', caps: {} }, { id: 'upnp:z2', name: 'Living room', kind: 'upnp', caps: {} }] },
};

async function run(browser, width, switchZone) {
  const { context, page, state } = await fx.create(browser, width, { fixtures, zone: 'z1' });
  await context.routeWebSocket(/\/v1\/events/, (w) => { w.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') w.send(JSON.stringify({ type: 'pong', id: j.id })); }); w.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [z1, z2] })); });
  let go = null; const hold = { wait: new Promise((r) => { go = r; }) };
  await context.route(/\/v1\/zones\/z[12]\/(queue|radio|play|transport)$/, async (r) => {
    // (answered here, so noted here: the fixture's own log never sees them)
    const q = r.request(); if (q.method() === 'GET') return r.fallback();
    const v = new URL(q.url()).pathname.slice(3);
    state.hits.push({ v, m: q.method(), q: '', body: q.postData(), at: Date.now() });
    if (switchZone && q.method() === 'POST' && v === '/zones/z1/queue') await hold.wait;
    return r.fulfill({ status: 200, contentType: 'application/json', body: '{}' }).catch(() => {});
  });
  const tag = `${width}${switchZone ? ', switched' : ''}`;
  await fx.go(page, '/album/lib-7');
  await page.waitForSelector('#isongs [data-smore="0"]');
  await page.waitForFunction(() => /Kitchen/.test((document.querySelector('#zonename, #zonebtn') || {}).textContent || ''));
  await page.click('#isongs [data-smore="0"]', { force: true });
  await page.waitForSelector('.popover.menu:not([hidden]) .pop-item');
  const n = state.hits.length;
  const sent = () => state.hits.slice(n).filter((h) => h.m !== 'GET').map((h) => `${h.m} ${h.v}${/\/radio$/.test(h.v) ? ' ' + h.body : ''}`);
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Start radio' }).click();
  if (switchZone) {
    for (let t = 0; t < 60 && !sent().includes('POST /zones/z1/queue'); t++) await page.waitForTimeout(50);
    check(sent().includes('POST /zones/z1/queue'), `${tag}: the song is on its way to the Kitchen (${sent().join(', ')})`);
    // the speaker switched to the Living room meanwhile, in the bar's zone picker
    await page.click('#zonebtn');
    await page.click('.popover:not([hidden]) [data-zone="z2"]');
    await page.waitForFunction(() => /Living room/.test((document.querySelector('#zonebtn') || {}).textContent || ''));
    go();
  }
  for (let t = 0; t < 40 && !sent().some((s) => /\/radio /.test(s)); t++) await page.waitForTimeout(50);
  const want = `Radio from “${fx.track.title}”${switchZone ? ' on Kitchen' : ''}: similar music follows`;
  await page.getByText(want, { exact: true }).first().waitFor({ timeout: 3000 }).catch(() => {});
  const radio = sent().filter((s) => /\/radio /.test(s));
  check(sent()[0] === 'POST /zones/z1/queue', `${tag}: Start radio replaces the Kitchen's queue (${sent().join(', ')})`);
  check(radio.join(',') === 'PUT /zones/z1/radio {"on":true}', `${tag}: and turns the Kitchen's radio on${switchZone ? ', not the Living room\'s' : ''} (${radio.join(', ')})`);
  const toasts = await page.$$eval('.toast, #toasts *', (els) => els.map((e) => e.textContent.trim()).filter(Boolean));
  check(await page.getByText(want, { exact: true }).count() > 0, `${tag}: the toast says "${want}" (${[...new Set(toasts)].join(' | ')})`);
  await page.screenshot({ path: path.join(SHOTS, `after-item-radio-${width}${switchZone ? '-switched' : ''}.png`) });
  check(state.errors.length === 0, `${tag}: no page errors ${state.errors.join('; ')}`);
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768]) { await run(browser, w, false); await run(browser, w, true); } } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
