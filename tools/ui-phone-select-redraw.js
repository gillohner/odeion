// Select survives a list drawn again where it stands. A playlist's songs show at once and their
// sources come a moment later; when they come the list is drawn again. Select started while they
// load (or a song pinned to a source further down) must not end Select, drop the ticks, flash the
// skeleton or jump to the top: the page stays where it was, the ticks stay and the bar acts on the
// list on screen. Opening another page ends Select; starting Select again starts with nothing
// ticked. In Search, "See all songs" keeps Select and the tick, with a box on every new row. The
// playlist changing under Select (a song above a tick taken out, a song in it twice, a filter, a
// Remove that takes a while): the bar's Remove deletes exactly the songs ticked on screen.
// Phones (360, 412, Android stub) and desktop (1280).
// Real candidate assets, isolated responses: node tools/ui-phone-select-redraw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-select-redraw-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// 24 library songs in one playlist, so the page scrolls
const N = 24;
const uri = (n) => `https://musicbrainz.org/recording/00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const tracks = Array.from({ length: N }, (_, k) => ({ ...fx.track, id: k + 1, mbid: uri(k + 1).split('/').pop(), canonical_uri: uri(k + 1), title: `Song ${k + 1}` }));
const summary = { ...fx.summary, items: N };
const detail = { summary, playlist: { ...summary, items: tracks.map((t) => ({ uri: t.canonical_uri })) }, tracks, entries: tracks.map((t) => ({ kind: 'track', uri: t.canonical_uri })), local_tracks: [] };
const libSource = { service: 'library', name: 'Your library', label: 'Your library', available: true, chosen: true };
const playFrom = {
  order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false,
  items: tracks.map((t, k) => ({ index: k, uri: t.canonical_uri, title: t.title, artist: 'Sefa', track_id: t.id, plays_from: { service: 'library', label: 'Your library' }, sources: [libSource] })),
};
const fixtures = { '/playlists/c1': { body: detail } };
const searchHits = Array.from({ length: 10 }, (_, k) => ({ kind: 'song', id: `lib-${k + 1}`, title: `Journey ${k + 1}`, subtitle: 'Sefa', keys: [`track:${k + 1}`], track_id: k + 1 }));

// What the list and the bar say
const look = (page, sec) => page.evaluate((s) => {
  const box = document.querySelector(s); const bar = document.querySelector('.selbar');
  return {
    selecting: !!box && box.classList.contains('selecting'),
    pressed: box && box.querySelector('[data-selstart]') && box.querySelector('[data-selstart]').getAttribute('aria-pressed'),
    boxes: box ? box.querySelectorAll('.selbox input').length : 0,
    ticked: box ? [...box.querySelectorAll('.selbox input')].filter((c) => c.checked).map((c) => Number(c.dataset.k)) : [],
    bar: !!bar, count: bar ? bar.querySelector('.selcount').textContent : null, bars: document.querySelectorAll('.selbar').length,
    top: document.querySelector('#main').scrollTop,
  };
}, sec);
const tick = (page, k) => page.evaluate((kk) => document.querySelector(`#plsongs .selbox input[data-k="${kk}"]`).click(), k);
const startSelect = (page, sec) => page.evaluate((s) => document.querySelector(`${s} [data-selstart]`).click(), sec);
// Watches #main for the loading skeleton from now on
const watchSkeleton = (page) => page.evaluate(() => { window.__sk = false; const mo = new MutationObserver(() => { if (document.querySelector('#main .sk-grid')) window.__sk = true; }); mo.observe(document.querySelector('#main'), { childList: true, subtree: true }); });

