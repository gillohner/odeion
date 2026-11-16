// A song row's buttons say which song they act on. The heart already did ("Add Journey to
// favourites"), but Play from here, Play next, Add to queue and ⋯ were named only by their
// tooltip, so a screen reader or a keyboard user going down a list heard "Play next, button" on
// every row with no song named. Now each is named after its song ("Play Journey", "Play Journey
// next", "Add Journey to the queue", "More for Journey") on Songs (trackRows, also Favourites)
// and on album pages (songRows, also artist and playlist pages); the short tooltips stay. A
// playlist's link row (cardRow) said "More: <name>", and kept the address after its title came;
// now "More for <title>" (and "Open <title> (new tab)"), as the songs beside it. At 1280, 1024 and 768 px.
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-row-names.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'row-names-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

// Three songs; the second's title has quotes and an ampersand (escaped in the name, not cut short)
const titles = ['Journey Through Time', 'Rock & "Roll"', 'Night Drive'];
const tracks = titles.map((title, i) => ({ ...fx.track, id: i + 1, title, mbid: null, canonical_uri: null }));
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: {}, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: tracks.map((t, i) => ({ no: i + 1, title: t.title, artist: 'Sefa', artist_id: 'lib-11', album_id: 'lib-7', duration_ms: 240000, uri: `track:${t.id}`, track_id: t.id, id: `lib-${t.id}`, page: `lib-${t.id}` })), total_ms: 720000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'albums' };
const fixtures = {
  '/item/album/lib-7': { body: albumPage },
  '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
};

// Each page: where it is, the row selector, and the buttons every row must name after its song
const pages = [
  { name: 'songs', url: '/tracks', row: '.main .track', buttons: [['[data-act="play"]', 'Play from here', (t) => `Play ${t}`], ['[data-act="next"]', 'Play next', (t) => `Play ${t} next`], ['[data-act="add"]', 'Add to queue', (t) => `Add ${t} to the queue`], ['[data-act="more"]', 'More', (t) => `More for ${t}`], ['[data-act="fav"]', 'Add to favourites', (t) => `Add ${t} to favourites`]] },
  { name: 'album', url: '/album/lib-7', row: '.main .srow', buttons: [['[data-splay]', 'Play from here', (t) => `Play ${t}`], ['[data-smore]', 'More', (t) => `More for ${t}`], ['[data-sfav]', 'Add to favourites', (t) => `Add ${t} to favourites`]] },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      for (const pg of pages) {
        const { page, context, state } = await fx.create(browser, width, { tracks, fixtures });
        try {
          await fx.go(page, pg.url);
          const drawn = await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length === n, [pg.row, titles.length], { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${width} ${pg.name}: ${titles.length} song rows drawn`);
          if (!drawn) continue;
          for (const [sel, tip, name] of pg.buttons) {
            // The name a screen reader reads (computed by the browser), per row in order
            const got = await page.$$eval(`${pg.row} ${sel}`, (bs) => bs.map((b) => ({ label: b.getAttribute('aria-label'), title: b.getAttribute('title') })));
            check(got.length === titles.length, `${width} ${pg.name}: every row has its ${tip} button (${got.length})`);
            titles.forEach((t, i) => {
              check(got[i] && got[i].label === name(t), `${width} ${pg.name}: row ${i + 1}'s ${tip} is named "${name(t)}" (${got[i] && got[i].label})`);
              check(got[i] && got[i].title === tip, `${width} ${pg.name}: row ${i + 1}'s tooltip still says "${tip}" (${got[i] && got[i].title})`);
            });
            // The accessibility tree agrees: one button with that exact name per song
            for (const t of titles) {
              const n = await page.getByRole('button', { name: name(t), exact: true }).count();
              check(n === 1, `${width} ${pg.name}: exactly one button is named "${name(t)}" (${n})`);
            }
          }
          // Keyboard: the ⋯ named for the second song opens that song's menu
          const more = page.getByRole('button', { name: `More for ${titles[1]}`, exact: true });
          const menu = await more.count() === 1 && await more.focus({ timeout: 2000 }).then(() => page.keyboard.press('Enter')).then(() => page.waitForSelector('.popover.menu:not([hidden])', { timeout: 3000 })).then(() => true, () => false);
          check(menu, `${width} ${pg.name}: Enter on "More for ${titles[1]}" opens its menu`);
          // (the button so named is the one in that song's row)
          const inRow = await page.evaluate((sel) => { const rows = [...document.querySelectorAll(sel)]; return rows.indexOf(document.activeElement && document.activeElement.closest(sel)); }, pg.row);
          check(inRow === 1, `${width} ${pg.name}: and it is the ⋯ in that song's row (row ${inRow + 1})`);
          await page.screenshot({ path: path.join(SHOTS, `${pg.name}-${width}.png`) });
          await page.keyboard.press('Escape');
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${width} ${pg.name}: nothing wider than the screen (${o})`);
          check(!state.errors.length, `${width} ${pg.name}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // A playlist's link row (cardRow) in the same list: its ⋯ uses the same words ("More for", was "More: ")
      const { page, context, state } = await fx.create(browser, width);
      try {
        await fx.go(page, '/playlist/c1');
        // (after the link's page has answered: the row then shows its title, Liner notes)
        const ok = await page.waitForFunction(() => [...document.querySelectorAll('.main .srow[data-card] .name')].some((n) => n.textContent.trim() === 'Liner notes'), null, { timeout: 8000 }).then(() => true, () => false);
        check(ok, `${width} playlist: the link row is drawn`);
        if (ok) {
          const r = await page.$$eval('.main .srow', (rows) => rows.map((row) => ({ name: row.querySelector('.name')?.textContent.trim(), more: row.querySelector('[data-smore], [data-cmore]')?.getAttribute('aria-label') })));
          check(r.length === 2 && r.every((x) => x.more === `More for ${x.name}`), `${width} playlist: the song's and the link's ⋯ are both "More for <its name>" (${r.map((x) => x.more).join(' / ')})`);
          const open = await page.$eval('.main .srow[data-card] a[target="_blank"]', (a) => a.getAttribute('aria-label')).catch(() => null);
          check(open === 'Open Liner notes (new tab)', `${width} playlist: the link's Open says its title too (${open})`);
        }
        check(!state.errors.length, `${width} playlist: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${passed} ok, ${failed} FAIL` : `${passed} ok, all passed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
