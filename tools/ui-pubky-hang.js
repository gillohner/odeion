// A page or a ⋯ menu while the node doesn't answer whether it holds a Pubky session. An album page
// and each menu ask that first (a menu offers "Copy to my homeserver" then). Before, a GET /pubky that
// never answered left the page, and every menu opened meanwhile, waiting for good: the 15 s limit only
// let the next caller ask again. Now
// after 15 s every page and menu waiting on the question goes on (without the homeserver copy, nothing kept),
// the next one asks again, and only the latest question's answer counts: one that answers late
// doesn't undo a newer one; a playlist's Share with no answer says it couldn't check rather than
// that you aren't signed in. A playlist another device changes meanwhile shows its new order at once
// rather than waiting on the question again; Home's Get started leaves out the Pubky sign-in step
// rather than offering it as if signed out; and an older question's late answer counts while no
// newer one has answered (a node that always answers after 15 s), and draws a playlist drawn
// without it again. At 360 (phone) and 1280 px.
//   node tools/ui-pubky-hang.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'pubky-hang-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 3000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };
const MENU = '.popover.menu:not([hidden])';
const COPY = 'Copy to my homeserver';
// An album of one library song (as the node sends its page, item.rs)
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' },
  songs: [{ no: 1, title: fx.track.title, artist: 'Sefa', artist_id: 'lib-11', album_id: 'lib-7', duration_ms: 240000, uri: null, track_id: 1, id: `mb-${fx.track.mbid}`, page: null }], total_ms: 240000, albums: [], versions: [], also_on: [], about: [], open_in: [] };
// (a matched song: only those can be copied to a homeserver)
const matched = { ...fx.track, canonical: [{ kind: 'recording', mbid: fx.track.mbid }] };
const fixtures = { '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
  // (Home's Get started: Play and the Pubky sign-in left, no homeserver on this node)
  '/me/first-steps': { body: { show: true, hidden: false, steps: [{ id: 'play', done: false, optional: false }, { id: 'pubky', done: false, optional: true }] } }, '/homeserver/bundled': { body: { configured: false, reachable: false } } };
