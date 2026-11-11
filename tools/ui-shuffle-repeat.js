// Shuffle and repeat work like a standard player, the same everywhere (TASK-shuffle-repeat-standard-1105,
// Part 2; owner: "shuffle and repeat buttons don't seem consistent"). Real candidate assets on the
// fixture origin (tools/ui-fixture.js): a zone "z1" playing a queue of 12 and a smart playlist of the
// same 12 songs; the event socket is this suite's. At 360, 390, 412, 768, 1024 and 1440 px, dark and
// light:
//   * the order Shuffle · Previous · Play · Next · Repeat in the bar and in Now playing, and both modes
//     on screen in one of them;
//   * shuffle from Now playing (or the bar where Now playing has no transport): one PUT {shuffle}, then
//     every copy on (.on, the dot under the icon, aria-pressed, "Shuffle: on") and no toast; the
//     node's next state turning it off (another app) turns every copy off;
//   * repeat cycles off → all → one (its icon) → off, every copy following;
//   * the Queue page's Shuffle is the same toggle, with its state, keeping the focus;
//   * the node's own zone event comes before each answer, as on a real node;
//   * a smart playlist's Play and Shuffle load the zone's queue in its order with shuffle off / on
//     (one POST load; Shuffle starts on a random song; repeat stays);
//   * a playlist's Play on the zone turns shuffle off before it plays;
//   * the zone's loads and toggles go one after another, in the order asked: a toggle after a queued
//     Shuffle turns it off, a queued load takes the repeat set meanwhile, and a press shows at once;
//   * a page with no songs of its own (found elsewhere) loads what is found with the mode asked for, and
//     keeps its turn among the zone's changes while it is found (its mode pending meanwhile, and its
//     load never putting that mode back over a newer press); a repeat press waits its turn too; an add to the queue waits for a load
//     asked before it; a playlist's Play counts as asking shuffle off; a toggle the node refuses
//     shows the mode as it is;
//   * Shuffle starts on a random song (on the zone and here: not always the first, over three);
//   * This browser: Play loads in order with shuffle off; shuffle on keeps the current song and draws
//     the rest, and Now playing's Up next shows the new order; off puts the queue back in its order
//     with the current song in its place; "Add to queue" while on ends the original order and "Play
//     next" is next in both; Shuffle loads with shuffle on; a move to the zone carries shuffle and
//     repeat, the items in their original order and the current song's place in it; a move back
//     carries shuffle on.
//   node tools/ui-shuffle-repeat.js [repo] [shots]   (UI_WIDTHS=390 for one width)
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'shuffle-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let checks = 0;
const check = (ok, what) => { checks++; console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const end = Date.now() + ms; for (;;) { try { if (await fn()) return true; } catch { /* not yet */ } if (Date.now() > end) return false; await sleep(60); } };
const json = (r, body, status = 200) => r.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

const N = 12;
const tracks = Array.from({ length: N }, (_, k) => ({ id: k + 1, title: `Song ${k + 1}`, artists: [{ id: 11, name: 'Band' }], album: { id: 7, title: 'Album' }, duration_ms: 200000 }));
const inOrder = tracks.map((t) => `track:${t.id}`);
// A second of silence: This browser plays it (no error toast in the way)
const WAV = (() => { const n = 8000; const b = Buffer.alloc(44 + n); b.write('RIFF', 0); b.writeUInt32LE(36 + n, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(8000, 24); b.writeUInt32LE(8000, 28); b.writeUInt16LE(1, 32); b.writeUInt16LE(8, 34); b.write('data', 36); b.writeUInt32LE(n, 40); b.fill(128, 44); return b; })();

const zoneOf = () => {
  const queue = tracks.map((t, k) => ({ id: k + 1, uri: `track:${t.id}`, title: t.title, artist: 'Band', album: 'Album', duration_ms: t.duration_ms }));
  return { id: 'z1', name: 'Living room', output: 'upnp:sp', transport: 'playing', now_playing: { item: queue[0], index: 0, position_ms: 30000, duration_ms: 200000 }, queue, queue_index: 0, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false } };
};

