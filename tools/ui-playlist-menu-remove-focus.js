// Phones hide a playlist row's Move up / Move down / × (no room next to the title): the row's ⋯
// menu offers them. Remove from playlist chosen there with the keyboard (Enter on the ⋯, arrows,
// Enter) took the song out, but the focus was left on the removed song's ⋯, gone with the redraw:
// the next Tab started at the top of the page. Now it goes on to the next song's ⋯ (the one before
// when the last song went), as the × pressed from the keyboard goes on to the next song's ×.
// A finger or a mouse leaves the focus as before (no ring on a tap). At 360 (browser and the
// Android app), 412 and 1280 px (desktop: the × from the keyboard, unchanged).
//   node tools/ui-playlist-menu-remove-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-menu-remove-focus-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

const song = (id, title) => ({ ...fx.track, id, title, canonical_uri: `https://musicbrainz.org/recording/0000000${id}-0000-4000-8000-000000000000`, mbid: null });
const SONGS = { A: song(1, 'Alpha'), B: song(2, 'Bravo'), C: song(3, 'Charlie'), D: song(4, 'Delta') };
const NAMES = { m1: 'Keyboard', m2: 'Last song', m3: 'Tapped' };
const sum = (id, n) => ({ ...fx.summary, id, name: NAMES[id], items: n });
const detailOf = (id, keys) => ({ summary: sum(id, keys.length), playlist: { ...sum(id, keys.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => SONGS[k]), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: [] });
const pfOf = (keys) => ({ order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: keys.map((k, i) => ({ index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', track_id: SONGS[k].id, plays_from: { service: 'library', label: 'Your library' }, sources: [] })) });
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

async function run(browser, width, android) {
  const tag = `${width}${android ? '-android' : ''}`; const phone = width < 600;
  const { page, context } = await fx.create(browser, width, { mobile: phone });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  const lists = { m1: ['A', 'B', 'C', 'D'], m2: ['A', 'B', 'C'], m3: ['A', 'B', 'C'] }; const writes = [];
  await page.route('**/v1/pubky', (r) => json(r, { user: null, sync: { running: false } }));
  await page.route('**/v1/storage', (r) => json(r, { homeserver: { mode: 'none' }, saving: { state: 'saved' } }));
  await page.route(/\/v1\/playlists\/(m\d+)(\/.*)?$/, async (r) => {
    const u = new URL(r.request().url()); const m = r.request().method();
    const [, , , id, ...rest] = u.pathname.split('/'); const sub = rest.join('/');
    if (m === 'DELETE' && sub.startsWith('items/')) { const k = Number(sub.split('/')[1]); writes.push(`${id} DELETE ${k}`); lists[id].splice(k, 1); return json(r, {}); }
    if (m !== 'GET') return r.fallback();
    if (sub === 'play-from') return json(r, pfOf(lists[id]));
    if (!sub) return json(r, detailOf(id, lists[id]));
    return r.fallback();
  });
  await page.route('**/v1/playlists', (r) => (r.request().method() === 'GET' ? json(r, Object.keys(NAMES).map((id) => sum(id, lists[id].length))) : r.fallback()));
  const order = () => page.evaluate(() => [...document.querySelectorAll('#plsongs .srow')].map((row) => { const n = row.querySelector('.name'); return (n ? n.textContent : row.textContent).trim().split(/\s/)[0]; }).filter(Boolean).join(','));
  // Which song's control has the focus: "<song>:<kind>" (kind smore = its ⋯, rm = its ×)
  const focused = () => page.evaluate(() => { const a = document.activeElement; const row = a && a.closest && a.closest('#plsongs .srow'); if (!row) return a ? `${a.tagName.toLowerCase()}${a.id ? '#' + a.id : ''}` : 'none'; const n = row.querySelector('.name'); const kind = a.dataset.smore != null ? 'smore' : a.dataset.rm != null ? 'rm' : a.tagName.toLowerCase(); return `${n.textContent.trim().split(/\s/)[0]}:${kind}`; });
  const ctl = (name, sel) => page.locator('#plsongs .srow', { hasText: name }).first().locator(sel);
  const open = async (id, want) => {
    await page.evaluate((x) => { const st = { v: 'playlist', arg: x }; history.pushState(st, '', `/playlist/${x}`); dispatchEvent(new PopStateEvent('popstate', { state: st })); }, id);
    return until(async () => (await order()) === want && !!(await page.$('#plsongs [data-smore]')), 5000);
  };
  const menuOpen = () => page.locator('.popover.menu:not([hidden]) .pop-item').first().isVisible().catch(() => false);
  // Enter on a song's ⋯, the arrows down to Remove from playlist, Enter (false: the menu has no such item)
  const keyRemove = async (name) => {
    await ctl(name, '[data-smore]').focus();
    await page.keyboard.press('Enter');
    if (!(await until(menuOpen, 3000))) return false;
    for (let i = 0; i < 30; i++) {
      const at = await page.evaluate(() => (document.activeElement && document.activeElement.closest('.popover.menu') ? document.activeElement.textContent.trim() : ''));
      if (at === 'Remove from playlist') { await page.keyboard.press('Enter'); return true; }
      await page.keyboard.press('ArrowDown');
    }
    await page.keyboard.press('Escape');
    return false;
  };
  try {
    await fx.go(page, '/playlists');
    if (phone) {
      // 1. Keyboard, from the ⋯ menu: the focus goes on to the next song's ⋯
      check(await open('m1', 'Alpha,Bravo,Charlie,Delta'), `${tag}: the playlist shows (${await order()})`);
      check(await keyRemove('Bravo'), `${tag}: Bravo's ⋯ menu offers Remove from playlist from the keyboard`);
      await until(async () => (await order()) === 'Alpha,Charlie,Delta', 4000);
      await until(async () => (await focused()) === 'Charlie:smore', 2000);
      check(writes.join() === 'm1 DELETE 1' && (await order()) === 'Alpha,Charlie,Delta', `${tag}: Bravo is removed (${writes}; ${await order()})`);
      check((await focused()) === 'Charlie:smore', `${tag}: the focus goes on to the next song's ⋯ (${await focused()})`);
      await page.screenshot({ path: path.join(SHOTS, `after-keyboard-${tag}.png`) });
      // and Tab goes on from there, not from the top of the page
      await page.keyboard.press('Tab');
      const afterTab = await focused();
      check(/^Delta:/.test(afterTab), `${tag}: Tab then goes on to the song after it (${afterTab})`);

      // 2. The last song: the focus goes to the song before it
      check(await open('m2', 'Alpha,Bravo,Charlie'), `${tag}: the second playlist shows (${await order()})`);
      check(await keyRemove('Charlie'), `${tag}: Charlie's ⋯ menu offers Remove from playlist`);
      await until(async () => (await order()) === 'Alpha,Bravo', 4000);
      await until(async () => (await focused()) === 'Bravo:smore', 2000);
      check((await focused()) === 'Bravo:smore', `${tag}: the last song removed: the focus goes to the song before it (${await focused()})`);

      // 3. A finger: tap ⋯, tap Remove from playlist: no focus moved onto another song's ⋯
      check(await open('m3', 'Alpha,Bravo,Charlie'), `${tag}: the third playlist shows (${await order()})`);
      await ctl('Alpha', '[data-smore]').click();
      await until(menuOpen, 3000);
      await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from playlist' }).click();
      await until(async () => (await order()) === 'Bravo,Charlie', 4000);
      await sleep(300);
      check((await order()) === 'Bravo,Charlie', `${tag}: a tap removes Alpha (${await order()})`);
      check(!/:smore$/.test(await focused()), `${tag}: a tap leaves no focus ring on another song's ⋯ (${await focused()})`);
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${tag}: no sideways overflow`);
    } else {
      // Desktop: the × shows in the row; from the keyboard it goes on to the next song's × (as before)
      check(await open('m1', 'Alpha,Bravo,Charlie,Delta'), `${tag}: the playlist shows (${await order()})`);
      await ctl('Bravo', '[data-rm]').focus();
      await page.keyboard.press('Enter');
      await until(async () => (await order()) === 'Alpha,Charlie,Delta', 4000);
      await until(async () => (await focused()) === 'Charlie:rm', 2000);
      check((await focused()) === 'Charlie:rm', `${tag}: × from the keyboard goes on to the next song's × (${await focused()})`);
    }
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
