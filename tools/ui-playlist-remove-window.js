// A playlist change by a song's place holds the list until it is drawn again. Every Remove, Move
// and pin on a playlist page goes by the song's place in the list. Before, after a Remove the old
// list stayed pressable until the redraw landed: x on Bravo in [Alpha, Bravo, Charlie, Delta], then
// quickly x on Charlie (place 2) deleted Delta, the server's place 2 by then. Two x presses before
// the first DELETE answered deleted two songs by old places too, and the Select bar's Remove gave
// the list back before its own redraw. Now the list (and the bar) take no presses from the press
// until a read made after the change is drawn; a failed redraw's Try again ends the hold, a change
// that never answers is cut after 30 s (the list stays held until the next library change, as the
// node may still make it, or 30 s more), and the keyboard focus comes back to its place.
// A read after a change that never answers ends in "Couldn't load this playlist" with Try again
// after 15 s (case 7); a keyboard Move whose library change redraws the list first keeps the focus
// on the moved song (case 8); and a playlist opened afresh doesn't take the focus back (case 9),
// after a failed read too (case 10). The Pubky user still to be asked doesn't hold the list either
// (case 11): a redraw meanwhile waits on the same Pubky question instead of asking again, and after
// 15 s with no answer the playlist goes on without it, its list drawn (held while its change is on
// its way), the late answer changing nothing; a library change while a Move is on its way keeps the list held
// (case 12); and the late answer of a read cut at 15 s, or of a slow read a newer one was drawn
// before, draws nothing (case 7).
// At 360 (phone) and 1280 px.
//   node tools/ui-playlist-remove-window.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-remove-window-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

