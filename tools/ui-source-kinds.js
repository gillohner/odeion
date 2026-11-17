// Your library on Sources says which rows are albums and which are songs. Spotify's list (albums
// first, then songs, under one heading) gave no kind at all, so an album and a song with the same
// title looked the same and both Play buttons were named "Play X on Spotify". TIDAL's list showed
// the service's own word ("track"), as did SoundCloud's likes. Now all three show "album" or "song"
// at the row's end, and Spotify's Play on an album is named "Play the album X on Spotify" (a song's
// stays "Play X on Spotify"; an untitled album's is "Play the album on Spotify"); the tooltip stays
// "Play on Spotify", and Enter on each plays that row. Bandcamp's purchases said "Sefa · track" under
// the title; they now end in "album" or "song" the same way, with only the artist under the title.
// Favourites' songs from other services said "Sefa · album" under an album's title, nothing for a
// song and the describer's raw "other" for a link it couldn't place; they now end in "album", "song",
// "artist" the same way (nothing for "other"), and an album's Play is named "Play the album X" so it
// differs from the same-titled song's "Play X"; on a phone the kind stays under the title (e.g.
// "Sefa · album"), where a row-end word would take the title's room. At 1280, 1024 and 768 px
// (Favourites also at 360 and 412).
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-source-kinds.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'source-kinds-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

