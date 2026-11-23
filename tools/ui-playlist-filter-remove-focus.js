// A playlist with a source chip on ("3 Your library") shows only that source's songs. Remove from
// playlist chosen from the keyboard (the ⋯ menu on phones, the × on a desktop) sent the focus on to
// the next song in the whole playlist; when that song was another source's, hidden by the chip, the
// focus fell to the page and the next Tab started at the top. Now it goes on to the next song that
// shows (the shown one before when none shows after it). And removing the chip's last song left an
// empty list with no chip on: now the whole playlist shows again with "N songs" on, and the focus
// goes on to the next song. At 360 (browser and the Android app), 412 and 1280 px.
//   node tools/ui-playlist-filter-remove-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-filter-remove-focus-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

const song = (id, title) => ({ ...fx.track, id, title, canonical_uri: `https://musicbrainz.org/recording/0000000${id}-0000-4000-8000-000000000000`, mbid: null });
// Alpha and Charlie play from the library, Bravo and Delta from TIDAL
const SONGS = { A: song(1, 'Alpha'), B: song(2, 'Bravo'), C: song(3, 'Charlie'), D: song(4, 'Delta') };
const FROM = { A: 'library', B: 'tidal', C: 'library', D: 'tidal' };
const NAMES = { f1: 'Next shown', f2: 'Last shown', f3: 'Chip emptied' };
const sum = (id, n) => ({ ...fx.summary, id, name: NAMES[id], items: n });
const detailOf = (id, keys) => ({ summary: sum(id, keys.length), playlist: { ...sum(id, keys.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => (FROM[k] === 'library' ? SONGS[k] : null)), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: [] });
const pfOf = (keys) => ({ order: { order: ['library', 'tidal'], services: [{ id: 'library', name: 'Your library', connected: true }, { id: 'tidal', name: 'TIDAL', connected: true }] }, own_order: false, items: keys.map((k, i) => ({ index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', duration_ms: 240000, track_id: FROM[k] === 'library' ? SONGS[k].id : null, stream_uri: FROM[k] === 'tidal' ? `tidal:track:${SONGS[k].id}` : null, plays_from: { service: FROM[k], label: FROM[k] === 'library' ? 'Your library' : 'TIDAL' }, sources: [] })) });
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

async function run(browser, width, android) {
  const tag = `${width}${android ? '-android' : ''}`; const phone = width < 600;
  const { page, context } = await fx.create(browser, width, { mobile: phone });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  const lists = { f1: ['A', 'B', 'C', 'D'], f2: ['A', 'B', 'C', 'D'], f3: ['A', 'B', 'C'] }; const writes = [];
  await page.route('**/v1/pubky', (r) => json(r, { user: null, sync: { running: false } }));
  await page.route('**/v1/storage', (r) => json(r, { homeserver: { mode: 'none' }, saving: { state: 'saved' } }));
  await page.route(/\/v1\/playlists\/(f\d+)(\/.*)?$/, async (r) => {
    const u = new URL(r.request().url()); const m = r.request().method();
    const [, , , id, ...rest] = u.pathname.split('/'); const sub = rest.join('/');
    if (m === 'DELETE' && sub.startsWith('items/')) { const k = Number(sub.split('/')[1]); writes.push(`${id} DELETE ${k}`); lists[id].splice(k, 1); return json(r, {}); }
    if (m !== 'GET') return r.fallback();
    if (sub === 'play-from') return json(r, pfOf(lists[id]));
    if (!sub) return json(r, detailOf(id, lists[id]));
    return r.fallback();
  });
  await page.route('**/v1/playlists', (r) => (r.request().method() === 'GET' ? json(r, Object.keys(NAMES).map((id) => sum(id, lists[id].length))) : r.fallback()));
  // The songs that show (a chip hides the other sources' rows)
  const shown = () => page.evaluate(() => [...document.querySelectorAll('#plsongs .srow')].filter((row) => row.getClientRects().length).map((row) => { const n = row.querySelector('.name'); return (n ? n.textContent : row.textContent).trim().split(/\s/)[0]; }).filter(Boolean).join(','));
  const all = () => page.evaluate(() => [...document.querySelectorAll('#plsongs .srow')].map((row) => { const n = row.querySelector('.name'); return (n ? n.textContent : row.textContent).trim().split(/\s/)[0]; }).filter(Boolean).join(','));
  const chipOn = () => page.evaluate(() => { const c = document.querySelector('#plsongs [data-pfilter][aria-pressed="true"]'); return c ? c.textContent.trim() : 'none'; });
  const focused = () => page.evaluate(() => { const a = document.activeElement; const row = a && a.closest && a.closest('#plsongs .srow'); if (!row) return a ? `${a.tagName.toLowerCase()}${a.id ? '#' + a.id : ''}` : 'none'; const n = row.querySelector('.name'); const kind = a.dataset.smore != null ? 'smore' : a.dataset.rm != null ? 'rm' : a.tagName.toLowerCase(); return `${n.textContent.trim().split(/\s/)[0]}:${kind}`; });
  const ctl = (name, sel) => page.locator('#plsongs .srow', { hasText: name }).first().locator(sel);
  const kind = phone ? 'smore' : 'rm';
  const open = async (id, want, chip) => {
    await page.evaluate((x) => { const st = { v: 'playlist', arg: x }; history.pushState(st, '', `/playlist/${x}`); dispatchEvent(new PopStateEvent('popstate', { state: st })); }, id);
    if (!(await until(async () => (await all()) === want && !!(await page.$(`#plsongs [data-pfilter="${chip}"]`)), 5000))) return false;
    await page.click(`#plsongs [data-pfilter="${chip}"]`);
    return until(async () => (await page.getAttribute(`#plsongs [data-pfilter="${chip}"]`, 'aria-pressed')) === 'true', 3000);
  };
  const menuOpen = () => page.locator('.popover.menu:not([hidden]) .pop-item').first().isVisible().catch(() => false);
  // From the keyboard: phones Enter on the song's ⋯, the arrows down to Remove from playlist, Enter;
  // a desktop Enter on its ×
  const keyRemove = async (name) => {
    if (!phone) { await ctl(name, '[data-rm]').focus(); await page.keyboard.press('Enter'); return true; }
    await ctl(name, '[data-smore]').focus();
    await page.keyboard.press('Enter');
    if (!(await until(menuOpen, 3000))) return false;
    for (let i = 0; i < 30; i++) {
      const at = await page.evaluate(() => (document.activeElement && document.activeElement.closest('.popover.menu') ? document.activeElement.textContent.trim() : ''));
      if (at === 'Remove from collection') { await page.keyboard.press('Enter'); return true; }
      await page.keyboard.press('ArrowDown');
    }
    await page.keyboard.press('Escape');
    return false;
  };
  try {
    await fx.go(page, '/playlists');
    // 1. Library chip on (Alpha, Charlie show): Alpha removed, the focus goes on to Charlie, not to the hidden Bravo
    check(await open('f1', 'Alpha,Bravo,Charlie,Delta', 'library'), `${tag}: the Library chip shows ${await shown()}`);
    check(await keyRemove('Alpha'), `${tag}: Alpha's Remove from collection from the keyboard`);
    await until(async () => (await all()) === 'Bravo,Charlie,Delta', 4000);
    await until(async () => (await focused()) === `Charlie:${kind}`, 2000);
    check(writes.join() === 'f1 DELETE 0' && (await shown()) === 'Charlie', `${tag}: Alpha is removed, the chip still on (${writes}; shows ${await shown()})`);
    check((await focused()) === `Charlie:${kind}`, `${tag}: the focus goes on to the next song that shows, Charlie (${await focused()})`);
    await page.screenshot({ path: path.join(SHOTS, `after-next-shown-${tag}.png`) });

    // 2. The last song that shows (Delta after it is hidden): the focus goes to the shown one before it
    check(await open('f2', 'Alpha,Bravo,Charlie,Delta', 'library'), `${tag}: the second collection's Library chip shows ${await shown()}`);
    check(await keyRemove('Charlie'), `${tag}: Charlie's Remove from collection from the keyboard`);
    await until(async () => (await all()) === 'Alpha,Bravo,Delta', 4000);
    await until(async () => (await focused()) === `Alpha:${kind}`, 2000);
    check((await focused()) === `Alpha:${kind}`, `${tag}: the last shown song removed: the focus goes to the shown one before it, Alpha (${await focused()})`);

    // 3. The chip's only song removed: the whole playlist shows again, "N songs" on, focus on the next song
    check(await open('f3', 'Alpha,Bravo,Charlie', 'tidal'), `${tag}: the TIDAL chip shows ${await shown()}`);
    check((await shown()) === 'Bravo', `${tag}: only Bravo shows under the TIDAL chip (${await shown()})`);
    check(await keyRemove('Bravo'), `${tag}: Bravo's Remove from collection from the keyboard`);
    await until(async () => (await all()) === 'Alpha,Charlie', 4000);
    await until(async () => (await shown()) === 'Alpha,Charlie' && (await focused()) === `Charlie:${kind}`, 2000);
    check((await shown()) === 'Alpha,Charlie', `${tag}: with no TIDAL song left the whole collection shows (${await shown()})`);
    check(/^2 songs/.test(await chipOn()), `${tag}: "2 songs" is the chip on (${await chipOn()})`);
    check((await focused()) === `Charlie:${kind}`, `${tag}: the focus goes on to the next song, Charlie (${await focused()})`);
    await page.screenshot({ path: path.join(SHOTS, `after-chip-emptied-${tag}.png`) });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${tag}: no sideways overflow`);
  } catch (e) { check(false, `${tag}: ${e.message}`); }
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) await run(browser, width, android);
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAILED` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
