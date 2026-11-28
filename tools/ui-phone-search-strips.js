// On a phone, Search's Artists and Albums are one row of covers that scrolls sideways, not a grid two
// covers wide: six artists and six albums took six rows of covers, so the first song started about four
// screens down. Part of a third cover shows, so the row says it goes on. Tablet and desktop keep the grid,
// so does a phone's Search filtered to one kind (no songs follow), and the Phone layout on a wide window
// keeps the grid's cover size.
// Real candidate assets, isolated responses: node tools/ui-phone-search-strips.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-search-strips-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Six of each, as a real search ("love", "night") finds; artist 11 and album 7 have pages in the fixture
const NAMES = ['Journey', 'Journey Band', 'Journey South', 'Journeyman', 'The Journey', 'Journey Home'];
const searchHits = [
  ...NAMES.map((t, i) => ({ kind: 'artist', id: i ? `mb-artist-${i}` : 'lib-11', title: `${t} Artists`, subtitle: '', keys: [`artist:${i ? `mb${i}` : 11}`] })),
  ...NAMES.map((t, i) => ({ kind: 'album', id: i ? `mb-album-${i}` : 'lib-7', title: `${t} Record`, subtitle: 'Sefa', keys: [`album:${i ? `mb${i}` : 7}`] })),
  ...NAMES.map((t, i) => ({ kind: 'song', id: `lib-${i + 1}`, title: `${t} Song`, subtitle: 'Sefa', keys: [`track:${i + 1}`], track_id: i + 1 })),
];
const tracks = NAMES.map((t, i) => ({ ...fx.track, id: i + 1, title: `${t} Song` }));

// A cover's place against its row's inner edges (the row clips at its padding box)
const inRow = (loc) => loc.evaluate((el) => { const b = el.getBoundingClientRect(); const row = el.parentElement; const rb = row.getBoundingClientRect(); const st = getComputedStyle(row);
  return { tile: el.matches('.tile'), left: Math.round(b.left), right: Math.round(b.right), inner: [Math.round(rb.left + parseFloat(st.paddingLeft)), Math.round(rb.right - parseFloat(st.paddingRight))] }; });
const inside = (c) => c.left >= c.inner[0] - 1 && c.right <= c.inner[1] + 1;
const measure = (page) => page.evaluate(() => {
  const r = (el) => el && el.getBoundingClientRect();
  const sec = (k) => {
    const s = document.querySelector(`#sresults [data-skind="${k}"]`); if (!s) return null;
    const tiles = [...s.querySelectorAll('.tile')].map(r);
    const row = s.querySelector('.tile')?.parentElement;
    return { tiles: tiles.length, rows: new Set(tiles.map((t) => Math.round(t.top))).size, height: Math.round(r(s).height),
      scrolls: !!row && row.scrollWidth > row.clientWidth + 1, rowWidth: row ? Math.round(r(row).width) : 0,
      tileWidth: tiles[0] ? Math.round(tiles[0].width) : 0, rowRight: row ? Math.round(r(row).right) : 0,
      third: tiles[2] && row ? Math.round(r(row).right - tiles[2].left) : 0,
      firstLeft: tiles[0] ? Math.round(tiles[0].left) : null, headLeft: Math.round(r(s.querySelector('h3')).left) };
  };
  const main = r(document.querySelector('#main'));
  const song = r(document.querySelector('#sresults [data-skind="song"] .list > *:first-child'));
  const mini = r(document.querySelector('footer.player'));
  return { shell: document.documentElement.dataset.shell, artist: sec('artist'), album: sec('album'),
    songDown: song ? Math.round(song.top - main.top) : null, screen: Math.round((mini ? mini.top : innerHeight) - main.top),
    overflowX: document.documentElement.scrollWidth - innerWidth, mainOverflowX: document.querySelector('#main').scrollWidth - document.querySelector('#main').clientWidth };
});

