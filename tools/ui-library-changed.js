// The library changed (a scan ended, someone edited a song or a playlist) while a long list is on
// screen. Real candidate assets on the fixture origin (tools/ui-fixture.js), 1,200 songs read a
// page at a time as the node gives them:
//   * Songs, scrolled far down: the page is drawn again where it stands. No skeleton, never fewer
//     rows than before, the same row at the same place, the changed title shown, the songs read
//     again in one answer per thousand, and the list still grows when its end is reached;
//   * Songs with a filter typed: the text, its caret and the focus stay, the matches are read with
//     the page (the whole list is never put in between), and the count says what is shown;
//   * ten changes within a quarter of a second draw the page twice (the first at once, the rest
//     together); on a node slower than the changes come there is one read at a time and each
//     answer is shown when it is in; a hidden tab draws nothing until it is looked at again, then once;
//   * another page opened during a read, then Songs again: the next change is still drawn; a read
//     that fails while Songs is drawn again leaves the page as it is, with the reason in a toast;
//   * opening Songs again still starts at its top with one page of songs;
//   * Albums and Artists sort by name exactly as before (numbers by value, accents and case
//     aside) without making a collator for every comparison.
//   node tools/ui-library-changed.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'library-changed-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const N = 1200;
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const NAMES = ['Blue 10', 'Blue 2', 'blue 2', 'BLUE 2', 'blue 1', 'Älv', 'Alv 2', 'alv 1', 'Zebra', 'Été', 'Ete 2', 'Öl', 'Ol 3', '10 Years', '9 Lives', 'ábaco', 'Abacus'];

