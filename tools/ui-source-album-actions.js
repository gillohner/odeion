// A streaming service's own album page (Sources › SoundCloud › a set, archive.org, Bandcamp …: the
// `source-item` page) has the same buttons as a library album: Play, Shuffle, the heart and ⋯.
// Before, it had Play, Add to queue, Collection and Bookmark as four worded buttons, which took two
// rows on a phone, and no Shuffle or Play next for the whole album. Now Add to queue, Play next and
// Add to collection… are in the ⋯ (as on a library album), and "in your library" is a link in the
// line under the title (as on a service's artist page). Checked at 360/412 px phones and 768/1280 px:
// the four buttons and nothing else, on one row on a phone with ♡ and ⋯ at the right edge (48 px, as a
// library album's), no sideways scroll, also with Android's font size at 130/150/200 %; Shuffle plays the whole album with shuffle on; the ⋯ lists
// Play next, Add to queue, Add to collection… and each does its job (the queue, the queue right after
// the song playing, the collection's POST with the album's three addresses); the heart says pressed
// or not in its title; "in your library" opens the library album.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-source-album-actions.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'source-album-actions-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const url = 'https://soundcloud.com/sefa/sets/journey';
// The songs play straight in this browser: a 20 s mono WAV on the fixture's origin, served with ranges
const WAV = (() => { const rate = 8000, n = rate * 20, b = Buffer.alloc(44 + n * 2); b.write('RIFF'); b.writeUInt32LE(b.length - 8, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40); return b; })();
const serveWav = (r) => {
  const m = /bytes=(\d+)-(\d*)/.exec(r.request().headers().range || '');
  const a = m ? Number(m[1]) : 0, z = m && m[2] ? Math.min(Number(m[2]), WAV.length - 1) : WAV.length - 1;
  return r.fulfill({ status: m ? 206 : 200, contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes', 'content-length': String(z - a + 1), ...(m ? { 'content-range': `bytes ${a}-${z}/${WAV.length}` } : {}) }, body: WAV.subarray(a, z + 1) });
};
const tracks = ['First', 'Second', 'Third'].map((title, i) => ({ url: `${fx.ORIGIN}/audio/${title.toLowerCase()}.wav`, title, track_no: i + 1, duration_ms: 200000, format: 'mp3' }));
const fixturesFor = (item) => ({
  '/sources': { body: [{ id: 'soundcloud', name: 'SoundCloud', enabled: true, status: 'signed in' }] },
  '/sources/soundcloud/likes': { body: { tracks: [], playlists: [{ id: 'abc', title: 'Journey', creator: 'Sefa' }] } },
  '/sources/soundcloud/playlists': { body: [] },
  '/sources/soundcloud/items/abc': { body: item },
  '/playlists': { body: [{ id: 'p1', name: 'Road trip', items: 4, covers: [], visibility: 'private', synced: false, layout: 'list', tracks_only: true }] },
  '/playlists/p1/items': { body: {} },
});
// (and a fourth song the service gives no address for: the album's buttons leave it out)
const item = { title: 'Journey', creator: 'Sefa', year: 2020, url, image: null, tracks: [...tracks, { url: '', title: 'Fourth', track_no: 4, duration_ms: 200000, format: 'mp3' }] };

const onBar = (page) => page.evaluate(() => (document.querySelector('.player .np-title') || {}).textContent || '');
const waitBar = (page, re) => page.waitForFunction((s) => new RegExp(s).test((document.querySelector('.player .np-title') || {}).textContent || ''), re.source, { timeout: 3000 }).catch(() => {});
const queueTitles = async (page, want) => {
  await page.waitForFunction((w) => { try { return (JSON.parse(localStorage.getItem('pm.browserQueue') || '[]')).map((x) => x.title).join() === w; } catch { return false; } }, want, { timeout: 3000 }).catch(() => {});
  return page.evaluate(() => { try { return (JSON.parse(localStorage.getItem('pm.browserQueue') || '[]')).map((x) => x.title); } catch { return null; } });
};
const openItem = async (page) => {
  await fx.go(page, '/sources/soundcloud');
  await page.click('[data-sitem="abc"]');
  await page.waitForSelector('#main .tracks .track');
};

async function run(browser, width, opts = {}) {
  const mobile = width < 600;
  const tag = `${width} px${mobile ? ' phone' : ''}`;
  const { page, context, state } = await fx.create(browser, width, { fixtures: fixturesFor({ ...item, ...(opts.inLibrary ? { library_album_id: 7 } : {}) }), realAudio: true, mobile });
  await page.route('**/audio/*.wav', serveWav);
  // The node's bookmarks: a press puts the album's address in
  let favs = [];
  await page.route('**/v1/favourites**', (r) => {
    const m = r.request().method();
    if (m === 'PUT') favs = [url]; else if (m === 'DELETE') favs = [];
    return r.fulfill({ contentType: 'application/json', body: JSON.stringify(m === 'PUT' ? { uri: url } : { uris: favs, tracks: [], albums: [], artists: [], other: [] }) });
  });
  const press = (sel) => (mobile ? page.locator(sel).first().tap() : page.locator(sel).first().click());
  try {
    await openItem(page);
    await page.waitForTimeout(200);
    if (opts.inLibrary) {
      const link = page.locator('#main .artist-hero .muted #ainlib');
      check(await link.count() === 1 && (await link.textContent()).trim() === 'in your library', `${tag}: an album you have says "in your library" in the line under its title`);
      check(!(await page.$('#main .actions #ainlib')), `${tag}: and it is not a button of the action row`);
      if (mobile) {
        // (a 44 px tap reaches it, as every link of a phone's line of links: not the "open" beside it)
        const hit = await page.evaluate(() => { const l = document.querySelector('#ainlib'); const r = l.getBoundingClientRect(); const x = r.left + r.width / 2; return [-20, -12, 12, 20].map((dy) => { const e = document.elementFromPoint(x, r.top + r.height / 2 + dy); return !!e && (e === l || l.contains(e)); }); });
        check(hit.every(Boolean), `${tag}: "in your library" answers a tap 20 px above or below its middle (${hit.join()})`);
      }
      await press('#ainlib');
      await page.waitForFunction(() => location.pathname === '/album/lib-7', null, { timeout: 3000 }).catch(() => {});
      check(await page.evaluate(() => location.pathname) === '/album/lib-7', `${tag}: "in your library" opens the library album (${await page.evaluate(() => location.pathname)})`);
      return;
    }
    const hero = await page.evaluate(() => {
      const acts = document.querySelector('#main .artist-hero .actions');
      const kids = [...acts.children].filter((e) => getComputedStyle(e).display !== 'none').map((e) => { const b = e.getBoundingClientRect(); return { id: e.id, top: b.top, mid: b.top + b.height / 2, left: b.left, right: b.right, w: b.width, h: b.height, text: e.textContent.trim(), title: e.getAttribute('title') || '', label: e.getAttribute('aria-label') || '' }; });
      return { kids, over: document.documentElement.scrollWidth - innerWidth, inner: innerWidth, old: ['#aadd', '#amore'].filter((s) => document.querySelector(s)) };
    });
    const ids = hero.kids.map((k) => k.id).join();
    check(ids === 'aplay,ashuffle,aheart,amenu', `${tag}: the album's buttons are Play, Shuffle, ♡, ⋯, as a library album's (${ids})`);
    check(!hero.old.length, `${tag}: no Add to queue or Collection button of their own (${hero.old.join(', ')})`);
    const by = Object.fromEntries(hero.kids.map((k) => [k.id, k]));
    check(by.ashuffle && by.ashuffle.text === 'Shuffle', `${tag}: Shuffle says Shuffle`);
    check(by.aheart && by.aheart.text === '' && by.aheart.title === 'Add to Bookmarks', `${tag}: the heart is an icon, named Add to Bookmarks (${by.aheart && JSON.stringify([by.aheart.text, by.aheart.title])})`);
    check(by.amenu && by.amenu.label === 'More', `${tag}: ⋯ is named More`);
    check(hero.over <= 0, `${tag}: no sideways scroll (${hero.over})`);
    if (mobile) check(hero.kids.every((k) => k.w >= 44 && k.h >= 44), `${tag}: each button is at least 44 px (${hero.kids.map((k) => `${k.id} ${Math.round(k.w)}x${Math.round(k.h)}`).join(', ')})`);
    if (mobile) {
      const mids = hero.kids.map((k) => k.mid);
      check(Math.max(...mids) - Math.min(...mids) <= 1, `${tag}: on a phone the four sit on one row (${mids.map(Math.round).join(', ')})`);
      check(by.aheart && by.amenu && by.aheart.w === 48 && by.amenu.w === 48 && by.amenu.h === 48, `${tag}: ♡ and ⋯ are 48 px, as on a library album (${by.amenu && [by.aheart.w, by.amenu.w, by.amenu.h].join('/')})`);
      check(by.amenu && Math.abs(by.amenu.right - (hero.inner - 14)) <= 1, `${tag}: ⋯ ends at the page's right edge (${by.amenu && Math.round(by.amenu.right)} of ${hero.inner})`);
      check(by.aheart && by.amenu && by.amenu.left - by.aheart.right <= 9, `${tag}: ♡ sits right next to ⋯ (${by.amenu && Math.round(by.amenu.left - by.aheart.right)} px apart)`);
      await page.screenshot({ path: path.join(SHOTS, `hero-${width}.png`) });
    } else if (width === 1280) await page.screenshot({ path: path.join(SHOTS, `hero-${width}.png`) });

    if (!by.ashuffle || !by.amenu || !by.aheart) { check(false, `${tag}: Shuffle, ♡ and ⋯ to press (${ids})`); return; }
    // Shuffle: the whole album, with shuffle on
    await press('#ashuffle');
    await page.waitForFunction(() => { try { return JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').length === 3; } catch { return false; } }, null, { timeout: 3000 }).catch(() => {});
    const sq = await page.evaluate(() => ({ q: (JSON.parse(localStorage.getItem('pm.browserQueue') || '[]')).map((x) => x.title).sort().join(), sh: localStorage.getItem('pm.shuffle') }));
    check(sq.q === 'First,Second,Third' && sq.sh === 'on', `${tag}: Shuffle plays the whole album (the three with an address) with shuffle on (${JSON.stringify(sq)})`);
    // Play: in order, shuffle off
    await press('#aplay');
    const pq = await queueTitles(page, 'First,Second,Third');
    await waitBar(page, /First/);
    check(pq && pq.join() === 'First,Second,Third' && /First/.test(await onBar(page)), `${tag}: Play plays the album in order (${JSON.stringify(pq)}, "${await onBar(page)}")`);

    // The ⋯
    const menu = async () => {
      await press('#amenu');
      await page.waitForSelector('.popover.menu:not([hidden]) .pop-item', { timeout: 2000 }).catch(() => {});
      return page.evaluate(() => { const m = document.querySelector('.popover.menu:not([hidden])'); return m ? { title: (m.querySelector('.pop-title') || {}).textContent || '', items: [...m.querySelectorAll('.pop-item')].map((b) => b.textContent.trim()) } : null; });
    };
    const pick = (label) => (mobile ? page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: label }).first().tap() : page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: label }).first().click());
    const m = await menu();
    check(!!m && m.title === 'Journey' && m.items.join('|') === 'Play next|Add to queue|Add to collection…', `${tag}: ⋯ lists Play next, Add to queue, Add to collection… under the album's name ${JSON.stringify(m)}`);
    if (mobile) await page.screenshot({ path: path.join(SHOTS, `menu-${width}.png`) });
    if (m) {
      await pick('Add to queue');
      const q = await queueTitles(page, 'First,Second,Third,First,Second,Third');
      check(q && q.join() === 'First,Second,Third,First,Second,Third', `${tag}: ⋯ › Add to queue adds the album to the end (${JSON.stringify(q)})`);
      check(/First/.test(await onBar(page)), `${tag}: and doesn't change what plays ("${await onBar(page)}")`);
    }
    if (await menu()) {
      await pick('Play next');
      const q = await queueTitles(page, 'First,First,Second,Third,Second,Third,First,Second,Third');
      check(q && q.join() === 'First,First,Second,Third,Second,Third,First,Second,Third', `${tag}: ⋯ › Play next puts the album right after the song playing (${JSON.stringify(q)})`);
    } else check(false, `${tag}: ⋯ › Play next (no menu)`);
    if (await menu()) {
      await pick('Add to collection');
      await page.waitForSelector('dialog [data-ppi]', { timeout: 2000 }).catch(() => {});
      const n0 = state.hits.length;
      const pl = page.locator('dialog [data-ppi]', { hasText: 'Road trip' });
      if (await pl.count()) { if (mobile) await pl.first().tap(); else await pl.first().click(); }
      await page.waitForFunction(() => !document.querySelector('dialog[open]'), null, { timeout: 2000 }).catch(() => {});
      await page.waitForTimeout(200);
      const post = state.hits.slice(n0).find((h) => h.m === 'POST' && h.v === '/playlists/p1/items');
      const uris = post ? JSON.parse(post.body || '{}').uris : null;
      check(!!post && JSON.stringify(uris) === JSON.stringify(tracks.map((t) => t.url)), `${tag}: ⋯ › Add to collection… › Road trip adds the album's three songs (${JSON.stringify(uris)})`);
    } else check(false, `${tag}: ⋯ › Add to collection… (no menu)`);

    // The heart: its title says what a press does, aria-pressed says the state
    await press('#aheart');
    await page.waitForFunction(() => { const b = document.querySelector('#aheart'); return b && b.getAttribute('aria-pressed') === 'true'; }, null, { timeout: 3000 }).catch(() => {});
    const h = await page.evaluate(() => { const b = document.querySelector('#aheart'); return b && { p: b.getAttribute('aria-pressed'), t: b.title, on: b.classList.contains('on') }; });
    check(h && h.p === 'true' && h.t === 'Remove from Bookmarks' && h.on, `${tag}: a press on ♡ bookmarks the album and says so (${JSON.stringify(h)})`);
    check(!state.errors.length, `${tag}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
}

// Android's larger font size (the app's WebView applies it as text zoom): the four stay on one row,
// as a library album's do (Shuffle, then Play, drop their icon first)
const ZOOM_ARGS = ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'];
async function zoomRun(browser, width, k) {
  const tag = `${width} px phone at ${Math.round(k * 100)} %`;
  const { page, context } = await fx.create(browser, width, { fixtures: fixturesFor(item), mobile: true });
  try {
    await openItem(page);
    await page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });
    await page.waitForTimeout(250);
    const r = await page.evaluate(() => {
      const kids = [...document.querySelectorAll('#main .artist-hero .actions > *')].map((e) => { const b = e.getBoundingClientRect(); return { id: e.id, mid: b.top + b.height / 2, right: b.right, h: b.height }; });
      return { kids, font: parseFloat(getComputedStyle(document.querySelector('#aplay')).fontSize), over: document.documentElement.scrollWidth - innerWidth, inner: innerWidth };
    });
    check(r.font > 14 * (1 + (k - 1) / 2), `${tag}: the larger font took effect (${r.font} px)`);
    const mids = r.kids.map((x) => x.mid);
    check(r.kids.length === 4 && Math.max(...mids) - Math.min(...mids) <= 1, `${tag}: Play, Shuffle, ♡ and ⋯ stay on one row (${r.kids.map((x) => `${x.id} ${Math.round(x.mid)}`).join(', ')})`);
    const m = r.kids.find((x) => x.id === 'amenu');
    check(!!m && Math.abs(m.right - (r.inner - 14)) <= 1 && r.over <= 0, `${tag}: ⋯ ends at the right edge, no sideways scroll (${m && Math.round(m.right)}, ${r.over})`);
    check(r.kids.every((x) => x.h >= 44), `${tag}: each still at least 44 px tall`);
    await page.screenshot({ path: path.join(SHOTS, `zoom-${width}-${Math.round(k * 100)}.png`) });
  } finally { await context.close(); }
}

(async () => {
  const zoomed = await chromium.launch({ args: ZOOM_ARGS });
  try { for (const [w, k] of [[360, 1.3], [360, 1.5], [360, 2], [412, 2]]) await zoomRun(zoomed, w, k); } finally { await zoomed.close(); }
  const browser = await chromium.launch();
  try {
    for (const w of [360, 412, 768, 1280]) await run(browser, w);
    for (const w of [360, 1280]) await run(browser, w, { inLibrary: true });
  } finally { await browser.close(); }
  console.log(`${passed} ok, ${failed} FAIL`);
  if (failed) process.exit(1);
})();
