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
  { name: 'songs', url: '/tracks', row: '.main .track', buttons: [['[data-act="play"]', 'Play from here', (t) => `Play ${t}`], ['[data-act="next"]', 'Play next', (t) => `Play ${t} next`], ['[data-act="add"]', 'Add to queue', (t) => `Add ${t} to the queue`], ['[data-act="more"]', 'More', (t) => `More for ${t}`], ['[data-act="fav"]', 'Add to Bookmarks', (t) => `Add ${t} to Bookmarks`]] },
  { name: 'album', url: '/album/lib-7', row: '.main .srow', buttons: [['[data-splay]', 'Play from here', (t) => `Play ${t}`], ['[data-smore]', 'More', (t) => `More for ${t}`], ['[data-sfav]', 'Add to Bookmarks', (t) => `Add ${t} to Bookmarks`]] },
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
          // (the button so named is the one in that song's row; the open menu has the focus now, so
          // the button itself is asked, not the focused element)
          const inRow = await more.count() === 1 ? await more.evaluate((b, sel) => [...document.querySelectorAll(sel)].indexOf(b.closest(sel)), pg.row) : -1;
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
        check(ok, `${width} collection: the link row is drawn`);
        if (ok) {
          const r = await page.$$eval('.main .srow', (rows) => rows.map((row) => ({ name: row.querySelector('.name')?.textContent.trim(), more: row.querySelector('[data-smore], [data-cmore]')?.getAttribute('aria-label') })));
          check(r.length === 2 && r.every((x) => x.more === `More for ${x.name}`), `${width} collection: the song's and the link's ⋯ are both "More for <its name>" (${r.map((x) => x.more).join(' / ')})`);
          const open = await page.$eval('.main .srow[data-card] a[target="_blank"]', (a) => a.getAttribute('aria-label')).catch(() => null);
          check(open === 'Open Liner notes (new tab)', `${width} collection: the link's Open says its title too (${open})`);
        }
        check(!state.errors.length, `${width} collection: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
      // The Queue: a row's Play from here (it showed the row's number, so it was read as "2"), its
      // grip, Move up, Move down and Remove named after the song too; the short tooltips stay
      {
        const saved = tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));
        const qfix = {}; for (const t of tracks) qfix[`/library/tracks/${t.id}`] = { body: t };
        const { page, context, state } = await fx.create(browser, width, { tracks, fixtures: qfix });
        try {
          await context.addInitScript((q) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', JSON.stringify({ index: 0, id: 1, pos: 0 })); }, JSON.stringify(saved));
          await fx.go(page, '/queue');
          const drawn = await page.waitForFunction((n) => document.querySelectorAll('.queue [data-qrow]').length === n, titles.length, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${width} queue: ${titles.length} rows drawn`);
          if (drawn) {
            const qButtons = [['[data-qi]', 'Play from here', (t) => `Play ${t}`], ['[data-qgrip]', 'Drag to reorder', (t) => `Drag ${t} to reorder (or Alt+↑/↓)`], ['[data-qup]', 'Move up', (t) => `Move ${t} up`], ['[data-qdown]', 'Move down', (t) => `Move ${t} down`], ['[data-qrm]', 'Remove', (t) => `Remove ${t} from the queue`], ['[data-qmore]', 'More', (t) => `More for ${t}`]];
            const rowsNow = () => page.$$eval('.queue [data-qrow]', (rows) => rows.map((r) => r.querySelector('.name').textContent.trim()));
            for (const [sel, tip, name] of qButtons) {
              const got = await page.$$eval(`.queue [data-qrow] ${sel}`, (bs) => bs.map((b) => ({ label: b.getAttribute('aria-label'), title: b.getAttribute('title') })));
              titles.forEach((t, i) => {
                check(got[i] && got[i].label === name(t), `${width} queue: row ${i + 1}'s ${tip} is named "${name(t)}" (${got[i] && got[i].label})`);
                check(got[i] && got[i].title === tip, `${width} queue: row ${i + 1}'s tooltip still says "${tip}" (${got[i] && got[i].title})`);
              });
            }
            // What the browser computes for the row's play button (not its number) and its Remove
            for (const t of titles) {
              for (const n of [`Play ${t}`, `Remove ${t} from the queue`]) {
                const c = await page.getByRole('button', { name: n, exact: true }).count();
                check(c === 1, `${width} queue: exactly one button is named "${n}" (${c})`);
              }
            }
            // Keyboard: "Move <second song> down" moves that song, and the names follow the songs
            const down = page.getByRole('button', { name: `Move ${titles[0]} down`, exact: true });
            const pressed = await down.focus({ timeout: 2000 }).then(() => page.keyboard.press('Enter')).then(() => true, () => false);
            check(pressed, `${width} queue: a button named "Move ${titles[0]} down" takes the focus`);
            const moved = pressed && await page.waitForFunction((want) => [...document.querySelectorAll('.queue [data-qrow] .name')].map((n) => n.textContent.trim()).join('|') === want, [titles[1], titles[0], titles[2]].join('|'), { timeout: 5000 }).then(() => true, () => false);
            check(moved, `${width} queue: Enter on "Move ${titles[0]} down" moves that song (${(await rowsNow()).join(' / ')})`);
            const after = await page.$$eval('.queue [data-qrow]', (rows) => rows.map((r) => ({ name: r.querySelector('.name').textContent.trim(), up: r.querySelector('[data-qup]').getAttribute('aria-label'), rm: r.querySelector('[data-qrm]').getAttribute('aria-label') })));
            check(after.every((r) => r.up === `Move ${r.name} up` && r.rm === `Remove ${r.name} from the queue`), `${width} queue: after the move each row's buttons name its own song (${after.map((r) => r.up).join(' / ')})`);
            await page.hover('.queue [data-qrow="1"]');
            await page.screenshot({ path: path.join(SHOTS, `queue-${width}.png`) });
          }
          check(!state.errors.length, `${width} queue: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // A playlist's rows: Move up, Move down and Remove name the song, and a link row's are renamed
      // with its title when its page answers (as its ⋯ and Open are)
      {
        const { page, context, state } = await fx.create(browser, width);
        try {
          await fx.go(page, '/playlist/c1');
          const ok = await page.waitForFunction(() => [...document.querySelectorAll('.main .srow[data-card] .name')].some((n) => n.textContent.trim() === 'Liner notes'), null, { timeout: 8000 }).then(() => true, () => false);
          check(ok, `${width} collection rows: drawn, with the link's title`);
          if (ok) {
            const r = await page.$$eval('#plsongs .srow', (rows) => rows.map((row) => ({ name: row.querySelector('.name').textContent.trim(), up: row.querySelector('[data-pup]')?.getAttribute('aria-label'), down: row.querySelector('[data-pdown]')?.getAttribute('aria-label'), rm: row.querySelector('[data-rm]')?.getAttribute('aria-label'), tips: ['[data-pup]', '[data-pdown]', '[data-rm]'].map((s) => row.querySelector(s)?.getAttribute('title')).join('|') })));
            check(r.length === 2, `${width} collection rows: a song and a link (${r.length})`);
            for (const x of r) {
              check(x.up === `Move ${x.name} up` && x.down === `Move ${x.name} down`, `${width} collection rows: "${x.name}" has Move ${x.name} up / down (${x.up} / ${x.down})`);
              check(x.rm === `Remove ${x.name} from this collection`, `${width} collection rows: "${x.name}" has Remove ${x.name} from this collection (${x.rm})`);
              check(x.tips === 'Move up|Move down|Remove', `${width} collection rows: "${x.name}"'s tooltips unchanged (${x.tips})`);
              // (and what the browser computes: one button so named, on this row)
              const c = await page.getByRole('button', { name: `Remove ${x.name} from this collection`, exact: true }).count();
              check(c === 1, `${width} collection rows: exactly one button is named "Remove ${x.name} from this collection" (${c})`);
            }
          }
          check(!state.errors.length, `${width} collection rows: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${passed} ok, ${failed} FAIL` : `${passed} ok, all passed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