async function playlist(browser, width, android) {
  const at = `${width}${android ? ' Android' : ''}`; const phone = width < 600;
  const { context, page, state } = await fx.create(browser, width, { mobile: phone, fixtures, tracks, collections: [summary] });
  try {
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
    let release; const held = new Promise((r) => { release = r; });
    // (gone: a song taken out elsewhere, from the next read on)
    let gone = -1; const without = (list) => list.filter((_, k) => k !== gone);
    await context.route('**/v1/playlists/c1/play-from', async (r) => { await Promise.race([held, new Promise((res) => setTimeout(res, 10000))]); await r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...playFrom, items: without(playFrom.items) }) }).catch(() => {}); });
    await context.route('**/v1/playlists/c1', (r) => (gone < 0 || r.request().method() !== 'GET' ? r.fallback() : r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...detail, summary: { ...summary, items: N - 1 }, playlist: { ...detail.playlist, items: without(detail.playlist.items) }, tracks: without(tracks), entries: without(detail.entries) }) })));
    const pins = [];
    await context.route('**/v1/playlists/c1/items/*/source', (r) => { pins.push(`${r.request().method()} ${new URL(r.request().url()).pathname}`); return r.fulfill({ contentType: 'application/json', body: '{}' }); });
    await fx.go(page, '/playlist/c1');
    await page.waitForSelector('#plsongs .srcchip.wait');
    // Select while the sources load, tick two songs, scroll down to the second
    await startSelect(page, '#plsongs');
    await tick(page, 1); await tick(page, 20);
    await page.evaluate(() => { const r = document.querySelector('#plsongs .srow[data-srow="20"]'); const m = document.querySelector('#main'); m.scrollTo({ top: m.scrollTop + r.getBoundingClientRect().top - m.getBoundingClientRect().top - 200, behavior: "instant" }); });
    await page.waitForTimeout(100);
    const before = await look(page, '#plsongs');
    await page.screenshot({ path: path.join(SHOTS, `select-loading-${at.replace(' ', '-').toLowerCase()}.png`) });
    await watchSkeleton(page);
    release();
    await page.waitForSelector('#plsongs [data-psrc]', { state: 'attached' }); // (a phone hides the chips while selecting)
    await page.waitForTimeout(300);
    const after = await look(page, '#plsongs');
    const sk = await page.evaluate(() => window.__sk);
    await page.screenshot({ path: path.join(SHOTS, `select-answered-${at.replace(' ', '-').toLowerCase()}.png`) });
    check(before.selecting && before.ticked.join() === '1,20' && before.top > 100, `${at}: Select on while the sources load, songs 2 and 21 ticked, scrolled down (${before.ticked}, ${before.top}px)`);
    check(after.selecting && after.bar && after.pressed === 'true', `${at}: Select stays on when the sources answer (selecting ${after.selecting}, bar ${after.bar}, pressed ${after.pressed})`);
    check(after.boxes === N && after.ticked.join() === '1,20', `${at}: every row has its box and the two ticks stay (${after.boxes} boxes, ticked ${after.ticked})`);
    check(after.count === '2 songs selected' && after.bars === 1, `${at}: the bar still says 2 songs selected (${after.count}, ${after.bars} bar)`);
    check(!sk, `${at}: no loading skeleton while the list is drawn again`);
    check(Math.abs(after.top - before.top) <= 2, `${at}: the page stays where it was (${before.top} → ${after.top}px)`);
    // The bar acts on the list on screen: All ticks every new row
    if (after.bar) {
      await page.evaluate(() => document.querySelector('.selbar [data-sel="all"]').click());
      const all = await look(page, '#plsongs');
      check(all.ticked.length === N && all.count === `${N} songs selected`, `${at}: All ticks every row of the list drawn again (${all.ticked.length}, ${all.count})`);
    } else check(false, `${at}: All ticks every row of the list drawn again (no bar)`);
    // Stop selecting and start again: nothing is ticked, the bar says 0 (ticked first on main's
    // fresh list when the redraw ended Select)
    if (!after.bar) { await startSelect(page, '#plsongs'); await tick(page, 4); }
    await page.evaluate(() => document.querySelector('.selbar [data-sel="cancel"]').click());
    await startSelect(page, '#plsongs');
    const again = await look(page, '#plsongs');
    check(again.selecting && again.ticked.length === 0 && again.count === '0 songs selected', `${at}: Select again starts with nothing ticked (${again.ticked.length} ticked, bar "${again.count}")`);
    await page.evaluate(() => document.querySelector('.selbar [data-sel="cancel"]').click());
    // Pinning a song further down draws the list again where it stands
    await page.evaluate(() => { const r = document.querySelector('#plsongs .srow[data-srow="19"]'); const m = document.querySelector('#main'); m.scrollTo({ top: m.scrollTop + r.getBoundingClientRect().top - m.getBoundingClientRect().top - 200, behavior: "instant" }); });
    await page.waitForTimeout(100);
    const top0 = await page.evaluate(() => document.querySelector('#main').scrollTop);
    await watchSkeleton(page);
    await page.evaluate(() => document.querySelector('#plsongs [data-psrc="19"]').click());
    await page.waitForSelector('.pop-item');
    await page.evaluate(() => [...document.querySelectorAll('.pop-item')].find((b) => /Your library/.test(b.textContent)).click());
    await page.waitForFunction(() => document.querySelector('#plsongs [data-psrc]'), null, { timeout: 5000 });
    await page.waitForTimeout(400);
    const pinTop = await page.evaluate(() => document.querySelector('#main').scrollTop);
    const pinned = pins.includes('PUT /v1/playlists/c1/items/19/source');
    const pinSk = await page.evaluate(() => window.__sk);
    check(pinned && !pinSk && Math.abs(pinTop - top0) <= 2, `${at}: pinning song 20 to a source keeps the page where it was (put ${pinned}, skeleton ${pinSk}, ${top0} → ${pinTop}px)`);
    // A ticked song taken out elsewhere: Select stays on with the other ticks (song 3, song 6 gone)
    await startSelect(page, '#plsongs'); await tick(page, 2); await tick(page, 5);
    gone = 5;
    await page.evaluate(() => document.querySelector('#plsongs [data-psrc="0"]').click());
    await page.waitForSelector('.pop-item');
    await page.evaluate(() => [...document.querySelectorAll('.pop-item')].find((b) => /Your library/.test(b.textContent)).click());
    await page.waitForFunction((n) => document.querySelectorAll('#plsongs .srow').length === n, N - 1, { timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(300);
    const less = await look(page, '#plsongs');
    check(less.selecting && less.ticked.join() === '2' && less.count === '1 song selected', `${at}: a ticked song gone, Select stays on with the other tick (${less.selecting}, ticked ${less.ticked}, "${less.count}")`);
    gone = -1;
    await page.evaluate(() => { const c = document.querySelector('.selbar [data-sel="cancel"]'); if (c) c.click(); });
    // Opening another page ends Select
    await startSelect(page, '#plsongs'); await tick(page, 3);
    await page.evaluate(() => document.querySelector('#main [data-back]').click());
    await page.waitForTimeout(400);
    const ended = await page.evaluate(() => ({ bars: document.querySelectorAll('.selbar').length, selecting: document.querySelectorAll('.selecting').length }));
    check(ended.bars === 0 && ended.selecting === 0, `${at}: Back to Playlists ends Select (${ended.bars} bar)`);
    const over = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    check(over <= 0, `${at}: nothing runs past the screen (${over})`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

async function search(browser, width, android) {
  const at = `${width}${android ? ' Android' : ''} search`; const phone = width < 600;
  const { context, page, state } = await fx.create(browser, width, { mobile: phone, searchHits, tracks });
  try {
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
    await fx.go(page, '/search?q=journey');
    const sec = '[data-skind="song"]';
    await page.waitForSelector(`${sec} [data-selstart]`);
    await startSelect(page, sec);
    await page.evaluate((s) => document.querySelector(`${s} .selbox input[data-k="2"]`).click(), sec);
    const before = await look(page, sec);
    await page.evaluate(() => document.querySelector('[data-sallsongs]').click());
    await page.waitForTimeout(200);
    const after = await look(page, sec);
    await page.screenshot({ path: path.join(SHOTS, `search-all-${at.replace(/ /g, '-').toLowerCase()}.png`) });
    check(before.boxes === 6 && before.ticked.join() === '2', `${at}: Select on the first 6 songs, the third ticked (${before.boxes}, ${before.ticked})`);
    check(after.selecting && after.bars === 1 && after.boxes === 10 && after.ticked.join() === '2' && after.count === '1 song selected', `${at}: See all songs keeps Select and the tick, a box on each of the 10 (${after.selecting}, ${after.boxes} boxes, ticked ${after.ticked}, "${after.count}")`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

// The playlist changing under Select: a song taken out above a tick (by its row's own Remove),
// a song in it twice, a source filter, the bar's Remove while a redraw comes, and the keyboard's
// Move down pressed twice. Remove after a redraw must delete exactly the songs ticked on screen.
async function edits(browser, width) {
  const at = `${width} edits`; const phone = width < 600;
  const { context, page, state } = await fx.create(browser, width, { mobile: phone, fixtures, tracks, collections: [summary] });
  try {
    let list = []; let deletes = []; let puts = []; let holdDelete = null; let holdPut = null; let failAt = -1;
    // (odd songs play from the library, even ones from Spotify, so the filter has two sources)
    const svc = (t) => (t.id % 2 ? 'library' : 'spotify');
    const body = () => ({ summary: { ...summary, items: list.length }, playlist: { ...summary, items: list.map((t) => ({ uri: t.canonical_uri })) }, tracks: list, entries: list.map((t) => ({ kind: 'track', uri: t.canonical_uri })), local_tracks: [] });
    const pfNow = () => ({ order: { order: ['library', 'spotify'], services: [{ id: 'library', name: 'Your library', connected: true }, { id: 'spotify', name: 'Spotify', connected: true }] }, own_order: false, items: list.map((t, k) => ({ index: k, uri: t.canonical_uri, title: t.title, artist: 'Sefa', track_id: t.id, plays_from: { service: svc(t), label: svc(t) }, sources: [{ ...libSource, service: svc(t) }] })) });
    await context.route('**/v1/playlists/c1/play-from', (r) => r.fulfill({ contentType: 'application/json', body: JSON.stringify(pfNow()) }));
    await context.route('**/v1/playlists/c1', async (r) => {
      const m = r.request().method();
      if (m === 'PUT') { const { uris } = JSON.parse(r.request().postData()); puts.push(uris); if (holdPut) await holdPut; const pool = [...list]; list = uris.map((u) => pool.splice(pool.findIndex((t) => t.canonical_uri === u), 1)[0]); }
      return r.fulfill({ contentType: 'application/json', body: JSON.stringify(body()) }).catch(() => {});
    });
    await context.route('**/v1/playlists/c1/items/*', async (r) => {
      if (r.request().method() !== 'DELETE') return r.fallback();
      const k = Number(new URL(r.request().url()).pathname.split('/').pop()); deletes.push(k);
      if (holdDelete) await holdDelete;
      if (deletes.length - 1 === failAt) return r.fulfill({ status: 500, contentType: 'application/json', body: '{"error":"homeserver did not answer"}' }).catch(() => {});
      list.splice(k, 1); return r.fulfill({ contentType: 'application/json', body: '{}' }).catch(() => {});
    });
    const S = (n) => tracks[n - 1];
    const titles = () => list.map((t) => t.title.replace('Song ', '')).join(',');
    const open = async (songs) => {
      list = songs; deletes = []; puts = []; holdDelete = null; holdPut = null; failAt = -1;
      await fx.go(page, '/playlist/c1');
      await page.waitForSelector('#plsongs [data-psrc]', { state: 'attached' });
      await page.waitForTimeout(200);
    };
    const rows = (n) => page.waitForFunction((nn) => document.querySelectorAll('#plsongs .srow').length === nn, n, { timeout: 5000 }).catch(() => {});
    const press = (s) => page.evaluate((ss) => document.querySelector(ss).click(), s);
    const removeTicked = async () => { await press('.selbar [data-sel="remove"]'); await page.waitForFunction(() => !document.querySelector('.selbar'), null, { timeout: 5000 }).catch(() => {}); await page.waitForTimeout(200); };

    // A song taken out above the tick: the tick moves up with its song, Remove takes that song
    await open([S(1), S(2), S(3), S(4), S(5), S(6)]);
    await startSelect(page, '#plsongs'); await tick(page, 3);
    await press('#plsongs [data-rm="1"]'); await rows(5); await page.waitForTimeout(200);
    let l = await look(page, '#plsongs');
    check(l.selecting && l.ticked.join() === '2' && l.count === '1 song selected', `${at}: song 2 taken out above the tick on song 4, the tick moves up with it (ticked ${l.ticked}, "${l.count}")`);
    await removeTicked();
    check(deletes.join() === '1,2' && titles() === '1,3,5,6', `${at}: Remove then deletes song 4 (DELETE items ${deletes}; left ${titles()})`);

    // A song in the list twice, its 2nd copy ticked: a song above taken out, still the 2nd copy
    await open([S(1), S(2), S(3), S(1), S(4)]);
    await startSelect(page, '#plsongs'); await tick(page, 3);
    await press('#plsongs [data-rm="1"]'); await rows(4); await page.waitForTimeout(200);
    l = await look(page, '#plsongs');
    check(l.selecting && l.ticked.join() === '2' && l.count === '1 song selected', `${at}: [1,2,3,1,4] with the 2nd 1 ticked, 2 taken out: the 2nd 1 stays ticked (ticked ${l.ticked}, "${l.count}")`);
    await removeTicked();
    check(deletes.join() === '1,2' && titles() === '1,3,4', `${at}: Remove then deletes the 2nd copy, the 1st stays first (DELETE items ${deletes}; left ${titles()})`);

    // A copy of the ticked song goes: which one was ticked isn't sure, so its tick is dropped
    await open([S(1), S(2), S(1), S(3)]);
    await startSelect(page, '#plsongs'); await tick(page, 2); await tick(page, 3);
    await press('#plsongs [data-rm="0"]'); await rows(3); await page.waitForTimeout(200);
    l = await look(page, '#plsongs');
    check(l.selecting && l.ticked.join() === '2' && l.count === '1 song selected', `${at}: [1,2,1,3] with the 2nd 1 and 3 ticked, the 1st 1 taken out: only 3 stays ticked (ticked ${l.ticked}, "${l.count}")`);
    await page.evaluate(() => { const c = document.querySelector('.selbar [data-sel="cancel"]'); if (c) c.click(); });

    // A filter hiding a ticked song: its tick goes, so Remove can't take a song not on screen
    await open([S(1), S(2), S(3), S(4), S(5), S(6)]);
    await startSelect(page, '#plsongs'); await tick(page, 1); await tick(page, 2);
    await press('#plsongs [data-pfilter="library"]');
    await page.waitForSelector('#plsongs .list-row.filtered', { state: 'attached', timeout: 5000 }).catch(() => {}); await page.waitForTimeout(200);
    l = await look(page, '#plsongs');
    const hidden = await page.evaluate(() => [...document.querySelectorAll('#plsongs .list-row.filtered .selbox input')].filter((c) => c.checked).length);
    check(l.selecting && l.ticked.join() === '2' && hidden === 0 && l.count === '1 song selected', `${at}: filtered to the library, the tick on hidden song 2 goes, song 3's stays (ticked ${l.ticked}, ${hidden} hidden ticked, "${l.count}")`);
    await press('.selbar [data-sel="all"]');
    l = await look(page, '#plsongs');
    const allLabel = await page.evaluate(() => document.querySelector('.selbar [data-sel="all"]').textContent);
    check(l.ticked.join() === '0,2,4' && l.count === '3 songs selected' && allLabel === 'None', `${at}: filtered, All ticks just the 3 songs on screen (ticked ${l.ticked}, "${l.count}", then "${allLabel}")`);
    await press('.selbar [data-sel="all"]'); await tick(page, 2);
    await removeTicked();
    check(deletes.join() === '2' && titles() === '1,2,4,5,6', `${at}: Remove deletes only song 3 (DELETE items ${deletes}; left ${titles()})`);

    // The bar's Remove takes a while: its buttons wait, and a redraw meanwhile ends Select
    await open([S(1), S(2), S(3), S(4), S(5), S(6)]);
    await startSelect(page, '#plsongs'); await tick(page, 1); await tick(page, 4);
    let release; holdDelete = new Promise((r) => { release = r; });
    await press('.selbar [data-sel="remove"]');
    await page.waitForTimeout(150);
    const busy = await page.evaluate(() => [...document.querySelectorAll('.selbar button')].map((b) => b.disabled));
    check(busy.length > 0 && busy.every(Boolean), `${at}: while Remove runs, the bar's buttons wait (${busy.filter(Boolean).length}/${busy.length} disabled)`);
    await press('#plsongs [data-pfilter=""]'); // (the page drawn again meanwhile)
    await page.waitForTimeout(400);
    const mid = await page.evaluate(() => ({ bar: !!document.querySelector('.selbar'), selecting: document.querySelectorAll('.selecting').length, remove: !!document.querySelector('.selbar [data-sel="remove"]:not(:disabled)') }));
    check(!mid.remove && !mid.bar && mid.selecting === 0, `${at}: a redraw while Remove runs ends Select, no second Remove to press (bar ${mid.bar}, working Remove ${mid.remove})`);
    const held = await page.evaluate(() => !!document.querySelector('#plsongs').closest('[inert]'));
    check(held, `${at}: the list drawn again takes no presses until Remove is done (inert ${held})`);
    if (!phone) {
      const b = await page.locator('#plsongs [data-rm="0"]').boundingBox();
      await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2); await page.waitForTimeout(200);
      check(deletes.join() === '4', `${at}: a row's own Remove pressed meanwhile does nothing (DELETE items so far ${deletes})`);
    }
    release(); holdDelete = null;
    await rows(4); await page.waitForTimeout(300);
    check(deletes.join() === '4,1' && titles() === '1,3,4,6', `${at}: just songs 2 and 5 go (DELETE items ${deletes}; left ${titles()})`);
    const freed = await page.evaluate(() => !document.querySelector('#plsongs').closest('[inert]'));
    check(freed, `${at}: the list takes presses again when Remove is done`);

    // Remove failing part way: Select ends (its ticks point at other songs now), the list is read again
    await open([S(1), S(2), S(3), S(4), S(5), S(6)]);
    await startSelect(page, '#plsongs'); await tick(page, 1); await tick(page, 4);
    failAt = 1;
    await press('.selbar [data-sel="remove"]');
    await rows(5); await page.waitForTimeout(400);
    const failed = await page.evaluate(() => ({ bar: !!document.querySelector('.selbar'), selecting: document.querySelectorAll('.selecting').length, rows: document.querySelectorAll('#plsongs .srow').length, inert: !!document.querySelector('#plsongs').closest('[inert]') }));
    check(deletes.join() === '4,1' && titles() === '1,2,3,4,6' && !failed.bar && failed.selecting === 0 && failed.rows === 5 && !failed.inert, `${at}: Remove failing on the 2nd song ends Select and shows the list as it is (DELETE items ${deletes}; left ${titles()}; bar ${failed.bar}, ${failed.rows} rows)`);

    if (!phone) {
      // From the keyboard, Move down twice moves the song two places: the focus follows it
      await open([S(1), S(2), S(3), S(4), S(5), S(6)]);
      await page.evaluate(() => document.querySelector('#plsongs [data-pdown="2"]').focus());
      await page.keyboard.press('Enter');
      await page.waitForFunction(() => document.activeElement && document.activeElement.matches('#plsongs [data-pdown="3"]'), null, { timeout: 5000 }).catch(() => {});
      const f1 = await page.evaluate(() => document.activeElement.dataset.pdown);
      await page.keyboard.press('Enter');
      await page.waitForFunction(() => document.activeElement && document.activeElement.matches('#plsongs [data-pdown="4"]'), null, { timeout: 5000 }).catch(() => {});
      const f2 = await page.evaluate(() => document.activeElement.dataset.pdown);
      check(puts.length === 2 && titles() === '1,2,4,5,3,6' && f1 === '3' && f2 === '4', `${at}: Move down twice from the keyboard moves song 3 two places, the focus with it (${puts.length} saves, ${titles()}, focus ${f1} then ${f2})`);
      // The focus moved elsewhere while the move is saved: it stays there
      let go; holdPut = new Promise((r) => { go = r; });
      await page.evaluate(() => document.querySelector('#plsongs [data-pdown="0"]').focus());
      await page.keyboard.press('Enter');
      await page.waitForTimeout(100);
      await page.evaluate(() => document.querySelector('#plplay').focus());
      go(); holdPut = null;
      await page.waitForFunction(() => document.querySelector('#plsongs .srow[data-srow="1"] .name').textContent === 'Song 1', null, { timeout: 5000 }).catch(() => {});
      await page.waitForTimeout(400);
      const f3 = await page.evaluate(() => document.activeElement && (document.activeElement.id || document.activeElement.dataset.pdown || document.activeElement.tagName));
      check(titles() === '2,1,4,5,3,6' && f3 === 'plplay', `${at}: the focus moved to Play while the move was saved stays on Play (${titles()}, focus ${f3})`);
    }
    const over = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    check(over <= 0, `${at}: nothing runs past the screen (${over})`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [360, 412]) await playlist(browser, w, false);
    await playlist(browser, 360, true);
    await playlist(browser, 1280, false);
    await edits(browser, 360);
    await edits(browser, 1280);
    await search(browser, 360, false);
    await search(browser, 1280, false);
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
