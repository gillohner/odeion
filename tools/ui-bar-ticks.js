// While a song plays, the player bar is told the position four times a second. A tick writes the
// time and the seek bar and nothing else: the mini player's cover, title and artists stay the same
// elements between two songs. Real candidate assets on the fixture origin (tools/ui-fixture.js), a
// node zone playing, its events sent by the test:
//   * 20 ticks of one song: nothing in the mini player is added or removed, its cover and the Play
//     button's icon are the elements they were, a keyboard's place on the title stays, and the
//     page itself (600 songs listed) sees no change;
//   * the ticks still do their job: the time, the seek bar and the phone's progress line move;
//   * another zone's ticks change nothing in the bar;
//   * a pause, a new song, an error and "Nothing playing" each show at once, and the playing song's
//     mark moves with the song;
//   * rows drawn later (a page opened while the song plays or is paused) get the mark without a tick;
//   * a volume change still reaches the sliders, shuffle and repeat changed elsewhere the bar's buttons.
// It prints what 40 ticks cost (layouts, style passes, script time at a quarter of the speed).
//   node tools/ui-bar-ticks.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'bar-ticks-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const tracks = Array.from({ length: 600 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song number ${i + 1}`, track_no: (i % 12) + 1 }));
const item = (n) => ({ id: n, uri: `track:${n}`, title: `Song number ${n}`, artist: 'Sefa, Dr. Peacock', album: 'Journey', duration_ms: 240000 });
const playing = (n, pos) => ({ item: item(n), index: n - 1, position_ms: pos, duration_ms: 240000, source_format: fmt, output_format: fmt, signal_path: { stages: [] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } });
const zone = (id, name, transport, np, extra = {}) => ({ id, name, output: `upnp:${id}`, transport, queue: [item(1), item(2)], queue_index: np ? np.index : null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: np, ...extra });
const outputs = ['kitchen', 'study'].map((id) => ({ id: `upnp:${id}`, name: id, kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }));

const open = async (browser, width, start) => {
  const zones = [zone('kitchen', 'Kitchen', start.transport, start.np), zone('study', 'Study', 'playing', playing(2, 5000))];
  const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, zone: 'kitchen', tracks, fixtures: { '/zones': { body: zones }, '/outputs': { body: outputs }, '/zones/kitchen/queue': { body: { items: zones[0].queue, index: 0, version: 1 } } } });
  const sock = { ws: null, seq: 0 };
  await context.routeWebSocket(/\/v1\/events/, (ws) => { sock.ws = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
  const send = async (z) => { sock.ws.send(JSON.stringify({ type: 'zone', seq: ++sock.seq, state: z })); await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))); };
  return { context, page, state, send };
};
// What changes under an element from now on (child lists only: elements and text nodes added or removed)
const watch = (page, sel) => page.evaluate((s) => { const el = document.querySelector(s); window.__w = window.__w || {}; const w = window.__w[s] = { n: 0 }; w.mo = new MutationObserver((recs) => { w.n += recs.length; }); w.mo.observe(el, { childList: true, subtree: true }); }, sel);
const seen = (page, sel) => page.evaluate((s) => { const w = window.__w[s]; w.n += w.mo.takeRecords().length; return w.n; }, sel);

