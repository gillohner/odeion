// A playlist's late draw. A draw of a playlist reads the playlist, then waits up to 1.2 s for where
// its songs play. Before, only the view was checked afterwards: an older draw that answered after a
// newer one (a library change redrew the page, or another device reordered the list) drew the old
// order over the new one, and every Remove on that page goes by a song's place in the list, so the
// × of one song deleted another. And a playlist opened while the previous one waited could get the
// previous one drawn under its address. Now only the latest draw of the playlist at the address
// draws anything (its error too). A list another device changed draws at once, its sources
// following, whatever the age of the answer kept from before and when no answer came yet; a source
// filter doesn't hide the rows meanwhile, and a song listed twice keeps each copy's own name. At 360
// (phone) and 1280 px.
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
const STREAMED = { P: { title: 'Papa', canonical_uri: 'https://tidal.com/browse/track/101' }, Q: { title: 'Quebec', canonical_uri: 'https://tidal.com/browse/track/102' } };
Object.assign(SONGS, STREAMED);
const sum = (id, name, n) => ({ ...fx.summary, id, name, items: n });
const detailOf = (id, name, keys) => ({ summary: sum(id, name, keys.length), playlist: { ...sum(id, name, keys.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => (STREAMED[k] ? null : SONGS[k])), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: [] });
const pfOf = (keys) => ({ order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: keys.map((k, i) => (STREAMED[k] ? { index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', track_id: null, stream_uri: `tidal:track:${SONGS[k].canonical_uri.split('/').pop()}`, page: SONGS[k].canonical_uri, plays_from: { service: 'tidal', label: 'TIDAL' }, sources: [], connect: [] } : { index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', track_id: SONGS[k].id, plays_from: { service: 'library', label: 'Your library' }, sources: [] })) });
// (`retitle`: a playlist's answer naming the song at a place otherwise, as a pin to another source can)
const retitle = { c6: { 2: 'Pinned' } };
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, mobile] of [[360, true], [1280, false]]) {
      const { page, state } = await fx.create(browser, width, { mobile });
      // The server's lists, each GET answered with the list as it was when asked; `hold`: the next
      // GET of that path waits for its release
      const lists = { c1: ['A', 'X', 'B', 'A'], c2: ['C'], c3: ['B', 'X'], c4: ['P', 'Q', 'A'], c5: ['A', 'X', 'B'], c6: ['P', 'Q', 'P'] };
      const names = { c1: 'Road trip', c2: 'Quiet hours', c3: 'Long drive', c4: 'Streamed', c5: 'Slow sources', c6: 'Twice' };
      const hold = {}; const deletes = []; const asks = [];
      const gate = (key, fail) => { let open; const p = new Promise((res) => { open = res; }); hold[key] = { p, fail }; return open; };
      let socket = null;
      await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
      await page.route(/\/v1\/playlists\/(c1|c2|c3|c4|c5|c6)(\/.*)?$/, async (r) => {
        const u = new URL(r.request().url()); const m = r.request().method();
        const [, , , id, ...rest] = u.pathname.split('/'); const sub = rest.join('/');
        if (m === 'DELETE' && sub.startsWith('items/')) { const k = Number(sub.split('/')[1]); deletes.push(k); lists[id].splice(k, 1); return json(r, {}); }
        if (m !== 'GET') return r.fallback();
        const key = `${id}${sub ? '/' + sub : ''}`; asks.push(key);
        const then = [...lists[id]];
        const h = hold[key]; delete hold[key]; if (h) await h.p;
        if (h && h.fail) return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'The node did not answer' }) });
        if (sub === 'play-from') { const a = pfOf(then); Object.entries(retitle[id] || {}).forEach(([k, t]) => { if (a.items[k]) a.items[k].title = t; }); return json(r, a); }
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
