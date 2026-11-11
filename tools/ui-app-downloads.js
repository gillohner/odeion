// Downloads in the Android app (crates/pm-offline through src/offline.rs): the page inside the app
// (window.pmDesktop with nativePlayer, Tauri's invoke stood in for by a fake that keeps what the
// app keeps and answers its storage state). Real candidate assets on the fixture origin
// (tools/ui-fixture.js):
//   * "Download" in the ⋯ menu of a library song, a library album and a playlist sends
//     downloads_keep with the item (and its name); kept, the menu says "Remove download";
//   * a playlist with a song from a streaming service says how many stay online; that song's own
//     menu has no Download (only the library's files are kept);
//   * the mark: song rows and album tiles kept on this phone carry it, others don't;
//   * the storage screen (Downloads, online): what is kept with its songs here and size, the
//     total against the limit, the download under way, Remove per item, Remove all (asked
//     first), the limit; reachable from Your library;
//   * a plain browser: no Download anywhere, and Downloads points to the app.
//   node tools/ui-app-downloads.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'app-downloads-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const tracks = [1, 2].map((id) => ({ ...fx.track, id, title: `Song ${id}`, album: { id: 7, title: 'Journey' } }));
const album = { id: 7, title: 'Journey', album_artist: 'Sefa', year: 2020, tracks: 2 };
const other = { id: 8, title: 'Other', album_artist: 'Dr. Peacock', year: 2021, tracks: 1 };
// The playlist's songs: one of the library, one of a streaming service
const pf = { order: { order: ['library', 'tidal'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: [
  { index: 0, uri: fx.SONG, title: 'Song 1', artist: 'Sefa', track_id: 1, plays_from: { service: 'library', label: 'Your library' }, sources: [] },
  { index: 1, uri: 'tidal:track:123', title: 'A TIDAL song', artist: 'Someone', plays_from: { service: 'tidal', label: 'TIDAL' }, sources: [] }] };

// The playlist: a library song and a streaming service's song
const playlist = { ...fx.detail, playlist: { ...fx.detail.playlist, items: [{ uri: fx.SONG }, { uri: 'tidal:track:123', title: 'A TIDAL song', artist: 'Someone' }] }, tracks: [fx.track, null], entries: [{ kind: 'track', uri: fx.SONG }, { kind: 'track', uri: 'tidal:track:123' }] };
// A library album's page (as tools/ui-busy.js has it)
const albumPage = { kind: 'album', id: 'lib-8', uri: null, target: {}, title: 'Other', artist: { name: 'Dr. Peacock', id: 'lib-22' }, album: null, year: 2021, image: null, source: 'library', source_id: '8', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [{ no: 1, title: 'Other, song one', artist: 'Dr. Peacock', duration_ms: 200000, uri: 'track:2', track_id: 2, id: 'lib-2', page: 'lib-2' }], total_ms: 200000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
// The app's side: what it keeps, its storage state, the calls
const FAKE_APP = () => {
  window.pmDesktop = { version: '0.2.2', platform: 'android', nativePlayer: true };
  const calls = window.__calls = [];
  const st = window.__dl = { items: [], songs: 0, bytes: 0, favourites: false, track_ids: [], album_ids: [], playlist_ids: [], limit: 10e9, free: 40e9, keep_free: 1e9, progress: { running: false, wanted: 0, bytes_wanted: 0, done: 0, bytes_done: 0, failed: 0 } };
  const copy = () => JSON.parse(JSON.stringify(st));
  window.__TAURI_INTERNALS__ = { invoke: async (cmd, args) => {
    calls.push([cmd, args || {}]);
    if (cmd === 'downloads_state') return copy();
    if (cmd === 'downloads_keep') {
      const { kind, id, on, title } = args;
      st.items = st.items.filter((x) => !(x.kind === kind && x.id === id));
      if (on) st.items.push({ kind, id, title: title || kind, sub: '', here: 0, bytes: 0, songs: kind === 'track' ? 1 : 2, online: 0 });
      // Song 2 stays kept by its album (as the app answers)
      if (!on && kind === 'track') return { ...copy(), still_kept: id === '2' };
      if (kind === 'album') st.album_ids = on ? [...new Set([...st.album_ids, Number(id)])] : st.album_ids.filter((x) => x !== Number(id));
      return copy();
    }
    if (cmd === 'downloads_remove_all') { Object.assign(st, { items: [], songs: 0, bytes: 0, track_ids: [], album_ids: [], playlist_ids: [] }); return copy(); }
    if (cmd === 'downloads_limit') { st.limit = args.bytes; return copy(); }
    if (cmd.startsWith('native_')) return null;
    throw `no command ${cmd}`;
  } };
};
const calls = (page, cmd) => page.evaluate((c) => window.__calls.filter((x) => !c || x[0] === c), cmd);
const menuLabels = (page) => page.evaluate(() => [...document.querySelectorAll('.menu .pop-item, .menu button, [role="menu"] button')].filter((b) => b.getClientRects().length).map((b) => b.textContent.trim()));
// Press the menu item named exactly `label`
const pick = (page, label) => page.evaluate((l) => { const b = [...document.querySelectorAll('.menu .pop-item, .menu button, [role="menu"] button')].filter((x) => x.getClientRects().length).find((x) => x.textContent.trim() === l); if (b) b.click(); return !!b; }, label);
const closeMenu = async (page) => { await page.keyboard.press('Escape'); await sleep(250); };

async function open(browser, w, app) {
  const run = await fx.create(browser, w, { tracks, fixtures: { '/library/albums': { body: [album, other] }, '/playlists/c1/play-from': { body: pf }, '/playlists/c1': { body: playlist }, '/item/album/lib-8': { body: albumPage }, '/item/album/lib-8/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } } });
  if (app) await run.context.addInitScript(FAKE_APP);
  await fx.go(run.page, '/');
  await run.page.waitForFunction(() => !!window.pmPlayer);
  await sleep(500);
  return run;
}
const goto = async (page, p) => { await page.evaluate((x) => { history.pushState(null, '', x); window.dispatchEvent(new PopStateEvent('popstate')); }, p); await sleep(900); };

(async () => {
  const browser = await chromium.launch();
  for (const w of [390, 1280]) {
    const at = `${w}px`;
    // A state reply started before a download event must not erase its song marks.
    {
      const r = await open(browser, w, true); const p = r.page;
      await p.evaluate(() => {
        const invoke = window.__TAURI_INTERNALS__.invoke;
        window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
          if (cmd !== 'downloads_state') return invoke(cmd, args);
          const old = JSON.parse(JSON.stringify(window.__dl));
          return new Promise((resolve) => { window.__finishState = () => resolve(old); });
        };
      });
      await goto(p, '/downloads');
      await p.waitForFunction(() => !!window.__finishState);
      await p.evaluate(() => {
        Object.assign(window.__dl, { items: [{ kind: 'track', id: '1', title: 'Song 1', here: 1, bytes: 100, songs: 1 }], songs: 1, bytes: 100, track_ids: [1] });
        window.dispatchEvent(new CustomEvent('pmdesktop:downloads', { detail: JSON.parse(JSON.stringify(window.__dl)) }));
        window.__finishState();
      });
      await sleep(200);
      await goto(p, '/tracks');
      check(await p.locator('.track[data-tid="1"] .dl-mark').count() === 1, `${at}: a late state reply preserves the newer downloaded mark`);
      await r.context.close();
    }
    const run = await open(browser, w, true);
    const { page, state } = run;
    // A library song: Download in its menu
    await goto(page, '/tracks');
    await page.click('.track[data-tid="1"] [data-act="more"]').catch(() => {});
    await sleep(400);
    let labels = await menuLabels(page);
    check(labels.includes('Download'), `${at}: a library song's ⋯ menu has Download (${labels.join(' | ')})`);
    await pick(page, 'Download'); await sleep(400);
    let keep = (await calls(page, 'downloads_keep')).pop();
    check(keep && keep[1].kind === 'track' && keep[1].id === '1' && keep[1].on === true && keep[1].title === 'Song 1', `${at}: it keeps the song (${JSON.stringify(keep && keep[1])})`);
    await page.click('.track[data-tid="1"] [data-act="more"]'); await sleep(400);
    labels = await menuLabels(page);
    check(labels.includes('Remove download') && !labels.includes('Download'), `${at}: kept, the menu says Remove download`);
    await closeMenu(page);
    // The mark: a song on this phone, an album kept
    await page.evaluate(() => { Object.assign(window.__dl, { track_ids: [1], album_ids: [7], songs: 1, bytes: 30e6 }); window.dispatchEvent(new CustomEvent('pmdesktop:downloads', { detail: JSON.parse(JSON.stringify(window.__dl)) })); }); await sleep(400);
    const marks = await page.evaluate(() => [...document.querySelectorAll('.track[data-tid]')].map((r) => [r.dataset.tid, !!r.querySelector('.dl-mark')]));
    check(JSON.stringify(marks) === '[["1",true],["2",false]]', `${at}: the song on this phone carries the mark, the other not (${JSON.stringify(marks)})`);
    // Remove download of a song kept only by itself (on the phone): said removed, not "stays"
    await page.click('.track[data-tid="1"] [data-act="more"]'); await sleep(400);
    await pick(page, 'Remove download'); await sleep(400);
    const rmToast = await page.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).join(' | '));
    check(/“Song 1” removed from this phone/.test(rmToast) && !/stays/.test(rmToast), `${at}: removing a song kept alone says it goes (${rmToast})`);
    await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('downloads_keep', { kind: 'track', id: '2', on: true, title: 'Song 2' })); await page.evaluate(() => window.dispatchEvent(new CustomEvent('pmdesktop:downloads', { detail: JSON.parse(JSON.stringify(window.__dl)) }))); await sleep(300);
    await page.click('.track[data-tid="2"] [data-act="more"]'); await sleep(400);
    await pick(page, 'Remove download'); await sleep(400);
    const stayToast = await page.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).join(' | '));
    check(/“Song 2” stays on this phone/.test(stayToast), `${at}: removing a song its album keeps says it stays (${stayToast})`);
    await goto(page, '/albums');
    const tiles = await page.evaluate(() => [...document.querySelectorAll('.tile[data-album]')].map((b) => [b.dataset.album, !!b.querySelector('.dl-mark')]));
    check(JSON.stringify(tiles) === '[["7",true],["8",false]]', `${at}: so does the album kept (${JSON.stringify(tiles)})`);
    await page.screenshot({ path: path.join(SHOTS, `albums-marked-${w}.png`) });
    // A library album's page: Download
    await goto(page, '/album/lib-8');
    await page.screenshot({ path: path.join(SHOTS, `album-page-${w}.png`) });
    await page.click('#imore').catch(() => {}); await sleep(400);
    labels = await menuLabels(page);
    check(labels.includes('Download'), `${at}: a library album's ⋯ menu has Download (${labels.join(' | ')})`);
    if (labels.includes('Download')) { await pick(page, 'Download'); await sleep(400); }
    keep = (await calls(page, 'downloads_keep')).pop();
    check(keep && keep[1].kind === 'album' && keep[1].id === '8' && keep[1].songs === 1, `${at}: it keeps the album, with its song count (${JSON.stringify(keep && keep[1])})`);
    await closeMenu(page);
    // Its song rows get the mark once the song is on the phone (the page's song rows too)
    await page.evaluate(() => { Object.assign(window.__dl, { track_ids: [1, 2] }); window.dispatchEvent(new CustomEvent('pmdesktop:downloads', { detail: JSON.parse(JSON.stringify(window.__dl)) })); }); await sleep(400);
    const rowMark = await page.evaluate(() => { const r = document.querySelector('[data-srow][data-tid="2"]'); return r ? !!r.querySelector('.dl-mark') : null; });
    check(rowMark === true, `${at}: the album page's song row carries the mark (${rowMark})`);
    // A playlist: Download says what stays online; the streaming song has no Download
    await goto(page, '/playlist/c1');
    await page.click('#plmore').catch(() => {}); await sleep(400);
    labels = await menuLabels(page);
    check(labels.includes('Download'), `${at}: a playlist's ⋯ menu has Download (${labels.join(' | ')})`);
    if (labels.includes('Download')) { await pick(page, 'Download'); await sleep(400); }
    keep = (await calls(page, 'downloads_keep')).pop();
    const toastText = await page.evaluate(() => [...document.querySelectorAll('.toast')].map((x) => x.textContent).join(' | '));
    check(keep && keep[1].kind === 'playlist' && keep[1].id === 'c1' && /1 song not in your library stays online/.test(toastText), `${at}: it keeps the playlist and says what stays online (${toastText})`);
    await closeMenu(page);
    const smore = await page.$$('[data-smore]');
    if (smore.length >= 2) {
      await smore[1].click(); await sleep(400);
      labels = await menuLabels(page);
      check(!labels.includes('Download') && !labels.includes('Remove download'), `${at}: the TIDAL song's menu has no Download (${labels.join(' | ')})`);
      await closeMenu(page);
    } else check(false, `${at}: the playlist's song rows (${smore.length})`);
    // The storage screen, from Your library
    await page.evaluate(() => { Object.assign(window.__dl, { items: [{ kind: 'album', id: '7', title: 'Journey', sub: 'Sefa', here: 2, bytes: 61e6, songs: 3, online: 0 }, { kind: 'playlist', id: 'c1', title: 'Music and notes', sub: '', here: 1, bytes: 30e6, songs: 1, online: 1 }, { kind: 'track', id: '2', title: 'Song 2', sub: 'Sefa', here: 0, bytes: 0, songs: 1, online: 0 }], songs: 3, bytes: 91e6, progress: { running: true, wanted: 3, bytes_wanted: 90e6, done: 1, bytes_done: 30e6, failed: 0, current: 'Song 2' } }); });
    await goto(page, '/library');
    const go = await page.$('[data-dlgo]');
    check(!!go, `${at}: Your library has a way to Downloads`);
    if (go) { await go.click(); await sleep(900); } else await goto(page, '/downloads');
    const text = await page.evaluate(() => document.querySelector('main').innerText);
    check(/91\.0 MB of 10\.0 GB on this phone/.test(text) && /3 songs/.test(text), `${at}: the total against the limit (${text.split('\n').slice(0, 3).join(' / ')})`);
    check(/Downloading 2 of 3: Song 2/.test(text), `${at}: the download under way`);
    // Progress alone: the status line changes in place, the lists and the limit stay as they are
    await page.evaluate(() => { const sel = document.querySelector('#dllimit'); sel.__kept = true; Object.assign(window.__dl.progress, { done: 1, failed: 1, current: 'Song 3' }); window.dispatchEvent(new CustomEvent('pmdesktop:downloads', { detail: JSON.parse(JSON.stringify(window.__dl)) })); }); await sleep(300);
    const inPlace = await page.evaluate(() => [!!document.querySelector('#dllimit').__kept, document.querySelector('.dl-note').innerText]);
    check(inPlace[0] && /Downloading 3 of 3: Song 3/.test(inPlace[1]), `${at}: a progress event updates the status line in place (${JSON.stringify(inPlace)})`);
    // A song done (the lists change): the lists again, the limit picker stays the same element
    await page.evaluate(() => { window.__dl.items[0].here = 3; window.__dl.items[0].bytes = 91e6; window.dispatchEvent(new CustomEvent('pmdesktop:downloads', { detail: JSON.parse(JSON.stringify(window.__dl)) })); }); await sleep(300);
    const kept2 = await page.evaluate(() => [!!document.querySelector('#dllimit').__kept, document.querySelector('#dllists').innerText]);
    check(kept2[0] && /3 of 3 songs here/.test(kept2[1]), `${at}: a song done redraws the lists, not the limit picker (${JSON.stringify(kept2[0])})`);
    const fetched = (await calls(page, 'downloads_state')).length;
    check(/Journey/.test(text) && /2 of 3 songs here/.test(text) && /61\.0 MB/.test(text) && /1 stay online/.test(text) && /waiting/.test(text), `${at}: each kept thing with its songs here and size`);
    await page.screenshot({ path: path.join(SHOTS, `storage-${w}.png`), fullPage: true });
    check((await calls(page, 'downloads_state')).length === fetched, `${at}: and asks the app nothing more`);
    await page.click('[data-dlrm="0"]'); await sleep(400);
    keep = (await calls(page, 'downloads_keep')).pop();
    check(keep && keep[1].kind === 'album' && keep[1].id === '7' && keep[1].on === false, `${at}: Remove takes one thing off (${JSON.stringify(keep && keep[1])})`);
    await page.selectOption('#dllimit', String(5e9)); await sleep(400);
    const lim = (await calls(page, 'downloads_limit')).pop();
    check(lim && lim[1].bytes === 5e9, `${at}: the limit is set (${JSON.stringify(lim && lim[1])})`);
    await page.click('#dlrmall'); await sleep(400);
    const asked = await page.evaluate(() => { const d = document.querySelector('dialog[open]'); return d ? d.innerText : ''; });
    check(/Remove all downloads\?/.test(asked) && /library and your playlists stay/.test(asked), `${at}: Remove all asks first`);
    await page.click('dialog[open] button.primary'); await sleep(500);
    check((await calls(page, 'downloads_remove_all')).length === 1, `${at}: and removes the phone's copies`);
    const empty = await page.evaluate(() => document.querySelector('main').innerText);
    check(/Nothing on this phone yet/.test(empty), `${at}: then the screen is empty`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join(' | ')})`);
    await run.context.close();

    // A plain browser: no Download, and Downloads points to the app
    const b = await open(browser, w, false);
    await goto(b.page, '/tracks');
    await b.page.click('.track[data-tid="1"] [data-act="more"]').catch(() => {}); await sleep(400);
    labels = await menuLabels(b.page);
    check(labels.length > 0 && !labels.includes('Download'), `${at}: a plain browser's song menu has no Download (${labels.join(' | ')})`);
    await closeMenu(b.page);
    await goto(b.page, '/downloads');
    const t = await b.page.evaluate(() => document.querySelector('main').innerText);
    check(/Get the app/.test(t), `${at}: a plain browser's Downloads points to the app`);
    check(!b.state.errors.length, `${at}: browser: no page errors (${b.state.errors.join(' | ')})`);
    await b.context.close();
  }
  await browser.close();
  if (failed) { console.error(`${failed} check(s) failed`); process.exit(1); }
  console.log('downloads in the app work');
})();
