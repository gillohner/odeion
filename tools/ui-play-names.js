// A list's Play (and ⋯, Earlier, Later) buttons say what they act on. Song rows already did
// ("Play Journey", "More for Journey"), but other lists' buttons were named only by their tooltip,
// so a screen reader or a keyboard user going down the list heard "Play, button" on every row with
// nothing named: Favourites' songs from other services (Play, More), the Playlists list (Play),
// Radio stations (Play), Spotify's library on Sources ("Play on Spotify") and the Play from order
// in Settings (Earlier, Later). Now each says its row's name: "Play Night Drive",
// "More for Night Drive", "Play Road trip", "Play Fixture FM", "Play Night Drive on Spotify",
// "Move TIDAL earlier"; the short tooltips stay. A Favourites song with no title is
// "Untitled on bandcamp" on its row, its ⋯ and its menu's heading (not its web address). And the button so named is the one that acts on
// that row (keyboard Enter). At 1280, 1024 and 768 px.
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-play-names.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'play-names-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

// Two rows each; the second's name has quotes and an ampersand (escaped, not cut short)
const names = ['Night Drive', 'Rock & "Roll"'];
const OTHER = ['https://example.com/night-drive', 'https://example.com/rock-roll'];
const stations = names.map((name, i) => ({ id: `st${i + 1}`, name, url: `https://radio.example/${i + 1}`, country: 'CH', tags: ['jazz'] }));
const services = [{ id: 'library', name: 'Your library', connected: true }, { id: 'tidal', name: 'TIDAL', connected: true }, { id: 'qobuz', name: 'Qobuz', connected: true }, { id: 'deezer', connected: true }]; // (one with no name: its id)

