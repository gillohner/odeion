// Where the focus goes in a collection drawn again while it is on one of its songs.
// 1. Another device changes the list (nobody pressed anything here): the focus was put back by its
//    place in the list, so with a song above it removed or moved, it went to the ×, Move or row of
//    whichever song now sat at that place, and the next Enter removed, moved or played that song.
//    Now it stays on the same song's same control wherever that song is now; when that song itself
//    went, it goes on to the next song (the one before when it was the last), its ⋯.
// 2. A song the focus was handed on to, when the person had scrolled away from the song it was on:
//    the page stays where they scrolled it (it jumped back to the song). In sight, it still follows.
// 3. After a Remove from the keyboard, the page read again before the read the Remove asked for has
//    answered (another device's change), and the answer for where the songs play, asked again, still
//    for the list before: the rows hold (#plsongs[inert]) while the list is read once more, and then
//    the focus the × handed on to the next song lands on that song's ⋯, not its × (that read isn't the
//    one the Remove asked for: a second Enter there would remove a song nobody picked).
// 3b. The same when every read after the Remove is overtaken by the next library change for longer
//    than the hold's 30 s (an import, a busy node): the hold's end draws it, the focus on Delta's ⋯.
//    (A computer only: phones hand on to the ⋯ in any case. The hold's 30 s run as 3 s here.)
// At 360 (browser and the Android app), 412 and 1280 px.
//   node tools/ui-playlist-focus-remote.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-focus-remote-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

