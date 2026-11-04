// Item pages tell switching from leaving (TASK-also-on-vs-open-in): the hero's icon row is "Also on"
// (switch to that service's page here) and "Open in" (the service's site, a new tab); the ⋯ menu has
// one "Open in…" row with a second menu (a direct row with one site) and "Also on…" the same way.
// The pages are mocked: a mix of a service that plays here and ones that only have a site.
//   node tools/ui-also-open.js <api token> <node URL>
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const problems = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 8000) => { const end = Date.now() + ms; for (;;) { if (await fn().catch(() => false)) return true; if (Date.now() > end) return false; await sleep(150); } };
const IMG = BASE + '/icon.svg';
const page = (kind, id, title, extra) => ({ kind, id, uri: null, target: {}, title, artist: kind === 'artist' ? null : { id: 'lib-7', name: 'Mock Artist' }, album: null, year: 2020, image: kind === 'artist' ? null : IMG, source: 'TIDAL', source_id: 'tidal', section: 'albums', plays_from: null, connect: [], reading: false, pending: false, songs: [], total_ms: 0, albums: [], versions: [], also_on: [], opened_as: null, plays_note: null, song_uri: null, link: 'https://tidal.com/album/5', on_pubky: null, about: [], blurb: null, open_on: [], related: [], concerts: [], ...extra });
// Your library plays here (it always counts as healthy); Spotify and Apple Music only have sites
const ALSO = [
  { page: 'lib-9', source: 'library', id: '9', label: 'Your library · album', url: '', uri: null },
  { page: 'spotify-3', source: 'spotify', id: '3', label: 'Spotify · album', url: 'https://open.spotify.com/album/3', uri: null },
  { page: 'u-apple', source: 'apple', id: 'x', label: 'Apple Music · album', url: 'https://music.apple.com/album/9', uri: null },
];
const OPEN = [
  { page: null, source: 'spotify', id: '3', label: 'Spotify · album', url: 'https://open.spotify.com/album/3', uri: null },
  { page: null, source: 'amazon', id: 'a', label: 'Amazon Music · album', url: 'https://music.amazon.com/albums/B0', uri: null },
  { page: null, source: 'apple', id: 'x', label: 'Apple Music · album', url: 'https://music.apple.com/album/9', uri: null },
  { page: null, source: 'discogs', id: 'd', label: 'Discogs · release', url: 'https://www.discogs.com/release/1', uri: null },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [w, h] of [[375, 812], [820, 1180], [1440, 900]]) {
      const ctx = await browser.newContext({ viewport: { width: w, height: h }, hasTouch: w < 1025, isMobile: w < 600 });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); }, TOKEN);
      const p = await ctx.newPage(); const errors = []; p.on('pageerror', (e) => errors.push(e.message));
      await p.route(/\/v1\/item\/album\/tidal-5$/, (r) => r.fulfill({ json: page('album', 'tidal-5', 'Mock Album', { also_on: ALSO, open_on: OPEN, songs: [{ no: 1, title: 'One', artist: 'Mock Artist', duration_ms: 1000, uri: 'tidal:track:1', track_id: null, page: null, image: IMG, id: 'tidal-1' }] }) }));
      await p.route(/\/v1\/item\/album\/lib-9$/, (r) => r.fulfill({ json: page('album', 'lib-9', 'Mock Album', { source: 'Your library', source_id: 'library', link: null }) }));
      await p.route(/\/v1\/item\/artist\/tidal-77$/, (r) => r.fulfill({ json: page('artist', 'tidal-77', 'Mock Artist', { also_on: [ALSO[0]], open_on: [OPEN[3]], link: 'https://tidal.com/artist/77' }) }));
      await p.route(/\/v1\/item\/[a-z]+\/[^/]+\/extras$/, (r) => r.fulfill({ json: { related: [], concerts: [], similar: [] } }));
      await p.goto(BASE + '/album/tidal-5'); await p.waitForSelector('.hero .actions', { timeout: 20000 });
      // The two groups
      const groups = await p.$$eval('.hero .also-icons', (gs) => gs.map((g) => ({ open: g.classList.contains('open-in'), label: g.querySelector('.also-label').textContent.trim(), items: [...g.querySelectorAll('.also-ico')].map((e) => ({ tag: e.tagName, aria: e.getAttribute('aria-label'), title: e.title, target: e.getAttribute('target'), rel: e.getAttribute('rel'), href: e.getAttribute('href') })) })));
      const also = groups.find((g) => !g.open); const open = groups.find((g) => g.open);
      check(also && also.label === 'Also on' && also.items.length === 1 && also.items[0].tag === 'BUTTON' && also.items[0].aria === 'Switch to Your library', `${w}: "Also on" holds the service that plays here, as a switch (${JSON.stringify(also && also.items)})`);
      check(open && /^Open in/.test(open.label) && open.items.length === 2 && open.items.every((i) => i.tag === 'A' && i.target === '_blank' && i.rel === 'noopener' && /^Open in .* \(new tab\)$/.test(i.aria)), `${w}: "Open in" holds the sites only, each a new-tab link (${JSON.stringify(open && open.items.map((i) => i.aria))})`);
      check(!(await p.$$eval('.hero .also-ico', (es) => es.some((e) => /^(spotify|apple|library)$/.test(e.title)))), `${w}: tooltips use display names, never the raw id`);
      check(!/Open on /.test(await p.textContent('#main')) && (await p.$$eval('#main [aria-label*="Open on "], #main [title*="Open on "]', (es) => es.length)) === 0, `${w}: nothing on the page says "Open on"`);
      // The ⋯ menu: one "Open in…" row, its second menu listing every site; "Also on…" the same way
      await p.click('#imore'); await p.waitForSelector('.popover.menu:not([hidden]) [data-mi]');
      const rows = await p.$$eval('.popover.menu:not([hidden]) [data-mi]', (bs) => bs.map((b) => b.textContent.trim()));
      check(rows.filter((r) => /^Open in/.test(r)).length === 1 && rows.includes('Open in…') && !rows.some((r) => /^Open on/.test(r)), `${w}: the menu has one "Open in…" row, no "Open on X" rows (${rows.filter((r) => /Open/.test(r)).join(' | ')})`);
      check(rows.includes('Also on…'), `${w}: and an "Also on…" row (the hero has a switch target)`);
      await p.locator('.popover.menu:not([hidden]) [data-mi]', { hasText: 'Open in…' }).click(); await sleep(300);
      const sub = await p.$$eval('.popover.menu:not([hidden]) [data-mi], .popover.menu:not([hidden]) .pop-title', (es) => es.map((e) => e.textContent.trim()));
      check(sub[0] === 'Open in' && sub.slice(1).length === 4 && ['Spotify', 'Amazon Music', 'Apple Music', 'Discogs'].every((n) => sub.includes(n)), `${w}: the second menu lists every site (${sub.join(' | ')})`);
      await p.keyboard.press('Escape'); await sleep(200);
      // Switching: "Also on" opens the service's page here, no new tab
      const pagesBefore = ctx.pages().length;
      await p.click('.hero .also-icons:not(.open-in) .also-ico'); await until(async () => /\/album\/lib-9$/.test(p.url()), 8000);
      check(/\/album\/lib-9$/.test(p.url()) && ctx.pages().length === pagesBefore, `${w}: "Also on" switches to the page here, in this tab`);
      // One site: a direct row
      await p.goto(BASE + '/artist/tidal-77'); await p.waitForSelector('.hero .actions', { timeout: 20000 });
      await p.click('#imore'); await p.waitForSelector('.popover.menu:not([hidden]) [data-mi]');
      const rows2 = await p.$$eval('.popover.menu:not([hidden]) [data-mi]', (bs) => bs.map((b) => b.textContent.trim()));
      check(rows2.includes('Open in Discogs') && !rows2.includes('Open in…'), `${w}: one site is a direct "Open in Discogs" row (${rows2.filter((r) => /Open/.test(r)).join(' | ')})`);
      await p.keyboard.press('Escape');
      check(errors.length === 0, `${w}: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
      await ctx.close();
    }
  } finally { await browser.close(); }
  if (problems.length) { console.log(`${problems.length} failed`); process.exit(1); }
  console.log('also-on / open-in ok');
})().catch((e) => { console.error(e); process.exit(1); });