const song = (id, title) => ({ ...fx.track, id, title, canonical_uri: `https://musicbrainz.org/recording/0000000${id}-0000-4000-8000-000000000000`, mbid: null });
const SONGS = { A: song(1, 'Alpha'), B: song(2, 'Bravo'), C: song(3, 'Charlie'), D: song(4, 'Delta') };
const NAMES = { c1: 'Road trip', c2: 'Bar remove', c3: 'Failed redraw', c4: 'Keyboard', c5: 'No answer', c6: 'Select kept', c7: 'Left meanwhile', c8: 'Hung read', c9: 'Move race', c10: 'Fresh open', c11: 'Cut, no change', c12: 'Failed, left', c13: 'Pubky slow', c14: 'Move, library change' };
const sum = (id, n) => ({ ...fx.summary, id, name: NAMES[id], items: n });
const detailOf = (id, keys) => ({ summary: sum(id, keys.length), playlist: { ...sum(id, keys.length), items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => SONGS[k]), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: [] });
const pfOf = (keys) => ({ order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: keys.map((k, i) => ({ index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', track_id: SONGS[k].id, plays_from: { service: 'library', label: 'Your library' }, sources: [] })) });
const json = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
const keyOf = (uri) => Object.keys(SONGS).find((k) => SONGS[k].canonical_uri === uri);

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, mobile] of [[360, true], [1280, false]]) {
      const { page, state } = await fx.create(browser, width, { mobile });
      await page.clock.install(); // (case 5 moves the clock on 30 s)
      // The server's lists; `hold[key]`: the next GET of that path (or 'del', the next DELETE's answer)
      // waits for its release
      const lists = { c1: ['A', 'B', 'C', 'D'], c2: ['A', 'B', 'C', 'D'], c3: ['A', 'B'], c4: ['A', 'B', 'C'], c5: ['A', 'B'], c6: ['A', 'B', 'C', 'D'], c7: ['A', 'B', 'C'], c8: ['A', 'B', 'C'], c9: ['A', 'B', 'C'], c10: ['A', 'B', 'C'], c11: ['A', 'B'], c12: ['A', 'B', 'C'], c13: ['A', 'B'], c14: ['A', 'B', 'C'] };
      const hold = {}; const writes = []; const gets = []; let mute = false; let failPut = false;
      // The Pubky user (GET /v1/pubky): none, signed in (pubkyAs, for Library › My music's "Save now"),
      // or (pubkyHang) no answer until pubkyFree(); a "Save now" that needs a new sign-in forgets
      // the user, so the next page asks again
      let pubkyHang = false; let pubkyAs = null; const pubkyWaits = [];
      const pubkyFree = () => { pubkyHang = false; pubkyWaits.splice(0).forEach((f) => f()); };
      await page.route('**/v1/pubky', (r) => (pubkyHang ? new Promise((res) => pubkyWaits.push(res)).then(() => json(r, { user: null }).catch(() => {})) : json(r, { user: pubkyAs, sync: { running: false } })));
      await page.route('**/v1/storage', (r) => json(r, { homeserver: { mode: 'none' }, saving: { state: 'saved' } }));
      await page.route('**/v1/pubky/sync', (r) => json(r, { pushed: 0, pulled: 0, pending: 0, errors: ['This node needs your approval for new folders on your homeserver. Log out and log in again with Pubky Ring to give it.'] }));
      // The node's event socket: a library change on demand
      let socket = null;
      await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
      const libraryChanged = () => socket && socket.send(JSON.stringify({ type: 'library_changed' }));
      // (`as`: the list that read answers with, as the server had it before)
      const gate = (key, fail, as) => { let open; const p = new Promise((res) => { open = res; }); hold[key] = { p, fail, as }; return open; };
      await page.route(/\/v1\/playlists\/(c\d+)(\/.*)?$/, async (r) => {
        const u = new URL(r.request().url()); const m = r.request().method();
        const [, , , id, ...rest] = u.pathname.split('/'); const sub = rest.join('/');
        if (m === 'DELETE' && sub.startsWith('items/')) {
          const k = Number(sub.split('/')[1]); writes.push(`${id} DELETE ${k}`);
          if (mute) return; // (never answers)
          lists[id].splice(k, 1);
          const h = hold.del; delete hold.del; if (h) await h.p;
          return json(r, {}).catch(() => {});
        }
        if (m === 'PUT' && !sub) { const b = JSON.parse(r.request().postData() || '{}'); writes.push(`${id} PUT ${b.uris.map(keyOf).join('')}`); if (failPut) return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'The node could not save it' }) }); const h = hold.put; delete hold.put; if (h) await h.p; lists[id] = b.uris.map(keyOf); return json(r, detailOf(id, lists[id])).catch(() => {}); }
        if (m !== 'GET') return r.fallback();
        const key = `${id}${sub ? '/' + sub : ''}`; gets.push(key);
        let then = [...lists[id]];
        const h = hold[key]; if (h && h.as) then = h.as; delete hold[key]; if (h) await h.p;
        if (h && h.fail) return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'The node did not answer' }) });
        if (sub === 'play-from') return json(r, pfOf(then));
        if (!sub) return json(r, detailOf(id, then)).catch(() => {}); // (a read cut at 15 s: the page no longer takes it)
        return r.fallback();
      });
      await page.route('**/v1/playlists', (r) => (r.request().method() === 'GET' ? json(r, Object.keys(NAMES).map((id) => sum(id, lists[id].length))) : r.fallback()));
      const order = () => page.evaluate(() => [...document.querySelectorAll('#plsongs .srow')].map((row) => { const n = row.querySelector('.name, .title, .t-title'); return (n ? n.textContent : row.textContent).trim().split(/\s/)[0]; }).filter(Boolean).join(','));
      const inert = () => page.evaluate(() => !!document.querySelector('#plsongs') && document.querySelector('#plsongs').inert);
      // A row's x by its song (a phone hides it: the row's ⋯ menu presses it)
      const rmOf = (name) => page.evaluate((n) => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => (x.getAttribute('aria-label') || '').includes(n)); return b ? b.dataset.rm : null; }, name);
      const pressRm = (name) => page.evaluate((n) => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => (x.getAttribute('aria-label') || '').includes(n)); if (b) b.click(); }, name);
      // A real tap or click where a row's x (desktop) or ⋯ (phone) is
      const tapRow = async (name) => {
        const box = await page.evaluate(([n, phone]) => { const row = [...document.querySelectorAll('#plsongs .srow')].find((x) => x.textContent.includes(n)); const b = row && (phone ? row.querySelector('[data-smore]') : row.querySelector('[data-rm]')); if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; }, [name, mobile]);
        if (box) await page.mouse.click(box.x, box.y);
        return !!box;
      };
      const open = async (id, want) => {
        await page.evaluate((x) => { const st = { v: 'playlist', arg: x }; history.pushState(st, '', `/playlist/${x}`); dispatchEvent(new PopStateEvent('popstate', { state: st })); }, id);
        await until(async () => (await order()) === want && !!(await page.$('#plsongs .srcchip:not(.wait), #plsongs [data-psrc]')), 4000);
      };
      try {
        await fx.go(page, '/playlist/c1');
        await until(async () => (await order()) === 'Alpha,Bravo,Charlie,Delta', 4000);
        await sleep(300);
        check((await order()) === 'Alpha,Bravo,Charlie,Delta', `${width}: the playlist shows as the server has it (${await order()})`);

        // 1. x on Bravo, then x on Charlie before the list is drawn again
        let release = gate('c1');
        await pressRm('Bravo');
        await until(async () => writes.length === 1, 2000);
        check(writes.join() === 'c1 DELETE 1', `${width}: x on Bravo deletes place 1 (${writes})`);
        check(await inert(), `${width}: the list takes no presses until it is drawn again`);
        const tapped = await tapRow('Charlie');
        await pressRm('Charlie'); // (the ⋯ menu's Remove presses the row's x)
        await sleep(400);
        check(tapped && writes.join() === 'c1 DELETE 1', `${width}: a tap and a press on Charlie's x meanwhile delete nothing (${writes})`);
        await page.screenshot({ path: path.join(SHOTS, `held-${width}.png`) });
        release();
        await until(async () => (await order()) === 'Alpha,Charlie,Delta' && !(await inert()), 3000);
        check((await order()) === 'Alpha,Charlie,Delta' && !(await inert()), `${width}: the new list shows and takes presses again (${await order()})`);
        check((await rmOf('Charlie')) === '1', `${width}: Charlie's x is for place 1 now (${await rmOf('Charlie')})`);
        await pressRm('Charlie');
        await until(async () => writes.length === 2, 2000);
        await until(async () => (await order()) === 'Alpha,Delta', 3000);
        check(writes[1] === 'c1 DELETE 1' && lists.c1.join() === 'A,D', `${width}: x on Charlie then deletes Charlie (${writes[1]}, server ${lists.c1})`);

        // 2. Two presses before the first DELETE answers
        release = gate('del');
        await pressRm('Alpha');
        await until(async () => writes.length === 3, 2000);
        await pressRm('Delta');
        await sleep(400);
        check(writes.slice(2).join() === 'c1 DELETE 0', `${width}: a second x before the first answers sends nothing (${writes.slice(2)})`);
        release();
        await until(async () => (await order()) === 'Delta' && !(await inert()), 3000);
        check((await order()) === 'Delta' && lists.c1.join() === 'D', `${width}: only Alpha went (${await order()}, server ${lists.c1})`);

        // 3. The Select bar's Remove holds the list until its own redraw lands
        await open('c2', 'Alpha,Bravo,Charlie,Delta');
        await page.evaluate(() => document.querySelector('#plsongs [data-selstart]').click());
        await page.evaluate(() => document.querySelector('#plsongs .selbox input[data-k="1"]').click());
        release = gate('c2');
        const w3 = writes.length;
        await page.evaluate(() => document.querySelector('.selbar [data-sel="remove"]').click());
        await until(async () => writes.length === w3 + 1, 2000);
        await sleep(300);
        check(writes.slice(w3).join() === 'c2 DELETE 1', `${width}: the bar's Remove deletes Bravo (${writes.slice(w3)})`);
        check(await inert(), `${width}: after the bar's Remove the list still takes no presses until it is drawn again`);
        await pressRm('Charlie');
        await sleep(300);
        check(writes.length === w3 + 1, `${width}: x on Charlie meanwhile deletes nothing (${writes.slice(w3)})`);
        release();
        await until(async () => (await order()) === 'Alpha,Charlie,Delta' && !(await inert()), 3000);
        check((await order()) === 'Alpha,Charlie,Delta' && !(await inert()) && !(await page.$('.selbar')), `${width}: the new list shows, takes presses, Select ended (${await order()})`);

        // 3b. Select on, a row's x: Select carries over to the new list and its bar takes presses
        await open('c6', 'Alpha,Bravo,Charlie,Delta');
        await page.evaluate(() => document.querySelector('#plsongs [data-selstart]').click());
        await page.evaluate(() => document.querySelector('#plsongs .selbox input[data-k="3"]').click());
        release = gate('c6');
        const w3b = writes.length;
        await pressRm('Alpha');
        await until(async () => writes.length === w3b + 1, 2000);
        await sleep(200);
        const heldBar = await page.evaluate(() => [...document.querySelectorAll('.selbar button')].every((b) => b.disabled));
        check(writes.slice(w3b).join() === 'c6 DELETE 0' && heldBar, `${width}: under Select, x on Alpha deletes it and the bar waits too (${writes.slice(w3b)}, bar disabled ${heldBar})`);
        release();
        await until(async () => (await order()) === 'Bravo,Charlie,Delta' && !(await inert()), 3000);
        await sleep(300);
        const bar = await page.evaluate(() => ({ count: (document.querySelector('.selbar .selcount') || {}).textContent, off: [...document.querySelectorAll('.selbar button')].filter((b) => b.disabled).map((b) => b.dataset.sel) }));
        check(bar.count === '1 song selected' && !bar.off.length, `${width}: the new list keeps the tick and its bar takes presses ("${bar.count}", disabled: ${bar.off})`);
        await page.evaluate(() => document.querySelector('.selbar [data-sel="cancel"]').click());
        check(!(await page.$('.selbar')), `${width}: and Cancel ends Select`);

        // 3c. The bar's Remove, then another playlist opened before its redraw lands: back on it, the
        // list takes presses again
        await open('c7', 'Alpha,Bravo,Charlie');
        await page.evaluate(() => document.querySelector('#plsongs [data-selstart]').click());
        await page.evaluate(() => document.querySelector('#plsongs .selbox input[data-k="0"]').click());
        const releaseC7 = gate('c7');
        const w3c = writes.length;
        await page.evaluate(() => document.querySelector('.selbar [data-sel="remove"]').click());
        await until(async () => writes.length === w3c + 1, 2000);
        await sleep(200);
        await open('c3', 'Alpha,Bravo');
        releaseC7(); await sleep(300);
        await open('c7', 'Bravo,Charlie');
        await sleep(300);
        check((await order()) === 'Bravo,Charlie' && !(await inert()), `${width}: back on it the list shows and takes presses (${await order()}, inert ${await inert()})`);
        await pressRm('Charlie');
        await until(async () => writes.length === w3c + 2, 2000);
        check(writes.slice(w3c).join() === 'c7 DELETE 0,c7 DELETE 1', `${width}: and x on Charlie deletes Charlie (${writes.slice(w3c)})`);
        await until(async () => (await order()) === 'Bravo', 3000);

        // 4. A redraw that fails: Try again draws the list, and it takes presses
        await open('c3', 'Alpha,Bravo');
        gate('c3', true)();
        const w4 = writes.length;
        await pressRm('Alpha');
        await until(async () => !!(await page.$('#navretry')), 3000);
        check(writes.slice(w4).join() === 'c3 DELETE 0' && !!(await page.$('#navretry')), `${width}: a failed redraw says so with Try again (${writes.slice(w4)})`);
        await page.click('#navretry');
        await until(async () => (await order()) === 'Bravo' && !(await inert()), 3000);
        check((await order()) === 'Bravo' && !(await inert()), `${width}: Try again draws the list and it takes presses (${await order()})`);
        await pressRm('Bravo');
        await until(async () => writes.length === w4 + 2, 2000);
        check(writes.slice(w4).join() === 'c3 DELETE 0,c3 DELETE 0', `${width}: and x works there (${writes.slice(w4)})`);

        // 5. A DELETE that never answers is cut after 30 s and the list read again
        await open('c5', 'Alpha,Bravo');
        mute = true; const w5 = writes.length;
        await pressRm('Alpha');
        await until(async () => writes.length === w5 + 1, 2000);
        await sleep(200);
        check(await inert(), `${width}: held while the DELETE goes unanswered`);
        await page.clock.fastForward(31000);
        mute = false;
        await until(async () => /didn.t answer in time/.test(await page.evaluate(() => [...document.querySelectorAll('.toast')].map((t) => t.textContent).join(' | '))), 3000);
        const toastText = await page.evaluate(() => [...document.querySelectorAll('.toast')].map((t) => t.textContent).join(' | '));
        check(/didn.t answer in time/.test(toastText), `${width}: after 30 s it is cut and says why ("${toastText}")`);
        await sleep(400);
        check(await inert() && writes.length === w5 + 1, `${width}: the list stays held after the cut (the node may still remove it; inert ${await inert()})`);
        await pressRm('Bravo'); await sleep(200);
        check(writes.length === w5 + 1, `${width}: …so x on Bravo meanwhile deletes nothing (${writes.slice(w5)})`);
        lists.c5.splice(0, 1); libraryChanged(); // (the node made the change after all)
        await until(async () => (await order()) === 'Bravo' && !(await inert()), 3000);
        check((await order()) === 'Bravo' && !(await inert()), `${width}: the library change reads it again and the list takes presses (${await order()})`);

        // 5b. Cut, and no library change: 30 s more and the list is read again
        await open('c11', 'Alpha,Bravo');
        mute = true; const w5b = writes.length;
        await pressRm('Alpha');
        await until(async () => writes.length === w5b + 1, 2000);
        await page.clock.fastForward(31000);
        mute = false; await sleep(400);
        check(await inert(), `${width}: cut and no library change: still held`);
        await page.clock.fastForward(31000);
        await until(async () => !(await inert()), 3000);
        check(!(await inert()) && (await order()) === 'Alpha,Bravo', `${width}: 30 s later it is read again and takes presses (${await order()})`);

        // 7. The read after a change never answers: Try again within 15 s, and the list works after it
        await open('c8', 'Alpha,Bravo,Charlie');
        const releaseHung = gate('c8', false, ['A', 'B', 'C']); // (released only after Try again: a late answer with Alpha still in)
        const w7 = writes.length;
        if (mobile) await pressRm('Alpha'); else { await page.focus('#plsongs [data-rm="0"]'); await page.keyboard.press('Enter'); }
        await until(async () => writes.length === w7 + 1, 2000);
        await sleep(300);
        check(await inert(), `${width}: held while the read after the change goes unanswered`);
        await page.clock.fastForward(16000);
        await until(async () => !!(await page.$('#navretry')), 3000);
        const notice = await page.evaluate(() => { const n = document.querySelector('#main .notice'); return n ? n.textContent.trim() : ''; });
        check(/Couldn.t load this playlist/.test(notice) && !!(await page.$('#navretry')), `${width}: after 15 s it says so with Try again ("${notice}")`);
        await page.screenshot({ path: path.join(SHOTS, `hung-read-${width}.png`) });
        if (mobile) await page.click('#navretry'); else { await page.focus('#navretry'); await page.keyboard.press('Enter'); }
        await until(async () => (await order()) === 'Bravo,Charlie' && !(await inert()), 3000);
        check((await order()) === 'Bravo,Charlie' && !(await inert()), `${width}: Try again draws the list and it takes presses (${await order()})`);
        if (!mobile) { await sleep(200); const f7 = await page.evaluate(() => { const a = document.activeElement; return a && a.dataset && a.dataset.rm != null ? `${a.dataset.rm}|${a.getAttribute('aria-label')}` : String(a && a.tagName); }); check(/^0\|.*Bravo/.test(f7), `${width}: after Try again from the keyboard the focus is on the next song's x (${f7})`); }
        releaseHung(); await sleep(400); // (the read cut at 15 s answers now, with the list as it was then)
        check((await order()) === 'Bravo,Charlie' && !(await inert()), `${width}: the cut read's late answer draws nothing (${await order()})`);
        await pressRm('Charlie');
        await until(async () => writes.length === w7 + 2, 2000);
        check(writes.slice(w7).join() === 'c8 DELETE 0,c8 DELETE 1', `${width}: and x on Charlie deletes Charlie (${writes.slice(w7)})`);
        await until(async () => (await order()) === 'Bravo', 3000);
        // A slow read that isn't cut, answering after a newer read was drawn: it draws nothing
        const releaseSlow = gate('c8', false, ['A', 'B', 'C']); const g7 = gets.length;
        libraryChanged(); await until(async () => gets.slice(g7).includes('c8'), 2000);
        libraryChanged(); await until(async () => gets.slice(g7).filter((g) => g === 'c8').length === 2, 2000); await sleep(400);
        check((await order()) === 'Bravo' && gets.slice(g7).filter((g) => g === 'c8').length === 2, `${width}: a newer read, after two library changes, is drawn first (${await order()})`);
        releaseSlow(); await sleep(400);
        check((await order()) === 'Bravo' && !(await inert()), `${width}: the slower read's late answer draws nothing (${await order()})`);

        // 11. A read under a hold, with the Pubky user still to be asked (a "Save now" that needs a
        // new sign-in forgot it) and no answer for it: after 15 s the list is drawn without it
        await open('c13', 'Alpha,Bravo');
        const releaseDel11 = gate('del'); const w11 = writes.length;
        await pressRm('Alpha');
        await until(async () => writes.length === w11 + 1, 2000);
        pubkyAs = 'fixtureuser';
        await page.evaluate(() => { const st = { v: 'library', arg: 'mine' }; history.pushState(st, '', '/library'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await until(async () => !!(await page.$('#sync')), 4000);
        const hadSync = !!(await page.$('#sync'));
        pubkyAs = null; pubkyHang = true;
        if (hadSync) await page.click('#sync');
        await until(async () => /needs your approval/.test(await page.evaluate(() => [...document.querySelectorAll('.toast')].map((t) => t.textContent).join(' | '))), 3000);
        await page.evaluate(() => { const st = { v: 'playlist', arg: 'c13' }; history.pushState(st, '', '/playlist/c13'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await until(async () => pubkyWaits.length > 0, 3000);
        check(hadSync && pubkyWaits.length > 0 && (await order()) === '', `${width}: Save now forgot the Pubky user and the playlist asks for it again (${pubkyWaits.length} waiting)`);
        // The page drawn again while that question is unanswered (as Try again does): it waits on
        // the same question, it doesn't ask a second time
        // (3 s on, so the question's 15 s end well before the redraw's own 15 s for its read)
        await page.clock.fastForward(3000);
        const asked11 = pubkyWaits.length; const g11 = gets.length;
        libraryChanged(); await sleep(400);
        check(pubkyWaits.length === asked11 && (await order()) === '', `${width}: drawn again meanwhile, the playlist asks for the Pubky user no second time (${asked11} then ${pubkyWaits.length} waiting, ${gets.length - g11} playlist reads)`);
        await page.clock.fastForward(12500);
        await until(async () => (await order()) === 'Bravo', 3000);
        const notice11 = await page.evaluate(() => { const n = document.querySelector('#main .notice'); return n ? n.textContent.trim() : ''; });
        check((await order()) === 'Bravo' && (await inert()) && !notice11, `${width}: no Pubky answer in 15 s: the list is drawn without it, still held while the DELETE is on its way (${await order()}, inert ${await inert()}, notice "${notice11}")`);
        // The Pubky answer coming late changes nothing
        pubkyFree(); await sleep(400);
        check((await order()) === 'Bravo' && (await inert()) && !(await page.$('#navretry')), `${width}: the Pubky user's late answer changes nothing (rows "${await order()}")`);
        releaseDel11();
        await until(async () => !(await inert()), 3000);
        check((await order()) === 'Bravo' && !(await inert()), `${width}: the DELETE's answer ends the hold and the list takes presses (${await order()})`);

        // 12. A Move whose PUT is still on its way, and a library change meanwhile: the list stays
        // held until the PUT's answer, then shows the move
        await open('c14', 'Alpha,Bravo,Charlie');
        const releasePut = gate('put'); const w12 = writes.length; const g12 = gets.length;
        await page.evaluate(() => document.querySelector('#plsongs [data-pdown="0"]').click());
        await until(async () => writes.length === w12 + 1, 2000);
        libraryChanged();
        await until(async () => gets.slice(g12).includes('c14'), 2000); await sleep(300);
        check(await inert() && (await order()) === 'Alpha,Bravo,Charlie', `${width}: a library change while the Move is on its way: the list is drawn again and still held (${await order()}, inert ${await inert()})`);
        await pressRm('Bravo'); await sleep(200);
        check(writes.slice(w12).join() === 'c14 PUT BAC', `${width}: …so x on Bravo meanwhile deletes nothing (${writes.slice(w12)})`);
        releasePut();
        await until(async () => (await order()) === 'Bravo,Alpha,Charlie' && !(await inert()), 3000);
        check((await order()) === 'Bravo,Alpha,Charlie' && !(await inert()), `${width}: the PUT's answer ends the hold with the move shown (${await order()})`);

        // 6. Keyboard (desktop): the focus comes back to the same place; a second Move meanwhile is held
        if (!mobile) {
          await open('c4', 'Alpha,Bravo,Charlie');
          await page.focus('#plsongs [data-rm="1"]');
          await page.keyboard.press('Enter');
          await until(async () => (await order()) === 'Alpha,Charlie' && !(await inert()), 3000);
          await sleep(200);
          const f = await page.evaluate(() => { const a = document.activeElement; return a && a.dataset ? `${a.dataset.rm}|${a.getAttribute('aria-label')}` : String(a && a.tagName); });
          check(/^1\|.*Charlie/.test(f), `${width}: after x from the keyboard the focus is on the next song's x (${f})`);
          release = gate('c4');
          const w6 = writes.length;
          await page.evaluate(() => document.querySelector('#plsongs [data-pdown="0"]').click());
          await until(async () => writes.length === w6 + 1, 2000);
          await page.evaluate(() => { const b = document.querySelector('#plsongs [data-pdown="0"]'); if (b) b.click(); });
          await sleep(300);
          check(writes.slice(w6).join() === 'c4 PUT CA', `${width}: a second Move before the list is drawn again sends nothing (${writes.slice(w6)})`);
          release();
          await until(async () => (await order()) === 'Charlie,Alpha' && !(await inert()), 3000);
          check((await order()) === 'Charlie,Alpha', `${width}: the moved list shows (${await order()})`);

          // 8. Keyboard Move, and its library change redraws the list while the Move's own redraw is
          // on its way: the focus stays with the moved song
          await open('c9', 'Alpha,Bravo,Charlie');
          const releaseOwn = gate('c9'); const g8 = gets.length; const w8 = writes.length;
          await page.focus('#plsongs [data-pdown="0"]');
          await page.keyboard.press('Enter');
          await until(async () => writes.length === w8 + 1 && gets.slice(g8).includes('c9'), 2000);
          const releaseLib = gate('c9'); libraryChanged(); // (the PUT's library change)
          await until(async () => gets.slice(g8).filter((k) => k === 'c9').length === 2, 2000);
          releaseOwn(); await sleep(300); // (the Move's own redraw lands first, behind the newer one)
          releaseLib();
          await until(async () => (await order()) === 'Bravo,Alpha,Charlie' && !(await inert()), 3000);
          await sleep(300);
          const f8 = await page.evaluate(() => { const a = document.activeElement; return a && a.closest('#plsongs') ? `${a.dataset.pdown ?? a.dataset.pup}|${a.getAttribute('aria-label')}` : String(a && a.tagName); });
          check(writes.slice(w8).join() === 'c9 PUT BAC' && /^1\|.*Alpha/.test(f8), `${width}: after Move down the focus is on Alpha's Move down, in its new place (${f8})`);

          // 8c. A Move the node refuses: the list stays, the focus stays on that song's Move down
          await open('c4', 'Charlie,Alpha');
          failPut = true; const w8c = writes.length;
          await page.focus('#plsongs [data-pdown="0"]');
          await page.keyboard.press('Enter');
          await until(async () => writes.length === w8c + 1, 2000);
          await until(async () => !(await inert()), 3000); await sleep(300);
          failPut = false;
          const f8c = await page.evaluate(() => { const a = document.activeElement; return a && a.closest('#plsongs') ? `${a.dataset.pdown ?? a.dataset.pup}|${a.getAttribute('aria-label')}` : String(a && a.tagName); });
          check((await order()) === 'Charlie,Alpha' && /^0\|.*Charlie/.test(f8c), `${width}: a refused Move leaves the list and the focus on Charlie's Move down (${await order()}, ${f8c})`);

          // 9. x from the keyboard, another playlist opened before the redraw, and this one opened
          // again: a fresh page, the focus not pulled into its list
          await open('c10', 'Alpha,Bravo,Charlie');
          const release9 = gate('c10'); const w9 = writes.length;
          await page.focus('#plsongs [data-rm="0"]');
          await page.keyboard.press('Enter');
          await until(async () => writes.length === w9 + 1, 2000);
          await sleep(200);
          await open('c4', 'Charlie,Alpha');
          release9(); await sleep(300);
          await open('c10', 'Bravo,Charlie');
          await sleep(300);
          const f9 = await page.evaluate(() => { const a = document.activeElement; return a && a.closest('#plsongs') ? `in the list: ${a.getAttribute('aria-label')}` : 'not in the list'; });
          check((await order()) === 'Bravo,Charlie' && !(await inert()) && f9 === 'not in the list', `${width}: opened again the list takes presses and the focus isn't pulled into it (${f9})`);

          // 10. x from the keyboard, its read cut at 15 s, Try again from the keyboard failing too,
          // another playlist opened instead, and this one opened again: a fresh page, the focus
          // not pulled into its list (Try again's own draw is in place only on its visit)
          await open('c12', 'Alpha,Bravo,Charlie');
          gate('c12'); const w10 = writes.length;
          await page.focus('#plsongs [data-rm="0"]');
          await page.keyboard.press('Enter');
          await until(async () => writes.length === w10 + 1, 2000);
          await page.clock.fastForward(16000);
          await until(async () => !!(await page.$('#navretry')), 3000);
          check(!!(await page.$('#navretry')), `${width}: its read cut at 15 s says so with Try again`);
          const g10 = gets.length; gate('c12', true)(); await page.focus('#navretry'); await page.keyboard.press('Enter');
          await until(async () => gets.slice(g10).includes('c12') && !!(await page.$('#navretry')), 3000);
          check(gets.slice(g10).includes('c12') && !!(await page.$('#navretry')), `${width}: Try again's read fails too and says so again`);
          await open('c4', 'Charlie,Alpha');
          await open('c12', 'Bravo,Charlie');
          await sleep(300);
          const f10 = await page.evaluate(() => { const a = document.activeElement; return a && a.closest('#plsongs') ? `in the list: ${a.getAttribute('aria-label')}` : 'not in the list'; });
          check((await order()) === 'Bravo,Charlie' && !(await inert()) && f10 === 'not in the list', `${width}: after a failed read, opened again the focus isn't pulled into its list (${f10})`);
        }
        await page.screenshot({ path: path.join(SHOTS, `after-${width}.png`) });
        check(!state.errors.length, `${width}: no page errors (${state.errors.join(' | ')})`);
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
