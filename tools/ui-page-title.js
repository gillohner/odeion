// The browser tab names the page. Before, every page's title was just "Odeion": a row of tabs,
// the long-press Back list and the history all read "Odeion, Odeion, Odeion", and a screen reader
// heard the same name on every page. Now the tab says the page, with the name its Back button
// gives it: "Albums · Odeion", "Journey · Odeion" (an album), "Music and notes · Odeion" (a
// playlist), "Results for “song” · Odeion" (an empty Search: "Search · Odeion"); a smart playlist
// by its heading; an artist's View all albums "Albums by Sefa · Odeion" (and the album opened
// from there says "Albums by Sefa" on its Back); Home is "Odeion" (the app's own name,
// tools/ui-brand.js). An album or an artist still loading says "Odeion", not the page it was
// opened from; a song that fails to load says "Odeion", not the playlist before it; Back gives
// the last page's title again. At 1280, 1024 and 768 px.
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-page-title.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'page-title-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };
const titled = (page, t, timeout = 4000) => page.waitForFunction((x) => document.title === x, t, { timeout }).then(() => true, () => false);

// (an item page's answer, as tools/ui-busy.js's)
const item = (kind, id, title, more = {}) => ({ kind, id, uri: null, target: {}, title, artist: null, album: null, year: 2020, image: null, source: 'library', source_id: id.slice(4), plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, songs: [], total_ms: 0, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library', ...more });
const album = item('album', 'lib-7', 'Journey', { artist: { name: 'Sefa', id: 'lib-11' } });
const extras = { body: { collections: [], related: [], concerts: [], similar: [] } };

(async () => {
  const browser = await chromium.launch(process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {});
  for (const width of [1280, 1024, 768]) {
    const { context, page, state } = await fx.create(browser, width, {
      collections: [fx.summary],
      // (the album answers after 1.2 s, so the page is seen while it loads; the song fails)
      delay: (v) => (v === '/item/album/lib-7' || v === '/item/artist/lib-11' ? 1200 : 0),
      fixtures: {
        '/library/albums': { body: [{ id: 7, title: 'Journey', album_artist: 'Sefa', year: 2019 }] },
        '/item/album/lib-7': { body: album },
        '/item/album/lib-7/extras': extras,
        '/item/artist/lib-11': { body: item('artist', 'lib-11', 'Sefa', { albums: [{ id: 'lib-7', title: 'Journey', year: 2019, release_type: 'album', released: '2019-05-01' }] }) },
        '/item/artist/lib-11/extras': extras,
        '/item/song/lib-1': { body: { error: 'Not found' }, status: 404 },
        '/smart/s1/tracks': { body: { playlist: { id: 's1', name: 'Late night' }, tracks: [] } },
      },
    });
    const at = `${width}px`;
    await fx.go(page, '/');
    check(await titled(page, 'Odeion'), `${at}: Home's tab says just Odeion (got "${await page.title()}")`);
    for (const [url, want] of [['/tracks', 'Songs · Odeion'], ['/playlists', 'Playlists · Odeion'], ['/queue', 'Queue · Odeion'], ['/settings', 'Settings · Odeion'], ['/search?q=song', 'Results for “song” · Odeion'], ['/search', 'Search · Odeion'], ['/smart/s1', 'Late night · Odeion']]) {
      await fx.go(page, url);
      check(await titled(page, want), `${at}: ${url} names its tab "${want}" (got "${await page.title()}")`);
    }

    // Albums › an album (opened by a click, as a person would): "Odeion" while it loads, then its name
    await fx.go(page, '/albums');
    check(await titled(page, 'Albums · Odeion'), `${at}: Albums names its tab (got "${await page.title()}")`);
    await page.locator('#main .tile[data-album]').first().click();
    await page.waitForURL(/\/album\/lib-7$/);
    await page.waitForTimeout(300);
    const loadingTitle = await page.title();
    check(loadingTitle === 'Odeion', `${at}: an album still loading doesn't keep the last page's name (got "${loadingTitle}")`);
    check(await titled(page, 'Journey · Odeion'), `${at}: the album's tab says its name (got "${await page.title()}")`);
    if (width === 1280) await page.screenshot({ path: path.join(SHOTS, `album-${width}.png`) });
    // The album › its artist (still loading): not named after the album
    await page.locator('#main [data-crumb-kind="artist"]').first().click();
    await page.waitForURL(/\/artist\/lib-11$/);
    await page.waitForTimeout(300);
    const artistLoading = await page.title();
    check(artistLoading === 'Odeion', `${at}: an artist still loading isn't named after the album it was opened from (got "${artistLoading}")`);
    check(await titled(page, 'Sefa · Odeion'), `${at}: the artist's tab says their name (got "${await page.title()}")`);
    await page.goBack();
    check(await titled(page, 'Journey · Odeion'), `${at}: Back gives the album's title again (got "${await page.title()}")`);
    await page.goBack();
    check(await titled(page, 'Albums · Odeion'), `${at}: Back again gives Albums' title (got "${await page.title()}")`);

    // A playlist, then its song (whose page fails): the song's tab isn't named after the playlist
    await fx.go(page, '/playlist/c1');
    check(await titled(page, 'Music and notes · Odeion'), `${at}: a playlist's tab says its name (got "${await page.title()}")`);
    await page.locator('#main a[data-ssong="0"]').click();
    await page.waitForURL(/\/song\/lib-1$/);
    const failedShown = await page.waitForFunction(() => /Not found/.test(document.querySelector('#main').textContent), null, { timeout: 4000 }).then(() => true, () => false);
    // (watched for a while after it draws: the title set after the draw too)
    const seen = new Set(); for (let k = 0; k < 6; k++) { seen.add(await page.title()); await page.waitForTimeout(100); }
    check(failedShown && seen.size === 1 && seen.has('Odeion'), `${at}: a page that failed to load isn't named after the playlist before it (got ${[...seen].map((t) => `"${t}"`).join(', ')})`);
    await page.goBack();
    check(await titled(page, 'Music and notes · Odeion'), `${at}: Back gives the playlist's title again (got "${await page.title()}")`);

    // An artist's View all albums: whose albums, not the library's Albums; the album opened from it
    // says the same on its Back (shown on a phone; at these widths the breadcrumb stands in for it)
    await fx.go(page, '/artist/lib-11/releases/albums');
    check(await titled(page, 'Albums by Sefa · Odeion'), `${at}: an artist's albums name the artist in the tab (got "${await page.title()}")`);
    await page.locator('#main .tile').first().click();
    await page.waitForURL(/\/album\/lib-7$/);
    check(await titled(page, 'Journey · Odeion'), `${at}: the album opened from them is named (got "${await page.title()}")`);
    const relBack = ((await page.locator('#main .back').first().textContent().catch(() => '')) || '').trim();
    check(relBack === 'Albums by Sefa', `${at}: its Back says whose albums (got "${relBack}")`);
    if (width === 1280) await page.screenshot({ path: path.join(SHOTS, `release-back-${width}.png`) });
    await page.goBack();
    check(await titled(page, 'Albums by Sefa · Odeion'), `${at}: Back gives the artist's albums' title again (got "${await page.title()}")`);

    check(!state.errors.length, `${at}: no page errors ${state.errors.join(' | ')}`);
    await context.close();
  }
  await browser.close();
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
