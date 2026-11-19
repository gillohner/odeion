// A playlist's late draw. A draw of a playlist reads the playlist, then waits up to 1.2 s for where
// its songs play. Before, only the view was checked afterwards: an older draw that answered after a
// newer one (a library change redrew the page, or another device reordered the list) drew the old
// order over the new one, and every Remove on that page goes by a song's place in the list, so the
// × of one song deleted another. And a playlist opened while the previous one waited could get the
// previous one drawn under its address. Now only the latest draw of the playlist at the address
// draws anything (its error too). A list another device changed draws at once, its sources
// following, whatever the age of the answer kept from before and when no answer came yet; a source
// filter doesn't hide the rows meanwhile, and a song listed twice keeps each copy's own name. An
// answer for where the songs play asked before another device changed the list isn't used by place
// on a first draw either; one asked after the read that doesn't fit (the list moved again) gives
// each row its own song's answer, by uri and copy, never another song's by place, and the list is
// read once more so Remove goes by the places the node has; a song that answer leaves out keeps the
// earlier answer's copy; one whose songs are named otherwise gives no row a chip by place, asked at
// most twice, and its rows wait for the read after each answer (the node names each song by the
// list's own uri, so an answer that doesn't fit is a list changed under the read, never a reason to
// let × act by place). The focus a hold took goes
// back to the same song's control wherever the node now has it, never to another song at its old
// place. At 360 (phone) and 1280 px.
//   node tools/ui-playlist-stale-draw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-stale-draw-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