// Each list: its address, extra fixtures, the row selector, each row's name, the buttons every row
// must name ([selector, tooltip, name(rowName) or null where that row has none]), and a keyboard
// press on one named button with what it must have done.
const lists = [
  {
    name: 'favourites', url: '/favourites', row: '#favother .list-row', names,
    fixtures: { '/favourites': { body: { uris: OTHER, tracks: [], albums: [], artists: [], other: OTHER } }, '/discover/describe': { body: OTHER.map((uri, i) => ({ uri, page: uri, title: names[i], artist: 'Someone Else', kind: 'track', site: null, also_on: [], connect: [] })) } },
    buttons: [['[data-dplay]', 'Play', (n) => `Play ${n}`], ['[data-dmore]', 'More', (n) => `More for ${n}`]],
    press: { name: `More for ${names[1]}`, did: async (page) => page.waitForFunction((t) => { const m = document.querySelector('.popover.menu:not([hidden])'); return !!m && m.textContent.includes(t); }, names[1], { timeout: 3000 }).then(() => true, () => false), what: 'opens that song\'s menu' },
  },
  {
    name: 'playlists', url: '/playlists', row: '.main [data-pl]', names,
    collections: names.map((name, i) => ({ ...fx.summary, id: `c${i + 1}`, name })),
    fixtures: { '/playlists/c2': { body: { ...fx.detail, summary: { ...fx.summary, id: 'c2', name: names[1] }, playlist: { ...fx.detail.playlist, id: 'c2', name: names[1] } } } },
    buttons: [['[data-plplay]', 'Play', (n) => `Play ${n}`]],
    press: { name: `Play ${names[1]}`, did: async (page, state) => { for (let i = 0; i < 40; i++) { if (state.hits.some((h) => h.v.startsWith('/playlists/c2'))) return true; await new Promise((r) => setTimeout(r, 50)); } return false; }, what: 'plays that playlist (asks for c2)' },
  },
  {
    name: 'radio', url: '/radio', row: '.main [data-st-row]', names,
    fixtures: { '/radio/stations': { body: { stations } }, '/radio/play': { body: {} } },
    buttons: [['[data-st-play]', 'Play', (n) => `Play ${n}`]],
    press: { name: `Play ${names[1]}`, did: async (page) => page.waitForFunction((t) => document.body.innerText.includes(`Tuning to ${t}`), names[1], { timeout: 3000 }).then(() => true, () => false), what: 'tunes to that station' },
  },
  {
    name: 'play-from', url: '/settings/playfrom', row: '#gorder .porder-row', names: services.map((s) => s.name || s.id),
    fixtures: { '/play-order': { body: { order: services.map((s) => s.id), services } } },
    buttons: [['[data-oup]', 'Earlier', (n) => `Move ${n} earlier`], ['[data-odown]', 'Later', (n) => `Move ${n} later`]],
    press: { name: 'Move TIDAL later', did: async (page, state) => { for (let i = 0; i < 40; i++) { const h = state.hits.find((x) => x.v === '/play-order' && x.m === 'PUT'); if (h) return JSON.parse(h.body).order.join(',') === 'library,qobuz,tidal,deezer'; await new Promise((r) => setTimeout(r, 50)); } return false; }, what: 'moves TIDAL after Qobuz' },
  },
  {
    name: 'spotify', url: '/sources/spotify', row: '.main [data-sitem]', names,
    fixtures: { '/sources': { body: [{ id: 'spotify', name: 'Spotify', description: '', status: 'ready', enabled: true }] }, '/sources/spotify/status': { body: { signed_in: true } }, '/sources/spotify/collection': { body: { items: names.map((title, i) => ({ kind: 'track', id: `sp${i + 1}`, title, creator: 'Sefa' })) } }, '/sources/spotify/playlists': { body: [] }, '/sources/spotify/devices': { body: [{ id: 'd1', name: 'Desk', type: 'Computer', is_active: true }] }, '/sources/spotify/play': { body: {} } },
    buttons: [['[data-splay]', 'Play on Spotify', (n) => `Play ${n} on Spotify`]],
    press: { name: `Play ${names[1]} on Spotify`, did: async (page, state) => { for (let i = 0; i < 40; i++) { if (state.hits.some((h) => h.m === 'POST' && h.v === '/sources/spotify/play' && JSON.parse(h.body).uris.join() === 'sp2')) return true; await new Promise((r) => setTimeout(r, 50)); } return false; }, what: 'plays that song on Spotify' },
  },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      for (const ls of lists) {
        const { page, context, state } = await fx.create(browser, width, { fixtures: ls.fixtures, collections: ls.collections || [] });
        try {
          await fx.go(page, ls.url);
          const drawn = await page.waitForFunction(([sel, n]) => document.querySelectorAll(sel).length === n, [ls.row, ls.names.length], { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${width} ${ls.name}: ${ls.names.length} rows drawn`);
          if (!drawn) { await page.screenshot({ path: path.join(SHOTS, `${ls.name}-${width}-undrawn.png`) }); continue; }
          for (const [sel, tip, name] of ls.buttons) {
            const got = await page.$$eval(`${ls.row} ${sel}`, (bs) => bs.map((b) => ({ label: b.getAttribute('aria-label'), title: b.getAttribute('title') })));
            check(got.length === ls.names.length, `${width} ${ls.name}: every row has its ${tip} button (${got.length})`);
            ls.names.forEach((n, i) => {
              check(got[i] && got[i].label === name(n), `${width} ${ls.name}: row ${i + 1}'s ${tip} is named "${name(n)}" (${got[i] && got[i].label})`);
              check(got[i] && got[i].title === tip, `${width} ${ls.name}: row ${i + 1}'s tooltip still says "${tip}" (${got[i] && got[i].title})`);
            });
            // The accessibility tree agrees: one button with that exact name per row
            for (const n of ls.names) {
              const c = await page.getByRole('button', { name: name(n), exact: true }).count();
              check(c === 1, `${width} ${ls.name}: exactly one button is named "${name(n)}" (${c})`);
            }
          }
          // Keyboard: the button so named acts on its own row
          const b = page.getByRole('button', { name: ls.press.name, exact: true });
          const pressed = await b.count() === 1 && await b.focus({ timeout: 2000 }).then(() => page.keyboard.press('Enter')).then(() => true, () => false);
          check(pressed && await ls.press.did(page, state), `${width} ${ls.name}: Enter on "${ls.press.name}" ${ls.press.what}`);
          await page.screenshot({ path: path.join(SHOTS, `${ls.name}-${width}.png`) });
          await page.keyboard.press('Escape');
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${width} ${ls.name}: nothing wider than the screen (${o})`);
          check(!state.errors.length, `${width} ${ls.name}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Favourites: a song from another service with no title. Its ⋯ is "More for Untitled on
      // bandcamp", and the menu it opens is headed the same, not by its raw web address.
      {
        const uri = 'https://someone.bandcamp.com/track/x7';
        const { page, context, state } = await fx.create(browser, width, { fixtures: { '/favourites': { body: { uris: [uri], tracks: [], albums: [], artists: [], other: [uri] } }, '/discover/describe': { body: [{ uri, page: uri, title: null, artist: 'Someone Else', kind: 'track', site: 'bandcamp', also_on: [], connect: [] }] } } });
        try {
          await fx.go(page, '/favourites');
          const b = page.getByRole('button', { name: 'More for Untitled on bandcamp', exact: true });
          const found = await b.waitFor({ timeout: 8000 }).then(() => true, () => false);
          check(found, `${width} favourites untitled: its ⋯ is named "More for Untitled on bandcamp"`);
          if (found) {
            await b.click();
            const head = await page.waitForFunction(() => { const m = document.querySelector('.popover.menu:not([hidden]) .pop-title'); return m && m.textContent; }, null, { timeout: 3000 }).then((h) => h.jsonValue(), () => null);
            check(head === 'Untitled on bandcamp', `${width} favourites untitled: its menu is headed "Untitled on bandcamp", like the row (${head})`);
            await page.screenshot({ path: path.join(SHOTS, `favourites-untitled-menu-${width}.png`) });
            await page.keyboard.press('Escape');
          }
          check(!state.errors.length, `${width} favourites untitled: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${passed} ok, ${failed} FAIL` : `${passed} ok, all passed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