// Every copy of each mode button: its state, and whether it shows (and shows the dot)
const modes = (page) => page.evaluate(() => {
  const vis = (b) => b.getClientRects().length > 0 && getComputedStyle(b).visibility !== 'hidden' && !b.closest('[hidden]');
  const one = (b) => { const a = getComputedStyle(b, '::after'); return { where: b.id ? 'bar' : 'np', on: b.classList.contains('on'), pressed: b.getAttribute('aria-pressed'), label: b.getAttribute('aria-label'), ic: (b.querySelector('use') || { getAttribute: () => '' }).getAttribute('href'), vis: vis(b), dot: a.content !== 'none' && a.width === '4px' }; };
  return { shuffle: [...document.querySelectorAll('#shufflebtn, [data-npt="shuffle"]')].map(one), repeat: [...document.querySelectorAll('#repeatbtn, [data-npt="repeat"]')].map(one) };
});
const allAre = (copies, on, label) => copies.length > 0 && copies.every((c) => c.on === on && c.pressed === String(on) && c.label === label && (!c.vis || c.dot === on));
const say = (copies) => copies.map((c) => `${c.where}${c.vis ? '' : '(hidden)'}:${c.on ? 'on' : 'off'}/${c.pressed}/${c.label}${c.vis ? (c.dot ? '/dot' : '/no dot') : ''}`).join(' ');
// The copy to press: Now playing's where it shows its transport, else the bar's
const pressMode = async (page, which) => {
  const np = page.locator(`#nowplaying:not([hidden]) [data-npt="${which}"]`);
  if (await np.count() && await np.isVisible()) return np.click();
  return page.locator(which === 'shuffle' ? '#shufflebtn' : '#repeatbtn').click();
};
// A press that lands now: Now playing redraws while a load is on its way, and a click that waits for a
// settled button lands after it; this one goes as soon as the button has its handler
const pressNow = (page, which) => until(async () => page.evaluate((which) => { const b = document.querySelector(`#nowplaying:not([hidden]) [data-npt="${which}"]`) || document.querySelector(which === 'shuffle' ? '#shufflebtn' : '#repeatbtn'); if (!b || !b.onclick) return false; b.click(); return true; }, which), 1500);
const openNp = async (page, w) => { if (await page.locator('#nowplaying:not([hidden])').count()) return; await page.click(w >= 600 && await page.locator('#npart').isVisible() ? '#npart' : '#np'); await page.waitForSelector('#nowplaying:not([hidden])'); await sleep(250); };
// (Escape closes an open sheet over a phone's player first, then Now playing)
const closeNp = async (page) => { for (let k = 0; k < 3 && await page.locator('#nowplaying:not([hidden])').count(); k++) { await page.keyboard.press('Escape'); await until(async () => !(await page.locator('#nowplaying:not([hidden])').count()), 1000); } };
// Play to: the bar's zone button, or on a phone (no zone button in the bar) the zone named at the top of Now playing
const openZonePop = async (page, w) => { if (await page.locator('#zonebtn').isVisible()) return page.click('#zonebtn'); await openNp(page, w); await page.click('#npzone'); await page.waitForSelector('#zonepop:not([hidden])'); };
// Up next: beside the player on wider screens, a sheet behind the dock's Up next on a phone
const openUpNext = async (page) => { const dq = page.locator('#nowplaying:not([hidden]) .np-dock [data-npsheet="queue"]'); if (await dq.count() && await dq.isVisible() && !(await page.locator('#nowplaying:not([hidden]) [data-npqi]').count())) { await dq.click(); await sleep(300); } };
const saved = (page) => page.evaluate(() => { try { return { q: JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').map((it) => it.uri), at: JSON.parse(localStorage.getItem('pm.browserAt') || 'null'), shuffle: localStorage.getItem('pm.shuffle'), repeat: localStorage.getItem('pm.repeat') }; } catch { return null; } });
const title = (uri) => `Song ${uri.split(':')[1]}`;
const upNext = (page) => page.evaluate(() => [...document.querySelectorAll('#nowplaying:not([hidden]) [data-npqi] .name')].map((e) => e.textContent.trim()));
// A song row's ⋯ menu, then one of its entries
const rowMenu = async (page, song, entry) => { const row = page.locator('.srow', { hasText: song }).first(); await row.locator('[data-smore]').click(); await page.locator('.popover .pop-item', { hasText: entry }).first().click(); await sleep(300); };
const isPermutation = (q) => q.length === N && [...q].sort().join() === [...inOrder].sort().join();

(async () => {
  const browser = await chromium.launch();
  try {
    // (UI_WIDTHS=390,1440 runs only those)
    for (const w of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [360, 390, 412, 768, 1024, 1440])) {
      for (const scheme of ['dark', 'light']) {
        const T = `${w} ${scheme}:`;
        const Z = zoneOf(); const sent = [];
        const fixtures = { '/zones': { body: [Z] }, '/zones/z1': { get body() { return Z; } }, '/zones/z1/queue': { get body() { return Z.queue; } }, '/outputs': { body: [{ id: 'upnp:sp', name: 'Speaker', kind: 'upnp', caps: { hw_volume: 'percent' } }] }, '/smart/s1/tracks': { body: { playlist: { id: 's1', name: 'Mix', rules: [], matches: 'all' }, tracks, matched: N } } };
        for (const t of tracks) fixtures[`/library/tracks/${t.id}`] = { body: t };
        // A page with no songs of its own: discovery finds three library songs for it
        fixtures['/discover/play'] = { body: { plays_from: { service: 'library', label: 'Your library' }, track_ids: [1, 2, 3] } };
        fixtures['/item/album/mb-1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e'] = { body: { kind: 'album', id: 'mb-1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e', title: 'Found elsewhere', section: 'albums', source: 'musicbrainz', target: { uri: 'https://musicbrainz.org/release-group/1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e' }, uri: 'https://example.com/album', open_on: [{ url: 'https://example.com/album' }], songs: [], releases: [], links: [], also_on: [], tags: [] } };
        let slowLoad = 0; let refuseShuffle = false; let loadGate = null; // (a load held until the test lets it go)
        let remoteRepeat = null; // (another app sets repeat after this one's PUT; its event comes before the 204)
        // The playing song's page: another copy of it in the library, for the Sources sheet's "Play from here"
        fixtures['/item/song/lib-1'] = { body: { kind: 'song', id: 'lib-1', title: 'Song 1', section: 'songs', source: 'library', also_on: [{ source: 'library', uri: 'track:9', label: 'Your library (other copy)' }], releases: [], links: [], tags: [] } };
        const { page, context, state } = await fx.create(browser, w, { mobile: w < 600, fixtures });
        await page.setViewportSize({ width: w, height: w < 600 ? 800 : 900 });
        await page.emulateMedia({ colorScheme: scheme });
        let seq = 0; let sock = null;
        const snapshot = () => sock && sock.send(JSON.stringify({ type: 'snapshot', seq: seq++, boot: 'fixture-boot', library_rev: 0, zones: [Z] }));
        await context.routeWebSocket(/\/v1\/events/, (ws) => { sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); snapshot(); });
        await page.route(/\/v1\/playlists\/c1\/play$/, (r) => { sent.push({ m: r.request().method(), v: '/playlists/c1/play', body: null }); return json(r, { queued: 1, missing: [] }); });
        await page.route(/\/v1\/zones\/z1\/(shuffle|repeat|load|transport|queue)(\?|$)/, async (r) => {
          const req = r.request(); const v = new URL(req.url()).pathname.slice(3); let body = null; try { body = JSON.parse(req.postData() || 'null'); } catch { /* none */ }
          const entry = { m: req.method(), v, body, at: Date.now() }; sent.push(entry);
          // (a real node's zone event comes before its answer: the answer is then dropped as not newer)
          const event = () => sock && sock.send(JSON.stringify({ type: 'zone', seq: seq++, state: Z }));
          if (/\/queue$/.test(v) && req.method() === 'POST') { return json(r, { ids: [] }); }
          if (/\/shuffle$/.test(v) && req.method() === 'PUT' && refuseShuffle) { refuseShuffle = false; return json(r, { error: 'refused for the test' }, 500); }
          if (/\/shuffle$/.test(v) && req.method() === 'PUT') { Z.shuffle = !!body.shuffle; Z.queue_version++; event(); await sleep(50); return json(r, Z); }
          if (/\/shuffle$/.test(v)) { Z.shuffle = true; return r.fulfill({ status: 204, body: '' }); }
          if (/\/repeat$/.test(v)) { Z.repeat = body.repeat; if (remoteRepeat) { Z.repeat = remoteRepeat; remoteRepeat = null; event(); await sleep(50); } entry.done = Date.now(); return r.fulfill({ status: 204, body: '' }); }
          if (/\/queue$/.test(v)) { if (req.method() === 'DELETE') { Z.queue = []; Z.now_playing = null; Z.transport = 'stopped'; return r.fulfill({ status: 204, body: '' }); } return json(r, Z.queue); }
          // (a slow one delays only its answer and event; after a reload they'd be another page's)
          if (/\/load$/.test(v)) { Z.shuffle = !!body.shuffle; Z.queue = body.items.map((it, k) => ({ ...it, id: 100 + k })); Z.queue_version++; Z.now_playing = { item: Z.queue[body.index || 0], index: body.index || 0, position_ms: 0, duration_ms: 200000 }; const asked = sock; if (slowLoad) { const d = slowLoad; slowLoad = 0; await sleep(d); } if (loadGate) await loadGate; if (sock === asked) event(); await sleep(50); entry.done = Date.now(); return json(r, Z); }
          return json(r, Z);
        });
        await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => r.fulfill({ status: 200, contentType: 'audio/wav', body: WAV }));
        // (the fixture sets This browser on every load: the zone this suite is on, kept per tab)
        await context.addInitScript(() => { if (!sessionStorage.getItem('pm.test.zone')) { localStorage.removeItem('pm.shuffle'); localStorage.removeItem('pm.repeat'); localStorage.removeItem('pm.browserQueue'); sessionStorage.setItem('pm.test.zone', 'z1'); } localStorage.setItem('pm.zone', sessionStorage.getItem('pm.test.zone')); });
        // A seeded Math.random (the same draws on every run: the random-start checks can't fail by chance)
        await context.addInitScript(() => { let a = 0x2545f491; Math.random = () => { a = (a + 0x6d2b79f5) | 0; let t = Math.imul(a ^ (a >>> 15), 1 | a); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; });
        const toasted = () => page.evaluate(() => { const t = document.querySelector('#toast'); return t.classList.contains('show') ? t.textContent : ''; });
        const asked = (m, re) => sent.filter((x) => x.m === m && re.test(x.v));

        // ---- A zone of the node
        await fx.go(page, '/albums'); await page.waitForSelector('#np'); await sleep(400);
        const barOrder = await page.evaluate(() => [...document.querySelector('#shufflebtn').parentElement.querySelectorAll(':scope > button')].map((b) => b.id));
        check(barOrder.join() === 'shufflebtn,prev,toggle,next,repeatbtn', `${T} the bar: Shuffle · Previous · Play · Next · Repeat (${barOrder.join(', ')})`);
        await openNp(page, w);
        const npOrder = await page.evaluate(() => [...document.querySelectorAll('#nowplaying .np-transport [data-npt]')].map((b) => b.dataset.npt));
        check(npOrder.join() === 'shuffle,prev,toggle,next,repeat', `${T} Now playing: Shuffle · Previous · Play · Next · Repeat (${npOrder.join(', ')})`);
        let m = await modes(page);
        check(m.shuffle.some((c) => c.vis) && m.repeat.some((c) => c.vis), `${T} both modes are on screen (${say(m.shuffle)} | ${say(m.repeat)})`);
        check(allAre(m.shuffle, false, 'Shuffle: off') && allAre(m.repeat, false, 'Repeat: off'), `${T} every copy says off at first: ${say(m.shuffle)} | ${say(m.repeat)}`);
        await page.screenshot({ path: `${SHOTS}/${w}-${scheme}-np-off.png` });

        await pressMode(page, 'shuffle');
        check(await until(async () => asked('PUT', /\/zones\/z1\/shuffle$/).length === 1), `${T} shuffle sends one PUT (${sent.map((x) => `${x.m} ${x.v}`).join(', ') || 'nothing'})`);
        const put = asked('PUT', /\/zones\/z1\/shuffle$/)[0];
        check(!!put && put.body && put.body.shuffle === true, `${T} …with {shuffle: true} (${JSON.stringify(put && put.body)})`);
        check(await until(async () => allAre((await modes(page)).shuffle, true, 'Shuffle: on')), `${T} every shuffle copy is on (accent, dot, aria-pressed): ${say((await modes(page)).shuffle)}`);
        await sleep(300);
        check(!(await toasted()), `${T} …and no toast: the button is the feedback (${await toasted()})`);
        await page.screenshot({ path: `${SHOTS}/${w}-${scheme}-np-shuffle-on.png` });
        // Another app turns it off: the node's next state turns every copy off
        Z.shuffle = false; snapshot();
        check(await until(async () => allAre((await modes(page)).shuffle, false, 'Shuffle: off')), `${T} the node's state turns every copy off again: ${say((await modes(page)).shuffle)}`);

        const words = { all: 'Repeat: the whole queue', one: 'Repeat: this song', off: 'Repeat: off' };
        for (const want of ['all', 'one', 'off']) {
          const before = asked('PUT', /\/zones\/z1\/repeat$/).length;
          await pressMode(page, 'repeat');
          check(await until(async () => asked('PUT', /\/zones\/z1\/repeat$/).length === before + 1), `${T} repeat sends a PUT`);
          const last = asked('PUT', /\/zones\/z1\/repeat$/).pop();
          check(!!last && last.body.repeat === want, `${T} …with ${want} (${JSON.stringify(last && last.body)})`);
          check(await until(async () => { const r = (await modes(page)).repeat; return allAre(r, want !== 'off', words[want]) && r.every((c) => (want === 'one') === /repeat-one/.test(c.ic)); }), `${T} every repeat copy shows ${want}${want === 'one' ? ' (the "1" icon)' : ''}: ${say((await modes(page)).repeat)}`);
        }
        // The node answers 204, but another app set repeat after this PUT (its event came first): the
        // newer one stays, and a load after it would carry it (the 204 isn't taken as the last word)
        {
          remoteRepeat = 'one'; const before = asked('PUT', /\/zones\/z1\/repeat$/).length;
          await pressMode(page, 'repeat');
          await until(async () => { const e = asked('PUT', /\/zones\/z1\/repeat$/)[before]; return !!e && !!e.done; });
          await sleep(500);
          check(allAre((await modes(page)).repeat, true, words.one), `${T} repeat answered 204 after another app's newer change keeps that change: ${say((await modes(page)).repeat)}`);
          Z.repeat = 'off'; snapshot();
          await until(async () => allAre((await modes(page)).repeat, false, words.off));
        }
        // Now playing › Sources › Play from here on a playing zone: one load, and the start's wait ends
        // with the node's answer (not 20 s later with "did not start playing"). From 600 px, where Now
        // playing has its sheet tabs (the switch itself is the same at every width)
        if (w >= 600) {
          const loads0 = asked('POST', /\/zones\/z1\/load$/).length;
          await until(async () => { const t = page.locator('#nowplaying [data-npsheet="sources"]:visible').first(); if (!(await t.count())) return false; await t.click({ timeout: 1000 }); return true; }, 5000);
          const sw = await page.waitForSelector('[data-npswitch]', { timeout: 8000 }).catch(() => null);
          check(!!sw, `${T} Sources offers "Play from here" on the other copy`);
          if (sw) {
            await page.locator('[data-npswitch]').first().click();
            check(await until(async () => asked('POST', /\/zones\/z1\/load$/).length === loads0 + 1, 6000), `${T} …which loads the zone once`);
            const ld = asked('POST', /\/zones\/z1\/load$/)[loads0];
            check(!!ld && ld.body.index === 0 && ld.body.items[0].uri === 'track:9' && ld.body.items.length === N && !('shuffle' in ld.body), `${T} …with the song swapped in its place and no reshuffle (${ld && JSON.stringify({ index: ld.body.index, first: ld.body.items[0].uri, n: ld.body.items.length, shuffle: ld.body.shuffle })})`);
            check(await until(async () => !(await page.locator('#np .np-wait').count()), 3000), `${T} …and "Starting on…" ends with the node's answer (${await page.locator('#np .np-wait').count()} left)`);
            check(!/did not start/.test(await toasted() || ''), `${T} …with no "did not start" error (${await toasted()})`);
          }
          const back = page.locator('#npsheetclose'); if (await back.isVisible().catch(() => false)) await back.click();
          Object.assign(Z, zoneOf()); snapshot(); await sleep(300);
          // (the checks below count the zone's loads from their own first one)
          const rest = sent.filter((x) => !(x.m === 'POST' && /\/zones\/z1\/load$/.test(x.v))); sent.length = 0; sent.push(...rest);
        }
        await closeNp(page);

        // The Queue page's Shuffle: the same toggle, with its state
        await fx.go(page, '/queue'); await page.waitForSelector('#qshuffle'); await sleep(200);
        const qs = () => page.evaluate(() => { const b = document.querySelector('#qshuffle'); return { pressed: b.getAttribute('aria-pressed'), text: b.textContent.trim() }; });
        let q = await qs();
        check(q.pressed === 'false' && /Shuffle off/.test(q.text), `${T} the Queue page's Shuffle says off (${JSON.stringify(q)})`);
        const puts0 = asked('PUT', /\/zones\/z1\/shuffle$/).length;
        await page.click('#qshuffle');
        check(await until(async () => asked('PUT', /\/zones\/z1\/shuffle$/).length === puts0 + 1 && (await qs()).pressed === 'true'), `${T} …pressed, it turns shuffle on (PUT) and says so (${JSON.stringify(await qs())})`);
        check(allAre((await modes(page)).shuffle, true, 'Shuffle: on'), `${T} …and the bar's copy follows: ${say((await modes(page)).shuffle)}`);
        await sleep(300);
        check(await page.evaluate(() => !!document.activeElement && document.activeElement.id === 'qshuffle'), `${T} …and keeps the focus across the page's redraw (${await page.evaluate(() => (document.activeElement && (document.activeElement.id || document.activeElement.tagName)) || '')})`);
        await page.click('#qshuffle');
        check(await until(async () => (await qs()).pressed === 'false' && asked('PUT', /\/zones\/z1\/shuffle$/).pop().body.shuffle === false), `${T} …pressed again, off (${JSON.stringify(await qs())})`);

        // A page's Play and Shuffle: the queue loaded in its order, with the mode
        await fx.go(page, '/smart/s1'); await page.waitForSelector('#smplay'); await sleep(200);
        Z.repeat = 'all'; snapshot(); await sleep(150);
        await page.click('#smplay');
        check(await until(async () => asked('POST', /\/zones\/z1\/load$/).length === 1), `${T} Play loads the zone's queue (${sent.slice(-3).map((x) => `${x.m} ${x.v}`).join(', ')})`);
        let ld = asked('POST', /\/zones\/z1\/load$/)[0];
        check(!!ld && ld.body.shuffle === false && ld.body.index === 0 && ld.body.items.map((x) => x.uri).join() === inOrder.join() && ld.body.repeat === 'all', `${T} …in its order, shuffle off, from the first song, repeat kept (${ld && JSON.stringify({ shuffle: ld.body.shuffle, index: ld.body.index, repeat: ld.body.repeat })})`);
        await page.click('#smshuffle');
        check(await until(async () => asked('POST', /\/zones\/z1\/load$/).length === 2), `${T} Shuffle loads the zone's queue`);
        ld = asked('POST', /\/zones\/z1\/load$/)[1];
        check(!!ld && ld.body.shuffle === true && ld.body.items.map((x) => x.uri).join() === inOrder.join() && ld.body.index >= 0 && ld.body.index < N, `${T} …in its order with shuffle on, from a song of it (${ld && JSON.stringify({ shuffle: ld.body.shuffle, index: ld.body.index })}); the node keeps the order for shuffle off`);
        check(await until(async () => allAre((await modes(page)).shuffle, true, 'Shuffle: on')), `${T} …and the bar says shuffle on: ${say((await modes(page)).shuffle)}`);
        for (const k of [3, 4]) { await sleep(200); await page.click('#smshuffle'); await until(async () => asked('POST', /\/zones\/z1\/load$/).length === k); }
        const starts = asked('POST', /\/zones\/z1\/load$/).slice(1).map((x) => x.body.index);
        check(starts.length === 3 && starts.some((i) => i !== 0), `${T} …from a random song, not always the first (indexes ${starts.join(', ')})`);
        // A playlist's Play on the zone: shuffle off first, then the list plays in its order
        await fx.go(page, '/playlist/c1'); await page.waitForSelector('#plplay'); await sleep(200);
        const n0 = sent.length;
        await page.click('#plplay');
        check(await until(async () => sent.slice(n0).some((x) => x.v === '/playlists/c1/play')), `${T} a playlist's Play plays it on the zone`);
        const pl = sent.slice(n0).map((x) => `${x.m} ${x.v}${x.body && 'shuffle' in x.body ? ` ${x.body.shuffle}` : ''}`);
        check(pl.indexOf('PUT /zones/z1/shuffle false') >= 0 && pl.indexOf('PUT /zones/z1/shuffle false') < pl.indexOf('POST /playlists/c1/play'), `${T} …after turning shuffle off (${pl.join(', ')})`);

        // Loads and toggles, one after another in the order asked
        await fx.go(page, '/smart/s1'); await page.waitForSelector('#smplay'); await sleep(200);
        Z.shuffle = false; Z.repeat = 'all'; snapshot(); await sleep(150);
        const n1 = sent.length; slowLoad = 900;
        await page.click('#smplay'); await sleep(50); await page.click('#smshuffle'); await sleep(50);
        check(allAre((await modes(page)).shuffle, true, 'Shuffle: on'), `${T} a queued Shuffle shows at once, before the node answers: ${say((await modes(page)).shuffle)}`);
        Z.repeat = 'one'; snapshot(); await sleep(100);
        if (!(await page.locator('#shufflebtn').isVisible())) await openNp(page, w);
        await pressMode(page, 'shuffle');
        check(allAre((await modes(page)).shuffle, false, 'Shuffle: off'), `${T} …and the toggle after it shows off at once: ${say((await modes(page)).shuffle)}`);
        check(await until(async () => sent.slice(n1).filter((x) => /\/(load|shuffle)$/.test(x.v) && x.m !== 'GET').length === 3, 8000), `${T} the three go out`);
        const order = sent.slice(n1).filter((x) => /\/(load|shuffle)$/.test(x.v) && x.m !== 'GET').map((x) => `${x.v.split('/').pop()}:${x.body.shuffle}`);
        check(order.join() === 'load:false,load:true,shuffle:false' && Z.shuffle === false, `${T} …in the order asked, the toggle turning the queued Shuffle off (${order.join(', ')}; shuffle ${Z.shuffle})`);
        const queued = sent.slice(n1).filter((x) => /\/load$/.test(x.v))[1];
        check(!!queued && queued.body.repeat === 'one', `${T} …and the queued load takes the repeat set while it waited (${queued && queued.body.repeat})`);
        await closeNp(page);
        // A page with no songs of its own: Shuffle loads what is found, with shuffle on
        await fx.go(page, '/album/mb-1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e'); await page.waitForSelector('#ishuffle'); await sleep(200);
        const n2 = asked('POST', /\/zones\/z1\/load$/).length;
        await page.click('#ishuffle');
        check(await until(async () => asked('POST', /\/zones\/z1\/load$/).length === n2 + 1, 6000), `${T} Shuffle on a page found elsewhere loads what discovery found`);
        const found = asked('POST', /\/zones\/z1\/load$/)[n2];
        check(!!found && found.body.shuffle === true && found.body.items.length === 3, `${T} …with shuffle on, the three songs found (${found && JSON.stringify({ shuffle: found.body.shuffle, n: found.body.items.length })})`);

        // A refused toggle shows the mode as it is again
        await fx.go(page, '/smart/s1'); await page.waitForSelector('#smplay'); await sleep(200);
        Z.shuffle = false; snapshot(); await sleep(150);
        refuseShuffle = true;
        if (!(await page.locator('#shufflebtn').isVisible())) await openNp(page, w);
        await pressMode(page, 'shuffle');
        check(await until(async () => /refused for the test/.test(await toasted())), `${T} a toggle the node refuses says so`);
        check(await until(async () => allAre((await modes(page)).shuffle, false, 'Shuffle: off')), `${T} …and every copy shows the mode as it is: ${say((await modes(page)).shuffle)}`);
        await closeNp(page);
        // An add to the queue waits for the load asked before it
        const n3 = sent.length; slowLoad = 4000;
        await page.click('#smplay'); await sleep(50);
        await rowMenu(page, 'Song 4', 'Add to queue');
        if (!(await page.locator('#repeatbtn').isVisible())) await openNp(page, w);
        await pressMode(page, 'repeat'); await closeNp(page);
        check(await until(async () => sent.slice(n3).filter((x) => x.m === 'POST' && /\/(load|queue)$/.test(x.v)).length === 2, 8000), `${T} the load and the add go out`);
        check(await until(async () => sent.slice(n3).some((x) => /\/repeat$/.test(x.v)), 8000), `${T} repeat pressed while the load waits goes out`);
        const [ld3, add3] = sent.slice(n3).filter((x) => x.m === 'POST' && /\/(load|queue)$/.test(x.v)); const rp3 = sent.slice(n3).find((x) => /\/repeat$/.test(x.v));
        check(!!rp3 && !!ld3 && ld3.done && rp3.at >= ld3.done, `${T} …and only once that load is answered (repeat at +${rp3 && ld3 ? rp3.at - ld3.at : '?'} ms, the load answered at +${ld3 && ld3.done ? ld3.done - ld3.at : '?'})`);
        check(!!ld3 && !!add3 && /load$/.test(ld3.v) && /queue$/.test(add3.v) && ld3.done && add3.at >= ld3.done, `${T} …the add goes only once the load it was asked after is answered (add at +${add3 && ld3 ? add3.at - ld3.at : '?'} ms, the load answered at +${ld3 && ld3.done ? ld3.done - ld3.at : '?'})`);
        // A playlist's Play asks shuffle off: a toggle after it, queued behind a slow Shuffle, turns it on
        slowLoad = 900; const n4 = sent.length;
        await page.click('#smshuffle'); await sleep(50);
        // (to the playlist within the app: a reload would start the zone's chain afresh)
        await page.evaluate(() => { history.pushState(null, '', '/playlist/c1'); window.dispatchEvent(new PopStateEvent('popstate', { state: null })); });
        await page.waitForSelector('#plplay');
        await page.click('#plplay'); await sleep(50);
        if (!(await page.locator('#shufflebtn').isVisible())) await openNp(page, w);
        await pressMode(page, 'shuffle');
        check(await until(async () => sent.slice(n4).some((x) => x.v === '/playlists/c1/play') && sent.slice(n4).filter((x) => /\/shuffle$/.test(x.v) && x.m === 'PUT').length === 2, 10000), `${T} the Shuffle, the playlist's Play and the toggle go out`);
        const plOrder = sent.slice(n4).filter((x) => /\/(load|shuffle|play)$/.test(x.v) && x.m !== 'GET').map((x) => `${x.v.split('/').pop()}${x.body && 'shuffle' in x.body ? ':' + x.body.shuffle : ''}`);
        check(plOrder.join() === 'load:true,shuffle:false,play,shuffle:true', `${T} …in order, the toggle turning shuffle on after the Play turned it off (${plOrder.join(', ')})`);
        await closeNp(page);
        // Discovery keeps its turn: a toggle pressed while a page is still being found goes after its load
        await fx.go(page, '/album/mb-1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e'); await page.waitForSelector('#ishuffle'); await sleep(200);
        Z.shuffle = false; snapshot();
        check(await until(async () => allAre((await modes(page)).shuffle, false, 'Shuffle: off')), `${T} (the page shows shuffle off before: ${say((await modes(page)).shuffle)})`);
        state.delay = (v) => (v === '/discover/play' ? 800 : 0);
        const n5 = sent.length;
        await page.click('#ishuffle'); await sleep(100);
        if (!(await page.locator('#shufflebtn').isVisible())) await openNp(page, w);
        check(allAre((await modes(page)).shuffle, true, 'Shuffle: on'), `${T} the page's Shuffle is the pending mode while its songs are found: ${say((await modes(page)).shuffle)}`);
        check(await pressNow(page, 'shuffle'), `${T} a toggle lands while the songs are found`);
        // (the page's load, going after the press, must not put its "on" back over the press's "off":
        // a third press while that load is on its way turns from the press, not from the load)
        // (the load is held until the third press has landed: the press is surely while it is on its way)
        let release; loadGate = new Promise((r) => { release = r; });
        check(await until(async () => sent.slice(n5).some((x) => /\/load$/.test(x.v)), 8000), `${T} the found page's load goes out`);
        // (pressed as soon as the button has its handler: Now playing redraws while the load is on its
        // way, and a click that waits for a settled button would land after it)
        await sleep(150);
        check(await pressNow(page, 'shuffle'), `${T} …a third press lands while that load is on its way`);
        loadGate = null; release();
        check(await until(async () => sent.slice(n5).filter((x) => /\/(load|shuffle)$/.test(x.v) && x.m !== 'GET').length === 3, 8000), `${T} …and the two toggles after it`);
        const dOrder = sent.slice(n5).filter((x) => /\/(load|shuffle)$/.test(x.v) && x.m !== 'GET').map((x) => `${x.v.split('/').pop()}:${x.body.shuffle}`);
        check(dOrder.join() === 'load:true,shuffle:false,shuffle:true', `${T} …the load first, then the toggles off and on again (${dOrder.join(', ')})`);
        state.delay = 0;
        await closeNp(page);

        // ---- This browser
        await page.evaluate(() => { sessionStorage.setItem('pm.test.zone', 'browser'); localStorage.removeItem('pm.repeat'); });
        await fx.go(page, '/smart/s1'); await page.waitForSelector('#smplay'); await sleep(200);
        await page.click('#smplay');
        check(await until(async () => { const s = await saved(page); return s.q.join() === inOrder.join() && !s.shuffle; }, 6000), `${T} here, Play queues the songs in order with shuffle off (${JSON.stringify(await saved(page))})`);
        check(allAre((await modes(page)).shuffle, false, 'Shuffle: off'), `${T} …and the buttons say off: ${say((await modes(page)).shuffle)}`);
        await openNp(page, w);
        await pressMode(page, 'shuffle');
        check(await until(async () => { const s = await saved(page); return s.shuffle === 'on' && s.q[0] === inOrder[0] && isPermutation(s.q) && s.q.join() !== inOrder.join(); }, 6000), `${T} shuffle on here: the current song stays first, the rest is drawn (${(await saved(page)).q.join(' ')})`);
        await openUpNext(page);
        check(await until(async () => { const s = await saved(page); const rows = await upNext(page); return rows.length > 0 && rows.join() === s.q.slice(s.at.index + 1, s.at.index + 1 + rows.length).map(title).join(); }), `${T} …and Now playing's Up next shows the drawn order (${(await upNext(page)).slice(0, 4).join(', ')})`);
        check(allAre((await modes(page)).shuffle, true, 'Shuffle: on'), `${T} …every copy on: ${say((await modes(page)).shuffle)}`);
        // Next, then off: back in order with the current song in its place
        const drawn = (await saved(page)).q; const now = drawn[1];
        await closeNp(page); await page.locator('body').click({ position: { x: 2, y: 2 } }).catch(() => {});
        await page.keyboard.press('ArrowRight');
        check(await until(async () => { const s = await saved(page); return s.at && s.q[s.at.index] === now; }, 6000), `${T} Next plays the drawn second song (${now})`);
        if (!(await page.locator('#shufflebtn').isVisible())) await openNp(page, w);
        await pressMode(page, 'shuffle');
        check(await until(async () => { const s = await saved(page); return !s.shuffle && s.q.join() === inOrder.join() && s.at && s.q[s.at.index] === now; }, 6000), `${T} shuffle off: the queue is in its order again, ${now} playing in its place (${JSON.stringify(await saved(page))})`);
        await pressMode(page, 'repeat');
        check(await until(async () => (await saved(page)).repeat === 'all' && allAre((await modes(page)).repeat, true, 'Repeat: the whole queue')), `${T} repeat here cycles to all, every copy following: ${say((await modes(page)).repeat)}`);
        // While shuffled: "Add to queue" ends the original order, "Play next" is next in both
        const curNow = (await saved(page)).q[(await saved(page)).at.index];
        await pressMode(page, 'shuffle'); await until(async () => (await saved(page)).shuffle === 'on');
        await closeNp(page);
        await rowMenu(page, 'Song 5', 'Add to queue'); await rowMenu(page, 'Song 7', 'Play next');
        check(await until(async () => { const s = await saved(page); return s.q.length === N + 2 && s.q[s.q.length - 1] === 'track:5' && s.q[s.at.index + 1] === 'track:7'; }, 6000), `${T} while shuffled, Add to queue goes last and Play next right after the current song (${(await saved(page)).q.join(' ')})`);
        if (!(await page.locator('#shufflebtn').isVisible())) await openNp(page, w);
        await pressMode(page, 'shuffle');
        const wantOrder = [...inOrder]; wantOrder.splice(inOrder.indexOf(curNow) + 1, 0, 'track:7'); wantOrder.push('track:5');
        check(await until(async () => { const s = await saved(page); return !s.shuffle && s.q.join() === wantOrder.join() && s.q[s.at.index] === curNow; }, 6000), `${T} …then shuffle off: in order, the next song after ${curNow}, the added one last (${(await saved(page)).q.join(' ')})`);
        await closeNp(page);
        const firsts = [];
        for (let k = 0; k < 3; k++) {
          const before = (await saved(page)).q.join();
          await page.click('#smshuffle');
          check(await until(async () => { const s = await saved(page); return s.shuffle === 'on' && isPermutation(s.q) && s.q.join() !== inOrder.join() && s.q.join() !== before; }, 6000), `${T} Shuffle here loads with shuffle on (${(await saved(page)).q.join(' ')})`);
          firsts.push((await saved(page)).q[0]);
        }
        check(firsts.some((f) => f !== inOrder[0]), `${T} …from a random song, not always the first (${firsts.join(', ')})`);
        // Move to the zone: shuffle and repeat go along, the items in their original order
        const cur = await until(async () => { const s = await saved(page); return s.at && s.at.index === 0; }, 4000) ? (await saved(page)).q[0] : null;
        const loads = asked('POST', /\/zones\/z1\/load$/).length;
        await openZonePop(page, w); await page.locator('[data-move="z1"]').click();
        check(await until(async () => asked('POST', /\/zones\/z1\/load$/).length === loads + 1, 8000), `${T} Move to sends the queue to the zone`);
        const mv = asked('POST', /\/zones\/z1\/load$/)[loads];
        check(!!mv && mv.body.shuffle === true && mv.body.repeat === 'all' && mv.body.items.map((x) => x.uri).join() === inOrder.join() && cur != null && mv.body.items[mv.body.index] && mv.body.items[mv.body.index].uri === cur, `${T} …with shuffle on and repeat all, the items in their original order and the current song's place in it (${mv && JSON.stringify({ shuffle: mv.body.shuffle, repeat: mv.body.repeat, index: mv.body.index, cur })})`);
        await page.screenshot({ path: `${SHOTS}/${w}-${scheme}-moved.png` });
        // …and back: the zone's shuffle comes along (its play order stands for the original order)
        await until(async () => (await page.evaluate(() => localStorage.getItem('pm.zone'))) === 'z1', 4000);
        const zq = Z.queue.map((x) => x.uri);
        const back = page.locator('[data-move="browser"]');
        if (!(await back.isVisible())) { await page.keyboard.press('Escape'); await sleep(300); if (!(await back.isVisible())) { await closeNp(page); await openZonePop(page, w); } }
        await back.click();
        check(await until(async () => { const s = await saved(page); return s.shuffle === 'on' && s.q.join() === zq.join(); }, 8000), `${T} a move back here keeps shuffle on (${JSON.stringify((await saved(page)).shuffle)})`);

        check(!state.errors.length, `${T} no page errors ${state.errors.slice(0, 3).join(' | ')}`);
        await context.close();
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${checks} checks, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