// The same title as an album and as a song; the quotes and ampersand must come through escaped
const T = 'Rock & "Roll"';
const items = [{ kind: 'album', id: 'spa1', title: T, creator: 'Sefa', year: 2024 }, { kind: 'track', id: 'spt1', title: T, creator: 'Sefa' }];
const spItems = [items[0], { kind: 'album', id: 'spa2', title: '', creator: 'Sefa' }, items[1]];
const spotify = { '/sources': { body: [{ id: 'spotify', name: 'Spotify', description: '', status: 'ready', enabled: true }] }, '/sources/spotify/status': { body: { signed_in: true } }, '/sources/spotify/signin': { body: { ways: [], in_use: { way: 'app' }, details: { status: { signed_in: true } } } }, '/sources/spotify/collection': { body: { items: spItems } }, '/sources/spotify/playlists': { body: [] }, '/sources/spotify/devices': { body: [{ id: 'd1', name: 'Desk', type: 'Computer', is_active: true }] }, '/sources/spotify/play': { body: {} } };
const tidal = { '/sources': { body: [{ id: 'tidal', name: 'TIDAL', description: '', status: 'ready', enabled: true }] }, '/sources/tidal/signin': { body: { ways: [], in_use: { way: 'device' }, details: { status: { signed_in: true } } } }, '/sources/tidal/collection': { body: { albums: [{ ...items[0], id: 'ta1' }], tracks: [{ ...items[1], id: 'tt1' }] } }, '/sources/tidal/playlists': { body: [] } };
const sc = { '/sources': { body: [{ id: 'soundcloud', name: 'SoundCloud', description: '', status: 'signed in', enabled: true }] }, '/sources/soundcloud/likes': { body: { tracks: [{ ...items[1], id: 'sct1' }, { ...items[1], id: 'sct2', title: 'Second' }], playlists: [] } }, '/sources/soundcloud/playlists': { body: [] } };
// Bought on Bandcamp: an album (downloading) and a song with a page here, and a song with neither
// page nor artist and a long title
const bcItems = [{ id: 'a1', title: T, artist: 'Sefa', kind: 'album', url: 'https://sefa.bandcamp.com/album/x', item: 'album:sefa.x', downloadable: true, downloaded: false }, { id: 't2', title: 'Second', artist: 'Sefa', kind: 'track', url: 'https://sefa.bandcamp.com/track/y', item: 'track:sefa.y', downloadable: true, downloaded: true }, { id: 't3', title: 'Third, a much longer title that has to share the row with its kind and its state', kind: 'track', url: 'https://example.com/z', downloadable: false, downloaded: false }];
const bandcamp = { '/sources': { body: [{ id: 'bandcamp', name: 'Bandcamp', description: '', status: 'signed in', enabled: true }] }, '/sources/bandcamp/collection': { body: bcItems }, '/sources/bandcamp/downloads': { body: [{ id: 'a1', state: 'downloading', bytes: 42, total: 100 }] } };
// Hearted elsewhere: the same title as a Bandcamp album and song, an artist, a link nobody described
const favOther = [
  { uri: 'https://sefa.bandcamp.com/album/x', page: 'https://sefa.bandcamp.com/album/x', title: T, artist: 'Sefa', kind: 'album', site: 'bandcamp', also_on: [] },
  { uri: 'https://sefa.bandcamp.com/track/x', page: 'https://sefa.bandcamp.com/track/x', title: T, artist: 'Sefa', kind: 'track', site: 'bandcamp', also_on: [] },
  { uri: 'https://open.spotify.com/artist/p', page: 'https://open.spotify.com/artist/p', title: 'Dr. Peacock', kind: 'artist', site: 'spotify', also_on: [] },
  { uri: 'https://example.com/live', page: 'https://example.com/live', title: null, kind: 'other', site: null, also_on: [] },
];
const favourites = { '/favourites': { body: { uris: favOther.map((o) => o.uri), tracks: [], albums: [], artists: [], other: favOther.map((o) => o.uri) } }, '/discover/describe': { body: favOther }, '/discover/play': { body: { plays_from: null, items: [], connect: [] } } };
const kinds = (page) => page.$$eval('.main [data-sitem]', (rs) => rs.map((r) => { const k = [...r.children].find((c) => c.matches('span.muted.small')); return k ? k.textContent.trim() : null; }));

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      // Spotify
      {
        const { page, context, state } = await fx.create(browser, width, { fixtures: spotify });
        try {
          await fx.go(page, '/sources/spotify');
          const drawn = await page.waitForFunction(() => document.querySelectorAll('.main [data-sitem]').length === 3, null, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${width} spotify: album and song rows drawn`);
          if (drawn) {
            const k = await kinds(page);
            check(k.join() === 'album,album,song', `${width} spotify: rows say album, album, song (${k.join()})`);
            const got = await page.$$eval('.main [data-sitem] [data-splay]', (bs) => bs.map((b) => ({ label: b.getAttribute('aria-label'), title: b.getAttribute('title') })));
            check(got[0] && got[0].label === `Play the album ${T} on Spotify`, `${width} spotify: the album's Play is "Play the album ${T} on Spotify" (${got[0] && got[0].label})`);
            check(got[1] && got[1].label === 'Play the album on Spotify', `${width} spotify: the untitled album's Play is "Play the album on Spotify" (${got[1] && got[1].label})`);
            check(got[2] && got[2].label === `Play ${T} on Spotify`, `${width} spotify: the song's Play is "Play ${T} on Spotify" (${got[2] && got[2].label})`);
            check(got.every((g) => g.title === 'Play on Spotify'), `${width} spotify: tooltips still "Play on Spotify"`);
            for (const [name, uri] of [[`Play the album ${T} on Spotify`, 'spa1'], [`Play ${T} on Spotify`, 'spt1']]) {
              const b = page.getByRole('button', { name, exact: true });
              const one = await b.count() === 1;
              check(one, `${width} spotify: exactly one button named "${name}"`);
              const before = state.hits.length;
              const pressed = one && await b.focus({ timeout: 2000 }).then(() => page.keyboard.press('Enter')).then(() => true, () => false);
              let played = false;
              for (let i = 0; pressed && i < 40 && !played; i++) { played = state.hits.slice(before).some((h) => h.m === 'POST' && h.v === '/sources/spotify/play' && JSON.parse(h.body).uris.join() === uri); if (!played) await new Promise((r) => setTimeout(r, 50)); }
              check(played, `${width} spotify: Enter on "${name}" plays ${uri}`);
            }
            await page.screenshot({ path: path.join(SHOTS, `spotify-${width}.png`) });
            const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
            check(o <= 0, `${width} spotify: nothing wider than the screen (${o})`);
          } else await page.screenshot({ path: path.join(SHOTS, `spotify-${width}-undrawn.png`) });
          check(!state.errors.length, `${width} spotify: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // TIDAL's collection and SoundCloud's likes
      for (const [svc, fixtures, want] of [['tidal', tidal, 'album,song'], ['soundcloud', sc, 'song,song']]) {
        const { page, context, state } = await fx.create(browser, width, { fixtures });
        try {
          await fx.go(page, `/sources/${svc}`);
          const drawn = await page.waitForFunction(() => document.querySelectorAll('.main [data-sitem]').length === 2, null, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${width} ${svc}: rows drawn`);
          if (drawn) {
            const k = await kinds(page);
            check(k.join() === want, `${width} ${svc}: rows say ${want}, not "track" (${k.join()})`);
            await page.screenshot({ path: path.join(SHOTS, `${svc}-${width}.png`) });
            const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
            check(o <= 0, `${width} ${svc}: nothing wider than the screen (${o})`);
          } else await page.screenshot({ path: path.join(SHOTS, `${svc}-${width}-undrawn.png`) });
          check(!state.errors.length, `${width} ${svc}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Favourites › From other services
      {
        const { page, context, state } = await fx.create(browser, width, { fixtures: favourites });
        try {
          await fx.go(page, '/favourites');
          const drawn = await page.waitForFunction(() => document.querySelectorAll('#favother .list-row').length === 4, null, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${width} favourites: other services' rows drawn`);
          if (drawn) {
            const rows = await page.$$eval('#favother .list-row', (rs) => rs.map((r) => { const k = [...r.children].find((c) => c.matches('span.muted.small') && c.checkVisibility()); return { kind: k ? k.innerText.trim() : '', sub: r.querySelector('.sub').innerText.trim() }; }));
            const k = rows.map((r) => r.kind).join();
            check(k === 'album,song,artist,', `${width} favourites: rows end in album, song, artist, nothing (${k})`);
            const subs = rows.map((r) => r.sub).join('|');
            check(subs === 'Sefa|Sefa||', `${width} favourites: the line under the title holds no kind (${subs})`);
            const text = await page.innerText('#favother');
            check(!/\b(other|track)\b/i.test(text), `${width} favourites: no raw "other" or "track" on the list`);
            const labels = await page.$$eval('#favother [data-dplay]', (bs) => bs.map((b) => b.getAttribute('aria-label')));
            check(labels[0] === `Play the album ${T}`, `${width} favourites: the album's Play is "Play the album ${T}" (${labels[0]})`);
            check(labels[1] === `Play ${T}`, `${width} favourites: the song's Play stays "Play ${T}" (${labels[1]})`);
            const b = page.getByRole('button', { name: `Play the album ${T}`, exact: true });
            const one = await b.count() === 1;
            check(one, `${width} favourites: exactly one button named "Play the album ${T}"`);
            const before = state.hits.length;
            const pressed = one && await b.focus({ timeout: 2000 }).then(() => page.keyboard.press('Enter')).then(() => true, () => false);
            let asked = false;
            for (let i = 0; pressed && i < 40 && !asked; i++) { asked = state.hits.slice(before).some((h) => h.v === '/discover/play' && decodeURIComponent(h.q).includes(favOther[0].uri)); if (!asked) await new Promise((r) => setTimeout(r, 50)); }
            check(asked, `${width} favourites: Enter on the album's Play plays that album`);
            await page.keyboard.press('Escape');
            await page.screenshot({ path: path.join(SHOTS, `favourites-${width}.png`) });
            const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
            check(o <= 0, `${width} favourites: nothing wider than the screen (${o})`);
          } else await page.screenshot({ path: path.join(SHOTS, `favourites-${width}-undrawn.png`) });
          check(!state.errors.length, `${width} favourites: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Favourites on a phone: the kind stays in the line under the title (a row-end word next to
      // a Connect chip left the title no room), never "other"
      if (width === 1280) for (const pw of [360, 412]) {
        const fav = { ...favourites, '/discover/describe': { body: favOther.map((o) => ({ ...o, connect: ['tidal'] })) } };
        const { page, context, state } = await fx.create(browser, pw, { mobile: true, fixtures: fav });
        try {
          await fx.go(page, '/favourites');
          const drawn = await page.waitForFunction(() => document.querySelectorAll('#favother .list-row').length === 4, null, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${pw} phone favourites: rows drawn`);
          if (drawn) {
            const rows = await page.$$eval('#favother .list-row', (rs) => rs.map((r) => ({ end: [...r.children].filter((c) => c.matches('span.muted.small') && c.checkVisibility()).map((c) => c.innerText.trim()).join(''), sub: r.querySelector('.sub').innerText.trim(), name: Math.round(r.querySelector('.grow').getBoundingClientRect().width) })));
            check(rows.map((r) => r.sub).join('|') === 'Sefa · album|Sefa · song|artist|', `${pw} phone favourites: the line under the title ends in the kind (${rows.map((r) => r.sub).join('|')})`);
            check(rows.every((r) => !r.end), `${pw} phone favourites: no kind at the row's end (${rows.map((r) => r.end).join()})`);
            // (the same room for the title as without a kind word: 28 px at 360 next to Connect TIDAL)
            check(rows.every((r) => r.name >= 25), `${pw} phone favourites: the title keeps its room (${rows.map((r) => r.name).join()} px)`);
            await page.screenshot({ path: path.join(SHOTS, `favourites-phone-${pw}.png`) });
            const o = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
            check(o <= 0, `${pw} phone favourites: nothing wider than the screen (${o})`);
          }
          check(!state.errors.length, `${pw} phone favourites: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Bandcamp's purchases (rows with no page here have no data-sitem)
      {
        const { page, context, state } = await fx.create(browser, width, { fixtures: bandcamp });
        try {
          await fx.go(page, '/sources/bandcamp');
          const drawn = await page.waitForFunction(() => document.querySelectorAll('#bcmine .list-row').length === 3, null, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${width} bandcamp: purchases drawn`);
          if (drawn) {
            const rows = await page.$$eval('#bcmine .list-row', (rs) => rs.map((r) => { const k = [...r.children].find((c) => c.matches('span.muted.small')); return { kind: k ? k.textContent.trim() : null, sub: r.querySelector('.sub').textContent.trim() }; }));
            const k = rows.map((r) => r.kind).join();
            check(k === 'album,song,song', `${width} bandcamp: rows end in album, song, song (${k})`);
            const subs = rows.map((r) => r.sub);
            check(subs.join('|') === 'Sefa|Sefa|', `${width} bandcamp: the line under the title is the artist alone (${subs.join('|')})`);
            // (the album downloading: its state follows the kind; the long title keeps room to be read)
            const ends = await page.$$eval('#bcmine .list-row', (rs) => rs.map((r) => [...r.children].filter((c) => c.matches('span.muted.small')).map((c) => c.textContent.trim()).join(' ')));
            check(ends[0] === 'album 42%', `${width} bandcamp: a downloading album ends in "album 42%" (${ends[0]})`);
            const nameW = await page.$eval('#bcmine .list-row:nth-child(3) .name', (n) => n.getBoundingClientRect().width);
            check(nameW >= 150, `${width} bandcamp: a long title keeps room next to its kind and state (${Math.round(nameW)} px)`);
            const text = await page.innerText('#bcmine');
            check(!/\btrack\b/i.test(text), `${width} bandcamp: no "track" on the list`);
            await page.screenshot({ path: path.join(SHOTS, `bandcamp-${width}.png`) });
            const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
            check(o <= 0, `${width} bandcamp: nothing wider than the screen (${o})`);
          } else await page.screenshot({ path: path.join(SHOTS, `bandcamp-${width}-undrawn.png`) });
          check(!state.errors.length, `${width} bandcamp: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${passed} ok, ${failed} FAIL` : `${passed} ok, all passed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
