// A held Enter in a collection removes one song, not every song after it.
// Remove from the keyboard hands the focus on to the next song (its × on a desktop, its ⋯ on a
// phone, where Remove is in the ⋯ menu). The key's repeats then reached that control: on a desktop
// a held Enter on Alpha's × went on removing Bravo, Charlie, Delta and Echo (the whole collection),
// on a phone it opened the next song's menu. Since #880 the chain also went on from the Songs box
// into "On this node only". Now a repeat of Enter or Space on a song's × or ⋯ does nothing; a new
// press still works (the next Enter removes the song the focus went on to).
// The same held Enter on the collection's own ⋯ pressed the menu's first item (Play next) again and
// again, on its Delete collection… pressed the dialog's Delete (it has the focus), and on the wide
// window's Delete armed and then confirmed it: the collection was gone. Menus and the app's dialogs
// now ignore a held key's repeats, and Delete arms once.
// At 360 (browser and the Android app), 412 and 1280 px.
//   node tools/ui-playlist-held-enter.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-held-enter-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

const song = (id, title) => ({ ...fx.track, id, title, canonical_uri: `https://musicbrainz.org/recording/0000000${id}-0000-4000-8000-000000000000`, mbid: null });
const SONGS = { A: song(1, 'Alpha'), B: song(2, 'Bravo'), C: song(3, 'Charlie'), D: song(4, 'Delta'), E: song(5, 'Echo') };
// Foxtrot and Golf are on this node only
const LOCAL = { F: { ...fx.track, id: 106, title: 'Foxtrot', canonical_uri: null, mbid: null }, G: { ...fx.track, id: 107, title: 'Golf', canonical_uri: null, mbid: null } };
const NAMES = { h1: 'Held on a song', h2: 'Held into this node only', h3: 'Held on this node only', h4: 'Held on its menu' };
const sum = (id, n) => ({ ...fx.summary, id, name: NAMES[id], items: n });
const detailOf = (id, keys, local) => ({ summary: sum(id, keys.length + local.length), playlist: { ...sum(id, keys.length + local.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => SONGS[k]), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: local.map((k) => LOCAL[k]) });
const pfOf = (keys) => ({ order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: keys.map((k, i) => ({ index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', track_id: SONGS[k].id, plays: { service: 'library', name: 'Your library' } })) });
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

async function run(browser, width, android) {
  const tag = `${width}${android ? '-android' : ''}`; const phone = width < 600;
  const { page, context, state } = await fx.create(browser, width, { mobile: phone });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
  const lists = { h1: ['A', 'B', 'C', 'D', 'E'], h2: ['A'], h3: [], h4: ['A', 'B'] };
  const locals = { h1: [], h2: ['F', 'G'], h3: ['F', 'G'], h4: [] };
  const writes = [];
  await page.route('**/v1/pubky', (r) => json(r, { user: null, sync: { running: false } }));
  await page.route('**/v1/storage', (r) => json(r, { homeserver: { mode: 'none' }, saving: { state: 'saved' } }));
  await page.route(/\/v1\/playlists\/(h\d+)(\/.*)?$/, async (r) => {
    const u = new URL(r.request().url()); const m = r.request().method();
    const [, , , id, ...rest] = u.pathname.split('/'); const sub = rest.join('/');
    if (m === 'DELETE' && sub.startsWith('items/')) { const k = Number(sub.split('/')[1]); writes.push(`${id} DELETE ${k}`); lists[id].splice(k, 1); return json(r, {}); }
    if (m === 'DELETE' && sub.startsWith('local/')) { const k = Number(sub.split('/')[1]); writes.push(`${id} DELETE local ${k}`); locals[id].splice(k, 1); return json(r, {}); }
    if (m === 'DELETE' && !sub) { writes.push(`${id} DELETE collection`); return json(r, {}); }
    if (m !== 'GET') { writes.push(`${id} ${m} ${sub}`); return r.fallback(); }
    if (sub === 'play-from') return json(r, pfOf(lists[id]));
    if (!sub) return json(r, detailOf(id, lists[id], locals[id]));
    return r.fallback();
  });
  await page.route('**/v1/playlists', (r) => (r.request().method() === 'GET' ? json(r, Object.keys(NAMES).map((id) => sum(id, lists[id].length + locals[id].length))) : r.fallback()));
  const all = () => page.evaluate(() => [...document.querySelectorAll('#plsongs .srow, #pllocal .srow')].map((row) => row.querySelector('.name').textContent.trim().split(/\s/)[0]).join(','));
  const focused = () => page.evaluate(() => {
    const a = document.activeElement; const row = a && a.closest && a.closest('#plsongs .srow, #pllocal .srow');
    if (!row) return a ? `${a.tagName.toLowerCase()}${a.id ? '#' + a.id : ''}` : 'none';
    const kind = a === row ? 'row' : a.dataset.smore != null ? 'smore' : a.dataset.rm != null ? 'rm' : a.dataset.lrm != null ? 'lrm' : 'other';
    return `${row.querySelector('.name').textContent.trim().split(/\s/)[0]}:${kind}`;
  });
  const menuOpen = () => page.evaluate(() => [...document.querySelectorAll('.popover.menu')].some((m) => !m.hidden && m.getClientRects().length > 0));
  const pending = () => page.evaluate(() => !!document.querySelector('#plsongs .chip-sk'));
  const open = async (id, want) => {
    await page.evaluate((x) => { const st = { v: 'playlist', arg: x }; history.pushState(st, '', `/playlist/${x}`); dispatchEvent(new PopStateEvent('popstate', { state: st })); }, id);
    return until(async () => (await page.textContent('main h1').catch(() => '')) === NAMES[id] && (await all()) === want && !(await pending()), 5000);
  };
  const ctl = (box, name, sel) => page.locator(`${box} .srow`, { hasText: name }).first().locator(sel);
  // Enter held: the first keydown, then repeats until `landed` (the focus on the control it goes on
  // to) and 10 more, so a slow redraw can't let the check pass without a repeat reaching it
  // (presses during it: on a menu's item, and on a song's ×, ⋯ or the collection's ⋯ / Delete)
  const hold = async (landed) => {
    await page.evaluate(() => { window.heldPresses = { item: 0, ctl: 0 }; if (window.heldCount) return; window.heldCount = true; document.addEventListener('click', (e) => { const t = e.target instanceof Element ? e.target : null; if (!t) return; if (t.closest('.popover.menu .pop-item')) window.heldPresses.item++; if (t.closest('[data-rm], [data-lrm], [data-smore], [data-cmore], #plmore, #pldel')) window.heldPresses.ctl++; }, true); });
    await page.keyboard.down('Enter');
    const end = Date.now() + 5000; let reached = false;
    while (Date.now() < end) { await sleep(60); await page.keyboard.down('Enter'); if (await landed()) { reached = true; break; } }
    for (let i = 0; i < 10; i++) { await sleep(60); await page.keyboard.down('Enter'); }
    await page.keyboard.up('Enter');
    await sleep(800); // (a press the repeats started would be on its way by now)
    return reached;
  };
  // The song's Remove, from the keyboard: its × where it shows (a desktop), else its ⋯ menu's
  // Remove from collection; then Enter held until the focus is on `next`
  const holdRemove = async (box, name, x, next) => {
    const rm = ctl(box, name, x);
    if (await rm.isVisible().catch(() => false)) await rm.focus();
    else {
      await ctl(box, name, '[data-smore]').focus(); await page.keyboard.press('Enter');
      if (!(await until(menuOpen, 3000))) return false;
      let found = false;
      for (let i = 0; i < 30 && !found; i++) {
        const at = await page.evaluate(() => (document.activeElement && document.activeElement.closest('.popover.menu') ? document.activeElement.textContent.trim() : ''));
        if (at === 'Remove from collection') found = true; else await page.keyboard.press('ArrowDown');
      }
      if (!found) { await page.keyboard.press('Escape'); return false; }
    }
    return hold(async () => (await focused()) === next);
  };
  // (writes that aren't a collection's: Play next, the queue)
  const presses = () => page.evaluate(() => window.heldPresses);
  const others = () => state.hits.filter((h) => h.m !== 'GET' && !h.v.startsWith('/playlists')).map((h) => `${h.m} ${h.v}`);
  const deletes = (id) => writes.filter((w) => w.startsWith(`${id} DELETE`));
  const kind = phone ? 'smore' : 'rm';
  try {
    await fx.go(page, '/playlists');

    // 1. Enter held on Alpha's Remove: Alpha goes, Bravo, Charlie, Delta and Echo stay, the focus
    // on Bravo (its × or ⋯), no menu open. A new Enter there removes Bravo (a press, not a repeat)
    check(await open('h1', 'Alpha,Bravo,Charlie,Delta,Echo'), `${tag}: the collection opens (${await all()})`);
    check(await holdRemove('#plsongs', 'Alpha', '[data-rm]', `Bravo:${kind}`), `${tag}: Alpha's Remove from the keyboard, Enter held till the focus is on Bravo`);
    await until(async () => (await all()) === 'Bravo,Charlie,Delta,Echo', 3000);
    check((await all()) === 'Bravo,Charlie,Delta,Echo', `${tag}: only Alpha is removed (${await all()})`);
    check(deletes('h1').length === 1, `${tag}: one removal sent (${deletes('h1').join('; ') || 'none'})`);
    { const n = await presses(); check(n.ctl === 1 && n.item === (phone ? 1 : 0), `${tag}: the held Enter pressed once (menu items ${n.item}, × or ⋯ ${n.ctl}; a phone's Remove from collection presses the row's ×)`); }
    await until(async () => (await focused()) === `Bravo:${kind}`, 2000);
    check((await focused()) === `Bravo:${kind}`, `${tag}: the focus is on Bravo, its ${kind} (${await focused()})`);
    check(!(await menuOpen()), `${tag}: the held key opened no menu`);
    await page.screenshot({ path: path.join(SHOTS, `${tag}-held.png`) });
    if (!phone) {
      await page.keyboard.press('Enter');
      check(await until(async () => (await all()) === 'Charlie,Delta,Echo', 3000), `${tag}: a new Enter on Bravo's × removes Bravo (${await all()})`);
    } else {
      await page.keyboard.press('Enter');
      check(await until(menuOpen, 3000), `${tag}: a new Enter on Bravo's ⋯ opens its menu`);
      await page.keyboard.press('Escape');
    }

    // 2. Enter held on the Songs card's last song, songs on this node only below: Alpha goes, the
    // focus goes on to Foxtrot's ⋯ and the repeats don't open its menu or remove it
    check(await open('h2', 'Alpha,Foxtrot,Golf'), `${tag}: the collection with songs on this node only opens (${await all()})`);
    check(await holdRemove('#plsongs', 'Alpha', '[data-rm]', 'Foxtrot:smore'), `${tag}: Alpha's Remove (the last in Songs), Enter held till the focus is on Foxtrot`);
    await until(async () => (await all()) === 'Foxtrot,Golf', 3000);
    check((await all()) === 'Foxtrot,Golf', `${tag}: only Alpha is removed (${await all()})`);
    check(deletes('h2').length === 1, `${tag}: one removal sent (${deletes('h2').join('; ') || 'none'})`);
    { const n = await presses(); check(n.ctl === 1 && n.item === (phone ? 1 : 0), `${tag}: the held Enter pressed once (menu items ${n.item}, × or ⋯ ${n.ctl}; a phone's Remove from collection presses the row's ×)`); }
    await until(async () => (await focused()) === 'Foxtrot:smore', 2000);
    check((await focused()) === 'Foxtrot:smore', `${tag}: the focus is on Foxtrot, its ⋯ (${await focused()})`);
    check(!(await menuOpen()), `${tag}: the held key opened no menu on this node only`);

    // 3. Enter held on Foxtrot's Remove on this node only: Golf stays
    check(await open('h3', 'Foxtrot,Golf'), `${tag}: the collection of songs on this node only opens (${await all()})`);
    check(await holdRemove('#pllocal', 'Foxtrot', '[data-lrm]', 'Golf:lrm'), `${tag}: Foxtrot's Remove, Enter held till the focus is on Golf`);
    await until(async () => (await all()) === 'Golf', 3000);
    check((await all()) === 'Golf', `${tag}: only Foxtrot is removed (${await all()})`);
    check(deletes('h3').length === 1, `${tag}: one removal sent (${deletes('h3').join('; ') || 'none'})`);
    check(!(await menuOpen()), `${tag}: the held key opened no menu`);

    // 4. Enter held on the collection's own ⋯: its menu opens (the focus on its first item, Play
    // next) and stays open, nothing is queued
    check(await open('h4', 'Alpha,Bravo'), `${tag}: the collection opens (${await all()})`);
    const hits0 = others().length;
    await page.locator('#plmore').focus();
    const atFirst = async () => page.evaluate(() => !!document.activeElement && !!document.activeElement.closest('.popover.menu'));
    check(await hold(atFirst), `${tag}: Enter held on the collection's ⋯, its menu takes the focus`);
    check(await menuOpen(), `${tag}: its menu is open`);
    { const n = await presses(); check(n.item === 0 && n.ctl === 1, `${tag}: no item of it was pressed, the ⋯ once (menu items ${n.item}, ⋯ ${n.ctl})`); }
    check(others().length === hits0, `${tag}: nothing queued (${others().slice(hits0).join('; ') || 'nothing sent'})`);
    await page.keyboard.press('Escape'); await until(async () => !(await menuOpen()), 2000);
    // 5. Delete: the wide window's button (arms with Really delete?), else the ⋯ menu's Delete
    // collection…, whose dialog has the focus on Delete. Held, the collection stays
    const del = page.locator('#pldel');
    if (await del.isVisible().catch(() => false)) {
      await del.focus();
      check(await hold(async () => /Really delete/.test(await del.textContent())), `${tag}: Enter held on Delete arms it`);
    } else {
      await page.locator('#plmore').focus(); await page.keyboard.press('Enter');
      check(await until(menuOpen, 3000), `${tag}: the collection's ⋯ opens`);
      let found = false;
      for (let i = 0; i < 30 && !found; i++) {
        const at = await page.evaluate(() => (document.activeElement && document.activeElement.closest('.popover.menu') ? document.activeElement.textContent.trim() : ''));
        if (at === 'Delete collection…') found = true; else await page.keyboard.press('ArrowDown');
      }
      check(found, `${tag}: its Delete collection…`);
      const inDialog = () => page.evaluate(() => !!document.activeElement && !!document.activeElement.closest('dialog[open]'));
      check(await hold(inDialog), `${tag}: Enter held on Delete collection…, the dialog takes the focus`);
      check(await page.evaluate(() => !!document.querySelector('dialog[open]')), `${tag}: the dialog asking is still open`);
      await page.keyboard.press('Escape');
    }
    check(!deletes('h4').length, `${tag}: the collection isn't deleted (${deletes('h4').join('; ') || 'nothing sent'})`);
    await page.screenshot({ path: path.join(SHOTS, `${tag}-delete-held.png`) });

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
