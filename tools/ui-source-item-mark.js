// A service's own album page (Sources › SoundCloud › a set, archive.org, Bandcamp …: the
// `source-item` page) marks the song that is playing, as every other song list does: the row's
// title in the accent colour with the playing mark before it, and aria-current="true" for a screen
// reader. Before, its rows had neither, so after "Play from here" nothing on the page said which
// song was on. Checked: the mark follows the song in place (the bar's Next moves it without
// redrawing the list), it is there when the page opens on a song already playing, and it goes when
// the song is from somewhere else, in this browser and on a node zone. Phone (360/412) and desktop (1280).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-source-item-mark.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'source-item-mark-shots'));
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
const other = { title: 'Elsewhere', creator: 'Sefa', year: 2021, url: 'https://soundcloud.com/sefa/sets/elsewhere', image: null, tracks: [{ url: `${fx.ORIGIN}/audio/away.wav`, title: 'Away', track_no: 1, duration_ms: 200000, format: 'mp3' }] };
const fixtures = {
  '/sources': { body: [{ id: 'soundcloud', name: 'SoundCloud', enabled: true, status: 'signed in' }] },
  '/sources/soundcloud/likes': { body: { tracks: [], playlists: [{ id: 'abc', title: 'Journey', creator: 'Sefa' }, { id: 'def', title: 'Elsewhere', creator: 'Sefa' }] } },
  '/sources/soundcloud/playlists': { body: [] },
  '/sources/soundcloud/items/abc': { body: item },
  '/sources/soundcloud/items/def': { body: other },
};

// Which rows are marked, and how
const marks = (page) => page.evaluate(() => [...document.querySelectorAll('#main .tracks .track')].map((r) => {
  const t = r.querySelector('.title');
  return { cur: r.classList.contains('cur'), aria: r.getAttribute('aria-current'), colour: getComputedStyle(t).color, before: getComputedStyle(t, '::before').content };
}));
const curRows = (m) => m.map((x, i) => (x.cur ? i : -1)).filter((i) => i >= 0);
const ariaRows = (m) => m.map((x, i) => (x.aria === 'true' ? i : -1)).filter((i) => i >= 0);
const onBar = (page) => page.evaluate(() => (document.querySelector('.player .np-title') || {}).textContent || '');
const openItem = async (page, id) => {
  await page.click(`[data-sitem="${id}"]`);
  await page.waitForSelector('#main .tracks .track');
};

async function run(browser, width) {
  const phone = width < 600;
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true, mobile: phone });
  await page.route('**/audio/*.wav', serveWav);
  const tag = `${width} px`;
  try {
    await fx.go(page, '/sources/soundcloud');
    await openItem(page, 'abc');
    let m = await marks(page);
    check(curRows(m).length === 0 && ariaRows(m).length === 0, `${tag}: nothing playing, no row is marked (${JSON.stringify(curRows(m))})`);
    await page.screenshot({ path: path.join(SHOTS, `idle-${width}.png`) });

    // Play from here on the second song: that row is the one marked
    await page.locator('#main .tracks .track').nth(1).hover().catch(() => {});
    await page.click('[data-aplay="1"]', { force: true });
    await page.waitForFunction(() => document.querySelector('#main .tracks .track.cur'), null, { timeout: 3000 }).catch(() => {});
    await page.waitForFunction(() => /Second/.test((document.querySelector('.player .np-title') || {}).textContent || ''), null, { timeout: 3000 }).catch(() => {});
    check(/Second/.test(await onBar(page)), `${tag}: the player is on song 2 ("${await onBar(page)}")`);
    m = await marks(page);
    check(JSON.stringify(curRows(m)) === '[1]', `${tag}: after Play from here on song 2, song 2 is marked (${JSON.stringify(curRows(m))})`);
    check(JSON.stringify(ariaRows(m)) === '[1]', `${tag}: and a screen reader hears it as the current one (aria-current ${JSON.stringify(ariaRows(m))})`);
    check(m[1] && m[1].before !== 'none' && m[1].colour !== m[0].colour, `${tag}: the marked row looks marked (playing mark ${m[1] && m[1].before}, colour ${m[1] && m[1].colour} vs ${m[0] && m[0].colour})`);
    await page.screenshot({ path: path.join(SHOTS, `playing-${width}.png`) });

    // The next song: the mark moves in place, the list is not drawn again
    await page.evaluate(() => { document.querySelector('#main .tracks').dataset.probe = 'same'; });
    // (the bar's Next; on a phone it sits in Now playing, so it is pressed from the page)
    if (await page.locator('#next').isVisible()) await page.click('#next');
    else await page.evaluate(() => document.querySelector('#next').click());
    await page.waitForFunction(() => { const r = document.querySelectorAll('#main .tracks .track'); return r[2] && r[2].classList.contains('cur'); }, null, { timeout: 3000 }).catch(() => {});
    await page.waitForFunction(() => /Third/.test((document.querySelector('.player .np-title') || {}).textContent || ''), null, { timeout: 3000 }).catch(() => {});
    check(/Third/.test(await onBar(page)), `${tag}: the player is on song 3 ("${await onBar(page)}")`);
    m = await marks(page);
    const same = await page.evaluate(() => document.querySelector('#main .tracks')?.dataset.probe === 'same');
    check(JSON.stringify(curRows(m)) === '[2]' && JSON.stringify(ariaRows(m)) === '[2]', `${tag}: the next song moves the mark to song 3 (${JSON.stringify(curRows(m))}/${JSON.stringify(ariaRows(m))})`);
    check(same, `${tag}: in place, the list is not drawn again`);

    // Another album of the service: none of its rows is the song on
    await page.goBack();
    await page.waitForSelector('[data-sitem="def"]');
    await openItem(page, 'def');
    m = await marks(page);
    check(curRows(m).length === 0 && ariaRows(m).length === 0, `${tag}: another album's page marks nothing (${JSON.stringify(curRows(m))})`);

    // Back on the album while its song plays: marked as the page opens
    await page.goBack();
    await page.waitForSelector('[data-sitem="abc"]');
    await openItem(page, 'abc');
    m = await marks(page);
    check(JSON.stringify(curRows(m)) === '[2]' && JSON.stringify(ariaRows(m)) === '[2]', `${tag}: opened again while song 3 plays, song 3 is marked at once (${JSON.stringify(curRows(m))}/${JSON.stringify(ariaRows(m))})`);
    if (phone) {
      const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      check(over <= 0, `${tag}: no sideways scroll (${over} px over)`);
    }
    await page.screenshot({ path: path.join(SHOTS, `reopened-${width}.png`) });
    check(state.errors.length === 0, `${tag}: no page errors ${JSON.stringify(state.errors)}`);
  } finally { await context.close(); }
}

