// Motion and lyrics (TASK-ui-motion-polish-1106; owner: "scrollbars on lyrics … make it like a
// player's", "small subtle animations … a more premium feeling"). Real candidate assets on the
// fixture origin (tools/ui-fixture.js), a Linn playing a song with synced lyrics, at 360, 390, 834
// and 1440 px:
//   * the lyrics panel has no scrollbars (its scrollbar takes no width, nothing scrolls sideways) and
//     long lines wrap;
//   * synced lyrics: the current line is larger and fuller than the others, its neighbours step
//     down, and the panel keeps the current line about a third of the way down;
//   * a tap on a line seeks there; a scroll by hand holds the following off for a few seconds;
//   * Now playing: a pause settles the cover back and pops the play icon; a track change cross-fades
//     the cover (the last one fades out over the new) and slides the title in;
//   * app-wide: a fresh page rises in and its first tiles follow one after another (the 13th and
//     later don't); Play to rises in as a sheet (phone) or a popover (desktop); the mini player
//     cross-fades on a track change;
//   * under reduced motion every transition and animation is off (the motion scale is 0);
//   * every transition and animation in app.css takes a duration from the scale (design-lint);
//   * no horizontal overflow on any main page.
//   node tools/ui-motion.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'motion-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const queue = [{ id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa' }, { id: 2, uri: 'track:2', title: 'Flowing River', artist: 'Dr. Peacock, Sefa' }];
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = { id: 'linn', name: 'Linn Klimax DS3', output: 'upnp:linn', transport: 'playing', queue, queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: np };
// 40 lines, 2 s apart; one of them long enough to wrap on a phone
const lyrics = { synced: true, source: 'file', lines: Array.from({ length: 40 }, (_, i) => ({ ms: i * 2000, text: i === 33 ? 'A line long enough that it has to wrap on a phone screen, and on a tablet it very nearly does as well' : `Line ${i + 1} of the song` })) };
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64');
const fixtures = { '/library/tracks/1/lyrics': { body: lyrics }, '/zones': { body: [zone] }, '/outputs': { body: [{ id: 'upnp:linn', name: 'Linn', kind: 'upnp', caps: { hw_volume: 'percent' } }] }, '/zones/linn/queue': { body: queue } };

