// A simpler phone UI (TASK-phone-ui-simplify-1108; owner, 2026-11-08: "I don't like the bottom player
// bar. too cramped make ui simpler. same for full screen song display. scroll feels weird … use swipe
// gestures"). Real candidate assets on the fixture origin (tools/ui-fixture.js) with a phone's viewport
// rules, a Linn playing with a queue, at 360x800, 390x844 and 412x915:
//   * the mini player: the song has at least 60 % of the bar, play/pause is one 48 px target, nothing
//     is wider than the screen; a swipe left / right on it is next / previous (Pointer Events, a
//     finger), a vertical swipe on the page or a sideways one on a list is not; up opens Now playing;
//   * Now playing: one screen that doesn't scroll, the zone named in its top bar, Up next, Lyrics and
//     More under the transport; each opens one sheet with a tab bar; it closes by a swipe down on its
//     head and by back (back closes the sheet first, then Now playing); a swipe up opens Up next, a
//     sideways swipe on the sheet changes its tab, a swipe on the cover is next; More has one Song
//     page link, Lyrics one heading and full-size −/+;
//   * a toast sits above the tabs, and in Now playing at the screen's bottom, never mid-screen;
//   * the album page: at least two songs above the fold at 360x800, ⋯ a 48 px target, no play
//     triangle per row, the playing song marked, "Not matched · Match" under Play (never above it),
//     Select's check and word on one centre line; a long press selects;
//   * the loader: one ring, centred in the mini player's cover square while a zone starts, turning (or
//     pulsing with reduced motion);
//   * Confirm matches: the header in the first frame, rows (not a grid of covers) while it loads, the
//     header where it was once the list comes, the file's name on one line;
//   * with the browser's own touch input (CDP): a swipe on the mini player while it redraws, a finger
//     held on a row, a sideways swipe on the sheet's list; the sheet's cover is inert; Escape closes
//     the sheet first; Sound from ··· gives the focus back; a playlist's header is not the album's;
//   * a desktop window 700 px wide (the phone arrangement above the player bar), and a tablet's open
//     Lyrics made narrow (Back closes the sheet first).
//   node tools/ui-phone-simplify.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-simplify-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(50); } };

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const TITLES = ['Wonderful Tonight', 'Crossroads', 'After Midnight', 'I Shot The Sheriff', 'Further Up The Road (Live)', 'Rambling On My Mind (Live)', 'Cocaine', 'Lay Down Sally'];
const queue = TITLES.map((title, i) => ({ id: i + 1, uri: `track:${i + 1}`, title, artist: 'Eric Clapton', duration_ms: 200000 + i * 7000 }));
const np = { item: { id: 1, uri: 'track:1', title: 'Wonderful Tonight', artist: 'Eric Clapton', album: 'Eric Clapton Story', duration_ms: 219000 }, index: 0, position_ms: 61000, duration_ms: 219000, source_format: fmt, output_format: fmt, signal_path: { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = { id: 'linn', name: 'Linn Klimax DS3:UPnP AV', output: 'upnp:linn', transport: 'playing', queue, queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: np };
const lyrics = { synced: true, source: 'lrclib', lines: Array.from({ length: 40 }, (_, i) => ({ ms: i * 2000, text: `Line ${i + 1} of the song` })) };
// An album of the library, one artist, not matched to MusicBrainz; its first song is the one playing
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: { album_id: 7 }, title: 'Eric Clapton Story', artist: { name: 'Eric Clapton', id: 'lib-31' }, album: null, year: 1990, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: queue.map((t, i) => ({ no: i + 1, title: t.title, artist: 'Eric Clapton', duration_ms: t.duration_ms, uri: t.uri, track_id: t.id, id: 'lib-' + t.id, page: 'lib-' + t.id })),
  total_ms: queue.reduce((a, t) => a + t.duration_ms, 0), albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
// Confirm matches: one folder, a song with two candidates and a long file name
const longFile = '/music/Alizee/Moi Lolita/4f14aed932a1a1a6-1-Moi..._Lolita__Single_Version_.flac';
const opt = (id, title) => ({ id, track_id: 11, recording_mbid: `00000000-0000-4000-8000-${String(id).padStart(12, '0')}`, release_mbid: null, title, artist: 'Alizée', release_title: null, score: 0.99, source: 'fingerprint' });
const sugg = { albums: [{ key: '/music/Alizee/Moi Lolita', album: 'Moi… Lolita', artist: 'Alizée', release: null, ready: 0, conflicts: 1, songs: [{ track: { id: 11, title: 'Moi… Lolita (Single Version)', artists: [{ name: 'Alizée' }], track_no: 1, duration_ms: 266000, file: { path: longFile } }, options: [opt(101, 'Moi… Lolita'), opt(102, 'Moi… Lolita (radio edit)')], status: 'conflict', pick: null }] }], songs: 43, total_albums: 1, offset: 0, next: null };
const libTrack = (t) => ({ id: t.id, title: t.title, artists: [{ id: 31, name: 'Eric Clapton' }], album: { id: 7, title: 'Eric Clapton Story' }, duration_ms: t.duration_ms, file: { container: 'flac', format: fmt } });
const fixtures = {
  ...Object.fromEntries(queue.map((t) => [`/library/tracks/${t.id}`, { body: libTrack(t) }])),
  '/library/tracks/1/lyrics': { body: lyrics }, '/zones': { body: [zone] }, '/outputs': { body: [{ id: 'upnp:linn', name: 'Linn', kind: 'upnp', caps: { hw_volume: 'percent' } }] }, '/zones/linn/queue': { body: queue },
  '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
  '/library/suggestions/albums': { body: sugg }, '/tags': { body: { needs_match: true, tags: [] } }, '/discover/uri': { status: 422, body: { error: 'no canonical identity' } }, '/library/identify': { body: { phase: 'needs_review', write_ids: false, progress: { running: false } } },
};

// A finger's swipe (Pointer Events, as Android's WebView sends them), from the middle of `sel` (or a
// point in it) by dx, dy, in steps
const swipe = (page, sel, dx, dy, at = [0.5, 0.5]) => page.evaluate(([s, ddx, ddy, fx0, fy0]) => {
  const box = document.querySelector(s); if (!box) return 'missing';
  const r = box.getBoundingClientRect(); const x0 = r.left + r.width * fx0; const y0 = r.top + r.height * fy0;
  const el = document.elementFromPoint(x0, y0) || box;
  const ev = (type, x, y) => el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, composed: true, pointerId: 7, pointerType: 'touch', isPrimary: true, clientX: x, clientY: y }));
  ev('pointerdown', x0, y0);
  for (let i = 1; i <= 6; i++) ev('pointermove', x0 + (ddx * i) / 6, y0 + (ddy * i) / 6);
  ev('pointerup', x0 + ddx, y0 + ddy);
  return el.className || el.tagName;
}, [sel, dx, dy, at[0], at[1]]);
const sent = (state, action) => state.hits.filter((h) => h.m === 'POST' && (h.body || '').includes(`"action":"${action}"`)).length;
// Every element inside the screen's width
const overflow = (page) => page.evaluate(() => {
  const vw = document.documentElement.clientWidth; const out = [];
  for (const e of document.querySelectorAll('body *')) { if (!e.getClientRects().length || e.closest('svg, [hidden], .np-bsheet[hidden]')) continue; const r = e.getBoundingClientRect(); if (r.width && (r.right > vw + 0.5 || r.left < -0.5) && getComputedStyle(e).position !== 'fixed' && !e.closest('[style*="translate"], .libnav, .chips, .scopechips, .np-stabs, .lyric-lines')) out.push(`${e.tagName.toLowerCase()}.${String(e.className).split(' ')[0]} ${Math.round(r.left)}..${Math.round(r.right)}`); }
  return { sw: document.documentElement.scrollWidth, vw, out: out.slice(0, 4) };
});

