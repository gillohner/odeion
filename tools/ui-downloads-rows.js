// Downloads, offline (the app's local page, no node): the song rows' own buttons work, as on
// every other page. Each row's Play from here, Play next, Add to queue and heart act on that
// row's song, in the Favourites, All songs and an opened album's lists alike (⋯ is hidden offline).
//   node tools/ui-downloads-rows.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'downloads-rows-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// Amber Lights (id 3) sorts first in All songs, so a row's place there is not its id
const tracks = [[1, 'Bright Morning'], [2, 'Cold River'], [3, 'Amber Lights']].map(([id, title]) => ({ ...fx.track, id, title }));
const offline = { offline: true, device_id: 'fixture-device', synced_at: null, bytes: 1, queued: 0, tracks, albums: [{ id: 7, title: 'Journey', track_ids: [1, 2, 3], cover: false }], playlists: [], favourites: { track_ids: [2] } };
// The browser's queue as the app saves it (every 2 s): waits until it is `want` (4 s at most)
const queueTitles = async (p, want) => {
  await p.waitForFunction((w) => JSON.stringify(JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').map((q) => q.title)) === w, JSON.stringify(want), { timeout: 4000 }).catch(() => {});
  return p.evaluate(() => JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').map((q) => q.title));
};
const rowBtn = async (p, list, nth, act) => { const r = `[data-list="${list}"] .track:nth-child(${nth})`; await p.hover(r); await p.click(`${r} [data-act="${act}"]`); };

(async () => {
  const browser = await chromium.launch();
  for (const w of [1280, 1024, 768]) {
    const at = `${w}px`;
    const run = await fx.create(browser, w, { offline: true, tracks, fixtures: { '/offline': { body: offline } } });
    const p = run.page;
    await fx.go(p, '/downloads');
    await p.waitForSelector('[data-list="all"] .track', { timeout: 8000 });
    await p.waitForFunction(() => !!window.pmPlayer); await sleep(300);
    const titles = await p.$$eval('[data-list="all"] .track .title a', (as) => as.map((a) => a.textContent));
    check(JSON.stringify(titles) === JSON.stringify(['Amber Lights', 'Bright Morning', 'Cold River']), `${at}: All songs lists the kept songs by title (${JSON.stringify(titles)})`);

    // Play from here on All songs' second row: it and the rest of the list
    await rowBtn(p, 'all', 2, 'play');
    let q = await queueTitles(p, ['Bright Morning', 'Cold River']);
    check(JSON.stringify(q) === JSON.stringify(['Bright Morning', 'Cold River']), `${at}: All songs › row 2 › Play from here queues Bright Morning and the rest of the list (${JSON.stringify(q)})`);
    await p.waitForFunction(() => window.pmPlayer.now().index === 0 && /\/tracks\/1\/file/.test(window.pmPlayer.now().src), null, { timeout: 3000 }).catch(() => {});
    const n = await p.evaluate(() => window.pmPlayer.now());
    check(n.index === 0 && /\/tracks\/1\/file/.test(n.src), `${at}: and starts it (${n.index}, ${n.src})`);

    // Add to queue on the first row: at the end
    await rowBtn(p, 'all', 1, 'add');
    q = await queueTitles(p, ['Bright Morning', 'Cold River', 'Amber Lights']);
    check(JSON.stringify(q) === JSON.stringify(['Bright Morning', 'Cold River', 'Amber Lights']), `${at}: All songs › row 1 › Add to queue adds Amber Lights at the end (${JSON.stringify(q)})`);

    // Play next on the Favourites row: right after the playing song
    await rowBtn(p, 'fav', 1, 'next');
    q = await queueTitles(p, ['Bright Morning', 'Cold River', 'Cold River', 'Amber Lights']);
    check(JSON.stringify(q) === JSON.stringify(['Bright Morning', 'Cold River', 'Cold River', 'Amber Lights']), `${at}: Favourites › Play next puts Cold River right after the playing song (${JSON.stringify(q)})`);

    // The heart (kept here and sent to the node when it is back; ⋯ is hidden offline)
    await rowBtn(p, 'all', 1, 'fav');
    await p.waitForSelector('[data-list="all"] .track:nth-child(1) [data-act="fav"].on', { timeout: 3000 }).catch(() => {});
    const puts = run.state.hits.filter((h) => h.v === '/favourites' && h.m === 'PUT').map((h) => JSON.parse(h.body || '{}').track_id);
    const on = await p.$eval('[data-list="all"] .track:nth-child(1) [data-act="fav"]', (b) => b.classList.contains('on'));
    check(JSON.stringify(puts) === '[3]' && on, `${at}: All songs › row 1 › heart saves Amber Lights (track ${JSON.stringify(puts)}) and fills (${on})`);
    await p.screenshot({ path: path.join(SHOTS, `downloads-heart-${w}.png`) });

    // An opened album's rows
    await p.click('[data-dlalbum="7"]');
    await p.waitForSelector('[data-list="album"] .track', { timeout: 5000 });
    await rowBtn(p, 'album', 3, 'play');
    q = await queueTitles(p, ['Amber Lights']);
    check(JSON.stringify(q) === JSON.stringify(['Amber Lights']), `${at}: the opened album › row 3 › Play from here queues Amber Lights, its last song (${JSON.stringify(q)})`);
    await p.screenshot({ path: path.join(SHOTS, `downloads-album-${w}.png`) });

    check(!run.state.errors.length, `${at}: no page errors (${run.state.errors.join(' | ')})`);
    await run.context.close();
  }
  // A phone (no hover): a tap on the row plays from there
  const ph = await fx.create(browser, 390, { offline: true, tracks, mobile: true, fixtures: { '/offline': { body: offline } } });
  await fx.go(ph.page, '/downloads');
  await ph.page.waitForSelector('[data-list="all"] .track', { timeout: 8000 }); await sleep(300);
  await ph.page.tap('[data-list="all"] .track:nth-child(2) .dur');
  const q = await queueTitles(ph.page, ['Bright Morning', 'Cold River']);
  check(JSON.stringify(q) === JSON.stringify(['Bright Morning', 'Cold River']), `390px touch: a tap on All songs › row 2 plays Bright Morning and the rest of the list (${JSON.stringify(q)})`);
  await ph.page.screenshot({ path: path.join(SHOTS, 'downloads-tap-390.png') });
  check(!ph.state.errors.length, `390px touch: no page errors (${ph.state.errors.join(' | ')})`);
  await ph.context.close();
  await browser.close();
  console.log(failed ? `${failed} FAILED` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
