// Confirm matches and the Identify card on the page's real assets, without a node: a loopback
// server serves crates/pm-api/static as the node does and answers /v1 itself (synthetic data,
// delays on demand). Checks, on a phone (390), a tablet (834) and a desktop (1440):
//   - a loading state at once, the first albums when they come (a slow answer);
//   - the owner's choices kept across "Show more albums", and the exact choices sent;
//   - confirm and "Not this" update the album where it is, with no new load of the page;
//   - answers that come after leaving the view (first page, more albums, a confirm), also after
//     leaving and coming back, draw nothing;
//   - what Confirm does to the files, as "Write IDs into my files" is set (on, off, unknown), and
//     files the IDs couldn't be written to are said;
//   - more than 500 chosen songs: the first 500 confirmed, the rest stay;
//   - "Not this" and "Which song is this?" send the song and recording they show.
// Any request outside the loopback server fails the run.
//   node tools/ui-identify-fixture.js [static dir] [screenshot dir]
const http = require('http');
const fs = require('fs');
const path = require('path');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const STATIC = process.argv[2] || path.join(__dirname, '..', 'crates', 'pm-api', 'static');
const SHOTS = process.argv[3] || '';
const problems = [];
let checks = 0;
const check = (ok, what) => { checks++; console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 6000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(50); } };

const REL = '1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e';
const rec = (i) => `00000000-0000-4000-8000-${String(i).padStart(12, '0')}`;
const track = (id, no, title, folder) => ({ id, title, artists: [{ name: 'Band' }], track_no: no, disc_no: null, duration_ms: (200 + no) * 1000, file: { path: `/music/${folder}/${String(no).padStart(2, '0')} ${title}.flac` } });
const opt = (id, trackId, recording, source, score, release, title) => ({ id, track_id: trackId, recording_mbid: recording, release_mbid: release, title, artist: 'Band', release_title: release ? 'Album' : null, score, source });
const albumA = () => ({
  key: '/music/Band/Album', album: 'Album', artist: 'Band', release: { mbid: REL, title: 'Album', songs: 3 }, ready: 2, conflicts: 1,
  songs: [
    { track: track(11, 1, 'Song 1', 'Band/Album'), options: [opt(101, 11, rec(1), 'album', 0.95, REL, 'Song 1')], status: 'single', pick: 101 },
    { track: track(12, 2, 'Song 2', 'Band/Album'), options: [opt(102, 12, rec(2), 'fingerprint', 0.93, REL, 'Song 2')], status: 'single', pick: 102 },
    { track: track(13, 3, 'Song 3', 'Band/Album'), options: [opt(104, 13, rec(9), 'fingerprint', 0.96, null, 'Song 3 (Live)'), opt(103, 13, rec(3), 'album', 0.95, REL, 'Song 3')], status: 'conflict', pick: null },
    { track: track(14, 4, 'Song 4', 'Band/Album'), options: [opt(105, 14, rec(4), 'search', 0.6, null, 'Song 4')], status: 'single', pick: null, hold: 'search' },
  ],
});
const albumB = () => ({ key: '/music/Other/Single', album: 'Single', artist: 'Other', release: null, ready: 1, conflicts: 0, songs: [{ track: track(21, 1, 'Lone', 'Other/Single'), options: [opt(201, 21, rec(21), 'fingerprint', 0.97, null, 'Lone')], status: 'single', pick: 201 }] });
const albumC = () => ({ key: '/music/Third/Later', album: 'Later', artist: 'Third', release: null, ready: 1, conflicts: 0, songs: [{ track: track(31, 1, 'After', 'Third/Later'), options: [opt(301, 31, rec(31), 'album', 0.9, null, 'After')], status: 'single', pick: 301 }] });
const big = () => ({ key: '/music/Box/Set', album: 'Box Set', artist: 'Band', release: null, ready: 520, conflicts: 0, songs: Array.from({ length: 520 }, (_, k) => ({ track: track(1000 + k, k + 1, `Part ${k + 1}`, 'Box/Set'), options: [opt(5000 + k, 1000 + k, rec(1000 + k), 'album', 0.95, null, `Part ${k + 1}`)], status: 'single', pick: 5000 + k })) });
const status = (o) => ({ phase: 'needs_review', progress: { running: false, total: 30, done: 30, identified: 20, suggested: 6, unknown: 4, errors: 0, finished_at: 1 }, counts: { total: 120, identified: 90, suggested: 6, unknown: 4, unchecked: 0, by: { tags: 10, album: 60, fingerprint: 8, you: 2, title_artist: 10 } }, acoustid: true, acoustid_own: true, acoustid_state: 'ok', write_ids: false, ...o });