const song = (id, title) => ({ ...fx.track, id, title, canonical_uri: `https://musicbrainz.org/recording/0000000${id}-0000-4000-8000-000000000000`, mbid: null });
// Alpha, Charlie and Echo play from the library, Bravo and Delta from TIDAL
const SONGS = { A: song(1, 'Alpha'), B: song(2, 'Bravo'), C: song(3, 'Charlie'), D: song(4, 'Delta'), E: song(5, 'Echo') };
const FROM = { A: 'library', B: 'tidal', C: 'library', D: 'tidal', E: 'library' };
const NAMES = { r1: 'Changed above', r3: 'Removed itself', r5: 'Scrolled away', r6: 'Asked again' };
const sum = (id, n) => ({ ...fx.summary, id, name: NAMES[id], items: n });
const detailOf = (id, keys) => ({ summary: sum(id, keys.length), playlist: { ...sum(id, keys.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => (FROM[k] === 'library' ? SONGS[k] : null)), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: [] });
const pfOf = (keys) => ({ order: { order: ['library', 'tidal'], services: [{ id: 'library', name: 'Your library', connected: true }, { id: 'tidal', name: 'TIDAL', connected: true }] }, own_order: false, items: keys.map((k, i) => ({ index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', duration_ms: 240000, track_id: FROM[k] === 'library' ? SONGS[k].id : null, stream_uri: FROM[k] === 'tidal' ? `tidal:track:${SONGS[k].id}` : null, plays_from: { service: FROM[k], label: FROM[k] === 'library' ? 'Your library' : 'TIDAL' }, sources: [] })) });
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

async function run(browser, width, android) {
  const tag = `${width}${android ? '-android' : ''}`; const phone = width < 600;
  const { page, context, state } = await fx.create(browser, width, { mobile: phone });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  // (window.__fastHold: a 30 s timer (a collection change's hold) runs 3 s)
  await context.addInitScript(() => { const st = window.setTimeout; window.setTimeout = (f, ms, ...a) => st(f, window.__fastHold && ms === 30000 ? 3000 : ms, ...a); });
  const lists = { r1: ['A', 'B', 'C', 'D'], r3: ['A', 'B', 'C', 'D'], r5: [...Array(20).fill('C'), 'A', 'B', ...Array(24).fill('D')], r6: ['A', 'B', 'C', 'D'] };
  // pfSlow: the next play-from answers wait; pfStale: the next ones (pfStaleN, 1 by default) are for
  // this list, not the node's;
  // detailWait: the next reads of the list wait this long each (one entry per read)
  let pfSlow = 0; let pfStale = null; let pfStaleN = 1; const detailWait = []; const asks = [];
  let socket = null;
  await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
  await page.route('**/v1/pubky', (r) => json(r, { user: null, sync: { running: false } }));
  await page.route('**/v1/storage', (r) => json(r, { homeserver: { mode: 'none' }, saving: { state: 'saved' } }));
  await page.route(/\/v1\/playlists\/(r\d+)(\/.*)?$/, async (r) => {
    const u = new URL(r.request().url()); const m = r.request().method();
    const [, , , id, ...rest] = u.pathname.split('/'); const sub = rest.join('/');
    if (m === 'DELETE' && sub.startsWith('items/')) { asks.push(`${id} DELETE`); lists[id].splice(Number(sub.split('/')[1]), 1); return json(r, {}); }
    if (m !== 'GET') return r.fallback();
    if (sub === 'play-from') { asks.push(`${id} play-from`); const keys = pfStale || [...lists[id]]; if (--pfStaleN <= 0) { pfStale = null; pfStaleN = 1; } const wait = pfSlow; if (wait) await sleep(wait); return json(r, pfOf(keys)); }
    if (!sub) { asks.push(`${id} read`); const wait = detailWait.shift(); if (wait) await sleep(wait); return json(r, detailOf(id, lists[id])); }
    return r.fallback();
  });
  await page.route('**/v1/playlists', (r) => (r.request().method() === 'GET' ? json(r, Object.keys(NAMES).map((id) => sum(id, lists[id].length))) : r.fallback()));
  const names = (onlyShown) => page.evaluate((o) => [...document.querySelectorAll('#plsongs .srow')].filter((row) => !o || row.getClientRects().length).map((row) => { const n = row.querySelector('.name'); return (n ? n.textContent : row.textContent).trim().split(/\s/)[0]; }).filter(Boolean).join(','), onlyShown);
  const all = () => names(false);
  const pending = () => page.evaluate(() => !!document.querySelector('#plsongs .chip-sk'));
  const inert = () => page.evaluate(() => !!document.querySelector('#plsongs[inert]'));
  const focused = () => page.evaluate(() => {
    const a = document.activeElement; const row = a && a.closest && a.closest('#plsongs .srow');
    if (!row) return a ? `${a.tagName.toLowerCase()}${a.id ? '#' + a.id : ''}` : 'none';
    const n = row.querySelector('.name'); const kind = a === row ? 'row' : a.dataset.smore != null ? 'smore' : a.dataset.rm != null ? 'rm' : a.dataset.pup != null ? 'pup' : a.dataset.pdown != null ? 'pdown' : a.tagName.toLowerCase();
    return `${n.textContent.trim().split(/\s/)[0]}:${kind}`;
  });
  // (in sight: wholly inside the page's own scrolling part)
  const inSight = () => page.evaluate(() => { const a = document.activeElement; const m = document.querySelector('main'); if (!a || !m) return false; const r = a.getBoundingClientRect(); const b = m.getBoundingClientRect(); return r.height > 0 && r.top >= b.top - 1 && r.bottom <= Math.min(b.bottom, innerHeight) + 1; });
  const kind = phone ? 'smore' : 'rm';
  const open = async (id, want) => {
    await page.evaluate((x) => { const st = { v: 'playlist', arg: x }; history.pushState(st, '', `/playlist/${x}`); dispatchEvent(new PopStateEvent('popstate', { state: st })); }, id);
    return until(async () => (await page.textContent('main h1').catch(() => '')) === NAMES[id] && (await all()) === want && !(await pending()), 5000);
  };
  // (a song's control: phones show the ⋯ only; data-rm etc. are there, hidden)
  const focusOn = async (k, on) => { const row = page.locator(`#plsongs .srow[data-srow="${k}"]`); if (on === 'row') await row.focus(); else await row.locator(`[data-${on}]`).focus(); };
  // Another device changes the list: the node says the library changed, the page reads it again
  const remote = async (id, keys, want) => { lists[id] = keys; socket.send(JSON.stringify({ type: 'library_changed' })); return until(async () => (await all()) === want && !(await pending()), 4000); };
  const menuOpen = () => page.locator('.popover.menu:not([hidden]) .pop-item').first().isVisible().catch(() => false);
  // From the keyboard: phones Enter on the song's ⋯, the arrows down to Remove from collection, Enter; a desktop Enter on its ×
  const keyRemove = async (k) => {
    if (!phone) { await focusOn(k, 'rm'); await page.keyboard.press('Enter'); return true; }
    await focusOn(k, 'smore'); await page.keyboard.press('Enter');
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
    for (let i = 0; i < 50 && !socket; i++) await sleep(100);
    check(!!socket, `${tag}: events connected`);

    // 1. Another device removes Alpha, then moves Charlie to the top, with the focus on Charlie's ×
    // (a phone: its ⋯), its row, its Move down: the focus stays on Charlie, the same control
    for (const on of phone ? [kind, 'row'] : [kind, 'row', 'pdown']) {
      const t2 = `${tag} (${on})`;
      lists.r1 = ['A', 'B', 'C', 'D'];
      check(await open('r1', 'Alpha,Bravo,Charlie,Delta'), `${t2}: the collection opens (${await all()})`);
      await focusOn(2, on);
      check((await focused()) === `Charlie:${on}`, `${t2}: the focus is on Charlie's ${on} (${await focused()})`);
      check(await remote('r1', ['B', 'C', 'D'], 'Bravo,Charlie,Delta'), `${t2}: another device removes Alpha (${await all()})`);
      await sleep(300);
      check((await focused()) === `Charlie:${on}`, `${t2}: the focus stays on Charlie's ${on}, not on Delta's, now at Charlie's old place (${await focused()})`);
      if (on === kind) await page.screenshot({ path: path.join(SHOTS, `after-removed-above-${tag}.png`) });
      check(await remote('r1', ['C', 'B', 'D'], 'Charlie,Bravo,Delta'), `${t2}: another device moves Charlie to the top (${await all()})`);
      await sleep(300);
      check((await focused()) === `Charlie:${on}`, `${t2}: after the move the focus is still on Charlie's ${on} (${await focused()})`);
    }

    // 1b. Another device removes Charlie itself while the focus is on its ×: on to Delta, the next song, its ⋯
    // (never its ×: Enter there would remove Delta); then Delta, the last, removed: on to Bravo, the one before
    check(await open('r3', 'Alpha,Bravo,Charlie,Delta'), `${tag}: the third collection opens (${await all()})`);
    await focusOn(2, kind);
    check(await remote('r3', ['A', 'B', 'D'], 'Alpha,Bravo,Delta'), `${tag}: another device removes Charlie (${await all()})`);
    await until(async () => (await focused()) === 'Delta:smore', 2000);
    check((await focused()) === 'Delta:smore', `${tag}: the focus goes on to Delta, the next song, its ⋯ (${await focused()})`);
    await page.screenshot({ path: path.join(SHOTS, `after-removed-itself-${tag}.png`) });
    await focusOn(2, kind);
    check(await remote('r3', ['A', 'B'], 'Alpha,Bravo'), `${tag}: another device removes Delta, the last song (${await all()})`);
    await until(async () => (await focused()) === 'Bravo:smore', 2000);
    check((await focused()) === 'Bravo:smore', `${tag}: the focus goes to Bravo, the song before, its ⋯ (${await focused()})`);

    // 2. A long collection, TIDAL chip on; Bravo removed while the answer for where the songs play is
    // slow, so every row shows meanwhile. The focus on Alpha far down; then the person scrolls to the
    // foot, away from it. The answer hides Alpha and the 20 songs above: the focus goes on to the first
    // Delta, its ⋯, and the page stays at the foot where they scrolled it
    {
      const t2 = `${tag} (scrolled away)`; const count = () => page.evaluate(() => [...document.querySelectorAll('#plsongs .srow')].filter((r) => r.getClientRects().length).length);
      check(await open('r5', lists.r5.map((k) => SONGS[k].title).join(',')), `${t2}: the collection opens`);
      await page.click('#plsongs [data-pfilter="tidal"]');
      check(await until(async () => (await count()) === 25, 3000), `${t2}: the TIDAL chip shows 25 songs (${await count()})`);
      pfSlow = 3500;
      check(await keyRemove(21), `${t2}: Bravo's Remove from collection from the keyboard`);
      check(await until(async () => (await pending()) && (await count()) === 45, 4000), `${t2}: Bravo is removed, every row shows while the answer is on its way (${await count()})`);
      await until(async () => !(await inert()), 2000);
      await focusOn(20, kind);
      check((await focused()) === `Alpha:${kind}`, `${t2}: the focus is on Alpha, far down the list (${await focused()})`);
      await page.evaluate(() => { const m = document.querySelector('main'); m.scrollTop = m.scrollHeight; });
      check(!(await inSight()), `${t2}: they scroll to the foot, Alpha out of sight`);
      check(await until(async () => !(await pending()) && (await count()) === 24, 6000), `${t2}: the answer comes and the chip hides Alpha and the 20 above it (${await count()})`);
      await until(async () => (await focused()) === 'Delta:smore', 2000);
      check((await focused()) === 'Delta:smore', `${t2}: the focus goes on to the first Delta, its ⋯ (${await focused()})`);
      await sleep(300);
      const foot = await page.evaluate(() => { const m = document.querySelector('main'); return m.scrollHeight - m.clientHeight - m.scrollTop; });
      check(foot <= 2 && !(await inSight()), `${t2}: the page stays at the foot, not scrolled back up to that Delta (${foot} px from the foot)`);
      await page.screenshot({ path: path.join(SHOTS, `after-scrolled-away-${tag}.png`) });
      pfSlow = 0;
    }

    // 3. Charlie removed from the keyboard; the read after it is slow, and meanwhile the library
    // changes (the page is read again). Both answers for where the songs play are for the list before
    // the Remove: the rows hold while the list is read once more, then the focus is on Delta's ⋯
    {
      const t2 = `${tag} (asked again)`;
      check(await open('r6', 'Alpha,Bravo,Charlie,Delta'), `${t2}: the collection opens (${await all()})`);
      pfStale = [...lists.r6]; pfStaleN = 2; detailWait.length = 0; detailWait.push(1500, 0, 1500);
      const from = asks.length;
      check(await keyRemove(2), `${t2}: Charlie's Remove from collection from the keyboard`);
      check(await until(async () => asks.slice(from).includes('r6 read'), 3000), `${t2}: Charlie is removed and the list is read (${asks.slice(from)})`);
      await sleep(300); socket.send(JSON.stringify({ type: 'library_changed' }));
      check(await until(async () => pfStale === null && (await inert()) && (await all()) === 'Alpha,Bravo,Delta', 5000), `${t2}: the answer that doesn't fit holds the rows while the list is read once more (${await all()}; inert ${await inert()}; ${asks.slice(from)})`);
      await page.screenshot({ path: path.join(SHOTS, `after-asked-again-hold-${tag}.png`) });
      check(await until(async () => !(await inert()) && !(await pending()), 5000), `${t2}: the read after it ends the hold`);
      const pfs = asks.slice(from).filter((x) => x === 'r6 play-from').length;
      check(pfs >= 2, `${t2}: where the songs play is asked again (${pfs} times after the Remove)`);
      await until(async () => (await focused()) === 'Delta:smore', 2000);
      check((await focused()) === 'Delta:smore', `${t2}: the focus is on Delta, its ⋯, not its × (${await focused()})`);
      detailWait.length = 0; pfStale = null; pfStaleN = 1;
    }

    if (!phone) {
      const t2 = `${tag} (overtaken)`;
      lists.r6 = ['A', 'B', 'C', 'D'];
      check(await open('r6', 'Alpha,Bravo,Charlie,Delta'), `${t2}: the collection opens (${await all()})`);
      await page.evaluate(() => { window.__fastHold = true; });
      const from = asks.length;
      // (each read waits 1.5 s; a library change every second overtakes it, for 4 s, past the hold's end)
      detailWait.length = 0; detailWait.push(...Array(8).fill(1500));
      check(await keyRemove(2), `${t2}: Charlie's Remove from the keyboard`);
      check(await until(async () => asks.slice(from).includes('r6 DELETE'), 3000), `${t2}: Charlie is removed`);
      for (let n = 0; n < 4; n++) { await sleep(1000); socket.send(JSON.stringify({ type: 'library_changed' })); }
      detailWait.length = 0;
      check(await until(async () => (await all()) === 'Alpha,Bravo,Delta' && !(await inert()) && !(await pending()), 6000), `${t2}: a read is drawn at last (${await all()}; ${asks.slice(from).filter((x) => x === 'r6 read').length} reads)`);
      await until(async () => (await focused()) === 'Delta:smore', 2000);
      check((await focused()) === 'Delta:smore', `${t2}: the focus is on Delta, its ⋯, not its × (${await focused()})`);
      await page.evaluate(() => { window.__fastHold = false; });
    }

    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${tag}: no sideways overflow`);
    check(!state.errors.length, `${tag}: no page errors${state.errors.length ? ` (${state.errors.join('; ')})` : ''}`);
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