(async () => {
  const browser = await chromium.launch();
  for (const width of [1280, 390]) {
    const at = `${width}px`;
    const { context, page, state, send } = await open(browser, width, { transport: 'playing', np: playing(1, 61000) });
    await fx.go(page, '/tracks');
    await page.waitForSelector('#main .track');
    await page.waitForFunction(() => /Song number 1/.test(document.querySelector('#np').innerText), null, { timeout: 5000 }).catch(() => {});
    await sleep(400);
    check(await page.evaluate(() => /Song number 1$/.test(document.querySelector('#np .np-title').innerText.trim())), `${at}: the bar names the song`);
    check(await page.evaluate(() => document.querySelector('#main .track[data-tid="1"]').classList.contains('cur') && document.querySelectorAll('#main .track.cur').length === 1), `${at}: the playing song is marked in the list`);

    // 20 ticks of the same song
    const link = width >= 600 ? await page.evaluate(() => { const a = document.querySelector('#np .np-title [data-np-song], #np .np-title a, #np .np-title button'); if (!a) return false; a.focus(); return document.activeElement === a; }) : null;
    await page.evaluate(() => { window.__cover = document.querySelector('#np .np-art'); window.__icon = document.querySelector('#toggle .ic'); window.__title = document.querySelector('#np .np-title'); });
    await watch(page, '#np'); await watch(page, '#toggle'); await watch(page, '#main'); await watch(page, '#quality');
    const before = await page.evaluate(() => ({ pos: document.querySelector('#pos').textContent, seek: document.querySelector('#seek').value, mini: getComputedStyle(document.querySelector('#miniprog')).getPropertyValue('--p') }));
    for (let i = 1; i <= 20; i++) await send(zone('kitchen', 'Kitchen', 'playing', playing(1, 61000 + i * 250)));
    check(await seen(page, '#np') === 0, `${at}: 20 ticks add or remove nothing in the mini player (${await seen(page, '#np')} changes)`);
    check(await page.evaluate(() => window.__cover === document.querySelector('#np .np-art') && window.__cover.isConnected && window.__title.isConnected), `${at}: its cover and title are the elements they were`);
    check(await seen(page, '#toggle') === 0 && await page.evaluate(() => window.__icon === document.querySelector('#toggle .ic')), `${at}: the Play button's icon is the element it was`);
    check(await seen(page, '#quality') === 0, `${at}: the sound pill is not written again (${await seen(page, '#quality')} changes)`);
    check(await seen(page, '#main') === 0, `${at}: the page sees no change (${await seen(page, '#main')})`);
    if (link !== null) check(link && await page.evaluate(() => !!document.activeElement && !!document.activeElement.closest('#np')), `${at}: a keyboard's place on the bar's title stays through the ticks`);
    const after = await page.evaluate(() => ({ pos: document.querySelector('#pos').textContent, seek: document.querySelector('#seek').value, mini: getComputedStyle(document.querySelector('#miniprog')).getPropertyValue('--p') }));
    check(before.pos === '1:01' && after.pos === '1:06' && Number(after.seek) > Number(before.seek), `${at}: the time and the seek bar move (${before.pos} → ${after.pos}, ${before.seek} → ${after.seek})`);
    check(parseFloat(after.mini) > parseFloat(before.mini), `${at}: the progress line moves (${before.mini} → ${after.mini})`);

    // another zone's ticks
    const np0 = await seen(page, '#np');
    for (let i = 1; i <= 8; i++) await send(zone('study', 'Study', 'playing', playing(2, 5000 + i * 250)));
    check(await seen(page, '#np') === np0 && await page.evaluate(() => document.querySelector('#pos').textContent) === after.pos, `${at}: another zone's ticks change nothing in the bar`);

    // what the ticks cost (printed, for the record)
    {
      const cdp = await context.newCDPSession(page); await cdp.send('Performance.enable'); await cdp.send('Emulation.setCPUThrottlingRate', { rate: 4 });
      const read = async () => Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map((m) => [m.name, m.value]));
      const a = await read();
      for (let i = 21; i <= 60; i++) await send(zone('kitchen', 'Kitchen', 'playing', playing(1, 61000 + i * 250)));
      const b = await read(); await cdp.send('Emulation.setCPUThrottlingRate', { rate: 1 });
      console.log(`     ${at}: 40 ticks, 600 songs listed: ${b.LayoutCount - a.LayoutCount} layouts, ${b.RecalcStyleCount - a.RecalcStyleCount} style passes, ${Math.round((b.ScriptDuration - a.ScriptDuration) * 1000)} ms script, ${Math.round((b.LayoutDuration - a.LayoutDuration + b.RecalcStyleDuration - a.RecalcStyleDuration) * 1000)} ms style and layout (4× slower CPU)`);
    }

    // pause
    await send(zone('kitchen', 'Kitchen', 'paused', playing(1, 80000)));
    check(await page.evaluate(() => document.querySelector('#toggle').getAttribute('aria-label') === 'Play' && /#i-play$/.test(document.querySelector('#toggle use').getAttribute('href'))), `${at}: a pause shows Play at once`);
    await send(zone('kitchen', 'Kitchen', 'playing', playing(1, 80250)));
    check(await page.evaluate(() => document.querySelector('#toggle').getAttribute('aria-label') === 'Pause' && /#i-pause$/.test(document.querySelector('#toggle use').getAttribute('href'))), `${at}: playing again shows Pause`);

    // the next song
    await send(zone('kitchen', 'Kitchen', 'playing', playing(2, 0)));
    check(await page.evaluate(() => /Song number 2$/.test(document.querySelector('#np .np-title').innerText.trim()) && document.querySelector('#pos').textContent === '0:00'), `${at}: the next song is named at once, from 0:00`);
    check(await page.evaluate(() => document.querySelector('#main .track[data-tid="2"]').classList.contains('cur') && document.querySelector('#main .track[data-tid="2"]').getAttribute('aria-current') === 'true' && !document.querySelector('#main .track[data-tid="1"]').classList.contains('cur') && document.querySelectorAll('#main .track.cur').length === 1), `${at}: the mark moves to the next song`);

    // rows drawn later: another page and back, while it plays
    await page.evaluate(() => history.pushState({}, '', '/albums')); await fx.go(page, '/albums'); await page.waitForSelector('#main .page-head');
    await fx.go(page, '/tracks'); await page.waitForSelector('#main .track');
    await sleep(150);
    check(await page.evaluate(() => { const r = document.querySelector('#main .track[data-tid="1"]'); return !!r && r.classList.contains('cur'); }), `${at}: a list opened while the song plays has it marked`);

    // paused by a lost device: said where the artist is; then an error stop; then nothing
    await send(zone('kitchen', 'Kitchen', 'paused', playing(1, 90000), { error: 'The USB DAC went away' }));
    check(await page.evaluate(() => /The USB DAC went away/.test(document.querySelector('#np .np-sub').innerText)), `${at}: a pause by a lost device says why`);
    await send(zone('kitchen', 'Kitchen', 'paused', playing(1, 90000)));
    check(await page.evaluate(() => !/USB DAC/.test(document.querySelector('#np .np-sub').innerText) && /Sefa/.test(document.querySelector('#np .np-sub').innerText)), `${at}: the artist is back once the reason is gone`);
    await send(zone('kitchen', 'Kitchen', 'stopped', null, { error: 'The device refused' }));
    check(await page.evaluate(() => /Stopped/.test(document.querySelector('#np .np-title').innerText) && /The device refused/.test(document.querySelector('#np .np-sub').innerText) && document.querySelectorAll('#main .track.cur').length === 0), `${at}: a stop on an error says so, and no song is marked`);
    await send(zone('kitchen', 'Kitchen', 'stopped', null));
    check(await page.evaluate(() => /Nothing playing/.test(document.querySelector('#np .np-title').innerText) && document.querySelector('#pos').textContent === '0:00'), `${at}: "Nothing playing" after it`);

    // a volume change still reaches the bar's slider
    if (width >= 600) {
      await send(zone('kitchen', 'Kitchen', 'stopped', null, { volume: { percent: 55, muted: false } }));
      check(await page.evaluate(() => Number(document.querySelector('#vol').value)) === 55, `${at}: a volume change reaches the bar's slider`);
      // shuffle and repeat changed elsewhere (another client) still reach the bar's buttons, and go off again
      const modes = () => page.evaluate(() => ['#shufflebtn', '#repeatbtn'].map((q) => document.querySelector(q).getAttribute('aria-pressed')).join(' '));
      await send(zone('kitchen', 'Kitchen', 'stopped', null, { volume: { percent: 55, muted: false }, repeat: 'all', shuffle: true }));
      check(await modes() === 'true true', `${at}: shuffle and repeat switched on elsewhere light the bar's buttons (${await modes()})`);
      await send(zone('kitchen', 'Kitchen', 'stopped', null, { volume: { percent: 55, muted: false } }));
      check(await modes() === 'false false', `${at}: and go out again (${await modes()})`);
    }
    check(!state.errors.length, `${at}: no page error (${state.errors.join(' | ') || 'none'})`);
    await page.screenshot({ path: path.join(SHOTS, `bar-${width}.png`) });
    await context.close();
  }
  {
    // paused before the page opened: no tick ever comes, the list still has its mark
    const { context, page, state } = await open(browser, 1280, { transport: 'paused', np: playing(3, 30000) });
    await fx.go(page, '/tracks'); await page.waitForSelector('#main .track'); await sleep(300);
    check(await page.evaluate(() => { const r = document.querySelector('#main .track[data-tid="3"]'); return !!r && r.classList.contains('cur') && r.getAttribute('aria-current') === 'true'; }), 'paused, no tick: the list has the song marked');
    check(!state.errors.length, `paused: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