// The server: the static assets as the node serves them, and /v1 from `fx` (per run)
let fx = null;
const server = http.createServer(async (req, res) => {
  const u = new URL(req.url, 'http://x');
  if (u.pathname.startsWith('/v1/')) {
    const p = u.pathname.slice(3);
    let body = '';
    for await (const c of req) body += c;
    const send = (st, json) => { res.writeHead(st, { 'content-type': 'application/json' }); res.end(json === undefined ? '' : JSON.stringify(json)); };
    fx.log.push({ method: req.method, path: p, query: u.search, body: body ? JSON.parse(body) : null, at: Date.now() });
    const h = fx.routes[`${req.method} ${p}`] || fx.routes[`${req.method} ${p.replace(/\/\d+(\/|$)/, '/:id$1')}`];
    if (h) { const [st, json, delay] = await h(u, body ? JSON.parse(body) : null); if (delay) await sleep(delay); return send(st, json); }
    const lists = { '/favourites': [], '/sources': [], '/zones': [], '/library/folders': [], '/library/webdav': [], '/library/hidden': [], '/presence': [] };
    if (p === '/info') return send(200, { id: 'n', name: 'Test node', version: '0', api: 'v1', library: { tracks: 120, albums: 10, artists: 3, files: 120, missing_files: 0, total_duration_ms: 1 }, zones: 0, outputs: 0, scanning: false });
    if (p === '/me') return send(200, { role: 'owner' });
    if (p === '/library/drives') return send(200, { roots: [], providers: [] });
    if (p in lists) return send(200, lists[p]);
    return send(200, {});
  }
  // A file of the static folder by its name (app.js, pwa.js, icons…), else the page itself
  const name = u.pathname.slice(1);
  const file = /^[\w.-]+$/.test(name) && name !== 'index.html' && fs.existsSync(path.join(STATIC, name)) ? name : 'index.html';
  const ext = path.extname(file);
  const type = { '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.webmanifest': 'application/manifest+json', '.html': 'text/html; charset=utf-8' }[ext] || 'application/octet-stream';
  res.writeHead(200, { 'content-type': type, 'cache-control': 'no-cache' });
  res.end(fs.readFileSync(path.join(STATIC, file)));
});