(async () => {
  const browser = await chromium.launch();
  try {
    for (const android of [false, true]) for (const [w, h] of [[360, 640], [412, 732]]) {
      const tag = `${android ? 'android' : 'browser'}-${w}`;
      const run = await fx.create(browser, w, { mobile: true, searchHits, tracks });
      if (android) await run.context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
      const { page, context } = run;
      try {
        await page.setViewportSize({ width: w, height: h });
        await fx.go(page, '/search?q=journey');
        await page.waitForSelector('#sresults [data-skind="song"] .list > *:first-child'); await page.waitForTimeout(300);
        const m = await measure(page);
        check(m.shell === 'phone', `${tag}: phone shell (${m.shell})`);
        for (const k of ['artist', 'album']) {
          const s = m[k];
          check(s && s.tiles === 6 && s.rows === 1, `${tag}: ${k}s are one row of covers (${s && `${s.tiles} covers on ${s.rows} rows, ${s.height} px tall`})`);
          check(s && s.scrolls && s.rowRight <= w, `${tag}: the ${k} row scrolls sideways inside the page (${s && `row ${s.rowWidth} px, ends at ${s.rowRight}`})`);
          // (two whole covers and a part of the third: the row shows it goes on)
          check(s && s.firstLeft === s.headLeft, `${tag}: the first ${k} cover lines up with the heading (${s && `${s.firstLeft} vs ${s.headLeft}`})`);
          check(s && s.tileWidth >= 120 && s.third >= 24 && s.third < s.tileWidth / 2, `${tag}: two ${k} covers and part of a third show (${s && `covers ${s.tileWidth} px, ${s.third} px of the third`})`);
        }
        check(m.overflowX <= 0 && m.mainOverflowX <= 0, `${tag}: the page itself doesn't scroll sideways (document ${m.overflowX}, main ${m.mainOverflowX})`);
        // Before: the first song began 3.5 to 3.8 screens down
        check(m.songDown != null && m.songDown < 2 * m.screen, `${tag}: the first song is less than two screens down (${m.songDown} px, a screen is ${m.screen} px)`);
        await page.screenshot({ path: path.join(SHOTS, `${tag}-search.png`) });
        // A swipe along the row shows the later covers; a tap on one opens its page
        const row = page.locator('#sresults [data-skind="artist"] .tile').first().locator('..');
        await row.evaluate((el) => { el.scrollLeft = el.scrollWidth; });
        await page.waitForTimeout(150);
        const last = await inRow(page.locator('#sresults [data-skind="artist"] .tile').last());
        check(inside(last), `${tag}: the last artist comes fully into view at the row's end (${last.left}-${last.right} in ${last.inner.join('-')})`);
        await page.screenshot({ path: path.join(SHOTS, `${tag}-search-swiped.png`) });
        // A swipe that ends near the start settles with the first cover in line with the heading again
        await row.evaluate((el) => { el.scrollLeft = 3; });
        await page.waitForFunction(() => { const s = document.querySelector('#sresults [data-skind="artist"]'); return Math.round(s.querySelector('.tile').getBoundingClientRect().left) === Math.round(s.querySelector('h3').getBoundingClientRect().left); }, null, { timeout: 2000 }).catch(() => {});
        const again = (await measure(page)).artist;
        check(again.firstLeft === again.headLeft, `${tag}: swiped back, the first cover rests in line with the heading (${again.firstLeft} vs ${again.headLeft})`);
        // The cover's own Play plays it, from Search
        const asked = run.state.hits.length;
        await page.locator('#sresults [data-skind="artist"] [data-tileplay]').first().tap();
        await page.waitForTimeout(400);
        const got = run.state.hits.slice(asked).map((x) => x.v);
        check(got.includes('/item/artist/lib-11') && new URL(page.url()).pathname === '/search', `${tag}: a cover's Play asks for that artist's songs and stays on Search (${got.join(', ')})`);
        await page.locator('#sresults [data-skind="artist"] .tile').first().tap();
        await page.waitForFunction(() => location.pathname.startsWith('/artist/'), null, { timeout: 3000 }).catch(() => {});
        check(new URL(page.url()).pathname === '/artist/lib-11', `${tag}: a tapped cover opens its page (${new URL(page.url()).pathname})`);
        check(!run.state.errors.length, `${tag}: no page errors (${run.state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // A keyboard reaches a cover off to the right: the row brings it into view (a tablet's keyboard
    // in the phone layout); wider than a phone the grid stays
    {
      const run = await fx.create(browser, 360, { searchHits, tracks });
      const { page, context } = run;
      try {
        await page.setViewportSize({ width: 360, height: 640 });
        await fx.go(page, '/search?q=journey');
        await page.waitForSelector('#sresults [data-skind="song"] .list > *:first-child'); await page.waitForTimeout(300);
        // Tab from the first album to the fifth (each cover, then its Play)
        await page.locator('#sresults [data-skind="album"] .tile').first().focus();
        for (let i = 0; i < 16 && !(await page.evaluate(() => document.activeElement?.dataset.tile === '4')); i++) await page.keyboard.press('Tab');
        await page.waitForTimeout(200);
        const f = await inRow(page.locator(':focus'));
        check(f.tile && inside(f), `360 keyboard: Tab to the fifth album scrolls it into view (${f.left}-${f.right} in ${f.inner.join('-')})`);
        // Every cover on the way is shown whole, not only every other one; and back with Shift+Tab
        const seen = [];
        await page.locator('#sresults [data-skind="album"] .tile').first().focus();
        for (let i = 0; i < 16 && seen.length < 5; i++) { await page.keyboard.press('Tab'); const c = await inRow(page.locator(':focus')); if (c.tile) seen.push(c); }
        await page.keyboard.press('Shift+Tab'); await page.keyboard.press('Shift+Tab');
        const backTab = await inRow(page.locator(':focus'));
        check(seen.length === 5 && seen.every(inside) && backTab.tile && inside(backTab), `360 keyboard: each cover Tab reaches is whole in the row, and with Shift+Tab (${seen.map((c) => `${c.left}-${c.right}`).join(', ')}; back ${backTab.left}-${backTab.right})`);
        await page.setViewportSize({ width: 768, height: 900 }); await page.waitForTimeout(300);
        const wide = await measure(page);
        check(wide.shell !== 'phone' && wide.artist.rows > 1 && !wide.artist.scrolls && wide.album.rows > 1, `768: tablet keeps the grid (${wide.shell}, artists on ${wide.artist.rows} rows, albums on ${wide.album.rows})`);
        await page.screenshot({ path: path.join(SHOTS, 'tablet-768-search.png') });
        await page.setViewportSize({ width: 360, height: 640 }); await page.waitForTimeout(300);
        const back = await measure(page);
        check(back.shell === 'phone' && back.artist.rows === 1 && back.album.rows === 1, `back at 360 px: one row each again (artists ${back.artist.rows}, albums ${back.album.rows})`);
        // Filtered to Albums (no songs follow): the covers you asked for stay a grid, two to a row
        await page.locator('[data-sfilters]').click(); await page.waitForTimeout(150);
        await page.locator('#sfacets [data-facet="kind:album"]').click(); await page.waitForTimeout(300);
        const only = await measure(page);
        check(!only.artist && only.album && only.album.rows === 3 && !only.album.scrolls, `360 filtered to Albums: a grid two covers wide (${only.album && `${only.album.tiles} on ${only.album.rows} rows`})`);
        await page.screenshot({ path: path.join(SHOTS, 'phone-360-albums-only.png') });
      } finally { await context.close(); }
    }
    // The Phone layout on a wide window: a row still, with covers the grid's size, not 40 % of the window
    {
      const run = await fx.create(browser, 1024, { searchHits, tracks });
      const { page, context } = run;
      try {
        await context.addInitScript(() => { localStorage.setItem('pm.layout', 'phone'); });
        await fx.go(page, '/search?q=journey');
        await page.waitForSelector('#sresults [data-skind="song"] .list > *:first-child'); await page.waitForTimeout(300);
        const m = await measure(page);
        check(m.shell === 'phone' && m.artist.rows === 1 && m.artist.tileWidth <= 168 && m.album.tileWidth <= 168, `1024 Phone layout: one row of covers at most the grid's size (${m.shell}, ${m.artist.rows} row, ${m.artist.tileWidth} px)`);
        await page.screenshot({ path: path.join(SHOTS, 'phone-layout-1024.png') });
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})();
