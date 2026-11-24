// Where the focus goes in a collection when the row it was on goes away or is hidden.
// 1. A source chip kept while the rows show unfiltered (the answer for where the songs play is
//    still on its way after a Remove): the focus on a song of another source fell to the page once
//    the answer came and the chip hid that row; the next Tab started at the top. Now it goes on to
//    the next song that shows (the shown one before when none shows after it), as Remove does: its
//    ⋯ or ×, or the row itself when the row had the focus.
// 2. Remove (the × on a desktop, the ⋯ menu on phones) on the last song of the Songs card, with
//    songs "On this node only" below: the Songs card goes and the focus fell to the page. Now it
//    goes to the first song on this node only, its × (its ⋯ from the menu).
// 3. The × on the last song on this node only: that section goes; the focus now goes to the Songs
//    card's last song that shows, its × (its ⋯ on a phone, where the × is in the menu).
// At 360 (browser and the Android app), 412 and 1280 px.
//   node tools/ui-playlist-focus-onward.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-focus-onward-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

const song = (id, title) => ({ ...fx.track, id, title, canonical_uri: `https://musicbrainz.org/recording/0000000${id}-0000-4000-8000-000000000000`, mbid: null });
// Alpha and Charlie play from the library, Bravo and Delta from TIDAL; Echo and Foxtrot are on this node only
const SONGS = { A: song(1, 'Alpha'), B: song(2, 'Bravo'), C: song(3, 'Charlie'), D: song(4, 'Delta') };
const LOCAL = { E: { ...fx.track, id: 105, title: 'Echo', canonical_uri: null, mbid: null }, F: { ...fx.track, id: 106, title: 'Foxtrot', canonical_uri: null, mbid: null } };
const FROM = { A: 'library', B: 'tidal', C: 'library', D: 'tidal' };
const NAMES = { k1: 'Chip back on', k2: 'Last synced', k3: 'Last local', k4: 'Chip back on, row' };
const sum = (id, n) => ({ ...fx.summary, id, name: NAMES[id], items: n });
const detailOf = (id, keys, local) => ({ summary: sum(id, keys.length + local.length), playlist: { ...sum(id, keys.length + local.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => (FROM[k] === 'library' ? SONGS[k] : null)), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: local.map((k) => LOCAL[k]) });
const pfOf = (keys) => ({ order: { order: ['library', 'tidal'], services: [{ id: 'library', name: 'Your library', connected: true }, { id: 'tidal', name: 'TIDAL', connected: true }] }, own_order: false, items: keys.map((k, i) => ({ index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', duration_ms: 240000, track_id: FROM[k] === 'library' ? SONGS[k].id : null, stream_uri: FROM[k] === 'tidal' ? `tidal:track:${SONGS[k].id}` : null, plays_from: { service: FROM[k], label: FROM[k] === 'library' ? 'Your library' : 'TIDAL' }, sources: [] })) });
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

async function run(browser, width, android) {
  const tag = `${width}${android ? '-android' : ''}`; const phone = width < 600;
  const { page, context, state } = await fx.create(browser, width, { mobile: phone });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  const lists = { k1: ['A', 'B', 'C', 'D'], k2: ['A'], k3: ['A', 'B', 'C'], k4: ['A', 'B', 'C', 'D'] };
  const locals = { k1: [], k2: ['E', 'F'], k3: ['E'], k4: [] };
  const writes = []; let pfSlow = 0;
  await page.route('**/v1/pubky', (r) => json(r, { user: null, sync: { running: false } }));
  await page.route('**/v1/storage', (r) => json(r, { homeserver: { mode: 'none' }, saving: { state: 'saved' } }));
  await page.route(/\/v1\/playlists\/(k\d+)(\/.*)?$/, async (r) => {
    const u = new URL(r.request().url()); const m = r.request().method();
    const [, , , id, ...rest] = u.pathname.split('/'); const sub = rest.join('/');
    if (m === 'DELETE' && sub.startsWith('items/')) { const k = Number(sub.split('/')[1]); writes.push(`${id} DELETE ${k}`); lists[id].splice(k, 1); return json(r, {}); }
    if (m === 'DELETE' && sub.startsWith('local/')) { const k = Number(sub.split('/')[1]); writes.push(`${id} DELETE local ${k}`); locals[id].splice(k, 1); return json(r, {}); }
    if (m !== 'GET') return r.fallback();
    if (sub === 'play-from') { const keys = [...lists[id]]; if (pfSlow) await sleep(pfSlow); return json(r, pfOf(keys)); }
    if (!sub) return json(r, detailOf(id, lists[id], locals[id]));
    return r.fallback();
  });
  await page.route('**/v1/playlists', (r) => (r.request().method() === 'GET' ? json(r, Object.keys(NAMES).map((id) => sum(id, lists[id].length + locals[id].length))) : r.fallback()));
  // (each row's song name, the first word of it; textContent, so a row not drawn yet still reads)
  const names = (sel, onlyShown) => page.evaluate(([s, o]) => [...document.querySelectorAll(s)].filter((row) => !o || row.getClientRects().length).map((row) => { const n = row.querySelector('.name'); return (n ? n.textContent : row.textContent).trim().split(/\s/)[0]; }).filter(Boolean).join(','), [sel, onlyShown]);
  const shown = () => names('#plsongs .srow', true);
  const all = () => names('#plsongs .srow, #pllocal .srow', false);
  const chipOn = () => page.evaluate(() => { const c = document.querySelector('#plsongs [data-pfilter][aria-pressed="true"]'); return c ? c.textContent.trim() : 'none'; });
  const pending = () => page.evaluate(() => !!document.querySelector('#plsongs .chip-sk'));
  const focused = () => page.evaluate(() => {
    const a = document.activeElement; const row = a && a.closest && a.closest('#plsongs .srow, #pllocal .srow');
    if (!row) return a ? `${a.tagName.toLowerCase()}${a.id ? '#' + a.id : ''}` : 'none';
    const n = row.querySelector('.name'); const kind = a === row ? 'row' : a.dataset.smore != null ? 'smore' : a.dataset.rm != null ? 'rm' : a.dataset.lrm != null ? 'lrm' : a.tagName.toLowerCase();
    return `${n.textContent.trim().split(/\s/)[0]}:${kind}`;
  });
  const ctl = (box, name, sel) => page.locator(`${box} .srow`, { hasText: name }).first().locator(sel);
  const kind = phone ? 'smore' : 'rm';
  const open = async (id, want) => {
    await page.evaluate((x) => { const st = { v: 'playlist', arg: x }; history.pushState(st, '', `/playlist/${x}`); dispatchEvent(new PopStateEvent('popstate', { state: st })); }, id);
    return until(async () => (await page.textContent('main h1').catch(() => '')) === NAMES[id] && (await all()) === want && !(await pending()), 5000);
  };
  const menuOpen = () => page.locator('.popover.menu:not([hidden]) .pop-item').first().isVisible().catch(() => false);
  // From the keyboard: phones Enter on the song's ⋯, the arrows down to Remove from collection, Enter;
  // a desktop Enter on its ×
  const keyRemove = async (name) => {
    if (!phone) { await ctl('#plsongs', name, '[data-rm]').focus(); await page.keyboard.press('Enter'); return true; }
    await ctl('#plsongs', name, '[data-smore]').focus();
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

    // 1. TIDAL chip on (Bravo, Delta show). Bravo removed while the answer for where the songs play
    // is slow: the rows show unfiltered meanwhile, and the focus is on Alpha (a library song: its
    // ⋯ or ×, or the row itself) when the answer comes and the chip hides it: the focus goes on to
    // Delta, the next song that shows, the same kind of control
    for (const [id, onRow] of [['k1', false], ['k4', true]]) {
      const want = onRow ? 'row' : kind; const t2 = `${tag}${onRow ? ' (row)' : ''}`;
      check(await open(id, 'Alpha,Bravo,Charlie,Delta'), `${t2}: the collection opens (${await all()})`);
      await page.click('#plsongs [data-pfilter="tidal"]');
      check(await until(async () => (await shown()) === 'Bravo,Delta', 3000), `${t2}: the TIDAL chip shows ${await shown()}`);
      pfSlow = 3500;
      check(await keyRemove('Bravo'), `${t2}: Bravo's Remove from collection from the keyboard`);
      check(await until(async () => (await pending()) && (await all()) === 'Alpha,Charlie,Loading…', 4000), `${t2}: Bravo is removed and the answer is on its way (${await all()})`);
      check((await shown()) === 'Alpha,Charlie,Loading…', `${t2}: meanwhile every row shows (${await shown()})`);
      await until(async () => !(await page.evaluate(() => !!document.querySelector('#plsongs[inert]'))), 2000);
      if (onRow) await page.locator('#plsongs .srow', { hasText: 'Alpha' }).first().focus(); else await ctl('#plsongs', 'Alpha', `[data-${kind}]`).focus();
      check((await focused()) === `Alpha:${want}`, `${t2}: the focus is on Alpha, a library song (${await focused()})`);
      if (!onRow) await page.screenshot({ path: path.join(SHOTS, `after-chip-pending-${tag}.png`) });
      check(await until(async () => !(await pending()) && (await shown()) === 'Delta', 6000), `${t2}: the answer comes and the TIDAL chip hides Alpha again (${await shown()}; chip ${await chipOn()})`);
      await until(async () => (await focused()) === `Delta:${want}`, 2000);
      check((await focused()) === `Delta:${want}`, `${t2}: the focus goes on to Delta, the next song that shows (${await focused()})`);
      if (!onRow) await page.screenshot({ path: path.join(SHOTS, `after-chip-back-on-${tag}.png`) });
      pfSlow = 0;
    }

    // 2. The Songs card's only song removed, two songs on this node only below: the focus goes to the first of those
    check(await open('k2', 'Alpha,Echo,Foxtrot'), `${tag}: the second collection opens (${await all()})`);
    check(await keyRemove('Alpha'), `${tag}: Alpha's Remove from collection from the keyboard`);
    check(await until(async () => (await all()) === 'Echo,Foxtrot', 4000), `${tag}: Alpha is removed (${writes.filter((w) => w.startsWith('k2'))}; ${await all()})`);
    check(await page.evaluate(() => { const s = document.querySelector('#plsongs'); return !s || !s.getClientRects().length; }), `${tag}: the Songs card goes`);
    const lkind = phone ? 'smore' : 'lrm';
    await until(async () => (await focused()) === `Echo:${lkind}`, 2000);
    check((await focused()) === `Echo:${lkind}`, `${tag}: the focus goes to Echo, the first song on this node only (${await focused()})`);
    await page.screenshot({ path: path.join(SHOTS, `after-last-synced-${tag}.png`) });

    // 3. The only song on this node only removed with its ×: the focus goes to the Songs card's last song that shows
    check(await open('k3', 'Alpha,Bravo,Charlie,Echo'), `${tag}: the third collection opens (${await all()})`);
    await ctl('#pllocal', 'Echo', '[data-lrm]').focus(); await page.keyboard.press('Enter');
    check(await until(async () => (await all()) === 'Alpha,Bravo,Charlie', 4000), `${tag}: Echo is removed (${writes.filter((w) => w.startsWith('k3'))}; ${await all()})`);
    check(await page.evaluate(() => !document.querySelector('#pllocal')), `${tag}: "On this node only" goes`);
    await until(async () => (await focused()) === `Charlie:${kind}`, 2000);
    check((await focused()) === `Charlie:${kind}`, `${tag}: the focus goes to Charlie, the Songs card's last song (${await focused()})`);
    await page.screenshot({ path: path.join(SHOTS, `after-last-local-${tag}.png`) });

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