// A node zone (Kitchen) already on song 2 of the album, the page opened fresh: the mark is drawn
// with the page (the queue holds the same address the page lists), then the zone moving on to
// song 3 moves it
async function nodeZone(browser, width) {
  const tag = `${width} px, node zone`;
  const np = (t) => ({ item: { uri: t.url, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: t.duration_ms }, position_ms: 1000, duration_ms: t.duration_ms });
  const Z = { id: 'z1', name: 'Kitchen', output: 'upnp:k', transport: 'playing', now_playing: np(tracks[1]), queue: [], queue_index: 1, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } };
  const zf = { ...fixtures, '/zones': { body: [Z] }, '/zones/z1': { body: Z }, '/outputs': { body: [{ id: 'upnp:k', name: 'Kitchen', kind: 'upnp', caps: { hw_volume: 'percent' } }] } };
  const { page, context, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1', mobile: width < 600 });
  let sock = null;
  await context.routeWebSocket(/\/v1\/events/, (ws) => { sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [Z] })); });
  try {
    await fx.go(page, '/sources/soundcloud');
    await page.waitForFunction(() => /Second/.test((document.querySelector('.player .np-title') || {}).textContent || ''), null, { timeout: 5000 }).catch(() => {});
    check(/Second/.test(await onBar(page)), `${tag}: Kitchen is on song 2 ("${await onBar(page)}")`);
    await openItem(page, 'abc');
    let m = await marks(page);
    check(JSON.stringify(curRows(m)) === '[1]' && JSON.stringify(ariaRows(m)) === '[1]', `${tag}: the page opens with song 2 marked (${JSON.stringify(curRows(m))}/${JSON.stringify(ariaRows(m))})`);
    await page.evaluate(() => { document.querySelector('#main .tracks').dataset.probe = 'same'; });
    sock.send(JSON.stringify({ type: 'zone', seq: 1, state: { ...Z, now_playing: np(tracks[2]), queue_index: 2 } }));
    await page.waitForFunction(() => { const r = document.querySelectorAll('#main .tracks .track'); return r[2] && r[2].classList.contains('cur'); }, null, { timeout: 3000 }).catch(() => {});
    m = await marks(page);
    const same = await page.evaluate(() => document.querySelector('#main .tracks')?.dataset.probe === 'same');
    check(JSON.stringify(curRows(m)) === '[2]' && JSON.stringify(ariaRows(m)) === '[2]' && same, `${tag}: Kitchen moving on moves the mark to song 3, in place (${JSON.stringify(curRows(m))}/${JSON.stringify(ariaRows(m))}, same list ${same})`);
    await page.screenshot({ path: path.join(SHOTS, `node-zone-${width}.png`) });
    check(state.errors.length === 0, `${tag}: no page errors ${JSON.stringify(state.errors)}`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [360, 412, 1280]) await run(browser, w);
    for (const w of [360, 1280]) await nodeZone(browser, w);
  } finally { await browser.close(); }
  console.log(`source item mark: ${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
