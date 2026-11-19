// This browser's "Starting on This browser…" names the song now starting. A song's Play pressed while
// the song before is still loading (a slow stream, a quick click on another row), or Next pressed then,
// shows the new song's name under it (before, the first song's name stayed until one played), and the
// spinner moves to the button just pressed (Next pressed soon after puts it on no row, nor do Next or a
// queue row on the bar's Play after a Pause). The new song playing ends the wait, also when the first
// song's file comes in late, and the same song played again while it loads keeps its one wait.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-browser-start-title.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/browser-start-title-shots');
fs.mkdirSync(shots, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, mbid: null, canonical_uri: null }));
const fixtures = { '/library/tracks': { body: { tracks, next: null } } };
for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };

// What the bar says: the wait's line and the song under it, or the song playing
const bar = (page) => page.evaluate(() => ({ wait: document.querySelector('#np .np-wait')?.textContent.trim() || null, sub: document.querySelector('#np .np-sub')?.textContent.trim() || '', title: document.querySelector('#np .np-title')?.textContent.trim() || '' }));
const spinning = (page) => page.evaluate(() => [...document.querySelectorAll('#tracklist .track')].map((r, i) => (r.querySelector('[data-act="play"].starting') ? i : -1)).filter((i) => i >= 0));
const playRow = async (page, i) => { const row = page.locator('#tracklist .track').nth(i); await row.hover(); await row.locator('[data-act="play"]').click(); };