async function open(browser, width) {
  const tracks = Array.from({ length: N }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song number ${i + 1}`, track_no: (i % 12) + 1 }));
  // 60 albums and artists: the tricky names first, then plain ones
  const albums = Array.from({ length: 60 }, (_, i) => ({ id: i + 1, title: NAMES[i] || `Plain ${String(i).padStart(2, '0')}`, album_artist: `Band ${60 - i}`, year: 1990, tracks: 1 }));
  const artists = albums.map((a) => ({ id: a.id, name: a.title, albums: 1, tracks: 3 }));
  const made = await fx.create(browser, width, { mobile: width < 600, tracks, fixtures: { '/library/albums': { body: albums }, '/library/albums/formats': { body: [] }, '/library/artists': { body: artists }, '/library/stats': { body: { tracks: N } } } });
  const { context, page } = made;
  const reads = { songs: [], search: 0, info: 0, slow: 0, open: 0, most: 0, fail: 0 };
  // The node's paging: `limit` songs after the id `after` (`reads.slow`: ms each answer takes; the
  // answer is the list as it was when asked)
  await context.route(/\/v1\/library\/tracks(\?|$)/, async (r) => {
    const u = new URL(r.request().url()); const after = Number(u.searchParams.get('after') || 0); const limit = Number(u.searchParams.get('limit') || 100); reads.songs.push(limit);
    const body = JSON.stringify({ tracks: tracks.filter((t) => t.id > after).slice(0, Math.min(limit, 1000)), next: null });
    reads.open++; reads.most = Math.max(reads.most, reads.open);
    if (reads.slow) await sleep(reads.slow);
    reads.open--;
    // (`reads.fail`: that many answers are "the library did not answer")
    if (reads.fail) { reads.fail--; return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'The library did not answer' }) }).catch(() => {}); }
    return r.fulfill({ contentType: 'application/json', body }).catch(() => {});
  });
  await context.route(/\/v1\/library\/search(\?|$)/, (r) => { const u = new URL(r.request().url()); const q = (u.searchParams.get('q') || '').toLowerCase(); reads.search++; return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ tracks: tracks.filter((t) => t.title.toLowerCase().includes(q)).slice(0, 500), albums: [], artists: [] }) }); });
  await context.route(/\/v1\/info(\?|$)/, (r) => { reads.info++; return r.fallback(); });
  let socket = null;
  await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
  // (every comparison a page makes by localeCompare with options is counted)
  await page.addInitScript(() => { window.__slowCompares = 0; const was = String.prototype.localeCompare; String.prototype.localeCompare = function (...a) { if (a.length > 1) window.__slowCompares++; return was.apply(this, a); }; }); // eslint-disable-line no-extend-native
  const changed = () => socket.send(JSON.stringify({ type: 'library_changed' }));
  const connected = async () => { for (let i = 0; i < 50 && !socket; i++) await sleep(100); if (!socket) throw new Error('events not connected'); };
  return { ...made, tracks, reads, changed, connected };
}
// What the page shows of Songs: its rows, where it is scrolled, and the first row wholly in sight
const seen = (page) => page.evaluate(() => {
  const main = document.getElementById('main');
  const rows = [...document.querySelectorAll('#tracklist .track')]; const mid = innerHeight / 2;
  const row = rows.find((r) => { const b = r.getBoundingClientRect(); return b.top <= mid && b.bottom > mid; });
  return { n: rows.length, top: Math.round(main.scrollTop), row: row ? row.querySelector('.title .lnk').textContent : null, tid: row ? Number(row.dataset.tid) : null, rowTop: row ? Math.round(row.getBoundingClientRect().top) : null };
});
// Noted from now on: a skeleton drawn into the page, and the fewest and most song rows it had
const watch = (page) => page.evaluate(() => {
  const count = () => document.querySelectorAll('#tracklist .track').length;
  window.__w = { skeleton: false, min: count(), max: count() };
  if (window.__wo) window.__wo.disconnect();
  window.__wo = new MutationObserver(() => { const n = count(); const w = window.__w; if (document.querySelector('#main .sk-grid, #main > .skeleton')) w.skeleton = true; w.min = Math.min(w.min, n); w.max = Math.max(w.max, n); });
  window.__wo.observe(document.getElementById('main'), { childList: true, subtree: true });
});
const watched = (page) => page.evaluate(() => window.__w);
const toRow = async (page, n) => {
  // (the list grows as its end comes near: down a screen at a time until that many rows are in)
  for (let i = 0; i < 80; i++) {
    // (looked at before it is scrolled: enough rows, and the page is left where it is)
    const have = await page.evaluate((want) => { const rows = document.querySelectorAll('#tracklist .track'); if (rows.length < want) { const main = document.getElementById('main'); main.scrollTo({ top: main.scrollHeight, behavior: 'instant' }); } return rows.length; }, n);
    if (have >= n) break; await sleep(120);
  }
  const have = await page.evaluate(() => document.querySelectorAll('#tracklist .track').length);
  if (have < n) throw new Error(`the list stopped growing at ${have} rows (wanted ${n})`);
  await page.evaluate((k) => document.querySelectorAll('#tracklist .track')[k].scrollIntoView({ block: 'center', behavior: 'instant' }), n - 200);
  await frames(page);
};

(async () => {
  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const at = `${width}px`;
    const t = await open(browser, width); const { page, tracks, reads } = t;
    await fx.go(page, '/tracks'); await page.waitForSelector('#tracklist .track'); await t.connected(); await frames(page);
    check((await seen(page)).n === 300 && reads.songs.join() === '300', `${at}: Songs opens with one page of 300 songs (${(await seen(page)).n} rows, asked for ${reads.songs.join()})`);

    // 0. The next part is asked for before the list's end is on screen
    const ahead = await page.evaluate(async () => {
      const main = document.getElementById('main'); const rows = document.querySelectorAll('#tracklist .track');
      // (the last row 900 px below the page's lower edge: not on screen)
      main.scrollTo({ top: main.scrollHeight - main.clientHeight - 900, behavior: 'instant' }); await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const below = Math.round(rows[rows.length - 1].getBoundingClientRect().bottom - main.getBoundingClientRect().bottom);
      for (let i = 0; i < 40 && document.querySelectorAll('#tracklist .track').length === rows.length; i++) await new Promise((r) => setTimeout(r, 50));
      return { below, n: document.querySelectorAll('#tracklist .track').length };
    });
    check(ahead.below > 500 && ahead.n === 600, `${at}: the next 300 songs come while the list's end is still ${ahead.below} px below the screen (${ahead.n} rows)`);
    // 1. Scrolled far down, the library changes
    // (the phone with 600 of the songs listed, the wide page with all 1,200: more than one answer)
    await toRow(page, width < 600 ? 600 : N);
    const a = await seen(page);
    tracks.find((x) => x.id === a.tid).title = 'Renamed while you looked';
    reads.songs.length = 0; await watch(page); t.changed();
    const drawn = await page.waitForFunction(() => [...document.querySelectorAll('#tracklist .track')].some((r) => r.innerText.includes('Renamed while you looked')), null, { timeout: 5000 }).then(() => true, () => false);
    await frames(page);
    const b = await seen(page); const w = await watched(page);
    check(drawn && b.row === 'Renamed while you looked', `${at}: the changed song shows its new title where the old one was (${JSON.stringify(b.row)})`);
    check(!w.skeleton && w.min >= a.n, `${at}: the page is not emptied for it: no skeleton, never fewer rows (${w.min} at least, ${a.n} before; skeleton ${w.skeleton})`);
    check(a.top > 15000 && Math.abs(b.top - a.top) <= 2 && Math.abs(b.rowTop - a.rowTop) <= 2 && b.n >= a.n, `${at}: the page stays where it was scrolled to (${a.top} -> ${b.top}; the row at ${a.rowTop} -> ${b.rowTop}; ${a.n} -> ${b.n} rows)`);
    // (as many as were listed, a thousand per answer and a page at least: what the node is asked for)
    const asks = []; for (let got = 0, more = true; more && got < a.n;) { const limit = Math.min(1000, Math.max(300, a.n - got)); const came = Math.min(limit, N - got); asks.push(limit); got += came; more = came === limit; }
    check(a.n >= (width < 600 ? 600 : N) && reads.songs.join() === asks.join(), `${at}: the ${a.n} songs listed are read again in one answer per thousand (asked for ${reads.songs.join(', ')}; expected ${asks.join(', ')})`);
    await page.screenshot({ path: path.join(SHOTS, `songs-kept-${width}.png`) });
    // (and the list goes on from there)
    await page.evaluate(() => { const main = document.getElementById('main'); main.scrollTo({ top: main.scrollHeight, behavior: 'instant' }); });
    const grew = b.n === N || await page.waitForFunction((n) => document.querySelectorAll('#tracklist .track').length > n, b.n, { timeout: 5000 }).then(() => true, () => false);
    check(grew, `${at}: the list goes on from there: ${b.n === N ? 'all of it is in' : 'more songs come towards its end'} (${b.n} -> ${(await seen(page)).n} rows)`);

    // 2. A filter typed, the focus in it
    await page.evaluate(() => { document.getElementById('main').scrollTo({ top: 0, behavior: 'instant' }); });
    await page.fill('#filter', 'number 11');
    await page.waitForFunction(() => document.querySelectorAll('#tracklist .track').length === 111, null, { timeout: 5000 }).catch(() => {});
    const c0 = await seen(page);
    await page.evaluate(() => { const f = document.querySelector('#filter'); f.focus(); f.setSelectionRange(3, 3); });
    tracks.find((x) => x.title === 'Song number 11').title = 'Song number 11 again';
    reads.songs.length = 0; reads.search = 0; await watch(page); t.changed();
    const drawn2 = await page.waitForFunction(() => [...document.querySelectorAll('#tracklist .track')].some((r) => r.innerText.includes('Song number 11 again')), null, { timeout: 5000 }).then(() => true, () => false);
    await sleep(500); await frames(page);
    const c = await page.evaluate(() => { const f = document.querySelector('#filter'); return { value: f.value, focus: document.activeElement === f, caret: f.selectionStart, n: document.querySelectorAll('#tracklist .track').length, count: document.querySelector('.page-head .count')?.dataset.filtered || '' }; });
    const w2 = await watched(page);
    check(c0.n === 111 && drawn2 && c.n === 111 && c.value === 'number 11', `${at}: with a filter typed, the page is drawn again with the filter's text and its matches (${c0.n} -> ${c.n} rows, "${c.value}")`);
    check(c.focus && c.caret === 3, `${at}: the focus and the caret stay in the filter (focus ${c.focus}, caret at ${c.caret})`);
    check(!w2.skeleton && w2.max <= 111 && c.count === '111 shown' && reads.search === 1, `${at}: the matches are read once, with the page; the whole list is never put in between, and the count says what is shown (${reads.search} search, ${w2.max} rows at most, "${c.count}", skeleton ${w2.skeleton})`);
    await page.fill('#filter', ''); await page.waitForFunction(() => document.querySelectorAll('#tracklist .track').length >= 300, null, { timeout: 5000 }).catch(() => {});

    // 3. Ten changes in half a second: the first at once, the rest together
    await sleep(1100); reads.songs.length = 0; reads.info = 0;
    for (let i = 0; i < 10; i++) { t.changed(); await sleep(20); }
    await sleep(200); const early = reads.songs.length;
    await sleep(2000); const late = reads.songs.length;
    check(early === 1 && late === 2 && reads.info === 2, `${at}: ten changes within a quarter of a second draw the page twice, once at once and once for the rest (${early} read at first, ${late} in all, /info ${reads.info} times)`);

    // 3b. A node slower than the changes come: each answer takes 1.5 s, a change comes every second.
    // One read at a time, and the first one's answer is shown when it is in (not thrown away for the next)
    await sleep(1100);
    tracks.find((x) => x.id === 2).title = 'Changed on a slow node';
    reads.songs.length = 0; reads.most = 0; reads.slow = 1500; await watch(page);
    const t0 = Date.now(); let shownAt = null;
    for (let i = 0; i < 5; i++) {
      t.changed();
      for (let k = 0; k < 10; k++) { await sleep(100); if (shownAt == null && await page.evaluate(() => [...document.querySelectorAll('#tracklist .track')].slice(0, 5).some((r) => r.innerText.includes('Changed on a slow node')))) shownAt = Date.now() - t0; }
    }
    const during = reads.songs.length;
    await page.waitForFunction(() => [...document.querySelectorAll('#tracklist .track')].slice(0, 5).some((r) => r.innerText.includes('Changed on a slow node')), null, { timeout: 8000 }).catch(() => {});
    await sleep(3400); reads.slow = 0;
    const w3 = await watched(page);
    check(shownAt != null && shownAt < 4000, `${at}: on a node that answers in 1.5 s while a change comes every second, the first answer is shown when it is in (after ${shownAt == null ? 'the changes ended' : shownAt + ' ms'})`);
    check(reads.most === 1 && during <= 4 && !w3.skeleton, `${at}: one read at a time meanwhile (${reads.most} at once at most, ${during} begun in 5 s, ${reads.songs.length} in all; skeleton ${w3.skeleton})`);

    // 3c. Another page opened while Songs is being read again, then Songs again: the read that was
    // cut off doesn't count, and the next change is drawn (it was ignored until a reload)
    await sleep(1100);
    reads.slow = 1500; t.changed(); await sleep(300);
    await page.evaluate(() => { history.pushState({}, '', '/albums'); dispatchEvent(new PopStateEvent('popstate')); });
    await page.waitForSelector('#albumgrid .tile'); reads.slow = 0;
    await page.evaluate(() => { history.pushState({}, '', '/tracks'); dispatchEvent(new PopStateEvent('popstate')); });
    await page.waitForSelector('#tracklist .track'); await sleep(1800);
    tracks.find((x) => x.id === 3).title = 'Changed after coming back';
    t.changed();
    const back = await page.waitForFunction(() => [...document.querySelectorAll('#tracklist .track')].slice(0, 5).some((r) => r.innerText.includes('Changed after coming back')), null, { timeout: 5000 }).then(() => true, () => false);
    check(back, `${at}: after another page was opened during a read and Songs opened again, the next change is drawn`);

    // 3e. A change that waits on Songs, another page opened, then a change while that page is open:
    // the later change is drawn when the wait is up (it was dropped: the page stayed as it was)
    await sleep(1100); reads.info = 0;
    t.changed(); await sleep(120); t.changed(); await sleep(60);
    await page.evaluate(() => { history.pushState({}, '', '/albums'); dispatchEvent(new PopStateEvent('popstate')); });
    await page.waitForSelector('#albumgrid .tile');
    const info0 = reads.info; t.changed(); await sleep(1800);
    check(info0 === 1 && reads.info === 2, `${at}: a change that comes on Albums while an earlier one on Songs still waits is drawn (/info read ${info0} time(s) before it, ${reads.info} in all)`);
    await page.evaluate(() => { history.pushState({}, '', '/tracks'); dispatchEvent(new PopStateEvent('popstate')); });
    await page.waitForSelector('#tracklist .track'); await sleep(300);

    // 3f. Play all still loading the library when Songs is drawn again in place: the new Play all is
    // drawn pressed (it could start a second load of the whole library), and works again after
    if (width === 1280) {
      await sleep(1100); reads.slow = 1200;
      await page.evaluate(() => { window.__shOld = document.getElementById('shuffle'); });
      await page.click('#shuffle'); await sleep(150); t.changed();
      const redrawn = await page.waitForFunction(() => { const b = document.getElementById('shuffle'); return !!b && b !== window.__shOld; }, null, { timeout: 6000 }).then(() => true, () => false);
      const during = await page.evaluate(() => { const b = document.getElementById('shuffle'); return b ? b.disabled : null; });
      reads.slow = 0;
      const after = await page.waitForFunction(() => { const b = document.getElementById('shuffle'); return b && !b.disabled; }, null, { timeout: 8000 }).then(() => true, () => false);
      check(redrawn && during === true && after, `${at}: Play all that is still loading when the page is drawn again in place is drawn pressed (${redrawn ? `disabled ${during}` : 'not drawn again'}), and can be pressed again once it has loaded (${after})`);
      await page.evaluate(() => { const a = document.querySelector('audio'); if (a) a.pause(); });
    }

    // 3d. A read that fails while Songs is drawn again: the page stays, with the reason in a toast
    await sleep(1100);
    await toRow(page, 600);
    const f0 = await seen(page);
    reads.fail = 1; await watch(page); t.changed(); await sleep(900);
    const f1 = await seen(page); const wf = await watched(page);
    const said = await page.evaluate(() => { const el = document.querySelector('#toast'); return el && el.classList.contains('show') ? el.textContent : ''; });
    check(f1.n === f0.n && Math.abs(f1.top - f0.top) <= 2 && !wf.skeleton && !(await page.$('#navretry')) && /did not answer/.test(said), `${at}: a failed read leaves the page as it is (${f0.n} -> ${f1.n} rows, at ${f0.top} -> ${f1.top}) and says why ("${said}")`);
    await page.evaluate(() => document.getElementById('main').scrollTo({ top: 0, behavior: 'instant' }));

    // 4. A hidden tab draws nothing, and once when it is looked at again
    const vis = (v) => page.evaluate((x) => { Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => x }); Object.defineProperty(document, 'hidden', { configurable: true, get: () => x === 'hidden' }); document.dispatchEvent(new Event('visibilitychange')); }, v);
    await sleep(1100); reads.songs.length = 0;
    await vis('hidden'); for (let i = 0; i < 3; i++) { t.changed(); await sleep(50); }
    await sleep(1300); const whileHidden = reads.songs.length;
    await vis('visible'); await sleep(1300);
    check(whileHidden === 0 && reads.songs.length === 1, `${at}: a hidden tab draws nothing (${whileHidden} reads) and draws once when it is looked at again (${reads.songs.length})`);

    // 5. Opened again, Songs starts at its top with one page
    await toRow(page, 600);
    await page.evaluate(() => { history.pushState({}, '', '/albums'); dispatchEvent(new PopStateEvent('popstate')); });
    await page.waitForSelector('#albumgrid .tile');
    reads.songs.length = 0;
    await page.evaluate(() => { history.pushState({}, '', '/tracks'); dispatchEvent(new PopStateEvent('popstate')); });
    await page.waitForSelector('#tracklist .track'); await frames(page);
    const e = await seen(page);
    check(e.n === 300 && e.top === 0 && reads.songs.join() === '300', `${at}: opened again, Songs starts at its top with one page (${e.n} rows, scrolled ${e.top}, asked for ${reads.songs.join()})`);

    // 6. Albums and Artists by name: the order as before, without a collator per comparison
    for (const [view, sel, name] of [['albums', '#albumgrid .tile', '.title, .name, .t'], ['artists', '#artistlist [data-artist]', '.name']]) {
      await page.evaluate(() => { window.__slowCompares = 0; });
      await page.evaluate((v) => { history.pushState({}, '', '/' + (v === 'albums' ? 'artists' : 'albums')); dispatchEvent(new PopStateEvent('popstate')); }, view);
      await sleep(300); await page.evaluate(() => { window.__slowCompares = 0; });
      await page.evaluate((v) => { history.pushState({}, '', '/' + v); dispatchEvent(new PopStateEvent('popstate')); }, view);
      await page.waitForFunction((s) => document.querySelectorAll(s).length === 60, sel, { timeout: 5000 }).catch(() => {});
      const o = await page.evaluate(([s, nm, names]) => {
        const got = [...document.querySelectorAll(s)].map((x) => (x.querySelector(nm) || x).textContent.trim()).filter((x) => names.includes(x));
        const want = [...names].sort((x, y) => x.localeCompare(y, undefined, { sensitivity: 'base', numeric: true }));
        const slow = window.__slowCompares; // (read before the line above could be counted: it ran after)
        return { got, want, slow };
      }, [sel, name, NAMES]);
      // (the reference sort above runs in the page too: its comparisons are taken off)
      const mine = await page.evaluate((names) => { const b = window.__slowCompares; [...names].sort((x, y) => x.localeCompare(y, undefined, { sensitivity: 'base', numeric: true })); return window.__slowCompares - b; }, NAMES);
      const made = o.slow - mine;
      // (names that differ only by case or accent keep the order the list came in, in both)
      check(o.got.length === NAMES.length && o.got.join('|') === o.want.join('|'), `${at}: ${view} by name are in the order they always were: numbers by value, accents and case aside (${o.got.join(', ')})`);
      check(made === 0, `${at}: drawing ${view} makes no collator per comparison (${made} comparisons by localeCompare with options)`);
    }
    await page.screenshot({ path: path.join(SHOTS, `albums-sorted-${width}.png`) });
    check(t.state.errors.length === 0, `${at}: no script errors (${t.state.errors.join(' | ')})`);
    await t.context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
