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
//   * the Queue page's Shuffle is the same toggle, with its state;
//   * a smart playlist's Play and Shuffle load the zone's queue in its order with shuffle off / on
//     (one POST load; Shuffle starts on a random song; repeat stays);
//   * This browser: Play loads in order with shuffle off; shuffle on keeps the current song and draws
//     the rest; off puts the queue back in its order with the current song in its place; Shuffle
//     loads with shuffle on; a move to the zone carries shuffle and repeat, the items in their
//     original order and the current song's place in it.
//   node tools/ui-shuffle-repeat.js [repo] [shots]
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
const openNp = async (page, w) => { if (await page.locator('#nowplaying:not([hidden])').count()) return; await page.click(w >= 600 && await page.locator('#npart').isVisible() ? '#npart' : '#np'); await page.waitForSelector('#nowplaying:not([hidden])'); await sleep(250); };
const closeNp = async (page) => { if (!(await page.locator('#nowplaying:not([hidden])').count())) return; await page.keyboard.press('Escape'); await until(async () => !(await page.locator('#nowplaying:not([hidden])').count()), 2000); };
const saved = (page) => page.evaluate(() => { try { return { q: JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').map((it) => it.uri), at: JSON.parse(localStorage.getItem('pm.browserAt') || 'null'), shuffle: localStorage.getItem('pm.shuffle'), repeat: localStorage.getItem('pm.repeat') }; } catch { return null; } });
const isPermutation = (q) => q.length === N && [...q].sort().join() === [...inOrder].sort().join();

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [360, 390, 412, 768, 1024, 1440]) {
      for (const scheme of ['dark', 'light']) {
        const T = `${w} ${scheme}:`;
        const Z = zoneOf(); const sent = [];
        const fixtures = { '/zones': { body: [Z] }, '/zones/z1': { get body() { return Z; } }, '/zones/z1/queue': { get body() { return Z.queue; } }, '/outputs': { body: [{ id: 'upnp:sp', name: 'Speaker', kind: 'upnp', caps: { hw_volume: 'percent' } }] }, '/smart/s1/tracks': { body: { playlist: { id: 's1', name: 'Mix', rules: [], matches: 'all' }, tracks, matched: N } } };
        const { page, context, state } = await fx.create(browser, w, { mobile: w < 600, fixtures });
        await page.setViewportSize({ width: w, height: w < 600 ? 800 : 900 });
        await page.emulateMedia({ colorScheme: scheme });
        let seq = 0; let sock = null;
        const snapshot = () => sock && sock.send(JSON.stringify({ type: 'snapshot', seq: seq++, boot: 'fixture-boot', library_rev: 0, zones: [Z] }));
        await context.routeWebSocket(/\/v1\/events/, (ws) => { sock = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); snapshot(); });
        await page.route(/\/v1\/zones\/z1\/(shuffle|repeat|load|transport)(\?|$)/, async (r) => {
          const req = r.request(); const v = new URL(req.url()).pathname.slice(3); let body = null; try { body = JSON.parse(req.postData() || 'null'); } catch { /* none */ }
          sent.push({ m: req.method(), v, body });
          if (/\/shuffle$/.test(v) && req.method() === 'PUT') { Z.shuffle = !!body.shuffle; Z.queue_version++; return json(r, Z); }
          if (/\/shuffle$/.test(v)) { Z.shuffle = true; return r.fulfill({ status: 204, body: '' }); }
          if (/\/repeat$/.test(v)) { Z.repeat = body.repeat; return r.fulfill({ status: 204, body: '' }); }
          if (/\/load$/.test(v)) { Z.shuffle = !!body.shuffle; Z.queue = body.items.map((it, k) => ({ ...it, id: 100 + k })); Z.queue_version++; Z.now_playing = { item: Z.queue[body.index || 0], index: body.index || 0, position_ms: 0, duration_ms: 200000 }; return json(r, Z); }
          return json(r, Z);
        });
        await page.route(/\/v1\/library\/tracks\/\d+\/file/, (r) => r.fulfill({ status: 200, contentType: 'audio/wav', body: WAV }));
        // (the fixture sets This browser on every load: the zone this suite is on, kept per tab)
        await context.addInitScript(() => { if (!sessionStorage.getItem('pm.test.zone')) { localStorage.removeItem('pm.shuffle'); localStorage.removeItem('pm.repeat'); localStorage.removeItem('pm.browserQueue'); sessionStorage.setItem('pm.test.zone', 'z1'); } localStorage.setItem('pm.zone', sessionStorage.getItem('pm.test.zone')); });
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

        // ---- This browser
        await page.evaluate(() => { sessionStorage.setItem('pm.test.zone', 'browser'); localStorage.removeItem('pm.repeat'); });
        await fx.go(page, '/smart/s1'); await page.waitForSelector('#smplay'); await sleep(200);
        await page.click('#smplay');
        check(await until(async () => { const s = await saved(page); return s.q.join() === inOrder.join() && !s.shuffle; }, 6000), `${T} here, Play queues the songs in order with shuffle off (${JSON.stringify(await saved(page))})`);
        check(allAre((await modes(page)).shuffle, false, 'Shuffle: off'), `${T} …and the buttons say off: ${say((await modes(page)).shuffle)}`);
        if (!(await page.locator('#shufflebtn').isVisible())) await openNp(page, w);
        await pressMode(page, 'shuffle');
        check(await until(async () => { const s = await saved(page); return s.shuffle === 'on' && s.q[0] === inOrder[0] && isPermutation(s.q) && s.q.join() !== inOrder.join(); }, 6000), `${T} shuffle on here: the current song stays first, the rest is drawn (${(await saved(page)).q.join(' ')})`);
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
        await closeNp(page);
        await page.click('#smshuffle');
        check(await until(async () => { const s = await saved(page); return s.shuffle === 'on' && isPermutation(s.q) && s.q.join() !== inOrder.join(); }, 6000), `${T} Shuffle here loads with shuffle on, from a random song (${(await saved(page)).q.join(' ')})`);
        // Move to the zone: shuffle and repeat go along, the items in their original order
        const cur = await until(async () => { const s = await saved(page); return s.at && s.at.index === 0; }, 4000) ? (await saved(page)).q[0] : null;
        const loads = asked('POST', /\/zones\/z1\/load$/).length;
        await page.click('#zonebtn'); await page.locator('[data-move="z1"]').click();
        check(await until(async () => asked('POST', /\/zones\/z1\/load$/).length === loads + 1, 8000), `${T} Move to sends the queue to the zone`);
        const mv = asked('POST', /\/zones\/z1\/load$/)[loads];
        check(!!mv && mv.body.shuffle === true && mv.body.repeat === 'all' && mv.body.items.map((x) => x.uri).join() === inOrder.join() && cur != null && mv.body.items[mv.body.index] && mv.body.items[mv.body.index].uri === cur, `${T} …with shuffle on and repeat all, the items in their original order and the current song's place in it (${mv && JSON.stringify({ shuffle: mv.body.shuffle, repeat: mv.body.repeat, index: mv.body.index, cur })})`);
        await page.screenshot({ path: `${SHOTS}/${w}-${scheme}-moved.png` });

        check(!state.errors.length, `${T} no page errors ${state.errors.slice(0, 3).join(' | ')}`);
        await context.close();
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${checks} checks, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
