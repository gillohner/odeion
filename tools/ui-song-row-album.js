// Song rows in the desktop layout under 900 px (tablets, a narrow window): the album column left
// the title about ten characters at 768 px ("Song numbe…"). There the album now joins the artist
// line and the title takes the width. Wider, the album keeps its column. Either way a row has one
// album link (one Tab stop) that opens the album, and the phone's rows are unchanged.
// Real UI in Chromium against fixtures (no node), 1280 to 600 px, plus 390 px as the phone.
//   node tools/ui-song-row-album.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'song-row-album-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const names = ['Sefa', 'Dr. Peacock', 'Angerfist'];
const tracks = Array.from({ length: 12 }, (_, k) => {
  const i = k + 1;
  return { ...fx.track, id: i, title: `Song number ${i} with a fairly long title`, artists: [{ id: 11 + (i % 3), name: names[i % 3] }], album: { id: 7 + (i % 4), title: `Album ${i % 4}` }, duration_ms: 200000 + i * 1000 };
});

// Per row: the album links a person can see, where they sit, and the title's share of the row
const rowReport = () => [...document.querySelectorAll('#main .tracks[data-list="all"] .track, #main .tracks[data-list="tr"] .track')].map((r) => {
  const shown = (e) => e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden';
  const albums = [...r.querySelectorAll('[data-act="album"]')].filter(shown);
  return {
    albums: albums.map((a) => ({ text: a.textContent.trim(), inBy: !!a.closest('.by'), inCol: !!a.closest('.album-col') })),
    focusable: [...r.querySelectorAll('[data-act="album"]')].filter((a) => shown(a) && a.tabIndex >= 0).length,
    share: r.querySelector('.title').getBoundingClientRect().width / r.getBoundingClientRect().width,
    by: r.querySelector('.by').innerText.trim(),
  };
});

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 901, 900, 820, 768, 721, 720, 600, 390]) {
      const T = `${width}`;
      const { page, context, state } = await fx.create(browser, width, { tracks });
      try {
        await fx.go(page, '/tracks');
        await page.waitForFunction(() => document.querySelectorAll('#main .track').length >= 12, null, { timeout: 8000 });
        const shell = await page.evaluate(() => document.documentElement.dataset.shell);
        const rows = await page.evaluate(rowReport);
        check(rows.length >= 12, `${T}: the Songs rows are drawn (${rows.length}, ${shell} layout)`);
        const want = (k) => `Album ${(k + 1) % 4}`;
        if (shell === 'phone') {
          // The phone's rows are another lane's design: no album in the row, as before
          check(rows.every((r) => r.albums.length === 0 && !/Album/.test(r.by)), `${T}: phone rows unchanged, no album in the row (${rows[0].by})`);
        } else {
          check(rows.every((r, k) => r.albums.length === 1 && r.albums[0].text === want(k)), `${T}: each row shows its album once (${rows.slice(0, 3).map((r) => r.albums.map((a) => a.text).join('+') || 'none').join(', ')})`);
          check(rows.every((r) => r.focusable === 1), `${T}: one album link per row for the keyboard (${rows[0].focusable})`);
          if (width > 900) {
            check(rows.every((r) => r.albums[0] && r.albums[0].inCol && !/Album/.test(r.by)), `${T}: the album keeps its column; the artist line is the artist only (${rows[0].by})`);
          } else {
            check(rows.every((r, k) => r.albums[0] && r.albums[0].inBy && r.by.endsWith(`· ${want(k)}`)), `${T}: the album joins the artist line (${rows[0].by})`);
          }
          if (width > 720 && width <= 900) check(rows[0].share >= 0.35, `${T}: the title has the row's width (${Math.round(rows[0].share * 100)} % of the row)`);
        }
        const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
        check(over <= 0, `${T}: nothing wider than the window (${over})`);
        await page.screenshot({ path: path.join(SHOTS, `songs-${width}.png`) });
        if (shell !== 'phone') {
          // The album link opens the album, by click and by keyboard
          const link = page.locator('#main .track [data-act="album"]:visible').first();
          await link.click();
          const byClick = await page.waitForFunction(() => location.pathname === '/album/lib-8', null, { timeout: 6000 }).then(() => true, () => false);
          check(byClick, `${T}: a click on the row's album opens it (${await page.evaluate(() => location.pathname)})`);
          await page.goBack();
          await page.waitForFunction(() => location.pathname === '/tracks' && document.querySelectorAll('#main .track').length >= 12, null, { timeout: 8000 });
          await page.locator('#main .track [data-act="album"]:visible').first().focus();
          await page.keyboard.press('Enter');
          const byKey = await page.waitForFunction(() => location.pathname === '/album/lib-8', null, { timeout: 6000 }).then(() => true, () => false);
          check(byKey, `${T}: Enter on the row's album opens it`);
        }
        if (width === 768) {
          // The same rows elsewhere (Favourites): the album joins the artist line there too
          state.favs = [fx.SONG];
          await fx.go(page, '/favourites');
          await page.waitForSelector('#main .tracks[data-list="fav"] .track', { timeout: 8000 });
          const fav = await page.evaluate(() => { const r = document.querySelector('#main .tracks[data-list="fav"] .track'); return { by: r.querySelector('.by').innerText.trim(), col: getComputedStyle(r.querySelector('.album-col')).display }; });
          check(/· Journey$/.test(fav.by) && fav.col === 'none', `${T}: Bookmarks' rows too (${fav.by}; column ${fav.col})`);
        }
        check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
    // A long artist and album (review): the album link starts inside the artist line and the line
    // ends in "…", never a link that is focusable but cut off whole
    for (const width of [768, 650]) {
      const T = `${width} long names`;
      const long = [{ ...tracks[0], artists: [{ id: 31, name: 'Somebody With A Rather Long Artist Name Ensemble' }], album: { id: 9, title: 'An Album Title That Goes On And On For Sixty Characters Or So' } }];
      const { page, context, state } = await fx.create(browser, width, { tracks: long });
      try {
        await fx.go(page, '/tracks');
        await page.waitForSelector('#main .track .by-album [data-act="album"]', { state: 'attached', timeout: 8000 });
        const r = await page.evaluate(() => {
          const by = document.querySelector('#main .track .by'); const a = by.querySelector('[data-act="album"]');
          return { display: getComputedStyle(a).display, cut: by.scrollWidth > by.clientWidth, href: a.getAttribute('href') };
        });
        check(r.display === 'inline' && r.cut && r.href === '/album/lib-9', `${T}: the album is an inline link, so the line ends in "…", with its address (${r.display}, ${r.href})`);
        // Reached with the keyboard, the album scrolls into the line's view (the same as a later artist credit)
        await page.locator('#main .track .by [data-act="album"]').focus();
        const seen = await page.evaluate(() => {
          const by = document.querySelector('#main .track .by'); const a = by.querySelector('[data-act="album"]');
          const br = by.getBoundingClientRect(); const ar = a.getClientRects()[0];
          return { focused: document.activeElement === a, left: ar.left, right: ar.right, byLeft: br.left, byRight: br.right };
        });
        check(seen.focused && seen.left < seen.byRight - 8 && seen.right > seen.byLeft + 8, `${T}: focused, the album link is in view (${Math.round(seen.left)}-${Math.round(seen.right)} in ${Math.round(seen.byLeft)}-${Math.round(seen.byRight)})`);
        await page.screenshot({ path: path.join(SHOTS, `long-${width}.png`) });
        check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
    // A song with no artists (untagged files): under 900 px its artist line is the album alone,
    // never "· Journey"; wider, the line is empty and the album keeps its column
    for (const width of [1024, 900, 768, 600]) {
      const T = `${width} no artists`;
      const bare = [
        { ...fx.track, id: 1, title: 'No artists in the answer', artists: undefined, artist: undefined },
        { ...fx.track, id: 2, title: 'Untagged rip', artists: [] },
        { ...fx.track, id: 3, title: 'Tagged song' },
      ];
      const { page, context, state } = await fx.create(browser, width, { tracks: bare });
      try {
        await fx.go(page, '/tracks');
        await page.waitForFunction(() => document.querySelectorAll('#main .track').length >= 3, null, { timeout: 8000 });
        const by = await page.evaluate(() => [...document.querySelectorAll('#main .track .by')].map((e) => e.innerText.trim()));
        if (width > 900) {
          check(by[0] === '' && by[1] === '' && by[2] === 'Sefa, Dr. Peacock', `${T}: the album keeps its column; no artists, no artist line (${JSON.stringify(by)})`);
        } else {
          check(by[0] === 'Journey' && by[1] === 'Journey', `${T}: a song with no artists reads the album alone (${JSON.stringify(by.slice(0, 2))})`);
          check(by[2] === 'Sefa, Dr. Peacock · Journey', `${T}: a song with artists keeps "artists · album" (${JSON.stringify(by[2])})`);
          const link = await page.evaluate(() => { const a = document.querySelectorAll('#main .track .by')[1].querySelector('[data-act="album"]'); return a ? { shown: a.getClientRects().length > 0, href: a.getAttribute('href') } : null; });
          check(!!link && link.shown && link.href === '/album/lib-7', `${T}: the album is still a link there (${JSON.stringify(link)})`);
          await page.locator('#main .track').nth(1).screenshot({ path: path.join(SHOTS, `no-artists-${width}.png`) });
        }
        check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `song row album: ${failed} FAIL` : 'song row album: all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