async function stillLoading(browser, width) {
  // 1. No song ever plays (a fixture's play() does nothing): Song 1's Play, then Song 2's, then Next
  const { page, context, state } = await fx.create(browser, width, { fixtures });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    await sleep(300);
    await playRow(page, 0);
    await page.waitForFunction(() => document.querySelector('#np .np-wait'));
    let b = await bar(page);
    assert.match(b.wait || '', /^Starting on This browser/, `${width}: the bar says it is starting (${JSON.stringify(b)})`);
    assert.equal(b.sub, 'Song 1', `${width}: under it, Song 1 (${JSON.stringify(b)})`);
    // (when each press reached the page: the Next below must come within 1.5 s of Song 2's Play,
    // or a slow runner would pass without the spinner being kept off Song 3)
    await page.evaluate(() => { window.__pressAt = {}; document.addEventListener('click', (e) => { const b = e.target.closest('#next, #tracklist .track [data-act="play"]'); if (b) window.__pressAt[b.id === 'next' ? 'next' : 'play'] = Date.now(); }, true); });
    await playRow(page, 1);
    await sleep(200);
    await page.screenshot({ path: path.join(shots, `second-play-${width}.png`) });
    b = await bar(page);
    assert.match(b.wait || '', /^Starting on This browser/, `${width}: still starting (${JSON.stringify(b)})`);
    assert.equal(b.sub, 'Song 2', `${width}: Song 2's Play while Song 1 loads: the bar names Song 2 (${JSON.stringify(b)})`);
    assert.deepEqual(await spinning(page), [1], `${width}: the spinner is on Song 2's Play only`);
    // Next while Song 2 still loads: Song 3
    await page.click('#next');
    const gap = await page.evaluate(() => window.__pressAt.next - window.__pressAt.play);
    assert.ok(gap >= 0 && gap < 1500, `${width}: Next came ${gap} ms after Song 2's Play (must be under 1.5 s for this check to mean anything)`);
    await sleep(200);
    await page.screenshot({ path: path.join(shots, `after-next-${width}.png`) });
    b = await bar(page);
    assert.equal(b.sub, 'Song 3', `${width}: Next while Song 2 loads: the bar names Song 3 (${JSON.stringify(b)})`);
    assert.match(await page.evaluate(() => document.querySelector('audio').getAttribute('src') || ''), /\/library\/tracks\/3\//, `${width}: Song 3 is the one loading`);
    assert.deepEqual(await spinning(page), [], `${width}: Next soon after Song 2's Play: no row's Play spins (Song 3 wasn't started from one)`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

// A 20 s mono WAV (8 kHz), served with ranges
const WAV = (() => { const rate = 8000, n = rate * 20, b = Buffer.alloc(44 + n * 2); b.write('RIFF'); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); return b; })();
const serveWav = (r) => {
  const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || '');
  const a = m ? Number(m[1]) : 0, z = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1;
  return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
};

async function slowThenFast(browser, width) {
  // 2. Real audio: Song 1's file answers 2.5 s late, Song 2's at once. Song 1's Play, then Song 2's:
  // the bar names Song 2 while it starts, then shows Song 2 playing, and Song 1's late file changes nothing
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true });
  let late = 0;
  await page.route(/\/v1\/library\/tracks\/\d+\/file/, async (r) => {
    if (/\/tracks\/1\/file/.test(r.request().url())) { await sleep(2500); late++; return serveWav(r).catch(() => {}); }
    return serveWav(r);
  });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    await sleep(300);
    await playRow(page, 0);
    await page.waitForFunction(() => document.querySelector('#np .np-wait'));
    assert.equal((await bar(page)).sub, 'Song 1', `${width}: Song 1 starting`);
    await playRow(page, 1);
    for (let i = 0; i < 100; i++) { const n = await page.evaluate(() => pmPlayer.now()); if (!n.paused && n.time > 0) break; await sleep(50); }
    const now = await page.evaluate(() => pmPlayer.now());
    assert.match(now.src, /\/library\/tracks\/2\/file/, `${width}: Song 2 plays (${now.src})`);
    assert.equal(now.paused, false, `${width}: it plays`);
    await sleep(300);
    let b = await bar(page);
    assert.equal(b.wait, null, `${width}: Song 2 playing ends the wait (${JSON.stringify(b)})`);
    assert.equal(b.title, 'Song 2', `${width}: the bar shows Song 2 (${JSON.stringify(b)})`);
    await page.screenshot({ path: path.join(shots, `slow-then-fast-${width}.png`) });
    await sleep(2600);
    b = await bar(page);
    assert.equal(b.wait, null, `${width}: after Song 1's late file, no wait (${JSON.stringify(b)})`);
    assert.equal(b.title, 'Song 2', `${width}: after Song 1's late file, still Song 2 (${JSON.stringify(b)})`);
    assert.match((await page.evaluate(() => pmPlayer.now())).src, /\/library\/tracks\/2\/file/, `${width}: still Song 2's file`);
    assert.equal(late, 1, `${width}: Song 1's file did come in late`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function pauseThen(browser, width, how) {
  // 4. A press that starts no wait leaves no spinner behind: Song 1 playing, the bar's Pause, then Next
  // (or Song 2's row on Queue) within 1.5 s (Song 2's file comes late, so its wait lasts): nothing
  // spins (before, Song 2's wait put the spinner on the bar's Play button, as if Pause had started it)
  const { page, context, state } = await fx.create(browser, width, { fixtures, realAudio: true });
  await page.route(/\/v1\/library\/tracks\/\d+\/file/, async (r) => { if (/\/tracks\/2\/file/.test(r.request().url())) await sleep(2500); return serveWav(r).catch(() => {}); });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    await sleep(300);
    await playRow(page, 0);
    for (let i = 0; i < 100; i++) { const n = await page.evaluate(() => pmPlayer.now()); if (!n.paused && n.time > 0) break; await sleep(50); }
    await page.waitForFunction(() => !document.querySelector('#np .np-wait'));
    const row2 = page.locator('[data-qrow]').filter({ hasText: 'Song 2' }).locator('[data-qi]');
    if (how === 'queue') { await page.click('.sidebar .nav-item[data-view="queue"]'); await row2.waitFor(); }
    await sleep(1600); // (Song 1's Play press is over)
    await page.evaluate(() => { window.__pressAt = {}; document.addEventListener('click', (e) => { const b = e.target.closest('#next, #toggle, [data-qi]'); if (b) window.__pressAt[b.id || 'row'] = Date.now(); }, true); });
    await page.click('#toggle');
    await page.waitForFunction(() => pmPlayer.now().paused);
    if (how === 'queue') await row2.click(); else await page.click('#next');
    const gap = await page.evaluate((k) => window.__pressAt[k] - window.__pressAt.toggle, how === 'queue' ? 'row' : 'next');
    assert.ok(gap >= 0 && gap < 1500, `${width}: ${how} came ${gap} ms after Pause (must be under 1.5 s for this check to mean anything)`);
    await page.waitForFunction(() => document.querySelector('#np .np-wait'));
    const b = await bar(page);
    assert.equal(b.sub, 'Song 2', `${width}: Song 2 starting (${JSON.stringify(b)})`);
    await page.screenshot({ path: path.join(shots, `pause-then-${how}-${width}.png`) });
    const spin = await page.evaluate(() => [...document.querySelectorAll('.starting')].map((e) => e.id || e.getAttribute('aria-label') || e.className));
    assert.deepEqual(spin, [], `${width}: ${how === 'queue' ? 'a queue row' : 'Next'} soon after Pause: nothing spins (${JSON.stringify(spin)})`);
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

async function sameSongKeepsWait(browser, width) {
  // 3. The same song played again while it still loads (its row on Queue tapped) is the same start:
  // its one wait goes on, so 20 s after the first press the bar says it did not start (a fresh wait
  // each time would never end while the song is tapped again)
  const { page, context, state } = await fx.create(browser, width, { fixtures });
  try {
    await fx.go(page, '/tracks');
    await page.waitForSelector('#tracklist .track');
    await sleep(300);
    await playRow(page, 1);
    const t0 = Date.now();
    await page.waitForFunction(() => document.querySelector('#np .np-wait'));
    await page.click('.sidebar .nav-item[data-view="queue"]');
    const cur = page.locator('[data-qrow]').filter({ hasText: 'Song 2' }).locator('[data-qi]');
    await cur.waitFor();
    await sleep(Math.max(0, t0 + 12000 - Date.now()));
    await cur.click();
    await sleep(300);
    let b = await bar(page);
    assert.equal(b.sub, 'Song 2', `${width}: still starting Song 2 (${JSON.stringify(b)})`);
    await sleep(Math.max(0, t0 + 21000 - Date.now()));
    b = await bar(page);
    assert.equal(b.wait, null, `${width}: 20 s after the first press the wait is over (${JSON.stringify(b)})`);
    assert.match(b.sub, /did not start playing in 20 seconds/, `${width}: and the bar says so (${JSON.stringify(b)})`);
    await page.screenshot({ path: path.join(shots, `same-song-${width}.png`) });
    assert.deepEqual(state.errors, [], `${width}: no page errors`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  try {
    for (const width of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [1280, 1024, 768])) {
      for (const [name, fn] of [['a second Play while the first song loads', stillLoading], ['a slow song, then a quick one', slowThenFast], ['Pause, then Next', (b, w) => pauseThen(b, w, 'next')], ['Pause, then a queue row', (b, w) => pauseThen(b, w, 'queue')], ...(width === 1280 ? [['the same song again', sameSongKeepsWait]] : [])]) {
        try { await fn(browser, width); console.log(`PASS ${width} ${name}`); } catch (e) { failed++; console.log(`FAIL ${width} ${name}: ${e.message}`); }
      }
    }
  } finally { await browser.close(); }
  if (failed) { console.log(`${failed} failed`); process.exit(1); }
  console.log('all passed');
})();
