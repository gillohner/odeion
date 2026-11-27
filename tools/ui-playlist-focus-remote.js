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
// 4. Another device moves the focused song far down the list: the focus stays on that song's control
//    and, as its row was in sight, the page follows it there (it was left off screen). Moved back up
//    while they had scrolled away from it, the page stays where they put it.
// 5. Another device shortens the list below (the page keeps its height a moment, a spacer at the
//    foot), then removes the focused song, whose row they had scrolled just past: the focus goes on to
//    the next song's ⋯ and the page stays put. (Whether the row was in sight was judged after the
//    spacer went, on a page that had scrolled, so the page jumped to that song.)
// 6. Another device removes the last song on this node only: the section goes, and the focus goes to
//    the Songs card's last song, its ⋯ (it fell to the page). The Songs card's last song removed while
//    songs on this node only are left (the card hides): to the first of those, its ⋯.
// 6b. Another device replaces the whole list at once (none of the songs around it left): the focus goes
//    to the song now at its place, its ⋯, not to the page.
// (Part 1's `row` cases pass without the fix in #905 too: a row already found its song by its
// data-np there; the ×, ⋯ and Move cases are the ones that guard it.)
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
const NAMES = { r1: 'Changed above', r3: 'Removed itself', r5: 'Scrolled away', r6: 'Asked again', r7: 'Moved far', r8: 'Spacer below', r9: 'Node only' };
// Songs on this node only (not identified yet): found by their own id
const LOCAL = { L: { ...fx.track, id: 91, title: 'Lima', canonical_uri: null, mbid: null }, M: { ...fx.track, id: 92, title: 'Mike', canonical_uri: null, mbid: null } };
const sum = (id, n) => ({ ...fx.summary, id, name: NAMES[id], items: n });
const detailOf = (id, keys, local = []) => ({ summary: sum(id, keys.length + local.length), playlist: { ...sum(id, keys.length + local.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => (FROM[k] === 'library' ? SONGS[k] : null)), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: local.map((k) => LOCAL[k]) });
const pfOf = (keys) => ({ order: { order: ['library', 'tidal'], services: [{ id: 'library', name: 'Your library', connected: true }, { id: 'tidal', name: 'TIDAL', connected: true }] }, own_order: false, items: keys.map((k, i) => ({ index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', duration_ms: 240000, track_id: FROM[k] === 'library' ? SONGS[k].id : null, stream_uri: FROM[k] === 'tidal' ? `tidal:track:${SONGS[k].id}` : null, plays_from: { service: FROM[k], label: FROM[k] === 'library' ? 'Your library' : 'TIDAL' }, sources: [] })) });
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

async function run(browser, width, android) {
  const tag = `${width}${android ? '-android' : ''}`; const phone = width < 600;
  const { page, context, state } = await fx.create(browser, width, { mobile: phone });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  // (window.__fastHold: a 30 s timer (a collection change's hold) runs 3 s)
  await context.addInitScript(() => { const st = window.setTimeout; window.setTimeout = (f, ms, ...a) => st(f, window.__fastHold && ms === 30000 ? 3000 : ms, ...a); });
  const lists = { r1: ['A', 'B', 'C', 'D'], r3: ['A', 'B', 'C', 'D'], r5: [...Array(20).fill('C'), 'A', 'B', ...Array(24).fill('D')], r6: ['A', 'B', 'C', 'D'], r7: ['A', ...Array(24).fill('C'), 'E'], r8: [...Array(10).fill('D'), 'A', 'B', ...Array(20).fill('C')], r9: ['A', 'B'] };
  const locals = { r9: ['L'] };
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
    if (!sub) { asks.push(`${id} read`); const wait = detailWait.shift(); if (wait) await sleep(wait); return json(r, detailOf(id, lists[id], locals[id])); }
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
  // (either box: "box:Name:kind")
  const where = () => page.evaluate(() => {
    const a = document.activeElement; const row = a && a.closest && a.closest('#plsongs .srow, #pllocal .srow');
    if (!row) return a ? `${a.tagName.toLowerCase()}${a.id ? '#' + a.id : ''}` : 'none';
    const n = row.querySelector('.name'); const k = a === row ? 'row' : a.dataset.smore != null ? 'smore' : a.dataset.lrm != null ? 'lrm' : a.dataset.rm != null ? 'rm' : a.tagName.toLowerCase();
    return `${row.closest('section').id}:${n.textContent.trim().split(/\s/)[0]}:${k}`;
  });
  const scrollTop = () => page.evaluate(() => document.querySelector('main').scrollTop);
  // (a control focused where it is, the page not scrolled to it)
  const focusStill = (sel) => page.evaluate((x) => document.querySelector(x).focus({ preventScroll: true }), sel);
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

    // 4. Alpha, at the top and in sight, moved to the foot by another device: the focus stays on its
    // control, and the page follows it; moved back up while they had scrolled away: the page stays
    {
      const t2 = `${tag} (moved far)`; lists.r7 = ['A', ...Array(24).fill('C'), 'E'];
      check(await open('r7', lists.r7.map((k) => SONGS[k].title).join(',')), `${t2}: the collection opens`);
      await focusOn(0, kind);
      check((await focused()) === `Alpha:${kind}` && (await inSight()), `${t2}: the focus is on Alpha's ${kind}, in sight (${await focused()})`);
      check(await remote('r7', [...Array(24).fill('C'), 'E', 'A'], [...Array(24).fill('Charlie'), 'Echo', 'Alpha'].join(',')), `${t2}: another device moves Alpha to the foot`);
      await sleep(300);
      check((await focused()) === `Alpha:${kind}`, `${t2}: the focus stays on Alpha's ${kind} (${await focused()})`);
      check(await inSight(), `${t2}: the page follows it to the foot, Alpha's ${kind} in sight (scrollTop ${await scrollTop()})`);
      await page.screenshot({ path: path.join(SHOTS, `after-moved-far-${tag}.png`) });
      await page.evaluate(() => document.querySelector('#plsongs .srow[data-srow="12"]').scrollIntoView({ block: 'start', behavior: 'instant' }));
      check(!(await inSight()), `${t2}: they scroll up to the middle, Alpha out of sight`);
      const before = await scrollTop();
      check(await remote('r7', ['C', 'A', ...Array(23).fill('C'), 'E'], ['Charlie', 'Alpha', ...Array(23).fill('Charlie'), 'Echo'].join(',')), `${t2}: another device moves Alpha back near the top`);
      await sleep(300);
      const after = await scrollTop();
      check((await focused()) === `Alpha:${kind}` && Math.abs(after - before) <= 2, `${t2}: the focus stays on Alpha and the page where they scrolled it (${await focused()}; scrollTop ${before} → ${after})`);
    }

    // 5. A spacer below (another device removed the 20 songs at the foot), Alpha's row scrolled just
    // past at the top; then another device removes Alpha: on to Bravo's ⋯, the page stays put
    {
      const t2 = `${tag} (spacer below)`; lists.r8 = [...Array(10).fill('D'), 'A', 'B', ...Array(20).fill('C')];
      check(await open('r8', lists.r8.map((k) => SONGS[k].title).join(',')), `${t2}: the collection opens`);
      // (Bravo's row, the one after Alpha, also wholly above the page's top edge and its sticky head)
      await page.evaluate(() => { const m = document.querySelector('main'); const row = document.querySelector('#plsongs .srow[data-srow="11"]'); const r = row.getBoundingClientRect(); m.scrollTop = Math.round(r.bottom - m.getBoundingClientRect().top + m.scrollTop + 70); });
      await focusStill(`#plsongs .srow[data-srow="10"] [data-${kind}]`);
      check((await focused()) === `Alpha:${kind}` && !(await inSight()), `${t2}: the focus is on Alpha's ${kind}, scrolled just past (${await focused()})`);
      const top0 = await scrollTop();
      check(await remote('r8', [...Array(10).fill('D'), 'A', 'B'], [...Array(10).fill('Delta'), 'Alpha', 'Bravo'].join(',')), `${t2}: another device removes the 20 songs at the foot`);
      await sleep(300);
      const spacer = await page.evaluate(() => document.querySelector('main').hasAttribute('data-hold'));
      check(spacer && Math.abs((await scrollTop()) - top0) <= 2 && (await focused()) === `Alpha:${kind}`, `${t2}: the page keeps its height and place, the focus on Alpha (spacer ${spacer}; scrollTop ${top0} → ${await scrollTop()}; ${await focused()})`);
      check(await remote('r8', [...Array(10).fill('D'), 'B'], [...Array(10).fill('Delta'), 'Bravo'].join(',')), `${t2}: another device removes Alpha`);
      await until(async () => (await focused()) === 'Bravo:smore', 2000);
      await sleep(200);
      const top1 = await scrollTop();
      check((await focused()) === 'Bravo:smore', `${t2}: the focus goes on to Bravo, its ⋯ (${await focused()})`);
      check(Math.abs(top1 - top0) <= 2 && !(await inSight()), `${t2}: the page stays where they scrolled it, not to Bravo (scrollTop ${top0} → ${top1})`);
      await page.screenshot({ path: path.join(SHOTS, `after-spacer-below-${tag}.png`) });
    }

    // 6. On this node only: its last song removed by another device, then the Songs card's last one
    {
      const t2 = `${tag} (node only)`; const lk = phone ? 'smore' : 'lrm';
      lists.r9 = ['A', 'B']; locals.r9 = ['L'];
      check(await open('r9', 'Alpha,Bravo') && await until(() => page.locator('#pllocal .srow').first().isVisible().catch(() => false), 3000), `${t2}: the collection opens, Lima on this node only`);
      await page.locator(`#pllocal .srow[data-srow="0"] [data-${lk}]`).focus();
      check((await where()) === `pllocal:Lima:${lk}`, `${t2}: the focus is on Lima's ${lk} (${await where()})`);
      locals.r9 = []; socket.send(JSON.stringify({ type: 'library_changed' }));
      check(await until(() => page.evaluate(() => !document.querySelector('#pllocal')), 4000), `${t2}: another device removes Lima, the section goes`);
      await until(async () => (await where()) === 'plsongs:Bravo:smore', 2000);
      check((await where()) === 'plsongs:Bravo:smore', `${t2}: the focus goes to Bravo, the Songs card's last song, its ⋯ (${await where()})`);
      await page.screenshot({ path: path.join(SHOTS, `after-node-only-gone-${tag}.png`) });
      lists.r9 = ['A']; locals.r9 = ['L', 'M'];
      check(await open('r9', 'Alpha') && await until(async () => (await page.locator('#pllocal .srow').count()) === 2, 3000), `${t2}: Alpha in Songs, Lima and Mike on this node only`);
      await focusOn(0, kind);
      check((await where()) === `plsongs:Alpha:${kind}`, `${t2}: the focus is on Alpha's ${kind} (${await where()})`);
      lists.r9 = []; socket.send(JSON.stringify({ type: 'library_changed' }));
      check(await until(() => page.evaluate(() => { const b = document.querySelector('#plsongs'); return !!b && b.hidden; }), 4000), `${t2}: another device removes Alpha, the Songs card hides`);
      await until(async () => (await where()) === 'pllocal:Lima:smore', 2000);
      check((await where()) === 'pllocal:Lima:smore', `${t2}: the focus goes to Lima, the first song on this node only, its ⋯ (${await where()})`);
    }

    // 6b. The whole list replaced at once, the focus on Bravo (the second song): on to Delta, now second, its ⋯
    {
      const t2 = `${tag} (replaced)`; lists.r9 = ['A', 'B', 'C']; locals.r9 = [];
      check(await open('r9', 'Alpha,Bravo,Charlie'), `${t2}: the collection opens (${await all()})`);
      await focusOn(1, kind);
      check((await focused()) === `Bravo:${kind}`, `${t2}: the focus is on Bravo's ${kind} (${await focused()})`);
      check(await remote('r9', ['E', 'D'], 'Echo,Delta'), `${t2}: another device replaces the whole list (${await all()})`);
      await until(async () => (await focused()) === 'Delta:smore', 2000);
      check((await focused()) === 'Delta:smore', `${t2}: the focus goes to Delta, the song now at Bravo's place, its ⋯ (${await focused()})`);
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