(async () => {
  const browser = await chromium.launch();
  for (const [w, h] of [[360, 800], [390, 844], [412, 915]]) {
    const at = `${w}x${h}`;
    const { page, context, state } = await fx.create(browser, w, { mobile: true, fixtures, delay: (v, m) => (v === '/library/suggestions/albums' ? 900 : m !== 'GET' && v.startsWith('/zones/linn/') && !/transport/.test(v) ? 4000 : 0) });
    await page.setViewportSize({ width: w, height: h });
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
    await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn'); localStorage.setItem('pm.theme', 'dark'); window.pmDesktop = { platform: 'android', version: '0.0.0' }; });

    // ---- the album page and the mini player ----
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs .srow'); await sleep(600);
    await page.screenshot({ path: path.join(SHOTS, `album-${w}.png`) });
    const bar = await page.evaluate(() => {
      const r = (e) => (e ? e.getBoundingClientRect() : null); const p = r(document.querySelector('.player')); const t = r(document.querySelector('#np .np-title')); const tg = r(document.querySelector('#toggle'));
      const vis = (s) => { const e = document.querySelector(s); return !!e && e.getClientRects().length > 0 && getComputedStyle(e).display !== 'none'; };
      const prog = document.querySelector('#miniprog'); const pr = r(prog);
      return { p, t, tg, pill: vis('.player #quality'), zoneBtn: vis('.player #zonebtn'), prog: pr && pr.height, progTop: pr && pr.top };
    });
    check(bar.t.width >= 0.6 * bar.p.width, `${at}: the mini player's title has ${Math.round((bar.t.width / bar.p.width) * 100)} % of the bar (>= 60 %)`);
    check(bar.tg.width >= 48 && bar.tg.height >= 48 && !bar.pill && !bar.zoneBtn, `${at}: one play/pause, ${bar.tg.width}x${bar.tg.height} px, no sound pill or zone button in the bar (${bar.pill}, ${bar.zoneBtn})`);
    check(bar.prog === 2 && Math.abs(bar.progTop - bar.p.top) <= 1, `${at}: a 2 px progress line on the bar's top edge (${bar.prog} px at ${bar.progTop})`);
    const ov1 = await overflow(page);
    check(ov1.sw <= ov1.vw && !ov1.out.length, `${at}: nothing on the album page is wider than the screen (${ov1.sw} of ${ov1.vw}${ov1.out.length ? ': ' + ov1.out.join(', ') : ''})`);
    const alb = await page.evaluate(() => {
      const fold = document.querySelector('.player').getBoundingClientRect().top;
      const rows = [...document.querySelectorAll('#isongs .srow')];
      const r = (s) => document.querySelector(s)?.getBoundingClientRect();
      const more = rows[0].querySelector('[data-smore]').getBoundingClientRect();
      const play = rows[0].querySelector('[data-splay]');
      const sel = document.querySelector('#isongs [data-selstart]'); const ic = sel.querySelector('.ic').getBoundingClientRect(); const tx = sel.querySelector('span').getBoundingClientRect();
      const m = r('#hmatch [data-htag-match]'); const playBtn = r('#iplay');
      const rowsH = rows.map((x) => Math.round(x.getBoundingClientRect().height));
      const cur = rows[0]; const curName = cur.querySelector('.name');
      return { above: rows.filter((x) => x.getBoundingClientRect().bottom <= fold + 0.5).length, more: [more.width, more.height], play: !!play && play.getClientRects().length > 0 && getComputedStyle(play).display !== 'none', dy: Math.abs((ic.top + ic.bottom) / 2 - (tx.top + tx.bottom) / 2), match: m ? { top: m.top, h: m.height, w: m.width } : null, playTop: playBtn.bottom, rowsH, cur: cur.classList.contains('cur') && getComputedStyle(curName).color !== getComputedStyle(rows[1].querySelector('.name')).color, artist: rows[1].querySelector('.sub').textContent.trim(), hnote: !!document.querySelector('#htags .hnote'), icons: ['#ifav', '#izone', '#imore'].map((s) => { const b = r(s); return [Math.round(b.width), Math.round(b.height)]; }) };
    });
    check(alb.above >= 2, `${at}: ${alb.above} song rows above the fold (>= 2)`);
    check(alb.rowsH.every((x) => x >= 56 && x <= 64), `${at}: song rows are 56-64 px (${[...new Set(alb.rowsH)].join(', ')})`);
    check(alb.more[0] >= 48 && alb.more[1] >= 48 && !alb.play, `${at}: a row's ⋯ is ${alb.more.join('x')} px and there is no play triangle (${alb.play})`);
    check(alb.dy <= 2, `${at}: Select's check and word share a centre line (|dy| ${alb.dy.toFixed(1)} px)`);
    check(!!alb.match && alb.match.top >= alb.playTop && alb.match.h >= 48 && !alb.hnote, `${at}: "Not matched · Match" sits under Play (${alb.match && Math.round(alb.match.top)} >= ${Math.round(alb.playTop)}), 48 px tall, no note above it`);
    check(alb.icons.every(([a, b]) => a === 48 && b === 48), `${at}: heart, Play to and ⋯ are icon buttons of one size (${alb.icons.map((x) => x.join('x')).join(', ')})`);
    check(alb.cur && alb.artist === '', `${at}: the playing song is marked, and a one-artist album leaves the artist off its rows ("${alb.artist}")`);
    // a long press on a row selects it (Android sends contextmenu for it)
    await page.evaluate(() => { const row = document.querySelectorAll('#isongs .srow')[1]; row.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true })); });
    await sleep(200);
    const selected = await page.evaluate(() => ({ on: document.querySelector('#isongs').classList.contains('selecting'), ticked: [...document.querySelectorAll('#isongs .selbox input')].map((c) => c.checked), menu: !document.querySelector('.popover.menu').hidden }));
    check(selected.on && selected.ticked[1] && selected.ticked.filter(Boolean).length === 1 && !selected.menu, `${at}: a long press starts selecting with that song ticked, no menu (${selected.on}, ${selected.ticked.filter(Boolean).length} ticked)`);
    await page.screenshot({ path: path.join(SHOTS, `album-select-${w}.png`) });
    await page.keyboard.press('Escape'); await page.evaluate(() => { const c = document.querySelector('.selbar [data-sel="cancel"]'); if (c) c.click(); }); await sleep(200);

    // ---- gestures on the mini player ----
    const n0 = sent(state, 'next'); const p0 = sent(state, 'previous');
    await swipe(page, '#main', 0, -260, [0.5, 0.6]); await swipe(page, '#isongs .srow:nth-child(3)', -90, 0); await sleep(500);
    check(sent(state, 'next') === n0 && sent(state, 'previous') === p0, `${at}: scrolling the page and a sideways swipe on a song row don't change the song`);
    await page.evaluate(() => { const row = document.querySelector('#isongs .srow:nth-child(3)'); if (row) { row.style.transform = ''; row.classList.remove('swiping', 'swipe-ready'); } });
    await swipe(page, '#np', -120, 6); await sleep(500);
    check(sent(state, 'next') === n0 + 1 && sent(state, 'previous') === p0, `${at}: a swipe left on the mini player is the next song (${sent(state, 'next') - n0})`);
    await swipe(page, '#np', 120, -4); await sleep(500);
    check(sent(state, 'previous') === p0 + 1 && sent(state, 'next') === n0 + 1, `${at}: a swipe right is the previous song (${sent(state, 'previous') - p0})`);
    await swipe(page, '#np', 4, 80); await sleep(300);
    check(await page.evaluate(() => document.querySelector('#nowplaying').hidden) && sent(state, 'stop') === 0 && sent(state, 'toggle') === 0, `${at}: a swipe down on the mini player does nothing`);
    // a toast: above the tabs and the mini player
    await page.evaluate(() => document.querySelector('#ifav').click()); await page.waitForSelector('#toast.show');
    const t1 = await page.evaluate(() => ({ t: document.querySelector('#toast').getBoundingClientRect(), p: document.querySelector('.player').getBoundingClientRect().top, vh: innerHeight }));
    check(t1.t.bottom <= t1.p - 4 && t1.t.bottom >= t1.p - 16 && t1.t.left >= 15 && t1.t.right <= w - 15, `${at}: the toast sits just above the mini player and the tabs (its bottom ${Math.round(t1.t.bottom)}, the bar at ${Math.round(t1.p)})`);

    // ---- Now playing ----
    await swipe(page, '#np', 0, -120); await page.waitForSelector('#nowplaying:not([hidden]) .np-dock'); await sleep(500);
    await page.screenshot({ path: path.join(SHOTS, `nowplaying-${w}.png`) });
    const one = await page.evaluate(() => {
      const ov = document.querySelector('#nowplaying'); const r = (s) => ov.querySelector(s)?.getBoundingClientRect();
      const dock = [...ov.querySelectorAll('.np-dock [data-npsheet]')].map((b) => b.textContent.trim());
      return { scroll: ov.scrollHeight - ov.clientHeight, pg: ov.querySelector('.np-page').scrollHeight - ov.querySelector('.np-page').clientHeight, dock, dockBottom: r('.np-dock').bottom, vh: innerHeight, zone: ov.querySelector('#npzone').textContent.trim(), q: r('#npquality'), play: r('.np-transport .play'), sheet: !ov.querySelector('.np-bsheet').hidden };
    });
    check(one.scroll <= 0 && one.pg <= 1 && one.dockBottom <= one.vh, `${at}: Now playing's first screen doesn't scroll (overflow ${one.scroll}/${one.pg} px, buttons end at ${Math.round(one.dockBottom)} of ${one.vh})`);
    check(one.dock.join(',') === 'Up next,Lyrics,More' && !one.sheet, `${at}: Up next, Lyrics and More under the player (${one.dock.join(', ')}), no sheet open`);
    check(/Linn/.test(one.zone) && one.q && one.q.height > 0, `${at}: the zone ("${one.zone}") and the quality line are on the first screen`);
    const ov2 = await overflow(page);
    check(ov2.sw <= ov2.vw && !ov2.out.length, `${at}: nothing in Now playing is wider than the screen${ov2.out.length ? ': ' + ov2.out.join(', ') : ''}`);
    // the cover: left is next
    const n1 = sent(state, 'next');
    await swipe(page, '#nowplaying .np-big', -140, 4); await sleep(600);
    check(sent(state, 'next') === n1 + 1 && !(await page.evaluate(() => document.querySelector('#nowplaying').hidden)), `${at}: a swipe left on the cover is the next song`);
    await swipe(page, '#nowplaying .np-transport', -140, 2); await sleep(400);
    check(sent(state, 'next') === n1 + 1, `${at}: a sideways swipe off the cover changes nothing`);
    // a sheet by its button, closed by a swipe down on its head
    await page.click('.np-dock [data-npsheet="queue"]'); await page.waitForSelector('.np-bsheet:not([hidden])'); await sleep(400);
    await page.screenshot({ path: path.join(SHOTS, `sheet-upnext-${w}.png`) });
    const sh = await page.evaluate(() => { const s = document.querySelector('.np-bsheet'); const tabs = [...s.querySelectorAll('[role="tab"]')].map((t) => `${t.textContent.trim()}:${t.getAttribute('aria-selected')}`); const rows = s.querySelectorAll('.np-qitem').length; const q = s.querySelector('.np-queue'); return { tabs, rows, border: q ? getComputedStyle(q).borderTopWidth : null, handle: !!s.querySelector('#npgrab'), r: s.getBoundingClientRect(), vh: innerHeight }; });
    check(sh.tabs.join(',') === 'Up next:true,Lyrics:false,More:false' && sh.rows === 7 && sh.border === '0px' && sh.handle && sh.r.bottom <= sh.vh + 0.5 && sh.r.top < 40, `${at}: Up next opens one full-height sheet with a handle and the tab bar (${sh.tabs.join(' ')}; ${sh.rows} rows, no box around them)`);
    await swipe(page, '.np-bsheet .np-sheet-head', 0, 140, [0.5, 0.2]); await sleep(500);
    check(await page.evaluate(() => document.querySelector('.np-bsheet').hidden && !document.querySelector('#nowplaying').hidden), `${at}: a swipe down on its head closes the sheet, Now playing stays`);
    // swipe up opens Up next; a sideways swipe moves to Lyrics; back closes the sheet, then Now playing
    await swipe(page, '#nowplaying .np-titles', 0, -140); await sleep(500);
    check(await page.evaluate(() => !document.querySelector('.np-bsheet').hidden && document.querySelector('.np-bsheet').dataset.tab === 'queue'), `${at}: a swipe up on the player opens Up next`);
    await swipe(page, '.np-bsheet .np-sheet-body', -140, 4); await sleep(1500);
    const ly = await page.evaluate(() => { const s = document.querySelector('.np-bsheet'); const head = s.querySelector('.lyric-head'); const nud = [...s.querySelectorAll('[data-lnudge]')].map((b) => { const r = b.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height), Math.round(r.right)]; }); return { tab: s.dataset.tab, h: [...s.querySelectorAll('h2, h3')].map((x) => x.textContent.trim()).filter((x) => /lyrics/i.test(x)).length, how: head ? head.textContent.trim() : '', nud, right: s.querySelector('.np-sheet-body').getBoundingClientRect().right }; });
    check(ly.tab === 'lyrics' && ly.h === 0 && /Synced/.test(ly.how), `${at}: a swipe left on the sheet goes to Lyrics, with no second "Lyrics" heading ("${ly.how}")`);
    check(ly.nud.length === 2 && ly.nud.every(([a, b]) => a >= 48 && b >= 48) && ly.right - ly.nud[1][2] <= 20, `${at}: the timing −/+ are 48 px targets at the row's right end (${ly.nud.map((x) => x.slice(0, 2).join('x')).join(', ')})`);
    await page.screenshot({ path: path.join(SHOTS, `sheet-lyrics-${w}.png`) });
    await page.goBack(); await sleep(500);
    check(await page.evaluate(() => document.querySelector('.np-bsheet').hidden && !document.querySelector('#nowplaying').hidden), `${at}: back closes the sheet first`);
    await page.goBack(); await sleep(500);
    check(await page.evaluate(() => document.querySelector('#nowplaying').hidden && location.pathname.includes('/album/lib-7')), `${at}: the next back closes Now playing, the album page stays`);
    // More: Details, Sources, Tags, one Song page link; a toast at the screen's bottom
    await page.click('#np'); await page.waitForSelector('#nowplaying:not([hidden]) .np-dock'); await sleep(300);
    await page.click('.np-dock [data-npsheet="more"]'); await page.waitForSelector('.np-bsheet:not([hidden]) #nppart-tags #nptagadd'); await sleep(600);
    await page.screenshot({ path: path.join(SHOTS, `sheet-more-${w}.png`) });
    const more = await page.evaluate(() => { const s = document.querySelector('.np-bsheet'); return { parts: [...s.querySelectorAll('.np-part h3')].map((x) => x.textContent.trim()), links: [...s.querySelectorAll('button, a')].filter((b) => /Song page/.test(b.textContent)).length, pathBox: getComputedStyle(s.querySelector('.path')).borderTopWidth, pathBg: getComputedStyle(s.querySelector('.path-head')).backgroundColor }; });
    check(more.parts.join(',') === 'Details,Sources,Tags' && more.links <= 1 && more.pathBox === '0px', `${at}: More holds Details, Sources and Tags (${more.parts.join(', ')}), ${more.links} Song page link, the status as plain text`);
    await page.fill('#nptagadd', 'test'); await page.press('#nptagadd', 'Enter'); await page.waitForSelector('#toast.show');
    const t2 = await page.evaluate(() => ({ t: document.querySelector('#toast').getBoundingClientRect(), vh: innerHeight }));
    check(t2.t.bottom <= t2.vh - 8 && t2.t.bottom >= t2.vh - 40, `${at}: in Now playing the toast sits at the screen's bottom (${Math.round(t2.t.bottom)} of ${t2.vh}), not mid-screen`);
    await page.evaluate(() => { document.querySelector('#npgrab').click(); }); await sleep(400);
    check(await page.evaluate(() => document.querySelector('.np-bsheet').hidden), `${at}: a tap on the handle closes the sheet`);
    await swipe(page, '#nowplaying .np-titles', 0, 160); await sleep(400);
    check(await page.evaluate(() => document.querySelector('#nowplaying').hidden), `${at}: a swipe down on the player closes Now playing`);

    // ---- the loader: a zone starting shows one ring, centred in the cover's square ----
    await page.evaluate(() => { const r = document.querySelectorAll('#isongs .srow')[2]; r.click(); }); await sleep(250);
    const spin = await page.evaluate(() => {
      const sp = document.querySelector('#np .spin'); if (!sp) return null; const a = sp.closest('.np-art').getBoundingClientRect(); const r = sp.getBoundingClientRect(); const cs = getComputedStyle(sp);
      return { dx: Math.abs((a.left + a.right) / 2 - (r.left + r.right) / 2), dy: Math.abs((a.top + a.bottom) / 2 - (r.top + r.bottom) / 2), w: r.width, h: r.height, anim: cs.animationName, n: document.querySelectorAll('#np .spin').length, inline: [...document.querySelectorAll('.spin')].every((x) => getComputedStyle(x).display !== 'inline') };
    });
    check(!!spin && spin.n === 1 && spin.dx <= 1 && spin.dy <= 1 && spin.w >= 14 && spin.w === spin.h && spin.anim !== 'none' && spin.inline, `${at}: one loader, centred in the mini player's cover square, ${spin && spin.w}x${spin && spin.h} px, animated (${spin && spin.anim})`);
    await page.screenshot({ path: path.join(SHOTS, `starting-${w}.png`), clip: { x: 0, y: h - 140, width: w, height: 140 } });

    // ---- Confirm matches: the header in the first frame, rows while it loads, no jump after ----
    await fx.go(page, '/identify'); await page.waitForSelector('#main .page-head h2');
    const first = await page.evaluate(() => ({ head: document.querySelector('#main .page-head h2').textContent, how: !!document.querySelector('#main .idhow'), skip: !!document.querySelector('#idskipall'), grid: !!document.querySelector('#main .sk-grid'), rows: document.querySelectorAll('#main .sk-srow').length, top: Math.round(document.querySelector('#main .idhow').getBoundingClientRect().bottom) }));
    await page.screenshot({ path: path.join(SHOTS, `identify-loading-${w}.png`) });
    check(/Confirm matches/.test(first.head) && first.how && first.skip && !first.grid && first.rows >= 3, `${at}: Confirm matches draws its header and rows (not a cover grid) while the list comes (${first.rows} rows)`);
    await page.waitForSelector('#main .idalbum'); await sleep(300);
    const fin = await page.evaluate(() => { const f = document.querySelector('.idpath summary'); const r = f.getBoundingClientRect(); return { top: Math.round(document.querySelector('#main .idhow').getBoundingClientRect().bottom), lineH: parseFloat(getComputedStyle(f).lineHeight) || 18, fh: r.height, ell: getComputedStyle(f).textOverflow, nestedBg: getComputedStyle(document.querySelector('.idopt')).backgroundColor }; });
    await page.screenshot({ path: path.join(SHOTS, `identify-${w}.png`) });
    check(Math.abs(fin.top - first.top) <= 4, `${at}: the header stays where it was once the list comes (${first.top} → ${fin.top})`);
    check(fin.fh <= fin.lineH + 4 && fin.ell === 'ellipsis' && /rgba\(0, 0, 0, 0\)|transparent/.test(fin.nestedBg), `${at}: the file's name is one ellipsised line (${Math.round(fin.fh)} px) and the candidates are rows, not cards in a card`);
    const ov3 = await overflow(page);
    check(ov3.sw <= ov3.vw && !ov3.out.length, `${at}: nothing on Confirm matches is wider than the screen${ov3.out.length ? ': ' + ov3.out.join(', ') : ''}`);
    check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
    await context.close();
  }
  // ---- real touch input (the browser's own touch handling: touch-action, implicit capture) ----
  {
    const w = 390; const h = 844; const at = `${w}x${h} touch`;
    const { page, context, state } = await fx.create(browser, w, { mobile: true, fixtures });
    await page.setViewportSize({ width: w, height: h });
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
    await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn'); localStorage.setItem('pm.theme', 'dark'); window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
    const cdp = await context.newCDPSession(page);
    const touch = async (pts, opts = {}) => {
      const [x0, y0] = pts[0];
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: x0, y: y0 }] });
      if (opts.hold) await sleep(opts.hold);
      for (let i = 1; i < pts.length; i++) { await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: pts[i][0], y: pts[i][1] }] }); await sleep(16); if (i === 2 && opts.mid) await page.evaluate(opts.mid); }
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    };
    const line = (x0, y0, dx, dy, n = 8) => Array.from({ length: n + 1 }, (_, i) => [x0 + (dx * i) / n, y0 + (dy * i) / n]);
    const mid = (sel) => page.$eval(sel, (e) => { const r = e.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; });
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs .srow'); await sleep(600);
    // a sideways swipe on the mini player while it redraws (as it does several times a second while playing)
    const n0 = sent(state, 'next');
    const [mx, my] = await mid('#np .np-text');
    await touch(line(mx + 60, my, -170, 0), { mid: () => { const np = document.querySelector('#np'); np.innerHTML = np.innerHTML; } }); await sleep(700);
    const after = await page.evaluate(() => ({ t: document.querySelector('#np').style.transform, cls: document.querySelector('#np').className }));
    check(sent(state, 'next') === n0 + 1 && !after.t && !/dragging/.test(after.cls), `${at}: a finger's swipe left on the mini player is the next song, also when the bar redraws during it (${sent(state, 'next') - n0}, transform "${after.t}")`);
    // a finger held on a row: selection with that row ticked
    const [rx, ry] = await mid('#isongs .srow:nth-child(2) .grow');
    await touch([[rx, ry], [rx, ry]], { hold: 900 }); await sleep(300);
    const sel = await page.evaluate(() => ({ on: document.querySelector('#isongs').classList.contains('selecting'), ticked: [...document.querySelectorAll('#isongs .selbox input')].map((c) => c.checked) }));
    check(sel.on && sel.ticked[1] && sel.ticked.filter(Boolean).length === 1, `${at}: a finger held on a song selects it (${sel.on}, ${sel.ticked.filter(Boolean).length} ticked)`);
    await page.evaluate(() => { const c = document.querySelector('.selbar [data-sel="cancel"]'); if (c) c.click(); }); await sleep(200);
    // the sheet: a finger's sideways swipe on its list changes the tab; Escape closes the sheet first
    await page.click('#np'); await page.waitForSelector('#nowplaying:not([hidden]) .np-dock'); await sleep(300);
    await page.click('.np-dock [data-npsheet="queue"]'); await page.waitForSelector('.np-bsheet:not([hidden])'); await sleep(400);
    const [bx, by] = await mid('.np-bsheet .np-sheet-body');
    await touch(line(bx + 120, by, -240, 6)); await sleep(800);
    check(await page.$eval('.np-bsheet', (s) => s.dataset.tab) === 'lyrics', `${at}: a finger's sideways swipe on the sheet's list moves to Lyrics`);
    const inert = await page.evaluate(() => ['.np-top', '.np-body', '.np-dock'].every((s) => document.querySelector(`#nowplaying ${s}`).inert));
    check(inert, `${at}: what the sheet covers is out of reach (inert) while it shows`);
    await page.keyboard.press('Escape'); await sleep(400);
    check(await page.evaluate(() => document.querySelector('.np-bsheet').hidden && !document.querySelector('#nowplaying').hidden), `${at}: Escape closes the sheet first`);
    // Sound from ···: the panel, and the focus back on ··· after it
    await page.click('#npmore'); await page.click('.popover.menu:not([hidden]) >> text=Sound'); await page.waitForSelector('#qpanel:not([hidden])');
    await page.keyboard.press('Escape'); await sleep(300);
    check(await page.evaluate(() => document.querySelector('#qpanel').hidden && document.activeElement && document.activeElement.id === 'npmore'), `${at}: Sound opens from Now playing's ···, and the focus comes back to ···`);
    await page.keyboard.press('Escape'); await sleep(300);
    check(await page.evaluate(() => document.querySelector('#nowplaying').hidden), `${at}: Escape again closes Now playing`);
    // a playlist page keeps its own header (the album layout is for the item pages)
    await fx.go(page, '/playlist/c1'); await page.waitForSelector('#plsongs .srow'); await sleep(400);
    const pl = await page.evaluate(() => { const hero = document.querySelector('#main > .hero'); const kids = [...hero.querySelectorAll(':scope > div:not(.art) > *')].map((e) => Math.round(e.getBoundingClientRect().width)); return { item: hero.classList.contains('item-hero'), min: Math.min(...kids.filter((x) => x > 0)), sw: document.documentElement.scrollWidth, vw: innerWidth }; });
    check(!pl.item && pl.min > 150 && pl.sw <= pl.vw, `${at}: a playlist's header is laid out as before (narrowest part ${pl.min} px, page ${pl.sw} of ${pl.vw})`);
    check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
    await context.close();
  }
  // ---- a desktop window 700 px wide: the phone arrangement above the player bar ----
  {
    const { page, context, state } = await fx.create(browser, 700, { fixtures });
    await page.setViewportSize({ width: 700, height: 900 });
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
    await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn'); localStorage.setItem('pm.theme', 'dark'); });
    await fx.go(page, '/albums'); await page.waitForSelector('#npart'); await sleep(500);
    await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden]) .np-dock'); await sleep(400);
    const d = await page.evaluate(() => { const bar = document.querySelector('.player').getBoundingClientRect().top; const dock = document.querySelector('.np-dock').getBoundingClientRect().bottom; const pg = document.querySelector('.np-page'); return { shell: document.documentElement.dataset.shell, bar, dock, over: pg.scrollHeight - pg.clientHeight }; });
    check(d.shell === 'desktop' && d.dock <= d.bar + 0.5 && d.over <= 1, `700x900 desktop: Now playing's first screen ends above the player bar (buttons end at ${Math.round(d.dock)}, the bar at ${Math.round(d.bar)}, overflow ${d.over})`);
    await page.click('.np-dock [data-npsheet="lyrics"]'); await page.waitForSelector('.np-bsheet:not([hidden])'); await sleep(400);
    const sb = await page.evaluate(() => ({ s: document.querySelector('.np-bsheet').getBoundingClientRect().bottom, bar: document.querySelector('.player').getBoundingClientRect().top }));
    check(sb.s <= sb.bar + 0.5, `700x900 desktop: the sheet ends above the player bar (${Math.round(sb.s)} / ${Math.round(sb.bar)})`);
    await page.goBack(); await sleep(400);
    check(await page.evaluate(() => document.querySelector('.np-bsheet').hidden && !document.querySelector('#nowplaying').hidden), '700x900 desktop: Back closes the sheet first');
    check(!state.errors.length, `700x900 desktop: no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
    await context.close();
  }
  // ---- a tablet's open Lyrics, then the window made narrow: Back still closes the sheet first ----
  {
    const { page, context, state } = await fx.create(browser, 1024, { fixtures });
    await page.setViewportSize({ width: 1024, height: 800 });
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
    await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn'); });
    await fx.go(page, '/albums'); await page.waitForSelector('#npart'); await sleep(500);
    await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden]) .np-sheet-tabs'); await sleep(300);
    await page.click('.np-sheet-tabs [data-npsheet="lyrics"]'); await sleep(300);
    await page.setViewportSize({ width: 680, height: 800 }); await sleep(700);
    const shown = await page.evaluate(() => { const s = document.querySelector('.np-bsheet'); return s && !s.hidden ? s.dataset.tab : null; });
    await page.goBack(); await sleep(400);
    check(shown === 'lyrics' && await page.evaluate(() => document.querySelector('.np-bsheet').hidden && !document.querySelector('#nowplaying').hidden), `1024 → 680: the open Lyrics becomes the phone's sheet (${shown}) and Back closes it first`);
    check(!state.errors.length, `1024 → 680: no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed');
  console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})();