// A playlist of two library songs, its order as the server has it when asked (`lists`)
const SONGS = { J: matched, B: { ...fx.track, id: 2, title: 'Bravo', canonical_uri: 'https://musicbrainz.org/recording/00000002-0000-4000-8000-000000000000', mbid: null } };
const plOf = (keys) => ({ summary: { ...fx.summary, items: keys.length }, playlist: { ...fx.summary, items: keys.map((k) => ({ uri: SONGS[k].canonical_uri })) }, tracks: keys.map((k) => SONGS[k]), entries: keys.map((k) => ({ kind: 'track', uri: SONGS[k].canonical_uri })), local_tracks: [] });
const pfOf = (keys) => ({ order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: keys.map((k, i) => ({ index: i, uri: SONGS[k].canonical_uri, title: SONGS[k].title, artist: 'Sefa', track_id: SONGS[k].id, plays_from: { service: 'library', label: 'Your library' }, sources: [] })) });

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, mobile] of [[360, true], [1280, false]]) {
      const { page, state } = await fx.create(browser, width, { mobile, fixtures, tracks: [matched] });
      await page.clock.install();
      // GET /pubky: the first two never answer until `late()` (then: no session); later ones answer
      // at once with a session (with `hangAll`: none answers until `late()` or `answer(n, body)`)
      const asks = []; const held = new Map(); let hangAll = false;
      const SESSION = { user: 'pk:fixture', sync: { running: false } };
      await page.route('**/v1/pubky', (r) => {
        const n = asks.push(Date.now()) - 1;
        if (n < 2 || hangAll) return new Promise((res) => { held.set(n, res); }).then((body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body || { user: null }) }).catch(() => {}));
        return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(SESSION) });
      });
      const late = () => { const all = [...held.values()]; held.clear(); all.forEach((f) => f()); };
      const answer = (n, body) => { const f = held.get(n); held.delete(n); if (f) f(body); };
      // The playlist, and the node's events (another device's change arrives as library_changed)
      let lists = ['J', 'B']; const plReads = []; const deletes = [];
      await page.route(/\/v1\/playlists\/c1(\/.*)?$/, (r) => {
        const m = r.request().method(); const sub = new URL(r.request().url()).pathname.split('/').slice(4).join('/');
        if (m === 'DELETE' && sub.startsWith('items/')) { const k = Number(sub.split('/')[1]); deletes.push(k); lists.splice(k, 1); return r.fulfill({ status: 200, contentType: 'application/json', body: '{}' }); }
        if (m !== 'GET') return r.fallback();
        if (!sub) plReads.push(Date.now());
        return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(sub === 'play-from' ? pfOf(lists) : plOf(lists)) });
      });
      let socket = null;
      await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
      const order = () => page.evaluate(() => [...document.querySelectorAll('#plsongs .track:not(.skeleton), #plsongs .list-row')].map((row) => { const n = row.querySelector('.name, .title, .t-title'); return (n ? n.textContent : row.textContent).trim().split(/\s/)[0]; }).filter(Boolean));
      const menuOpen = () => page.locator(MENU).count().then((n) => n > 0);
      const offersCopy = () => page.locator(MENU).getByText(COPY, { exact: false }).count().then((n) => n > 0);
      const closeMenu = async () => { await page.keyboard.press('Escape'); await until(async () => !(await menuOpen()), 2000); };
      const ROW_MORE = '#tracklist .track:first-child [data-act="more"]';
      try {
        // 1. An album page asks first too: no answer, it draws after 15 s (before: never)
        await fx.go(page, '/album/lib-7');
        await until(async () => asks.length > 0, 3000); await sleep(400);
        check(asks.length === 1 && !(await page.$('#isongs')), `${width}: the album page asks the node and waits (${asks.length} asked)`);
        await page.clock.fastForward(16000);
        const drawn = await until(async () => !!(await page.$('#isongs [data-smore="0"]')), 3000);
        check(drawn, `${width}: no answer in 15 s: the album page draws anyway`);
        // 2. On Songs, a row's ⋯ asks again (nothing was kept), waits, then opens after 15 s
        await page.evaluate(() => { const st = { v: 'tracks' }; history.pushState(st, '', '/tracks'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await page.waitForSelector('#tracklist .track');
        await page.click(ROW_MORE, { force: true });
        await until(async () => asks.length > 1, 2000); await sleep(400);
        check(asks.length === 2 && !(await menuOpen()), `${width}: the row's ⋯ asks again and waits (${asks.length} asked)`);
        await page.clock.fastForward(16000);
        const opened = await until(menuOpen, 3000);
        check(opened, `${width}: no answer in 15 s: the ⋯ menu opens anyway`);
        check(opened && !(await offersCopy()), `${width}: without an answer it offers no homeserver copy`);
        await page.screenshot({ path: path.join(SHOTS, `menu-after-wait-${width}.png`) });
        if (opened) await closeMenu();
        // 3. The next ⋯ asks again and gets its answer at once
        await page.click(ROW_MORE, { force: true });
        const again = await until(async () => (await menuOpen()) && (await offersCopy()), 3000);
        check(asks.length === 3 && again, `${width}: the next ⋯ asks again and offers the homeserver copy (${asks.length} asked)`);
        if (await menuOpen()) await closeMenu();
        // 4. The two earlier questions answering late (no session) don't undo the newer answer
        late(); await sleep(400);
        await page.click(ROW_MORE, { force: true });
        await until(menuOpen, 3000); await sleep(200);
        check((await offersCopy()) && asks.length === 3, `${width}: the late earlier answers leave the newer one (${asks.length} asked, copy offered ${await offersCopy()})`);
        await page.screenshot({ path: path.join(SHOTS, `menu-after-late-${width}.png`) });
        if (await menuOpen()) await closeMenu();
        // 5. A playlist (opened afresh, the node never answering now): it draws after 15 s, and its
        // Share then doesn't say you aren't signed in, as nobody knows: it says it couldn't check
        hangAll = true; const a5 = asks.length;
        await fx.go(page, '/playlist/c1');
        await until(async () => asks.length > a5, 3000); await sleep(300);
        await page.clock.fastForward(16000);
        check(await until(async () => !!(await page.$('#plshare')), 3000), `${width}: no answer in 15 s: the playlist draws anyway`);
        const a5b = asks.length;
        await page.click('#plshare');
        await until(async () => asks.length > a5b, 2000); await sleep(300);
        await page.clock.fastForward(16000);
        await until(async () => /Couldn.t check/.test(await page.evaluate(() => [...document.querySelectorAll('.toast')].map((t) => t.textContent).join(' | '))), 3000);
        const said = await page.evaluate(() => [...document.querySelectorAll('.toast')].map((t) => t.textContent).join(' | '));
        const dialog = await page.evaluate(() => [...document.querySelectorAll('dialog[open]')].map((x) => x.textContent).join(' | '));
        check(/Couldn.t check your Pubky sign-in/.test(said) && !/signed in to Pubky yet/.test(dialog), `${width}: Share with no answer says it couldn't check ("${said.trim()}"), not that you aren't signed in ("${dialog.trim().slice(0, 60)}")`);
        await page.screenshot({ path: path.join(SHOTS, `share-unchecked-${width}.png`) });
        // 6. Still no answer, the playlist open over 15 s: another device moves a song. The new order
        // shows at once (before: the redraw asked again and waited up to 15 s, the old rows taking
        // presses meanwhile), × removes the song shown, and the node's late answer draws it again,
        // past the question's 15 s too (before: only an answer within them did; Copy link stayed
        // missing till the next draw)
        await page.keyboard.press('Escape').catch(() => {});
        check(JSON.stringify(await order()) === '["Journey","Bravo"]', `${width}: the playlist shows as the server has it (${await order()})`);
        lists = ['B', 'J'];
        const a6 = asks.length; const r6 = plReads.length;
        socket.send(JSON.stringify({ type: 'library_changed' }));
        const fast = await until(async () => JSON.stringify(await order()) === '["Bravo","Journey"]', 1200);
        check(fast, `${width}: no answer: the new order shows within 1.2 s (${await order()})`);
        await page.evaluate(() => { const b = [...document.querySelectorAll('#plsongs [data-rm]')].find((x) => /Journey/.test(x.getAttribute('aria-label') || '')); if (b) b.click(); });
        await until(async () => deletes.length === 1, 2000);
        check(JSON.stringify(deletes) === '[1]' && JSON.stringify(lists) === '["B"]', `${width}: × removes the song shown (DELETE ${deletes}, server now ${lists})`);
        await until(async () => JSON.stringify(await order()) === '["Bravo"]', 3000);
        await page.screenshot({ path: path.join(SHOTS, `playlist-moved-unanswered-${width}.png`) });
        const asked6 = [...held.keys()].filter((n) => n >= a6);
        check(asked6.length > 0, `${width}: the redraw still asks the node (${asked6.length} asked)`);
        // (the × draws the list again, its sources too: wait for those reads to end first)
        for (let n = -1; n !== plReads.length;) { n = plReads.length; await sleep(400); }
        await page.clock.fastForward(16000);
        for (let n = -1; n !== plReads.length;) { n = plReads.length; await sleep(400); }
        const r6b = plReads.length;
        asked6.forEach((n) => answer(n, SESSION));
        check(await until(async () => plReads.length > r6b, 2000) && r6b > r6, `${width}: the node's answer after its 15 s draws the playlist again (${plReads.length - r6b} more reads)`);
        // 7. Home's Get started, the node not answering: no "Sign in with Pubky Ring" as if signed out
        await fx.go(page, '/');
        const a7 = asks.length;
        await until(async () => asks.length > a7, 3000); await sleep(300);
        await page.clock.fastForward(16000);
        // (Home waits on the question first, then Get started asks again)
        await until(async () => asks.length > a7 + 1, 3000); await sleep(300);
        await page.clock.fastForward(16000);
        const card = await until(async () => !!(await page.$('#h-setup .setup-step')), 3000);
        const steps = await page.evaluate(() => [...document.querySelectorAll('#h-setup .setup-step')].map((x) => x.dataset.step));
        check(card && steps.includes('play') && !steps.includes('pubky') && !(await page.$('#setupsignin')), `${width}: no answer: Get started leaves the Pubky step out (${steps})`);
        await page.screenshot({ path: path.join(SHOTS, `home-unanswered-${width}.png`) });
        // 8. A node that always answers after 15 s: a ⋯ gives up on its question, the next ⋯ asks
        // again, and the first question's late answer (a session) counts as no newer one answered
        await page.evaluate(() => { const st = { v: 'tracks' }; history.pushState(st, '', '/tracks'); dispatchEvent(new PopStateEvent('popstate', { state: st })); });
        await page.waitForSelector('#tracklist .track');
        const a8 = asks.length;
        await page.click(ROW_MORE, { force: true });
        await until(async () => asks.length > a8, 2000);
        await page.clock.fastForward(16000);
        check(await until(menuOpen, 3000) && !(await offersCopy()), `${width}: no answer in 15 s: the ⋯ opens without the homeserver copy`);
        await closeMenu();
        const b8 = asks.length;
        await page.click(ROW_MORE, { force: true });
        await until(async () => asks.length > b8, 2000);
        answer(a8, SESSION); await sleep(300);
        await page.clock.fastForward(16000);
        const opened8 = await until(menuOpen, 3000);
        check(opened8 && (await offersCopy()), `${width}: the first question's late answer counts: the next ⋯ offers the homeserver copy`);
        if (opened8) await closeMenu();
        check(!state.errors.length, `${width}: no page errors (${state.errors.join(' | ')})`);
      } catch (e) {
        check(false, `${width}: ${e.message.split('\n')[0]}`);
        await page.screenshot({ path: path.join(SHOTS, `error-${width}.png`) }).catch(() => {});
      }
      late();
      await page.context().close();
    }
  } finally {
    await browser.close();
  }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