const openNp = async (browser, w, h, opts = {}) => {
  const { page, context, state } = await fx.create(browser, w, { mobile: w < 600, fixtures, ...opts });
  await page.setViewportSize({ width: w, height: h });
  // The fixture's context asks for reduced motion (its suites measure end states); the motion is the
  // point here. A 1×1 cover, so the cross-fade has an image to fade
  await page.emulateMedia({ reducedMotion: opts.reduced ? 'reduce' : 'no-preference' });
  await page.route(/\/cover(\?|$)/, (r) => r.fulfill({ contentType: 'image/png', body: PNG }));
  let seq = 0;
  await context.routeWebSocket(/\/v1\/events/, (ws) => { state.ws = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: seq++, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  // Another state of the zone, as the node would send it
  state.send = (z) => state.ws.send(JSON.stringify({ type: 'snapshot', seq: seq++, boot: 'fixture-boot', library_rev: 0, zones: [z] }));
  await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn'); });
  await fx.go(page, '/albums'); await page.waitForSelector('#np'); await sleep(500);
  await page.click(w < 600 ? '#np' : '#npart'); await page.waitForSelector('#nowplaying:not([hidden]) .np-player'); await sleep(300);
  await page.click(w >= 600 ? '.np-sheet-tabs [data-npsheet="lyrics"]' : '.np-dock [data-npsheet="lyrics"]'); await sleep(300);
  await page.waitForSelector('#nplyrics .lyric-line.on', { timeout: 8000 }); await sleep(900);
  return { page, context, state };
};
const lyricBox = (page) => page.evaluate(() => {
  const box = document.querySelector('#nplyrics .lyric-lines'); const b = box.getBoundingClientRect();
  const on = box.querySelector('.lyric-line.on'); const lines = [...box.querySelectorAll('.lyric-line')];
  const size = (el) => { const r = el.getBoundingClientRect(); return { h: r.height, w: r.width, top: r.top - b.top, op: parseFloat(getComputedStyle(el).opacity), fs: parseFloat(getComputedStyle(el).fontSize), scale: el.getBoundingClientRect().width / el.offsetWidth }; };
  const k = lines.indexOf(on);
  return { scrollbar: box.offsetWidth - box.clientWidth, sidew: box.scrollWidth - box.clientWidth, synced: box.classList.contains('synced'), n: lines.length, k, scrollTop: box.scrollTop, boxH: box.clientHeight, on: on && size(on), prev: k > 0 && size(lines[k - 1]), prev2: k > 1 && size(lines[k - 2]), far: k > 5 && size(lines[k - 5]), longWraps: lines[33].getBoundingClientRect().height > lines[0].getBoundingClientRect().height * 1.6, role: on && on.getAttribute('role') };
});

(async () => {
  const browser = await chromium.launch();
  for (const [w, h] of [[360, 800], [390, 844], [834, 1112], [1440, 900]]) {
    const at = `${w}`;
    const { page, context, state } = await openNp(browser, w, h);
    const a = await lyricBox(page);
    check(a.scrollbar === 0 && a.sidew <= 0, `${at}: the lyrics panel shows no scrollbar (${a.scrollbar} px) and nothing scrolls sideways (${a.sidew})`);
    check(a.longWraps, `${at}: a long line wraps`);
    check(a.synced && a.on && a.prev && a.on.scale > 1.1 && a.prev.scale <= 1.01 && a.on.op === 1 && a.prev.op < 1 && a.prev2.op < a.prev.op && a.far.op < a.prev2.op, `${at}: the current line is larger (×${a.on && a.on.scale.toFixed(2)}) and fuller (${a.on && a.on.op}) than its neighbours (${a.prev && a.prev.op}, ${a.prev2 && a.prev2.op}, far ${a.far && a.far.op})`);
    check(a.on && a.on.top > a.boxH * 0.15 && a.on.top < a.boxH * 0.55, `${at}: the current line sits about a third of the way down the panel (${a.on && Math.round(a.on.top)} of ${a.boxH})`);
    check(a.role === 'button', `${at}: a synced line is a button (tap to play from there)`);
    await page.$eval('#nplyrics', (el) => el.scrollIntoView({ block: 'start' })); await sleep(300);
    await page.screenshot({ path: path.join(SHOTS, `lyrics-${w}.png`) });
    // A tap on a line seeks there (the node is asked for position = the line's time)
    const hits0 = state.hits.length;
    // (a line far enough down that the phone's taller lyrics sheet can still put it at its bottom)
    const TAP = w < 600 ? 50000 : 20000;
    // (the line is put at the very bottom of the box first, outside the band the follower keeps, and
    // clicked where it is: so the follow after the seek is what moves it)
    await page.$eval(`#nplyrics .lyric-line[data-ms="${TAP}"]`, (ln) => { const box = ln.closest('.lyric-lines'); box.dataset.auto = '1'; box.scrollTop += ln.getBoundingClientRect().bottom - box.getBoundingClientRect().bottom + 2; setTimeout(() => { delete box.dataset.auto; }, 50); });
    await sleep(120);
    const placed = await page.$eval(`#nplyrics .lyric-line[data-ms="${TAP}"]`, (ln) => { const b = ln.closest('.lyric-lines').getBoundingClientRect(); const o = ln.getBoundingClientRect(); return (o.top - b.top + o.height / 2) / b.height; });
    check(placed > 0.7, `${at}: the line to tap sits low in the box before the tap (${Math.round(placed * 100)}%)`);
    // (a real tap where the line is: its touch sets the hold the seek must end)
    const lr = await page.$eval(`#nplyrics .lyric-line[data-ms="${TAP}"]`, (ln) => { const r = ln.getBoundingClientRect(); return { x: r.left + Math.min(40, r.width / 2), y: r.top + r.height / 2 }; });
    if (w < 600) await page.touchscreen.tap(lr.x, lr.y); else await page.mouse.click(lr.x, lr.y);
    await sleep(400);
    const seek = state.hits.slice(hits0).find((x) => /\/zones\/linn\/transport|\/zones\/linn\/seek/.test(x.v) && x.body && x.body.includes(String(TAP)));
    check(!!seek, `${at}: a tap on a line asks the node to play from its time (${seek ? seek.v : 'no seek sent'})`);
    // …and once the node is there, the panel follows that line at once (the tap's own hold is over)
    state.send({ ...zone, now_playing: { ...np, position_ms: TAP + 500 } }); await sleep(1000);
    const followed = await page.evaluate(() => { const box = document.querySelector('#nplyrics .lyric-lines'); const on = box.querySelector('.lyric-line.on'); const b = box.getBoundingClientRect(); const o = on && on.getBoundingClientRect(); return { ms: on && on.dataset.ms, frac: on ? (o.top - b.top + o.height / 2) / b.height : null }; });
    check(followed.ms === String(TAP) && followed.frac != null && followed.frac > 0.15 && followed.frac < 0.55, `${at}: the tapped line is followed at once (${followed.ms}, ${followed.frac != null ? Math.round(followed.frac * 100) : '?'}% down the panel)`);
    state.send({ ...zone, now_playing: { ...np, position_ms: 61000 } }); await sleep(900);
    // A scroll by hand holds the following off: the box stays where the finger left it
    // (the song moves on to line 51 meanwhile: the panel must not follow it during the hold, and
    // must follow the next line once the hold is over)
    await page.$eval('#nplyrics .lyric-lines', (box) => { box.dispatchEvent(new WheelEvent('wheel', { deltaY: 1, bubbles: true })); box.scrollTop = 0; });
    state.send({ ...zone, now_playing: { ...np, position_ms: 70000 } }); await sleep(1600);
    const held = await page.evaluate(() => ({ top: document.querySelector('#nplyrics .lyric-lines').scrollTop, on: document.querySelector('#nplyrics .lyric-line.on')?.dataset.ms }));
    check(held.top === 0 && held.on === '70000', `${at}: after a scroll by hand the panel stays put for a while (at ${held.top}, the current line moved on to ${held.on})`);
    await sleep(2600); state.send({ ...zone, now_playing: { ...np, position_ms: 74000 } }); await sleep(900);
    const resumed = await page.$eval('#nplyrics .lyric-lines', (box) => box.scrollTop);
    check(resumed > 0, `${at}: once the hold is over the panel follows again (at ${resumed})`);
    // A pause: the cover settles back and the play icon pops
    state.send({ ...zone, transport: 'paused' }); await sleep(900);
    const paused = await page.evaluate(() => { const ov = document.querySelector('#nowplaying'); const big = ov.querySelector('.np-big'); const m = new DOMMatrix(getComputedStyle(big).transform); const b = ov.querySelector('.np-transport [data-npt="toggle"]'); return { t: ov.dataset.transport, scale: m.a, label: b.getAttribute('aria-label'), swapped: !!b.querySelector('.ic.swap') }; });
    check(paused.t === 'paused' && paused.scale < 0.99 && paused.label === 'Play' && paused.swapped, `${at}: a pause settles the cover back (×${paused.scale.toFixed(2)}) and swaps the icon to Play with a pop`);
    // Under the finger the fill is where the finger is (no glide), and letting go without a change lets go of the drag
    const drag = await page.evaluate(() => { const sk = document.querySelector('#npseek'); sk.dataset.dragging = '1'; const t = getComputedStyle(sk).transitionProperty; sk.dispatchEvent(new PointerEvent('pointerup', { bubbles: true })); const bar = document.querySelector('#seek'); bar.dataset.dragging = '1'; bar.dispatchEvent(new PointerEvent('pointerup', { bubbles: true })); return new Promise((r) => setTimeout(() => r({ t, cleared: !sk.dataset.dragging, barCleared: !bar.dataset.dragging }), 20)); });
    check(drag.t === 'none' && drag.cleared && drag.barCleared, `${at}: the seek fill doesn't glide under a drag (${drag.t}) and a drag let go without a change is over, in Now playing (${drag.cleared}) and the bar (${drag.barCleared})`);
    // A real drag on the seek bar still sends the seek (with a mouse: a phone's touch emulation gives a
    // range input no change from a synthetic mouse drag)
    if (w >= 600) {
    const sb = await page.$eval('#npseek', (el) => { const r = el.getBoundingClientRect(); return { x: r.left, y: r.top + r.height / 2, w: r.width }; });
    const h0 = state.hits.length;
    await page.mouse.move(sb.x + sb.w * 0.3, sb.y); await page.mouse.down(); await page.mouse.move(sb.x + sb.w * 0.6, sb.y, { steps: 5 }); await page.mouse.up(); await sleep(400);
    const dragged = state.hits.slice(h0).find((x) => /transport|seek/.test(x.v) && x.body && /seek/.test(x.body));
    const after2 = await page.$eval('#npseek', (el) => !el.dataset.dragging);
    check(!!dragged && after2, `${at}: a real drag on the seek bar sends the seek (${dragged ? dragged.v : 'none'}) and lets go (${after2})`);
    }
    // A track change: the last cover fades out over the new one, the title slides in
    const next = { ...zone, transport: 'playing', queue_index: 1, now_playing: { ...np, item: { ...np.item, id: 2, uri: 'track:2', title: 'Flowing River' }, position_ms: 1000 } };
    state.send(next); await sleep(150);
    const mini = await page.evaluate(() => { const el = document.querySelector('#np'); return { swap: el.classList.contains('swap'), anim: getComputedStyle(el).animationName }; });
    check(mini.swap && mini.anim === 'fadein', `${at}: the mini player cross-fades on the track change (${mini.anim})`);
    const swap = await page.evaluate(() => { const ov = document.querySelector('#nowplaying'); const prev = ov.querySelector('.np-big .np-prev'); return { cls: ov.classList.contains('np-swap'), prev: !!prev, fading: !!prev && getComputedStyle(prev).animationName === 'fadeout', title: ov.querySelector('.np-titles h1').textContent.trim(), anim: getComputedStyle(ov.querySelector('.np-titles')).animationName }; });
    await sleep(900);
    const after = await page.evaluate(() => ({ prev: !!document.querySelector('#nowplaying .np-big .np-prev'), scale: new DOMMatrix(getComputedStyle(document.querySelector('#nowplaying .np-big')).transform).a }));
    check(swap.cls && swap.title === 'Flowing River' && swap.anim === 'npslide' && swap.fading && !after.prev && after.scale > 0.99, `${at}: a track change slides the title in (${swap.anim}), fades the last cover out over the new (${swap.fading}) and the cover is back at rest (×${after.scale.toFixed(2)}; old cover gone ${!after.prev})`);
    check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
    await context.close();
  }
  // A fresh page rises in, its first tiles one after another; Play to rises in
  for (const [w, h] of [[390, 844], [1440, 900]]) {
    const { page, context } = await fx.create(browser, w, { mobile: w < 600, fixtures: { ...fixtures, '/library/albums': { body: Array.from({ length: 16 }, (_, i) => ({ id: 70 + i, title: `Album ${i}`, album_artist: 'Sefa', year: 2017, tracks: 10 })) } } });
    await page.setViewportSize({ width: w, height: h }); await page.emulateMedia({ reducedMotion: 'no-preference' });
    await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn'); });
    await fx.go(page, '/'); await page.waitForSelector('#main'); await sleep(400);
    // To Albums the way the app's own Back and links go (a fresh page, not a reload)
    await page.evaluate(() => { history.pushState({}, '', '/albums'); dispatchEvent(new PopStateEvent('popstate', { state: null })); });
    await page.waitForSelector('#main .grid .tile', { timeout: 8000 }); await sleep(80);
    const r = await page.evaluate(() => { const m = document.querySelector('#main'); const tiles = [...document.querySelectorAll('#main .grid > .tile')]; const d = (i) => tiles[i] ? getComputedStyle(tiles[i]).animationDelay : null; const a = (i) => tiles[i] ? getComputedStyle(tiles[i]).animationName : null; return { view: m.classList.contains('view-in'), anim: getComputedStyle(m).animationName, n: tiles.length, d1: d(0), d3: d(2), a3: a(2), a13: a(12) }; });
    check(r.view && r.anim === 'viewin', `${w}: a fresh page rises in (${r.anim})`);
    check(r.n >= 13 && r.a3 === 'fadein' && parseFloat(r.d3) > parseFloat(r.d1) && r.a13 === 'none', `${w}: the first tiles follow one after another (3rd ${r.d3} after the 1st ${r.d1}); the 13th doesn't move (${r.a13})`);
    await sleep(1000);
    const later = await page.evaluate(() => document.querySelector('#main').classList.contains('view-in'));
    check(!later, `${w}: the page's entrance is over after a second (so what arrives later doesn't move)`);
    // (a phone's bar has no zone button: Now playing's zone label presses it)
    if (w < 600) await page.$eval('#zonebtn', (b) => b.click()); else await page.click('#zonebtn'); await sleep(50);
    const pop = await page.evaluate(() => getComputedStyle(document.querySelector('#zonepop')).animationName);
    check(pop === (w < 600 ? 'sheetin' : 'popin-rise'), `${w}: Play to rises in (${pop})`);
    await context.close();
  }
  // Reduced motion: the scale is 0, and no element has a running transition or animation
  {
    const { page, context } = await openNp(browser, 390, 844, { reduced: true });
    await sleep(300);
    const r = await page.evaluate(() => {
      const cs = getComputedStyle(document.documentElement);
      const durs = ['--dur-fast', '--dur-base', '--dur-slow'].map((k) => cs.getPropertyValue(k).trim());
      let moving = 0; for (const el of document.querySelectorAll('body *')) { const s = getComputedStyle(el); if ((s.transitionDuration.split(',').some((d) => parseFloat(d) > 0) && s.transitionProperty !== 'none') || (s.animationName !== 'none' && parseFloat(s.animationDuration) > 0)) moving++; }
      return { durs, moving };
    });
    check(r.durs.every((d) => d === '0ms' || d === '0s') && r.moving === 0, `reduced motion: the scale is 0 (${r.durs.join(' ')}) and nothing transitions or animates (${r.moving} elements)`);
    await context.close();
  }
  // The scale itself: design-lint's rule 4 (every duration from the scale)
  {
    const { execFileSync } = require('node:child_process');
    let out = ''; let ok = true;
    try { out = execFileSync(process.execPath, [path.join(__dirname, 'design-lint.js'), path.join(path.resolve(process.argv[2] || path.join(__dirname, '..')), 'crates/pm-api/static')], { encoding: 'utf8' }); } catch (e) { ok = false; out = String(e.stdout || e.message); }
    check(ok && !/raw duration/.test(out), `design-lint: every transition and animation takes its duration from the motion scale${ok ? '' : ': ' + out.split('\n').filter((l) => /raw duration/.test(l)).slice(0, 3).join(' | ')}`);
  }
  // No horizontal overflow on the main pages
  for (const [w, h] of [[360, 800], [390, 844], [834, 1112], [1440, 900]]) {
    const { page, context } = await fx.create(browser, w, { mobile: w < 600, fixtures });
    await page.setViewportSize({ width: w, height: h });
    const bad = [];
    for (const p of ['/', '/search', '/library', '/albums', '/zones', '/settings']) {
      await fx.go(page, p); await page.waitForSelector('#main'); await sleep(350);
      const o = await page.evaluate(() => ({ doc: document.documentElement.scrollWidth - document.documentElement.clientWidth, main: document.querySelector('#main').scrollWidth - document.querySelector('#main').clientWidth }));
      if (o.doc > 0 || o.main > 1) bad.push(`${p} (${o.doc}/${o.main})`);
    }
    check(!bad.length, `${w}: no horizontal overflow on the main pages${bad.length ? ': ' + bad.join(', ') : ''}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
