// A page or a ⋯ menu while the node doesn't answer whether it holds a Pubky session. An album page
// and each menu ask that first (a menu offers "Copy to my homeserver" then). Before, a GET /pubky that
// never answered left the page, and every menu opened meanwhile, waiting for good: the 15 s limit only
// let the next caller ask again. Now
// after 15 s every page and menu waiting on the question goes on (without the homeserver copy, nothing kept),
// the next one asks again, and only the latest question's answer counts: one that answers late
// doesn't undo a newer one; a playlist's Share with no answer says it couldn't check rather than
// that you aren't signed in. At 360 (phone) and 1280 px.
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
const fixtures = { '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, mobile] of [[360, true], [1280, false]]) {
      const { page, state } = await fx.create(browser, width, { mobile, fixtures, tracks: [matched] });
      await page.clock.install();
      // GET /pubky: the first two never answer until `late()` (then: no session); later ones answer
      // at once with a session
      const asks = []; const lates = []; let hangAll = false;
      await page.route('**/v1/pubky', (r) => {
        asks.push(Date.now());
        if (asks.length <= 2 || hangAll) return new Promise((res) => { lates.push(res); }).then(() => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ user: null }) }).catch(() => {}));
        return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ user: 'pk:fixture', sync: { running: false } }) });
      });
      const late = () => lates.splice(0).forEach((f) => f());
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