const song = (id, title) => ({ ...fx.track, id, title, canonical_uri: `https://musicbrainz.org/recording/0000000${id}-0000-4000-8000-000000000000`, mbid: null });
const SONGS = { A: song(1, 'Alpha'), X: song(2, 'Xylo'), B: song(3, 'Bravo'), C: song(4, 'Charlie') };
// Songs that play from a service, not the library: their rows take their names from where they play
const STREAMED = { P: { title: 'Papa', canonical_uri: 'https://tidal.com/browse/track/101' }, Q: { title: 'Quebec', canonical_uri: 'https://tidal.com/browse/track/102' }, Y: { title: 'Yankee', canonical_uri: 'https://tidal.com/browse/track/103' }, Z: { title: 'Xray', canonical_uri: 'https://tidal.com/browse/track/104' } };
Object.assign(SONGS, STREAMED);
const sum = (id, name, n) => ({ ...fx.summary, id, name, items: n });
const detailOf = (id, name, keys) => ({ summary: sum(id, name, keys.length), playlist: { ...sum(id, name, keys.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => (STREAMED[k] ? null : SONGS[k])), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: [] });
const pfOf = (keys) => ({ order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: keys.map((k, i) => (STREAMED[k] ? { index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', track_id: null, stream_uri: `tidal:track:${SONGS[k].canonical_uri.split('/').pop()}`, page: SONGS[k].canonical_uri, plays_from: { service: 'tidal', label: 'TIDAL' }, sources: [], connect: [] } : { index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', track_id: SONGS[k].id, plays_from: { service: 'library', label: 'Your library' }, sources: [] })) });
// (`retitle`: a playlist's answer naming the song at a place otherwise, as a pin to another source can)
const retitle = { c6: { 2: 'Pinned' } };
// (`renamed`: a playlist whose answer names each song under another uri than the list's: a stand-in for a list changed under the read, which a real node's answer is the only way to show (play_from.rs answers each song by the list's own uri))
const renamed = { c8: true, c11: true, c22: true };
// (`renamedBySong`: the same, each song under its own other uri wherever it is, so a song moved moves its uri too)
const renamedBySong = { c17: true };
// (`slow`: a playlist whose answer for where its songs play always comes after the draw's 1.2 s wait)
const slow = { c11: 1600 };
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, mobile] of [[360, true], [1280, false]]) {
      const { page, state } = await fx.create(browser, width, { mobile });
      // The server's lists, each GET answered with the list as it was when asked; `hold`: the next
      // GET of that path waits for its release
      const lists = { c1: ['A', 'X', 'B', 'A'], c2: ['C'], c3: ['B', 'X'], c4: ['P', 'Q', 'A'], c5: ['A', 'X', 'B'], c6: ['P', 'Q', 'P'], c7: ['P', 'Q', 'A'], c8: ['P', 'Q', 'A'], c9: ['P', 'Q', 'A'], c10: ['Y', 'P', 'Q'], c11: ['P', 'Q', 'A'], c12: ['Y', 'P', 'Q'], c13: ['A', 'X', 'B', 'C'], c14: ['Y', 'P', 'Q'], c15: ['Y', 'P'], c16: ['A'], c17: ['A', 'X', 'B'], c18: ['C'], c19: ['X'], c20: ['C'], c21: ['X'], c22: ['P', 'Q', 'A'] };
      const names = { c1: 'Road trip', c2: 'Quiet hours', c3: 'Long drive', c4: 'Streamed', c5: 'Slow sources', c6: 'Twice', c7: 'Moved meanwhile', c8: 'Other names', c9: 'Moved twice', c10: 'Swapped', c11: 'Slow other names', c12: 'Added meanwhile', c13: 'Turned round', c14: 'Moved under the focus', c15: 'All swapped', c16: 'Replaced twice', c17: 'Other names moved', c18: 'Back and changed', c19: 'Back to Xylo', c20: 'Back and forth', c21: 'Back, then off again', c22: 'Other names reordered' };
      const hold = {}; const deletes = []; const asks = [];
      // (`fresh`: the held GET answers with the list as it is when released)
      const gate = (key, fail, fresh) => { let open; const p = new Promise((res) => { open = res; }); hold[key] = { p, fail, fresh }; return open; };
      let socket = null;
      await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
      await page.route(/\/v1\/playlists\/(c1|c2|c3|c4|c5|c6|c7|c8|c9|c10|c11|c12|c13|c14|c15|c16|c17|c18|c19|c20|c21|c22)(\/.*)?$/, async (r) => {
        const u = new URL(r.request().url()); const m = r.request().method();
        const [, , , id, ...rest] = u.pathname.split('/'); const sub = rest.join('/');
        if (m === 'DELETE' && sub.startsWith('items/')) { const k = Number(sub.split('/')[1]); deletes.push(k); lists[id].splice(k, 1); return json(r, {}); }
        if (m !== 'GET') return r.fallback();
        const key = `${id}${sub ? '/' + sub : ''}`; asks.push(key);
        let then = [...lists[id]];
        const h = hold[key]; delete hold[key]; if (h) await h.p;
        if (h && h.fresh) then = [...lists[id]];
        if (sub === 'play-from' && slow[id]) await sleep(slow[id]);
        if (h && h.fail) return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'The node did not answer' }) });
        if (sub === 'play-from') { const a = pfOf(then); Object.entries(retitle[id] || {}).forEach(([k, t]) => { if (a.items[k]) a.items[k].title = t; }); if (renamed[id]) a.items.forEach((x, k) => { x.uri = `https://node.example/song/${id}-${k}`; }); if (renamedBySong[id]) a.items.forEach((x, k) => { x.uri = `https://node.example/song/${id}-${then[k]}`; }); return json(r, a); }
        if (!sub) return json(r, detailOf(id, names[id], then));
        return r.fallback();
      });
      await page.route('**/v1/playlists', (r) => (r.request().method() === 'GET' ? json(r, [sum('c1', names.c1, 4), sum('c2', names.c2, 1), sum('c3', names.c3, 2)]) : r.fallback()));
      const order = () => page.evaluate(() => [...document.querySelectorAll('#plsongs .track:not(.skeleton), #plsongs .list-row')].map((row) => { const n = row.querySelector('.name, .title, .t-title'); return (n ? n.textContent : row.textContent).trim().split(/\s/)[0]; }).filter(Boolean));
      const title = () => page.evaluate(() => (document.querySelector('#main .hero h1') || {}).textContent || '');
      try {
        // 1. One playlist drawn twice, the older draw answering last
        await fx.go(page, '/playlist/c1');
        await page.waitForSelector('#plsongs [data-rm="3"]', { state: 'attached' }); // (a phone hides it: the row's ⋯ menu presses it)
        for (let i = 0; i < 50 && !socket; i++) await sleep(100);
        check(!!socket, `${width}: events connected`);
        const first = await order();
        check(JSON.stringify(first) === '["Alpha","Xylo","Bravo","Alpha"]', `${width}: the playlist shows as the server has it (${first})`);
        // The first redraw's read is held; meanwhile another device moves the last Alpha up to second place
        const release = gate('c1');
        const asked = asks.length;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        check(await until(async () => asks.slice(asked).includes('c1'), 2000), `${width}: a library change reads the playlist again`);
        lists.c1 = ['A', 'A', 'X', 'B'];
        socket.send(JSON.stringify({ type: 'library_changed' }));
        await until(async () => JSON.stringify(await order()) === '["Alpha","Alpha","Xylo","Bravo"]', 3000);
        check(JSON.stringify(await order()) === '["Alpha","Alpha","Xylo","Bravo"]', `${width}: the newer draw shows the new order (${await order()})`);
        release(); await sleep(1500);
        const after = await order();
        check(JSON.stringify(after) === '["Alpha","Alpha","Xylo","Bravo"]', `${width}: the older draw answering late doesn't put the old order back (${after})`);
        await page.screenshot({ path: path.join(SHOTS, `late-draw-${width}.png`) });
        // Bravo's × deletes Bravo on the server (its place in the newer list, 3), not Xylo
        const bravoRm = await page.evaluate(() => { const row = [...document.querySelectorAll('#plsongs [data-rm]')].find((b) => /Bravo/.test(b.getAttribute('aria-label') || '')); return row ? row.dataset.rm : null; });
        check(bravoRm === '3', `${width}: Bravo's Remove is for place 3 (${bravoRm})`);
        await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Bravo/.test(x.getAttribute('aria-label') || '')); if (b) b.click(); });
        await until(async () => deletes.length === 1, 2000);
        check(JSON.stringify(deletes) === '[3]' && JSON.stringify(lists.c1) === '["A","A","X"]', `${width}: Remove deleted Bravo (DELETE ${deletes}, server now ${lists.c1})`);
        await until(async () => JSON.stringify(await order()) === '["Alpha","Alpha","Xylo"]', 3000);
        check(JSON.stringify(await order()) === '["Alpha","Alpha","Xylo"]', `${width}: the list shows Bravo gone (${await order()})`);

        // 2. A playlist opened while the previous one still waits for where its songs play
        const errs = state.errors.length;
        // (a playlist not opened yet: where its songs play isn't known, so its draw waits for it)
        const releasePf = gate('c3/play-from');
        await page.evaluate(() => { const st = { v: 'playlists' }; history.pushState(st, '', '/playlists'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await page.waitForSelector('#main [data-pl="c3"]');
        const at = asks.length;
        await page.click('#main [data-pl="c3"] .name');
        check(await until(async () => asks.slice(at).includes('c3') && asks.slice(at).includes('c3/play-from'), 2000), `${width}: the first playlist is asked for`);
        await sleep(100);
        await page.evaluate(() => { const st = { v: 'playlist', arg: 'c2' }; history.pushState(st, '', '/playlist/c2'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await until(async () => (await title()) === 'Quiet hours', 3000);
        check((await title()) === 'Quiet hours', `${width}: the second playlist opens ("${await title()}")`);
        await sleep(1600); // past the first one's 1.2 s wait
        const t2 = await title(); const o2 = await order();
        check(t2 === 'Quiet hours' && JSON.stringify(o2) === '["Charlie"]', `${width}: the first playlist never draws under the second's address ("${t2}", ${o2}, ${await page.evaluate(() => location.pathname)})`);
        releasePf(); await sleep(400);
        check((await title()) === 'Quiet hours', `${width}: its late sources don't draw it either ("${await title()}")`);
        await page.screenshot({ path: path.join(SHOTS, `other-playlist-${width}.png`) });

        // 3. An older draw whose read fails after a newer draw showed the list: no error over it
        const releaseFail = gate('c2', true);
        const asked3 = asks.length;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        check(await until(async () => asks.slice(asked3).includes('c2'), 2000), `${width}: the playlist is read again`);
        lists.c2 = ['C', 'A'];
        socket.send(JSON.stringify({ type: 'library_changed' }));
        await until(async () => JSON.stringify(await order()) === '["Charlie","Alpha"]', 3000);
        releaseFail(); await sleep(500);
        const o3 = await order(); const notice = await page.locator('#main .notice.warn').count();
        check(JSON.stringify(o3) === '["Charlie","Alpha"]' && notice === 0, `${width}: the older draw's failure leaves the newer list (${o3}, ${notice} error notices)`);

        // 4. Where the songs play, kept from before, goes by place too: after another device moved a
        // streamed song, a redraw asks for it again rather than naming each row after the old order
        await page.evaluate(() => { const st = { v: 'playlist', arg: 'c4' }; history.pushState(st, '', '/playlist/c4'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await until(async () => JSON.stringify(await order()) === '["Papa","Quebec","Alpha"]', 3000);
        check(JSON.stringify(await order()) === '["Papa","Quebec","Alpha"]', `${width}: the streamed playlist shows as the server has it (${await order()})`);
        lists.c4 = ['Q', 'P', 'A'];
        const asked4 = asks.length;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        await until(async () => asks.slice(asked4).includes('c4'), 2000); await sleep(800);
        const o4 = await order();
        check(JSON.stringify(o4) === '["Quebec","Papa","Alpha"]', `${width}: the redraw names each row after the song now in its place (${o4})`);
        check(asks.slice(asked4).includes('c4/play-from'), `${width}: where they play is asked for again (${asks.slice(asked4)})`);
        const papaRm = await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Papa/.test(x.getAttribute('aria-label') || '')); return b ? b.dataset.rm : null; });
        check(papaRm === '1', `${width}: Papa's Remove is for place 1, where the server has it (${papaRm})`);
        const asked5 = asks.length;
        socket.send(JSON.stringify({ type: 'library_changed' })); await until(async () => asks.slice(asked5).includes('c4'), 2000); await sleep(300);
        check(!asks.slice(asked5).includes('c4/play-from') && JSON.stringify(await order()) === '["Quebec","Papa","Alpha"]', `${width}: an unchanged list keeps the answer it has (${asks.slice(asked5)})`);
        await page.screenshot({ path: path.join(SHOTS, `streamed-moved-${width}.png`) });

        // 5. The list another device changed is drawn at once, while where its songs play is asked
        // again (before: the old order stayed up to 1.2 s, each × removing by the new places), each
        // streamed song named as the earlier answer had it, its chip a spinner till the answer. From a
        // fresh open (another playlist's answer kept meanwhile), where the answer came within the wait
        const goPl = (id) => page.evaluate((arg) => { const st = { v: 'playlist', arg }; history.pushState(st, '', `/playlist/${arg}`); dispatchEvent(new PopStateEvent('popstate', { state: st })); }, id);
        await goPl('c2'); await until(async () => (await title()) === 'Quiet hours', 3000);
        await sleep(300);
        const asked5b = asks.length;
        await goPl('c4');
        await until(async () => JSON.stringify(await order()) === '["Quebec","Papa","Alpha"]' && (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        check(asks.slice(asked5b).includes('c4/play-from'), `${width}: opened again after another playlist, where its songs play is asked (${asks.slice(asked5b)})`);
        const releasePf5 = gate('c4/play-from');
        lists.c4 = ['A', 'Q', 'P'];
        const asked6 = asks.length;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        await until(async () => asks.slice(asked6).includes('c4/play-from'), 2000);
        const fast = await until(async () => JSON.stringify(await order()) === '["Alpha","Quebec","Papa"]', 600);
        check(fast, `${width}: the new order shows at once, its sources still coming (${await order()})`);
        const spin = await page.locator('#plsongs .srcchip.wait').count();
        const loadingRows = await page.locator('#plsongs', { hasText: 'Loading…' }).count();
        check(spin === 3 && loadingRows === 0, `${width}: each row waits for its source, streamed songs keep their names (${spin} spinners, ${loadingRows} Loading…)`);
        const papaRm5 = await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Papa/.test(x.getAttribute('aria-label') || '')); return b ? b.dataset.rm : null; });
        check(papaRm5 === '2', `${width}: Papa's Remove is for place 2 while its source is asked (${papaRm5})`);
        const left = await page.evaluate(() => { const b = document.querySelector('#plplay'); b.click(); return new Promise((res) => setTimeout(() => res((document.querySelector('#toast') || {}).textContent || ''), 300)); });
        check(!/left out/.test(left), `${width}: Play meanwhile keeps the streamed songs ("${left.trim()}")`);
        await page.screenshot({ path: path.join(SHOTS, `moved-pending-${width}.png`) });
        releasePf5();
        await until(async () => (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        const o6 = await order();
        check(JSON.stringify(o6) === '["Alpha","Quebec","Papa"]' && (await page.locator('#plsongs .srcchip.wait').count()) === 0, `${width}: the answer fills the chips, the order kept (${o6})`);
        await page.screenshot({ path: path.join(SHOTS, `moved-answered-${width}.png`) });

        // 6. The same with the answer kept from before older than 30 s (the page open a while): the
        // new order still shows at once, the streamed songs named as that answer had them. With a
        // source filter on, the rows waiting for their source aren't hidden meanwhile
        await page.click('#plsongs [data-pfilter="tidal"]');
        await until(async () => (await page.locator('#plsongs .filtered').count()) === 1, 2000);
        check((await page.locator('#plsongs .filtered').count()) === 1, `${width}: the TIDAL filter hides the library song`);
        await page.evaluate(() => { const real = Date.now; window.realNow = real; Date.now = () => real.call(Date) + 31000; });
        const releasePf6 = gate('c4/play-from');
        lists.c4 = ['P', 'A', 'Q'];
        const asked7 = asks.length;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        await until(async () => asks.slice(asked7).includes('c4/play-from'), 2000);
        const fast6 = await until(async () => JSON.stringify(await order()) === '["Papa","Alpha","Quebec"]', 600);
        check(fast6, `${width}: an answer older than 30 s: the new order shows at once (${await order()})`);
        const papaRm6 = await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Papa/.test(x.getAttribute('aria-label') || '')); return b ? b.dataset.rm : null; });
        check(papaRm6 === '0', `${width}: Papa's Remove is for place 0 meanwhile (${papaRm6})`);
        const hidden6 = await page.locator('#plsongs .filtered').count();
        check(hidden6 === 0 && (await page.locator('#plsongs .srcchip.wait').count()) === 3, `${width}: under the filter, the rows waiting for their source stay shown (${hidden6} hidden)`);
        await page.screenshot({ path: path.join(SHOTS, `moved-aged-filtered-${width}.png`) });
        releasePf6();
        await until(async () => (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        check((await page.locator('#plsongs .filtered').count()) === 1 && JSON.stringify(await order()) === '["Papa","Alpha","Quebec"]', `${width}: the answer filters again, the order kept (${await order()})`);
        await page.click('#plsongs [data-pfilter=""]');
        await page.evaluate(() => { Date.now = window.realNow; });

        // 7. A playlist whose first answer is slower than the 1.2 s wait, changed on another device
        // before any answer came: the new order shows at once too
        const releaseFirst = gate('c5/play-from');
        await goPl('c5');
        await until(async () => JSON.stringify(await order()) === '["Alpha","Xylo","Bravo"]', 3000);
        await sleep(1500); // (past the wait: the rows are drawn, their sources still coming)
        check(JSON.stringify(await order()) === '["Alpha","Xylo","Bravo"]' && (await page.locator('#plsongs .srcchip.wait').count()) === 3, `${width}: a slow first answer: the songs show, their sources coming (${await order()})`);
        const releaseSecond = gate('c5/play-from');
        lists.c5 = ['B', 'A', 'X'];
        const asked8 = asks.length;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        await until(async () => asks.slice(asked8).includes('c5'), 2000);
        const fast7 = await until(async () => JSON.stringify(await order()) === '["Bravo","Alpha","Xylo"]', 600);
        check(fast7, `${width}: no answer yet: the new order shows at once (${await order()})`);
        const bravoRm7 = await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Bravo/.test(x.getAttribute('aria-label') || '')); return b ? b.dataset.rm : null; });
        check(bravoRm7 === '0', `${width}: Bravo's Remove is for place 0 meanwhile (${bravoRm7})`);
        // (the first answer, for the old order, then comes: it fills no chips by the wrong places)
        releaseFirst(); await sleep(300);
        check(JSON.stringify(await order()) === '["Bravo","Alpha","Xylo"]', `${width}: the first answer, for the old order, leaves the new one (${await order()})`);
        releaseSecond();
        await until(async () => (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        check(JSON.stringify(await order()) === '["Bravo","Alpha","Xylo"]' && (await page.locator('#plsongs .srcchip.wait').count()) === 0, `${width}: then the chips fill, the order kept (${await order()})`);

        // 8. A song listed twice, each copy named otherwise by where it plays (a pin): moved on another
        // device, each copy keeps its own name while the sources are asked again
        await goPl('c6');
        await until(async () => JSON.stringify(await order()) === '["Papa","Quebec","Pinned"]' && (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        check(JSON.stringify(await order()) === '["Papa","Quebec","Pinned"]', `${width}: a song listed twice shows each copy (${await order()})`);
        const releasePf8 = gate('c6/play-from');
        lists.c6 = ['Q', 'P', 'P'];
        const asked9 = asks.length;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        await until(async () => asks.slice(asked9).includes('c6/play-from'), 2000);
        await until(async () => (await order())[0] === 'Quebec', 600);
        check(JSON.stringify(await order()) === '["Quebec","Papa","Pinned"]', `${width}: moved, the first copy keeps its name and the second its own (${await order()})`);
        releasePf8();
        await until(async () => (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);

        // 9. Opened afresh, where its songs play answering at once, and another device moving a song
        // before the list itself answers: that answer is for the old places, so the rows don't take
        // their names (or their sources) from it by place: each streamed song keeps its own name,
        // its source asked again (before: Quebec's row showed Papa, and its × removed Quebec)
        const releaseRead9 = gate('c7', false, true);
        const asked10 = asks.length;
        await goPl('c7');
        await until(async () => asks.slice(asked10).includes('c7') && asks.slice(asked10).includes('c7/play-from'), 2000);
        await sleep(300); // (where they play has answered, for the old order)
        lists.c7 = ['Q', 'P', 'A'];
        releaseRead9();
        await until(async () => (await order()).length === 3, 3000);
        const o9 = await order();
        check(JSON.stringify(o9) === '["Quebec","Papa","Alpha"]', `${width}: an answer for the old places: each row shows the song now in its place (${o9})`);
        const papaRm9 = await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Papa/.test(x.getAttribute('aria-label') || '')); return b ? b.dataset.rm : null; });
        check(papaRm9 === '1', `${width}: Papa's Remove is for place 1, where the server has it (${papaRm9})`);
        await until(async () => asks.slice(asked10).filter((x) => x === 'c7/play-from').length > 1 && (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        check(asks.slice(asked10).filter((x) => x === 'c7/play-from').length > 1 && JSON.stringify(await order()) === '["Quebec","Papa","Alpha"]', `${width}: where they play is asked again and fills in, the order kept (${asks.slice(asked10)}, ${await order()})`);
        await page.screenshot({ path: path.join(SHOTS, `moved-before-read-${width}.png`) });

        // 10. Sources that answer under other names than the list's (the node names each under the
        // list's own uri): no row takes the answer at its place, whose song it can't tell, and where
        // the songs play is asked at most twice (before c72c7851: asked again and again)
        const asked11 = asks.length;
        await goPl('c8');
        await until(async () => asks.slice(asked11).filter((x) => x === 'c8').length >= 2 && (await page.locator('#plsongs .srcchip.wait').count()) === 0, 4000);
        await sleep(1500);
        const pf11 = asks.slice(asked11).filter((x) => x === 'c8/play-from').length;
        const reads11 = asks.slice(asked11).filter((x) => x === 'c8').length;
        const chips11 = await page.locator('#plsongs .srcchip.tidal, #plsongs .srcchip:not(.wait):not(.none)').count();
        const o11 = await order();
        check((await page.locator('#plsongs .srcchip.wait').count()) === 0 && o11.length === 3 && o11[2] === 'Alpha' && !o11.includes('Papa') && !o11.includes('Quebec') && chips11 === 0, `${width}: an answer under other names: no row takes a name or chip by place (${chips11} chips, ${o11})`);
        check(pf11 >= 1 && pf11 <= 2 && reads11 <= 3, `${width}: where they play is asked at most twice, the list read at most three times (${pf11}, ${reads11})`);
        await page.screenshot({ path: path.join(SHOTS, `other-names-${width}.png`) });
        const inertNow = () => page.evaluate(() => { const box = document.querySelector('#plsongs'); return !!box && box.inert; });
        // 10b. The same other names, the library changing twice: each redraw's answer doesn't fit the
        // list read, so its rows wait for the read after it, then take presses again, the order kept.
        // (A node names each song by the list's own uri, play_from.rs: an answer that doesn't fit is a
        // list changed under the read, and the rows can't tell which song each place has now)
        await page.evaluate(() => { window.__holds = 0; window.__holdMo = new MutationObserver((recs) => { window.__holds += recs.filter((r) => r.oldValue === null && (r.target.id === 'plsongs' || r.target.id === 'pllocal') && r.target.hasAttribute('inert')).length; }); window.__holdMo.observe(document.getElementById('main'), { subtree: true, attributes: true, attributeFilter: ['inert'], attributeOldValue: true }); });
        const asked11b = asks.length;
        for (let i = 0; i < 2; i++) { const n = asks.length; socket.send(JSON.stringify({ type: 'library_changed' })); await until(async () => asks.slice(n).includes('c8'), 2000); await sleep(1500); }
        const holds11b = await page.evaluate(() => { window.__holdMo.disconnect(); return window.__holds; });
        const reads11b = asks.slice(asked11b).filter((x) => x === 'c8').length;
        check(holds11b === 2 && reads11b === 4 && !(await inertNow()) && JSON.stringify(await order()) === JSON.stringify(o11), `${width}: under other names, each library change holds the rows till the read after it, then they take presses (${holds11b} holds, ${asks.slice(asked11b)})`);

        // 11. Opened again within 30 s after another device moved a song, and moved once more before
        // the new answer for where the songs play comes: that answer, asked after the read, doesn't
        // fit the list read. Each row takes its own song's answer, not the one at its place (before:
        // Quebec's row showed Papa and Papa's showed Alpha till the next change)
        await goPl('c9');
        await until(async () => JSON.stringify(await order()) === '["Papa","Quebec","Alpha"]' && (await page.locator('#plsongs .srcchip:not(.wait)').count()) === 3, 3000);
        await page.evaluate(() => { const st = { v: 'playlists' }; history.pushState(st, '', '/playlists'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await page.waitForSelector('#main [data-pl="c1"]');
        const releasePf12 = gate('c9/play-from', false, true);
        lists.c9 = ['Q', 'P', 'A'];
        const asked12 = asks.length;
        await goPl('c9');
        await until(async () => asks.slice(asked12).includes('c9/play-from'), 2000);
        lists.c9 = ['P', 'A', 'Q'];
        releasePf12();
        await until(async () => (await order()).length === 3 && (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        await sleep(300);
        await until(async () => JSON.stringify(await order()) === '["Papa","Alpha","Quebec"]', 3000);
        const o12 = await order();
        check(JSON.stringify(o12) === '["Papa","Alpha","Quebec"]', `${width}: an answer that doesn't fit: the list is read again and shows as the node has it (${o12})`);
        check(asks.slice(asked12).filter((x) => x === 'c9').length === 2 && asks.slice(asked12).filter((x) => x === 'c9/play-from').length === 1, `${width}: the list read once more, where they play not asked again (${asks.slice(asked12)})`);
        const papaRm12 = await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Papa/.test(x.getAttribute('aria-label') || '')); return b ? b.dataset.rm : null; });
        check(papaRm12 === '0', `${width}: Papa's Remove is for place 0, where the node has it (${papaRm12})`);
        await page.screenshot({ path: path.join(SHOTS, `moved-twice-${width}.png`) });

        // 12. A song swapped for another between the read and the answer (list [Yankee, Papa, Quebec]
        // read, the answer for [Papa, Xray, Quebec]), then swapped back before the list is read again:
        // Yankee's row never shows Xray (before: the answer at its place, Xray, showed and played on
        // Yankee's row while its × removed Yankee). It keeps the earlier answer's Yankee, and × on it
        // removes Yankee on the server
        await goPl('c10');
        await until(async () => JSON.stringify(await order()) === '["Yankee","Papa","Quebec"]' && (await page.locator('#plsongs .srcchip:not(.wait)').count()) === 3, 3000);
        await page.evaluate(() => { const st = { v: 'playlists' }; history.pushState(st, '', '/playlists'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await page.waitForSelector('#main [data-pl="c1"]');
        lists.c10 = ['P', 'Y', 'Q'];
        const releasePf13 = gate('c10/play-from', false, true);
        const asked13 = asks.length;
        await goPl('c10');
        // (the answer comes within the draw's 1.2 s wait; the read after it is held)
        await until(async () => asks.slice(asked13).includes('c10/play-from'), 1000);
        lists.c10 = ['P', 'Z', 'Q'];
        const releaseRead13 = gate('c10', false, true);
        releasePf13();
        await until(async () => asks.slice(asked13).filter((x) => x === 'c10').length === 2, 2000);
        await sleep(200);
        const o13 = await order();
        check(JSON.stringify(o13) === '["Papa","Yankee","Quebec"]', `${width}: an answer naming another song at a row's place: the row keeps its own song (${o13})`);
        const yankeeChip = await page.evaluate(() => { const row = [...document.querySelectorAll('#plsongs .track, #plsongs .list-row')].find((x) => /Yankee/.test(x.textContent)); const c = row && row.querySelector('.srcchip'); return c ? c.className : 'none'; });
        check(!/\bwait\b/.test(yankeeChip) && /srcchip/.test(yankeeChip), `${width}: it keeps the earlier answer's source (${yankeeChip})`);
        // (and till that read has drawn them, the rows take no presses: each goes by a place the
        // node no longer has, Yankee's × would remove Xray. Before: × sent DELETE 1 at once)
        const inert13 = await page.evaluate(() => { const box = document.querySelector('#plsongs'); return !!box && box.inert; });
        check(inert13, `${width}: while the list is read again its rows take no presses`);
        const delHeld = deletes.length;
        await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Yankee/.test(x.getAttribute('aria-label') || '')); if (b) b.click(); });
        await sleep(300);
        check(deletes.length === delHeld, `${width}: Yankee's × pressed meanwhile removes nothing (${deletes.slice(delHeld)})`);
        await page.screenshot({ path: path.join(SHOTS, `swapped-${width}.png`) });
        lists.c10 = ['P', 'Y', 'Q'];
        releaseRead13();
        await sleep(400);
        const o13b = await order();
        check(JSON.stringify(o13b) === '["Papa","Yankee","Quebec"]' && !o13b.includes('Xray'), `${width}: read again, the list shows as the node has it, Yankee its own (${o13b})`);
        const del13 = deletes.length;
        await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Yankee/.test(x.getAttribute('aria-label') || '')); if (b) b.click(); });
        await until(async () => deletes.length > del13, 2000);
        check(JSON.stringify(deletes.slice(del13)) === '[1]' && JSON.stringify(lists.c10) === '["P","Q"]', `${width}: Yankee's × removes Yankee (DELETE ${deletes.slice(del13)}, server now ${lists.c10})`);
        await until(async () => JSON.stringify(await order()) === '["Papa","Quebec"]', 3000);
        check(JSON.stringify(await order()) === '["Papa","Quebec"]', `${width}: the list shows Yankee gone (${await order()})`);
        check(!(await page.evaluate(() => document.querySelector('#plsongs').inert)), `${width}: the read again ends the hold: the rows take presses again`);

        // 12b. The same, and another device adds a song before the list is read again: that read
        // isn't the list the answer was checked against, so where its songs play is asked again and
        // the new streamed song gets its name and source (before: the older answer was used by uri,
        // not asked again, and Xray's row stayed blank till the next change)
        await goPl('c12');
        await until(async () => JSON.stringify(await order()) === '["Yankee","Papa","Quebec"]' && (await page.locator('#plsongs .srcchip:not(.wait)').count()) === 3, 3000);
        await page.evaluate(() => { const st = { v: 'playlists' }; history.pushState(st, '', '/playlists'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await page.waitForSelector('#main [data-pl="c1"]');
        lists.c12 = ['P', 'Y', 'Q'];
        const releasePf15 = gate('c12/play-from', false, true);
        const asked15 = asks.length;
        await goPl('c12');
        await until(async () => asks.slice(asked15).includes('c12/play-from'), 1000);
        lists.c12 = ['Q', 'P', 'Y'];
        const releaseRead15 = gate('c12', false, true);
        releasePf15();
        await until(async () => asks.slice(asked15).filter((x) => x === 'c12').length === 2, 2000);
        await sleep(200);
        lists.c12 = ['Q', 'P', 'Y', 'Z'];
        releaseRead15();
        await until(async () => JSON.stringify(await order()) === '["Quebec","Papa","Yankee","Xray"]' && (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        const o15 = await order();
        check(JSON.stringify(o15) === '["Quebec","Papa","Yankee","Xray"]', `${width}: a song another device added meanwhile shows its name (${o15})`);
        const xrayChip = await page.evaluate(() => { const row = [...document.querySelectorAll('#plsongs .track, #plsongs .list-row')].find((x) => /Xray/.test(x.textContent)); const c = row && row.querySelector('.srcchip'); return c ? c.className : 'none'; });
        check(/srcchip/.test(xrayChip) && !/\b(wait|none)\b/.test(xrayChip), `${width}: and where it plays (${xrayChip})`);
        check(asks.slice(asked15).filter((x) => x === 'c12/play-from').length === 2, `${width}: where they play asked again for that read (${asks.slice(asked15)})`);
        check(!(await page.evaluate(() => document.querySelector('#plsongs').inert)), `${width}: its rows take presses`);
        await page.screenshot({ path: path.join(SHOTS, `added-meanwhile-${width}.png`) });

        // 12e. As 12, but another device puts other songs in every place: an answer naming none of
        // the list's songs, seen first, may be such a change and not a node naming them otherwise,
        // so the rows wait for the read after it (as every answer that doesn't fit does, 10b)
        await goPl('c15');
        await until(async () => JSON.stringify(await order()) === '["Yankee","Papa"]' && (await page.locator('#plsongs .srcchip:not(.wait)').count()) === 2, 3000);
        await page.evaluate(() => { const st = { v: 'playlists' }; history.pushState(st, '', '/playlists'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await page.waitForSelector('#main [data-pl="c1"]');
        lists.c15 = ['P', 'Y'];
        const releasePf18 = gate('c15/play-from', false, true); const asked18 = asks.length;
        await goPl('c15');
        await until(async () => asks.slice(asked18).includes('c15/play-from'), 1000);
        lists.c15 = ['Z', 'Q'];
        const releaseRead18 = gate('c15', false, true);
        releasePf18();
        await until(async () => asks.slice(asked18).filter((x) => x === 'c15').length === 2, 2000);
        await sleep(200);
        const inert18 = await page.evaluate(() => { const box = document.querySelector('#plsongs'); return !!box && box.inert; });
        check(inert18, `${width}: an answer naming none of the list's songs, seen first: the rows wait for the read after it`);
        releaseRead18();
        await until(async () => JSON.stringify(await order()) === '["Xray","Quebec"]' && !(await page.evaluate(() => document.querySelector('#plsongs').inert)), 3000);
        check(JSON.stringify(await order()) === '["Xray","Quebec"]', `${width}: then the list shows the node's songs (${await order()})`);

        // 12f. Another device replaces a one-song list twice, no song kept (Alpha, then Xylo, then
        // Bravo): the list read has Xylo, the answer asked after it is already for Bravo, and the
        // answer before it (Alpha's) names none of the list read too. That one was checked against
        // another list, so this is a list changed, not a node naming songs otherwise: the rows wait
        // for the read after it and a × reaches the node only then (before: the rows showed Xylo and
        // its × removed Bravo, place 0)
        await goPl('c16');
        await until(async () => JSON.stringify(await order()) === '["Alpha"]' && (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
        await page.evaluate(() => { const st = { v: 'playlists' }; history.pushState(st, '', '/playlists'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await page.waitForSelector('#main [data-pl="c1"]');
        lists.c16 = ['X'];
        const releasePf19 = gate('c16/play-from', false, true); const asked19 = asks.length; const del19 = deletes.length;
        await goPl('c16');
        await until(async () => asks.slice(asked19).includes('c16/play-from'), 1000);
        lists.c16 = ['B'];
        const releaseRead19 = gate('c16', false, true);
        releasePf19();
        await until(async () => asks.slice(asked19).filter((x) => x === 'c16').length === 2, 2000);
        await sleep(200);
        const inert19 = await page.evaluate(() => { const box = document.querySelector('#plsongs'); return !!box && box.inert; });
        await page.evaluate(() => { const b = document.querySelector('#plsongs [data-rm="0"]'); if (b) b.click(); });
        await sleep(300);
        check(inert19 && deletes.length === del19, `${width}: a list replaced twice meanwhile: the rows wait for the read after it, a × doesn't reach the node (held ${inert19}, DELETE ${deletes.slice(del19)})`);
        releaseRead19();
        await until(async () => JSON.stringify(await order()) === '["Bravo"]' && !(await page.evaluate(() => document.querySelector('#plsongs').inert)), 3000);
        check(JSON.stringify(await order()) === '["Bravo"]' && JSON.stringify(lists.c16) === '["B"]', `${width}: then the list shows Bravo, still on the node (${await order()}, server ${lists.c16})`);

        // 12g. A node naming the songs otherwise (each under its own other uri), opened again unchanged:
        // the answer doesn't fit the list read, so the rows wait for the read after it, as for any list
        // changed under the read (an answer the same as before is no proof: 12k, 12l). And another
        // device moving a song between the list read and the answer asked after it: the rows wait for
        // the read after it and a × reaches the node only then (before: the rows kept the old order and
        // the × of Alpha's row, place 0, removed Bravo)
        const away = async () => { await page.evaluate(() => { const st = { v: 'playlists' }; history.pushState(st, '', '/playlists'); dispatchEvent(new PopStateEvent('popstate', { state: st })); }); await page.waitForSelector('#main [data-pl="c1"]'); };
        const settled = (o) => until(async () => JSON.stringify(await order()) === JSON.stringify(o) && (await page.locator('#plsongs .srcchip.wait').count()) === 0 && !(await inertNow()), 4000);
        await goPl('c17');
        check(await settled(['Alpha', 'Xylo', 'Bravo']), `${width}: under other names by song, the list shows (${await order()})`);
        await sleep(1500);
        // (opened again with nothing changed: the same answer again, still held till the read after it)
        await away();
        await page.evaluate(() => { window.__holds20 = 0; window.__holdMo20 = new MutationObserver((recs) => { window.__holds20 += recs.filter((r) => r.oldValue === null && r.target.id === 'plsongs' && r.target.hasAttribute('inert')).length; }); window.__holdMo20.observe(document.getElementById('main'), { subtree: true, attributes: true, attributeFilter: ['inert'], attributeOldValue: true }); });
        const asked20a = asks.length;
        await goPl('c17');
        await until(async () => asks.slice(asked20a).includes('c17/play-from'), 2000);
        await settled(['Alpha', 'Xylo', 'Bravo']); await sleep(1500);
        const holds20a = await page.evaluate(() => { window.__holdMo20.disconnect(); return window.__holds20; });
        check(holds20a === 1 && !(await inertNow()), `${width}: under other names by song, opened again unchanged: held till the read after it, then the rows take presses (${holds20a} holds)`);
        await away();
        const releasePf20 = gate('c17/play-from', false, true); const asked20 = asks.length; const del20 = deletes.length;
        await goPl('c17');
        await until(async () => asks.slice(asked20).includes('c17/play-from'), 1000);
        lists.c17 = ['B', 'A', 'X'];
        const releaseRead20 = gate('c17', false, true);
        releasePf20();
        await until(async () => asks.slice(asked20).filter((x) => x === 'c17').length === 2, 2000);
        await sleep(200);
        const inert20 = await inertNow();
        await page.evaluate(() => { const b = document.querySelector('#plsongs [data-rm="0"]'); if (b) b.click(); });
        await sleep(300);
        check(inert20 && deletes.length === del20, `${width}: under other names, a song moved meanwhile: the rows wait for the read after it, a × doesn't reach the node (held ${inert20}, DELETE ${deletes.slice(del20)})`);
        releaseRead20();
        await settled(['Bravo', 'Alpha', 'Xylo']);
        check(JSON.stringify(await order()) === '["Bravo","Alpha","Xylo"]' && JSON.stringify(lists.c17) === '["B","A","X"]', `${width}: then the list shows the node's order, every song still on it (${await order()}, server ${lists.c17})`);

        // 12h. A one-song list another device keeps changing (Charlie shown): read as Alpha, Xylo
        // before the answer after the read (held, then read again, which fits), then read as Alpha
        // again and Bravo before the answer after that read. The list read the time an answer was off
        // before (Alpha) is the one read now, but an answer fitted a read since: it no longer counts,
        // so this is a change, held (before: the rows showed Alpha and its × removed Bravo)
        await goPl('c18');
        check(await settled(['Charlie']), `${width}: a one-song list shows (${await order()})`);
        await away();
        lists.c18 = ['A'];
        const releasePf21 = gate('c18/play-from', false, true); const asked21 = asks.length;
        await goPl('c18');
        await until(async () => asks.slice(asked21).includes('c18/play-from'), 1000);
        lists.c18 = ['X'];
        const releaseRead21 = gate('c18', false, true);
        releasePf21();
        await until(async () => asks.slice(asked21).filter((x) => x === 'c18').length === 2, 2000);
        await sleep(200);
        const inert21a = await inertNow();
        releaseRead21();
        check(inert21a && await settled(['Xylo']), `${width}: replaced meanwhile: held, then the list shows Xylo (held ${inert21a}, ${await order()})`);
        await away();
        lists.c18 = ['A'];
        const releasePf22 = gate('c18/play-from', false, true); const asked22 = asks.length; const del22 = deletes.length;
        await goPl('c18');
        await until(async () => asks.slice(asked22).includes('c18/play-from'), 1000);
        lists.c18 = ['B'];
        const releaseRead22 = gate('c18', false, true);
        releasePf22();
        await until(async () => asks.slice(asked22).filter((x) => x === 'c18').length === 2, 2000);
        await sleep(200);
        const inert22 = await inertNow();
        await page.evaluate(() => { const b = document.querySelector('#plsongs [data-rm="0"]'); if (b) b.click(); });
        await sleep(300);
        check(inert22 && deletes.length === del22, `${width}: changed back and again meanwhile: the rows wait for the read after it, a × doesn't reach the node (held ${inert22}, DELETE ${deletes.slice(del22)})`);
        releaseRead22();
        await settled(['Bravo']);
        check(JSON.stringify(await order()) === '["Bravo"]' && JSON.stringify(lists.c18) === '["B"]', `${width}: then the list shows Bravo, still on the node (${await order()}, server ${lists.c18})`);

        // (`offList`: open a one-song list again, read as `read`, `then` put in before the answer asked
        // after that read; resolves with the release of the read after it, to which `next` is put first)
        const offList = async (id, read, then) => {
          await away(); lists[id] = read;
          const releasePf = gate(`${id}/play-from`, false, true); const asked = asks.length;
          await goPl(id);
          await until(async () => asks.slice(asked).includes(`${id}/play-from`), 1000);
          lists[id] = then;
          const releaseRead = gate(id, false, true);
          releasePf();
          await until(async () => asks.slice(asked).filter((x) => x === id).length === 2, 2000);
          await sleep(200);
          return (next) => { if (next) lists[id] = next; releaseRead(); };
        };
        const heldNoDelete = async () => { const del = deletes.length; const held = await inertNow(); await page.evaluate(() => { const b = document.querySelector('#plsongs [data-rm="0"]'); if (b) b.click(); }); await sleep(300); return { held, sent: deletes.slice(del) }; };

        // 12i. Xylo shown; read as Alpha with Xylo back before the answer (the same answer as before:
        // held, then read again, which fits); then the same once more: held too (a × waits)
        await goPl('c19');
        check(await settled(['Xylo']), `${width}: a one-song list shows (${await order()})`);
        let go = await offList('c19', ['A'], ['X']);
        const h23a = await heldNoDelete(); go();
        check(h23a.held && await settled(['Xylo']), `${width}: read as Alpha, Xylo back meanwhile: held, then Xylo (held ${h23a.held}, ${await order()})`);
        go = await offList('c19', ['A'], ['X']);
        const h23 = await heldNoDelete(); go();
        check(h23.held && !h23.sent.length, `${width}: the same again, after a read that fitted: held, a × doesn't reach the node (held ${h23.held}, DELETE ${h23.sent})`);
        await settled(['Xylo']);
        check(JSON.stringify(await order()) === '["Xylo"]' && JSON.stringify(lists.c19) === '["X"]', `${width}: then the list shows Xylo, still on the node (${await order()}, server ${lists.c19})`);

        // 12j. Charlie shown; read as Alpha, Xylo before the answer (held, a changed answer), and Alpha
        // back before the read after it (the list as read, so the hold ends with Alpha); then read as
        // Alpha with Xylo before the answer again: the same answer as before, held too
        await goPl('c20');
        check(await settled(['Charlie']), `${width}: a one-song list shows (${await order()})`);
        go = await offList('c20', ['A'], ['X']);
        const h24a = await heldNoDelete(); go(['A']);
        check(h24a.held && await settled(['Alpha']), `${width}: changed and back meanwhile: held, then Alpha (held ${h24a.held}, ${await order()})`);
        go = await offList('c20', ['A'], ['X']);
        const h24 = await heldNoDelete(); go();
        check(h24.held && !h24.sent.length, `${width}: changed the same way again: held, a × doesn't reach the node (held ${h24.held}, DELETE ${h24.sent})`);
        await settled(['Xylo']);
        check(JSON.stringify(await order()) === '["Xylo"]' && JSON.stringify(lists.c20) === '["X"]', `${width}: then the list shows Xylo, still on the node (${await order()}, server ${lists.c20})`);

        // 12k. Xylo shown, its answer fitting; read as Alpha with Xylo back before the answer (the same
        // answer as before: held), Alpha again before the read after it (the list as read: the hold
        // ends with Alpha); then read as Alpha with Xylo back before the answer once more. The rows
        // show Alpha, the node has Xylo: held, a × doesn't reach the node (before: an answer the same
        // as one before for the same list read counted as the node naming songs otherwise, the rows
        // showed Alpha unheld and its × removed Xylo)
        await goPl('c21');
        check(await settled(['Xylo']), `${width}: a one-song list shows (${await order()})`);
        go = await offList('c21', ['A'], ['X']);
        const h25a = await heldNoDelete(); go(['A']);
        check(h25a.held && await settled(['Alpha']), `${width}: read as Alpha, Xylo back meanwhile, Alpha again: held, then Alpha (held ${h25a.held}, ${await order()})`);
        go = await offList('c21', ['A'], ['X']);
        const h25 = await heldNoDelete(); go();
        check(h25.held && !h25.sent.length, `${width}: then Xylo back before the answer once more: held, a × doesn't reach the node (held ${h25.held}, DELETE ${h25.sent})`);
        await settled(['Xylo']);
        check(JSON.stringify(await order()) === '["Xylo"]' && JSON.stringify(lists.c21) === '["X"]', `${width}: then the list shows Xylo, still on the node (${await order()}, server ${lists.c21})`);

        // 12l. A node naming the songs otherwise by place (the same other uris whatever the order),
        // opened twice unchanged, then another device reordering the list between the read and the
        // answer: the answer is the same as before and the list read the same, but the node has
        // another order: held, a × doesn't reach the node (before: no hold, × on row 0 removed Quebec)
        await goPl('c22');
        const settled3 = () => until(async () => (await order()).length === 3 && (await page.locator('#plsongs .srcchip.wait').count()) === 0 && !(await inertNow()), 4000);
        check(await settled3(), `${width}: under other names by place, the list shows (${await order()})`);
        const o26 = await order();
        await away(); await goPl('c22'); check(await settled(o26), `${width}: opened again unchanged, the list shows as before (${await order()})`);
        go = await offList('c22', ['P', 'Q', 'A'], ['Q', 'P', 'A']);
        const h26 = await heldNoDelete(); go();
        check(h26.held && !h26.sent.length, `${width}: under other names by place, reordered meanwhile: held, a × doesn't reach the node (held ${h26.held}, DELETE ${h26.sent})`);
        await until(async () => !(await inertNow()), 4000);
        check(!(await inertNow()) && JSON.stringify(lists.c22) === '["Q","P","A"]', `${width}: then the rows take presses, every song still on the node (server ${lists.c22})`);

        // 12c. Keyboard (desktop): × on Xylo, and another device turns the list round while the read
        // after it is on its way: the focus goes to the × of the song after Xylo, Bravo, where the
        // node now has it (before: the × at Xylo's old place, Charlie's, so a second Enter removed
        // Charlie)
        if (!mobile) {
          const rmFocus = () => page.evaluate(() => { const a = document.activeElement; return a && a.dataset && a.dataset.rm != null ? `${a.dataset.rm}|${a.getAttribute('aria-label')}` : String(a && a.tagName); });
          const inert = () => page.evaluate(() => { const b = document.querySelector('#plsongs'); return !b || b.inert; });
          await goPl('c13');
          await until(async () => JSON.stringify(await order()) === '["Alpha","Xylo","Bravo","Charlie"]' && (await page.locator('#plsongs .srcchip.wait').count()) === 0, 3000);
          const release16 = gate('c13', false, true); const del16 = deletes.length; const asked16 = asks.length;
          await page.focus('#plsongs [data-rm="1"]'); await page.keyboard.press('Enter');
          await until(async () => deletes.length > del16 && asks.slice(asked16).includes('c13'), 2000);
          lists.c13 = ['B', 'C', 'A'];
          release16();
          await until(async () => JSON.stringify(await order()) === '["Bravo","Charlie","Alpha"]' && !(await inert()), 3000);
          await sleep(300);
          const f16 = await rmFocus();
          check(JSON.stringify(deletes.slice(del16)) === '[1]' && /^0\|.*Bravo/.test(f16), `${width}: after × with the list moved meanwhile, the focus is on the next song's ×, Bravo's, in its new place (${f16})`);

          // 12d. The list read again under a hold (an answer for where the songs play naming another
          // song at a row's place, as in 12), the focus on Yankee's ×, and another device moving
          // Yankee before that read answers: the focus stays on Yankee's ×, in its new place
          // (before: on the × now at its old place, Quebec's)
          await goPl('c14');
          await until(async () => JSON.stringify(await order()) === '["Yankee","Papa","Quebec"]' && (await page.locator('#plsongs .srcchip:not(.wait)').count()) === 3, 3000);
          const releasePf17 = gate('c14/play-from', false, true); const asked17 = asks.length;
          lists.c14 = ['P', 'Y', 'Q'];
          socket.send(JSON.stringify({ type: 'library_changed' }));
          await until(async () => JSON.stringify(await order()) === '["Papa","Yankee","Quebec"]' && asks.slice(asked17).includes('c14/play-from'), 3000);
          await page.focus('#plsongs [data-rm="1"]');
          const f17a = await rmFocus();
          lists.c14 = ['P', 'Z', 'Q'];
          const releaseRead17 = gate('c14', false, true); const reads17 = asks.slice(asked17).filter((x) => x === 'c14').length;
          releasePf17();
          await until(async () => asks.slice(asked17).filter((x) => x === 'c14').length > reads17, 2000);
          const held17 = await inert();
          lists.c14 = ['Y', 'Q', 'P'];
          releaseRead17();
          await until(async () => JSON.stringify(await order()) === '["Yankee","Quebec","Papa"]' && !(await inert()), 3000);
          await sleep(300);
          const f17 = await rmFocus();
          check(/^1\|.*Yankee/.test(f17a) && held17 && /^0\|.*Yankee/.test(f17), `${width}: read again under a hold, the focus comes back to Yankee's ×, in its new place (${f17a} → ${f17}, held ${held17})`);
          await page.screenshot({ path: path.join(SHOTS, `focus-by-song-${width}.png`) });
        }

        // 13. The same other names, each answer slower than the 1.2 s wait: still asked at most twice
        // (before: each slow answer drew the page again, which asked again, for good)
        const asked14 = asks.length;
        await goPl('c11');
        await sleep(8000);
        const pf14 = asks.slice(asked14).filter((x) => x === 'c11/play-from').length;
        check(pf14 >= 1 && pf14 <= 2 && (await page.locator('#plsongs .srcchip.wait').count()) === 0, `${width}: slow answers under other names: asked at most twice, no row left waiting (${pf14} asked)`);
        // (a slow answer that doesn't fit holds the rows till the read after it: that hold ends too)
        check(!(await page.evaluate(() => { const b = document.querySelector('#plsongs'); return !b || b.inert; })), `${width}: slow answers under other names: the rows take presses again`);
        check(state.errors.length === errs && !state.errors.length, `${width}: no page errors (${state.errors.join(' | ')})`);
      } catch (e) {
        check(false, `${width}: ${e.message.split('\n')[0]}`);
        await page.screenshot({ path: path.join(SHOTS, `error-${width}.png`) }).catch(() => {});
      }
      await page.context().close();
    }
  } finally {
    await browser.close();
  }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
