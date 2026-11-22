// Two library list ends.
// 1. Songs whose answer has no songs while the library count says there are some (the count and the
//    list a moment apart, e.g. a rescan between them) drew only the head, with a count and Play all
//    over nothing. Now it shows Songs' empty state and offers no Play all.
// 2. The A–Z row on Albums and Artists put "#" between O and P (and other letters) when a name began
//    with a letter that doesn't split into a base letter (Œ, Æ, Ø, Ł, Đ, Ħ, Ŋ, Ŧ…): those sort with their
//    base letter but were filed under "#". Now they are under that letter, and "#" is first or last.
// At 1280, 1024, 768 and 360 px. Isolated API fixtures; no node needed.
//   node tools/ui-songs-empty-az.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'songs-empty-az-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const stats = (tracks) => ({ tracks, albums: 0, artists: 0, files: tracks, missing_files: 0, total_duration_ms: 0 });
// 45 names (the A–Z row shows from 41), some starting with letters NFKD leaves whole
const NAMES = ['Aaron', 'Æon Flux', 'Apple', 'Bee', 'Đorđe', 'Ħal', 'Ibis', 'Lima', 'Łódź', 'Ŋoro', 'Oasis', 'Œuvre', 'Opus', 'Ørsted', 'Pink', 'Ŧana', 'Ulm', 'Zed', 'Ωmega'];
const names = [...NAMES, ...Array.from({ length: 45 - NAMES.length }, (_, i) => `Mid ${i + 1}`)];
const albums = names.map((t, i) => ({ id: i + 1, title: t, album_artist: `Band ${i + 1}`, year: 2000, tracks: 1 }));
const artists = names.map((n, i) => ({ id: i + 1, name: n, albums: 1, tracks: 1 }));

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 360]) {
      // 1. Songs: a count of 5 and an empty answer
      {
        const { page, context, state } = await fx.create(browser, width, { fixtures: { '/library/stats': { body: stats(5) }, '/library/tracks': { body: { tracks: [], next: null } } } });
        try {
          await fx.go(page, '/tracks');
          await page.waitForSelector('.main .page-head', { timeout: 8000 });
          await page.waitForTimeout(150);
          const s = await page.evaluate(() => ({
            empty: !!document.querySelector('.main .empty'), title: document.querySelector('.main .empty h3')?.textContent || '',
            play: !!document.querySelector('#shuffle'), list: !!document.querySelector('#tracklist'),
            count: document.querySelector('.main .page-head .count')?.textContent || '',
          }));
          check(s.empty && s.title === 'No songs yet', `${width} songs empty answer: the empty state "No songs yet" (${s.empty}, ${s.title})`);
          check(!s.play, `${width} songs empty answer: no Play all`);
          check(!s.list, `${width} songs empty answer: no empty song list`);
          check(!/[1-9]/.test(s.count), `${width} songs empty answer: the head counts no songs (${s.count})`);
          await page.screenshot({ path: path.join(SHOTS, `songs-empty-${width}.png`) });
          check(!state.errors.length, `${width} songs empty answer: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // ... and a normal answer still lists and offers Play all
      {
        const { page, context, state } = await fx.create(browser, width, { fixtures: { '/library/stats': { body: stats(1) } } });
        try {
          await fx.go(page, '/tracks');
          const listed = await page.waitForSelector('#tracklist .track, #tracklist [data-i]', { timeout: 8000 }).then(() => true, () => false);
          check(listed, `${width} songs: a song is listed`);
          check(await page.locator('#shuffle').count() === 1, `${width} songs: Play all is there`);
          check(!state.errors.length, `${width} songs: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // 2. The A–Z row on Albums and Artists
      for (const view of ['albums', 'artists']) {
        const { page, context, state } = await fx.create(browser, width, { fixtures: { '/library/albums': { body: albums }, '/library/albums/formats': { body: [] }, '/library/artists': { body: artists }, '/library/stats': { body: stats(45) } } });
        try {
          await page.addInitScript(([v]) => localStorage.setItem(`pm.list.${v}`, JSON.stringify({ sort: 'name' })), [view]);
          await fx.go(page, `/${view}`);
          const shown = await page.waitForSelector('[data-az]', { state: 'attached', timeout: 8000 }).then(() => true, () => false);
          check(shown, `${width} ${view}: the A–Z row is there`);
          if (!shown) continue;
          const letters = await page.$$eval('[data-az]', (bs) => bs.map((b) => b.dataset.az));
          const at = letters.indexOf('#');
          check(at === 0 || at === letters.length - 1, `${width} ${view}: "#" is first or last (${letters.join(' ')})`);
          const az = letters.filter((l) => l !== '#');
          check(az.join('') === [...az].sort().join(''), `${width} ${view}: the letters are in A–Z order (${az.join('')})`);
          for (const l of ['A', 'D', 'H', 'L', 'N', 'O', 'T']) check(az.includes(l), `${width} ${view}: ${l} is in the row`);
          // O jumps to the first O name; Œuvre and Ørsted are in the O run
          await page.locator('[data-az="O"]').click();
          await page.waitForTimeout(150);
          const sel = view === 'albums' ? '#albumgrid .tile' : '#artistlist [data-artist]';
          const first = await page.$$eval(sel, (els) => els.slice(0, 4).map((e) => e.textContent));
          check(first[0] && first[0].includes('Oasis'), `${width} ${view}: O starts at Oasis (${(first[0] || '').trim().slice(0, 40)})`);
          check(first.some((t) => t.includes('Œuvre')) && first.some((t) => t.includes('Ørsted')), `${width} ${view}: Œuvre and Ørsted come under O`);
          // "#" jumps to a name that is no letter (Ωmega sorts after the Latin names)
          await page.locator('[data-az="#"]').click();
          await page.waitForTimeout(150);
          const hash = await page.$eval(sel, (e) => e.textContent).catch(() => '');
          check(hash.includes('Ωmega'), `${width} ${view}: "#" starts at Ωmega (${hash.trim().slice(0, 40)})`);
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${width} ${view}: nothing wider than the screen (${o})`);
          await page.screenshot({ path: path.join(SHOTS, `${view}-az-${width}.png`) });
          check(!state.errors.length, `${width} ${view}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