(async () => {
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const BASE = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch();
  try {
    for (const [name, w, h] of [['phone', 390, 844], ['tablet', 834, 1194], ['desktop', 1440, 900]]) {
      const W = `${name}:`;
      // No service worker: its cache would answer instead of this server
      const ctx = await browser.newContext({ viewport: { width: w, height: h }, reducedMotion: 'reduce', hasTouch: name !== 'desktop', serviceWorkers: 'block' });
      await ctx.addInitScript(() => { localStorage.setItem('pm.token', 't'); localStorage.setItem('pm.zone', 'browser'); });
      const outside = [];
      await ctx.route((url) => url.origin !== BASE && !/^(data|blob):/.test(url.protocol), (r) => { outside.push(r.request().url()); return r.abort(); });
      await ctx.routeWebSocket(/\/v1\/events/, (ws) => ws.close());
      const page = await ctx.newPage();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      const shot = async (what) => { if (SHOTS) await page.screenshot({ path: `${SHOTS}/${name}-${what}.png` }); };
      const nav = async (v) => page.evaluate((v) => { history.pushState({ v }, '', `/${v}`); window.dispatchEvent(new PopStateEvent('popstate', { state: { v } })); }, v);
      const main = page.locator('#main');
      const asked = (m, p) => fx.log.filter((x) => x.method === m && x.path === p);
      const base = (o = {}) => ({
        log: [],
        routes: {
          'GET /library/identify': async () => [200, status(o.status || {})],
          'GET /library/suggestions/albums': async (u) => { const off = Number(u.searchParams.get('offset') || 0); return off === 0 ? [200, { albums: [albumA(), albumB()], songs: 7, total_albums: 3, offset: 0, next: 2 }, o.firstDelay || 0] : [200, { albums: [albumC()], songs: 7, total_albums: 3, offset: 2, next: null }, o.moreDelay || 0]; },
          'POST /library/suggestions/confirm': async (_, b) => [200, o.confirm ? o.confirm(b) : { confirmed: b.choices.map((c) => c.track_id), count: b.choices.length }, o.confirmDelay || 0],
          'DELETE /library/suggestions/:id': async () => [204],
          ...(o.routes || {}),
        },
      });
      const open = async (o) => { fx = base(o); await page.goto(`${BASE}/identify`); };
      const section = (k) => page.locator('section.idalbum').nth(k);

      // 1. A loading state at once; the first albums when they come
      await open({ firstDelay: 1500 });
      const t0 = Date.now();
      check(await until(async () => (await page.locator('.idloading').count()) === 1, 3000), `${W} a loading state shows at once`);
      const shownAt = Date.now() - t0;
      check(shownAt < 1000, `${W} … within a second of opening (${shownAt} ms)`);
      await shot('loading');
      check(await until(async () => (await page.locator('section.idalbum').count()) === 2, 6000), `${W} the first albums show when they come`);
      check(await page.locator('.idloading').count() === 0, `${W} and the loading state is gone`);

      // 2. Choices kept across "Show more albums"; the exact choices sent; the album updated in place
      await open({ moreDelay: 600 });
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      await section(0).locator('input[name="pick-11"][value=""]').check();
      await section(0).locator('input[name="pick-13"][value="103"]').check();
      await page.locator('#idmore').click();
      check(await until(async () => (await page.locator('section.idalbum').count()) === 3), `${W} more albums are added`);
      check(await section(0).locator('input[name="pick-11"][value=""]').isChecked(), `${W} "Leave for later" on a pre-ticked song is kept after Show more`);
      check(await section(0).locator('input[name="pick-13"][value="103"]').isChecked(), `${W} the conflict's choice is kept after Show more`);
      check(await section(0).locator('input[name="pick-12"][value="102"]').isChecked(), `${W} untouched songs keep what is offered`);
      const loadsBefore = asked('GET', '/library/suggestions/albums').length;
      await section(0).locator('[data-confirm]').click();
      check(await until(async () => asked('POST', '/library/suggestions/confirm').length === 1), `${W} confirm sent`);
      const sent = asked('POST', '/library/suggestions/confirm')[0].body;
      check(JSON.stringify(sent) === JSON.stringify({ choices: [{ track_id: 12, suggestion_id: 102, recording_mbid: rec(2) }, { track_id: 13, suggestion_id: 103, recording_mbid: rec(3) }] }), `${W} exactly the chosen songs, suggestions and recordings are sent: ${JSON.stringify(sent)}`);
      check(await until(async () => (await section(0).locator('.idrow').count()) === 2), `${W} the confirmed songs leave the album`);
      check(await section(0).locator('.idrow[data-track="11"]').count() === 1 && await section(0).locator('.idrow[data-track="14"]').count() === 1, `${W} the songs left for later stay`);
      check(await section(0).locator('input[name="pick-11"][value=""]').isChecked(), `${W} … with the owner's choice`);
      check(asked('GET', '/library/suggestions/albums').length === loadsBefore, `${W} the page isn't loaded again after a confirm`);
      check((await page.locator('.page-head .count').textContent()).trim() === '5', `${W} the head's count goes down by the songs confirmed`);
      check(await page.locator('section.idalbum').count() === 3, `${W} the other albums stay`);
      // "Not this": names the song and recording, removes that option where it is
      await section(0).locator('[data-reject="105"]').click();
      check(await until(async () => asked('DELETE', '/library/suggestions/105').length === 1), `${W} "Not this" sent`);
      check(asked('DELETE', '/library/suggestions/105')[0].query === `?track_id=14&recording_mbid=${encodeURIComponent(rec(4))}`, `${W} … with the song and recording it shows (${asked('DELETE', '/library/suggestions/105')[0].query})`);
      check(await until(async () => (await section(0).locator('.idrow[data-track="14"]').count()) === 0), `${W} a song with no option left leaves the album`);
      check(asked('GET', '/library/suggestions/albums').length === loadsBefore, `${W} the page isn't loaded again after "Not this"`);
      check(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth) <= 0, `${W} the review fits the width`);
      const box = await section(0).locator('[data-confirm]').boundingBox();
      check(box && box.height >= (name === 'desktop' ? 28 : 32), `${W} Confirm is big enough to tap (${box && Math.round(box.height)} px)`);
      await shot('after-confirm');

      // 3. Late answers after leaving the view draw nothing (first page; more; confirm), also
      // after leaving and coming back
      await open({ firstDelay: 1200 });
      await sleep(100);
      await nav('albums');
      await sleep(1600);
      check(await page.locator('section.idalbum').count() === 0 && await page.locator('.idloading').count() === 0, `${W} a first page that comes after leaving draws nothing`);
      await open({ moreDelay: 1200 });
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      await page.locator('#idmore').click();
      await nav('albums');
      await sleep(1600);
      check(await page.locator('section.idalbum').count() === 0, `${W} more albums that come after leaving draw nothing`);
      await open({ moreDelay: 1200 });
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      await page.locator('#idmore').click();
      await nav('albums');
      await nav('identify');
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      await sleep(1600);
      check(await page.locator('section.idalbum').count() === 2, `${W} leaving and coming back: the old "Show more" answer adds nothing`);
      await open({ confirmDelay: 1200 });
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      await section(0).locator('[data-confirm]').click();
      await nav('albums');
      await sleep(1600);
      check(await page.locator('section.idalbum').count() === 0, `${W} a confirm answer that comes after leaving draws nothing`);

      // 4. What Confirm does to the files, as the setting is
      const intro = async () => { await until(async () => (await page.locator('.idintro').count()) === 1); return (await page.locator('.idintro').textContent()) || ''; };
      await open({ status: { write_ids: false } });
      check(/your files stay as they are \("Write IDs into my files" is off\)/.test(await intro()), `${W} Write IDs off: files stay as they are`);
      await open({ status: { write_ids: true } });
      const on = await intro();
      check(/is on, so its MusicBrainz IDs are also written into the file/.test(on) && !/stay as they are/.test(on), `${W} Write IDs on: says the IDs are written into the file`);
      await shot('write-ids-on');
      await open({ routes: { 'GET /library/identify': async () => [500, { error: 'busy' }] } });
      const unknown = await intro();
      check(/With "Write IDs into my files" on/.test(unknown) && !/stay as they are/.test(unknown), `${W} setting unknown: no promise either way`);
      // Files the IDs couldn't be written to are said
      await open({ status: { write_ids: true }, confirm: (b) => ({ confirmed: b.choices.map((c) => c.track_id), count: b.choices.length, files: { written: 1, skipped: [{ track_id: 12, path: '/music/Band/Album/02 Song 2.flac', reason: 'read-only share' }] } }) });
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      await section(0).locator('[data-confirm]').click();
      check(await until(async () => /IDs not written into 1 file \(read-only share\)/.test(await page.locator('#toast').textContent())), `${W} a file the IDs couldn't be written to is said`);

      // 5. More than 500 chosen: the first 500 confirmed, the rest stay
      fx = base({ routes: { 'GET /library/suggestions/albums': async () => [200, { albums: [big()], songs: 520, total_albums: 1, offset: 0, next: null }] } });
      await page.goto(`${BASE}/identify`);
      await until(async () => (await page.locator('section.idalbum').count()) === 1, 8000);
      const bigBtn = page.locator('section.idalbum [data-confirm]');
      check((await bigBtn.textContent()).trim() === 'Confirm the first 500 of 520', `${W} over 500: says the first 500 go (${(await bigBtn.textContent()).trim()})`);
      await bigBtn.click();
      check(await until(async () => asked('POST', '/library/suggestions/confirm').length === 1), `${W} one batch sent`);
      check(asked('POST', '/library/suggestions/confirm')[0].body.choices.length === 500, `${W} … of 500 songs`);
      check(await until(async () => (await page.locator('section.idalbum .idrow').count()) === 20, 8000), `${W} the 20 others stay`);
      check((await bigBtn.textContent()).trim() === 'Confirm 20 songs', `${W} … and can be confirmed next`);

      // 6. "Which song is this?" sends the song and recording picked
      const loose = { id: 77, title: 'Loose Song', artists: [{ id: 1, name: 'Band' }], album: null, track_no: 1, disc_no: null, duration_ms: 200000, canonical: null, genres: [], file: { path: '/music/x/Loose Song.flac', container: 'flac', format: { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 }, duration_ms: 200000, size_bytes: 1, mtime_unix: 0 } };
      fx = base({ routes: {
        'GET /library/tracks': async () => [200, { tracks: [loose] }],
        'GET /library/stats': async () => [200, { tracks: 1, albums: 0, artists: 1, files: 1, missing_files: 0, total_duration_ms: 200000 }],
        'GET /library/tracks/:id/candidates': async () => [200, { track: loose, needs_contact: false, suggestions: [opt(901, 77, rec(77), 'search', 0.9, null, 'Loose Song'), opt(902, 77, rec(78), 'search', 0.8, null, 'Loose Song')] }],
        'POST /library/suggestions/:id/accept': async () => [200, { ...loose, canonical: [{ recording: { mbid: rec(78) } }, { source: 'confirmed_by_user' }] }],
      } });
      await page.goto(`${BASE}/tracks`);
      const match = page.locator('[data-act="identify"]').first();
      // The Match button shows where there is room for it (phones reach "Which song is this?"
      // through the song's menu instead: not driven here)
      if (await until(async () => (await match.count()) > 0, 8000) && !(await match.isVisible())) {
        console.log(`note ${W} the Match button is hidden at this width by design; "Which song is this?" checked at the wider ones`);
      } else if (await match.count()) {
        await match.click();
        const second = page.locator('dialog [data-ci="1"]');
        await until(async () => (await second.count()) > 0);
        await second.click();
        check(await until(async () => asked('POST', '/library/suggestions/902/accept').length === 1), `${W} "Which song is this?" accepts the one picked`);
        const q = (asked('POST', '/library/suggestions/902/accept')[0] || {}).query;
        check(q === `?track_id=77&recording_mbid=${encodeURIComponent(rec(78))}`, `${W} … naming its song and recording (${q})`);
      } else {
        check(false, `${W} the tracks view shows a Match button for an unmatched song`);
      }

      // 7. PR #250 review: paging by folder, not by position. The node's list of albums shrinks as
      // songs are confirmed; "Show more" asks for the albums after the last one shown, so none is
      // skipped, and an album shown already isn't shown twice.
      const live = { albums: [albumA(), albumB(), albumC()] };
      const keyOf = (a) => a.key;
      const pageOf = (u) => {
        const now = live.albums.filter((a) => a.songs.length).sort((x, y) => (keyOf(x) < keyOf(y) ? -1 : 1));
        const after = u.searchParams.get('after'); const off = Number(u.searchParams.get('offset') || 0);
        const from = after != null ? now.filter((a) => keyOf(a) > after) : now.slice(off);
        const first = (after == null && off === 0) ? from.slice(0, 2) : from.slice(0, 2);
        const more = from.length > first.length;
        const songs = now.reduce((n, a) => n + a.songs.length, 0);
        return { albums: first, songs, total_albums: now.length, offset: off, next: more ? off + first.length : null, next_after: more ? keyOf(first[first.length - 1]) : null };
      };
      fx = base({ routes: {
        'GET /library/suggestions/albums': async (u) => [200, pageOf(u)],
        'POST /library/suggestions/confirm': async (_, b) => { const done = new Set(b.choices.map((c) => c.track_id)); live.albums.forEach((a) => { a.songs = a.songs.filter((g) => !done.has(g.track.id)); }); return [200, { confirmed: [...done], count: done.size }]; },
      } });
      // Keys sort: /music/Band/Album < /music/Other/Single < /music/Third/Later
      await page.goto(`${BASE}/identify`);
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      // The whole second album is confirmed before more albums load
      await page.locator('section.idalbum').nth(1).locator('[data-confirm]').click();
      await until(async () => asked('POST', '/library/suggestions/confirm').length === 1);
      await until(async () => (await page.locator('section.idalbum').count()) === 1);
      await page.locator('#idmore').click();
      check(await until(async () => (await page.locator('section.idalbum').count()) === 2), `${W} after a whole album is confirmed, Show more still brings the next album`);
      const moreQ = asked('GET', '/library/suggestions/albums').slice(-1)[0].query;
      check(/after=%2Fmusic%2FOther%2FSingle/.test(moreQ), `${W} Show more asks for the albums after the last one shown (${moreQ})`);
      check(await page.locator('section.idalbum h3', { hasText: 'Later' }).count() === 1, `${W} … the third album`);
      // An album the next page repeats (a run added to it meanwhile) is shown once
      fx.routes['GET /library/suggestions/albums'] = async (u) => (u.searchParams.get('after') ? [200, { albums: [albumA(), albumC()], songs: 6, total_albums: 3, offset: 2, next: null, next_after: null }] : [200, { albums: [albumA(), albumB()], songs: 6, total_albums: 3, offset: 0, next: 2, next_after: '/music/Other/Single' }]);
      await page.goto(`${BASE}/identify`);
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      await page.locator('#idmore').click();
      await until(async () => (await page.locator('section.idalbum h3', { hasText: 'Later' }).count()) === 1);
      check(await page.locator('input[name="pick-11"]').count() === 2, `${W} a repeated album is shown once (one set of radios per song)`);
      // Counts follow "Not this": song 3's conflict ends when one of its two options goes
      fx = base();
      await page.goto(`${BASE}/identify`);
      await until(async () => (await page.locator('section.idalbum').count()) === 2);
      const headA = page.locator('section.idalbum').first().locator('.section-head');
      check(/1 needs your pick/.test(await headA.textContent()), `${W} the album says a song needs your pick`);
      await page.locator('[data-reject="104"]').click();
      await until(async () => asked('DELETE', '/library/suggestions/104').length === 1);
      check(await until(async () => !/needs your pick/.test(await page.locator('section.idalbum').first().locator('.section-head').textContent())), `${W} … and stops saying so once its options agree`);
      // The radios' names are the options, not the buttons and links next to them
      check(await page.locator('.idopt label a, .idopt label button').count() === 0, `${W} no link or button inside an option's label`);
      const named = await page.locator('input[name="pick-12"]').first().evaluate((el) => [...el.labels].map((l) => l.textContent).join(' ') || (el.getAttribute('aria-labelledby') || '').split(' ').map((id) => (document.getElementById(id) || {}).textContent || '').join(' '));
      check(/Song 2/.test(named) && !/Not this|MusicBrainz/.test(named), `${W} an option's radio is named by the option only ("${named.trim().slice(0, 60)}")`);
      // The last song confirmed: "Nothing to confirm"
      fx = base({ routes: { 'GET /library/suggestions/albums': async () => [200, { albums: [albumB()], songs: 1, total_albums: 1, offset: 0, next: null, next_after: null }] } });
      await page.goto(`${BASE}/identify`);
      await until(async () => (await page.locator('section.idalbum').count()) === 1);
      await page.locator('section.idalbum [data-confirm]').click();
      check(await until(async () => /Nothing to confirm/.test(await page.locator('#main').textContent())), `${W} after the last song is confirmed the page says there is nothing to confirm`);

      check(!errors.length, `${W} no page errors ${errors.slice(0, 3).join(' | ')}`);
      check(!outside.length, `${W} nothing asked outside the loopback server ${outside.slice(0, 3).join(' ')}`);
      await ctx.close();
    }
  } finally {
    await browser.close();
    server.close();
  }
  console.log(`\n${checks} checks, ${problems.length} problem(s)`);
  process.exit(problems.length ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
