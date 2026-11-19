// Every point of the phone mini player is a tap on what's nearest. Real candidate assets on the
// fixture origin (tools/ui-fixture.js): no node, no network. At 360 and 412 px, in a browser and in
// the Android app, a song playing, without and with a files job's chip beside Play, listening along
// with someone while a job runs (the job's chip, then the along chip with Leave), and on a
// listener's account listening along (no job chip there), on a touch screen:
//   * every point of the bar below its 1 px top line hits the song (opens Now playing), Play, the
//     job's chip or Leave; none is dead (only the along chip's words, which do nothing, may take
//     their own column) (the bar's 12 px left edge, the 6 px above and below the song and the gap
//     before Play did nothing);
//   * which one is the nearer in its column: the song up to half the gap after its text, Play from
//     there on (it reached 6 px into an 8 px gap, taking taps closer to the song), the chip sharing
//     each 4 px gap half and half; the gaps beside the along chip go to the control beside them;
//   * Play takes 44 px or more each way, the job's chip and Leave the whole height of the bar;
//   * listening along, real taps: just past the song (on a listener's account) opens Now playing,
//     just after the along chip plays or pauses, at the bar's top edge above Leave leaves;
//   * nothing drawn moves: the cover starts 12 px in, Play ends 4 px before the screen's edge, both
//     centred in the bar;
//   * real taps: one in the bar's left edge and one just past the song's text open Now playing and
//     don't play or pause; one just before Play plays or pauses;
//   * the song's focus ring is drawn inside the bar (its box now reaches the bar's edges, which
//     clip a ring drawn outside it);
//   * with larger text the bar grows and still has no dead point; gone offline with the chip still
//     drawn for a job, the chip is left out and the gap it leaves is shared as without one.
// With a mouse the song still takes the edges and its half of the gap; Play has no reach.
//   node tools/ui-phone-mini-reach.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-mini-reach-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(50); } };
// (Play on a zone that plays sends pause)
const playPause = (state) => state.hits.filter((h) => h.m === 'POST' && /"action":"(toggle|play|pause)"/.test(h.body || '')).length;

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const queue = [{ id: 1, uri: 'track:1', title: 'Wonderful Tonight', artist: 'Eric Clapton', duration_ms: 219000 }, { id: 2, uri: 'track:2', title: 'Crossroads', artist: 'Eric Clapton', duration_ms: 250000 }];
const np = { item: { ...queue[0], album: 'Eric Clapton Story' }, index: 0, position_ms: 61000, duration_ms: 219000, source_format: fmt, output_format: fmt, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = { id: 'linn', name: 'Linn Klimax DS3', output: 'upnp:linn', transport: 'playing', queue, queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: np };
const job = { id: 'j1', kind: 'copy', state: 'running', total: 25, done: 7, failed: 0, skipped: 0, bytes_total: 750e6, bytes_done: 210e6, errors: [], errors_count: 0, current_folder: '/music', speed_bps: 2.4e6, eta_s: 600 };
// Listening along with someone (presence.rs's GET /v1/presence): their along chip (on a phone the name) with Leave
const along = { key: 'k-anna', name: 'Anna', mine: true, you: false, hosting: false, zone: { id: 'linn', name: 'Linn Klimax DS3' }, your_zone: 'linn' };
const fixtures = (withJob, withAlong) => ({
  '/zones': { body: [zone] }, '/zones/linn/queue': { body: queue },
  '/outputs': { body: [{ id: 'upnp:linn', name: 'Linn', kind: 'upnp', caps: { hw_volume: 'percent' } }] },
  '/pubky/files/jobs': { body: withJob ? { current: job, queued: [], last: null } : { current: null, queued: [], last: null } },
  '/presence': { body: withAlong ? [along] : [] }, '/me/presence': { body: { visible: 'hidden' } }, '/presence/along': { body: zone },
});
// The runs: a song alone; a files job's chip beside Play; listening along with someone while a job
// runs (the job's chip, the along chip, then Play); and a listener's account (body.listener: the
// job's chip is never shown to one) listening along
const RUNS = [
  { job: false, along: false, name: 'no job' },
  { job: true, along: false, name: 'a files job running' },
  { job: true, along: true, name: 'a files job running, listening along', file: 'job-along' },
  { job: true, along: true, listener: true, name: 'a listener listening along', file: 'listener-along' },
];

// Who each point of the bar goes to, and the drawn boxes (the song's is its cover and text). The
// along chip's words do nothing: their column may stay with the chip, and they're no one's nearest.
const measure = (page) => page.evaluate(() => {
  const bar = document.querySelector('.player'); const b = bar.getBoundingClientRect();
  const np = document.querySelector('#np'); const tg = document.querySelector('#toggle'); const job = document.querySelector('#jobchip'); const al = document.querySelector('#along'); const lv = document.querySelector('#alongleave');
  const r = (e) => { const x = e.getBoundingClientRect(); return { left: x.left, right: x.right, top: x.top, bottom: x.bottom, w: x.width, h: x.height }; };
  const art = r(np.querySelector('.np-art')); const text = r(np.querySelector('.np-text'));
  const shown = (e) => !e.hidden && e.getBoundingClientRect().width > 0;
  const drawn = { np: { left: Math.min(art.left, text.left), right: Math.max(art.right, text.right) }, toggle: r(tg) };
  if (shown(job)) drawn.job = r(job);
  let words = null; if (shown(al)) { drawn.leave = r(lv); words = { left: al.getBoundingClientRect().left, right: lv.getBoundingClientRect().left }; }
  const who = (x, y) => { const h = document.elementFromPoint(x, y); return !h ? 'nothing' : tg.contains(h) ? 'toggle' : np.contains(h) ? 'np' : job.contains(h) ? 'job' : lv.contains(h) ? 'leave' : al.contains(h) ? 'along' : h === bar || bar.contains(h) ? 'bar' : h.tagName.toLowerCase(); };
  const near = (x) => { const d = Object.entries(drawn).map(([k, v]) => [k, x < v.left ? v.left - x : x > v.right ? x - v.right : 0]); const m = Math.min(...d.map((e) => e[1])); return { m, d: Object.fromEntries(d) }; };
  const dead = []; const far = []; const top = b.top + bar.clientTop;
  for (let y = Math.ceil(top); y < b.bottom; y += 1) {
    for (let x = 0; x < innerWidth; x += 1) {
      const h = who(x, y);
      if (!(h in drawn)) { if (!(words && x >= words.left && x < words.right && (h === 'along' || h === 'bar')) && dead.length < 6) dead.push(`${x},${y}:${h}`); continue; }
      const n = near(x); if (n.d[h] > n.m + 1 && far.length < 6) far.push(`${x},${y}:${h}(${n.d[h]} vs ${n.m})`);
    }
  }
  const run = (k, x0, y0, dx, dy) => { let n = 0; for (let x = x0, y = y0; x >= 0 && x < innerWidth && y >= top && y < b.bottom; x += dx, y += dy) if (who(x, y) === k) n++; return n; };
  const span = (k) => { const v = drawn[k]; const cx = (v.left + v.right) / 2; const cy = (v.top + v.bottom) / 2; let w = 0; let h = 0; for (let x = 0; x < innerWidth; x++) if (who(x, cy) === k) w++; for (let y = Math.ceil(top); y < b.bottom; y++) if (who(cx, y) === k) h++; return { w, h }; };
  return { bar: { top, bottom: b.bottom, h: b.bottom - top }, drawn, art, dead, far, tgSpan: span('toggle'), jobSpan: drawn.job ? span('job') : null, leaveSpan: drawn.leave ? span('leave') : null, listener: document.body.classList.contains('listener'), inner: innerWidth, coarse: matchMedia('(pointer: coarse)').matches, run: run('np', 0, (b.top + b.bottom) / 2, 1, 0) };
});

(async () => {
  const browser = await chromium.launch();
  for (const width of [360, 412]) {
    for (const android of [false, true]) {
      for (const R of RUNS) {
        const withJob = R.job && !R.listener; const withAlong = R.along;
        const where = `${width} px, ${android ? 'Android app' : 'browser'}, ${R.name}`;
        const { context, page, state } = await fx.create(browser, width, { mobile: true, fixtures: fixtures(R.job, R.along), ...(R.listener ? { role: 'listener' } : {}) });
        await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
        await context.addInitScript((a) => { localStorage.setItem('pm.zone', 'linn'); if (a) window.pmDesktop = { platform: 'android', version: '0.0.0' }; }, android);
        await fx.go(page, '/');
        await page.waitForFunction(() => /Wonderful Tonight/.test(document.querySelector('#np .np-title')?.textContent || ''));
        if (withJob) check(await until(() => page.isVisible('#jobchip')), `${where}: the files job's chip shows`);
        if (withAlong) check(await until(() => page.isVisible('#along')), `${where}: the along chip shows`);
        if (R.listener) check(await until(() => page.evaluate(() => document.body.classList.contains('listener'))) && !(await page.isVisible('#jobchip')), `${where}: a listener's account, no job chip`);
        await sleep(400);
        const m = await measure(page);
        check(m.coarse, `${where}: a coarse pointer`);
        check(m.dead.length === 0, `${where}: no point of the bar is dead (${m.dead.join(' ') || 'none'})`);
        check(m.far.length === 0, `${where}: each point goes to the nearer control in its column (${m.far.join(' ') || 'all'})`);
        check(m.tgSpan.w >= 44 && m.tgSpan.h >= 44 && m.tgSpan.h >= m.bar.h - 1, `${where}: Play takes ${m.tgSpan.w} x ${m.tgSpan.h} px (>= 44, the bar's height ${Math.round(m.bar.h)})`);
        if (withJob) check(m.jobSpan.h >= m.bar.h - 1 && m.jobSpan.w >= m.drawn.job.w + 3, `${where}: the chip (${Math.round(m.drawn.job.w)} x ${Math.round(m.drawn.job.h)} drawn) takes ${m.jobSpan.w} x ${m.jobSpan.h} px`);
        if (withAlong) check(m.leaveSpan.h >= m.bar.h - 1 && m.leaveSpan.w >= m.drawn.leave.w + 4, `${where}: Leave (${Math.round(m.drawn.leave.w)} x ${Math.round(m.drawn.leave.h)} drawn) takes ${m.leaveSpan.w} x ${m.leaveSpan.h} px`);
        const mid = (m.bar.top + m.bar.bottom) / 2;
        check(Math.abs(m.art.left - 12) < 0.5 && Math.abs(m.drawn.toggle.right - (m.inner - 4)) < 0.5, `${where}: nothing drawn moves (cover at ${m.art.left}, Play ends at ${m.drawn.toggle.right} of ${m.inner})`);
        check(Math.abs((m.art.top + m.art.bottom) / 2 - mid) <= 1 && Math.abs((m.drawn.toggle.top + m.drawn.toggle.bottom) / 2 - mid) <= 1, `${where}: the cover and Play are centred in the bar`);
        await page.screenshot({ path: path.join(SHOTS, `mini-${width}-${android ? 'app' : 'web'}${R.file ? '-' + R.file : withJob ? '-job' : ''}.png`) });
        // Larger text (as Android's font size makes it): the bar grows, Play and the chip still take its whole height
        await page.addStyleTag({ content: ':root { --fs-md: 22px; --fs-xs: 18px; --fs-sm: 19px; }' }); await sleep(100);
        const big = await measure(page);
        check(big.bar.h > m.bar.h + 4 && big.dead.length === 0 && big.tgSpan.h >= big.bar.h - 1 && (!withJob || big.jobSpan.h >= big.bar.h - 1) && (!withAlong || big.leaveSpan.h >= big.bar.h - 1), `${where}, larger text: the bar is ${Math.round(big.bar.h)} px, no point dead (${big.dead.join(' ') || 'none'}), Play takes ${big.tgSpan.h} px${withJob ? `, the chip ${big.jobSpan.h} px` : ''}${withAlong ? `, Leave ${big.leaveSpan.h} px` : ''} of its height`);
        await page.evaluate(() => document.querySelectorAll('style').forEach((e) => { if (/--fs-md: 22px/.test(e.textContent)) e.remove(); }));
        if (withAlong) {
          // Real taps: just past the song (no job's chip there) opens Now playing; just after the
          // along chip plays or pauses; at the bar's top edge above Leave asks the node to stop listening along
          const t0 = playPause(state);
          const npHidden = () => page.evaluate(() => document.querySelector('#nowplaying').hidden);
          if (R.listener) {
            await page.touchscreen.tap(m.drawn.np.right + 2, mid);
            check(await until(async () => !(await npHidden()), 2000) && playPause(state) === t0, `${where}: a tap just past the song (x ${Math.round(m.drawn.np.right + 2)}) opens Now playing, no play/pause`);
            await page.keyboard.press('Escape'); check(await until(npHidden, 2000), `${where}: Escape closes Now playing again`);
          }
          const after = (await page.evaluate(() => document.querySelector('#along').getBoundingClientRect().right)) + 1;
          await page.touchscreen.tap(after, mid);
          check(await until(() => playPause(state) === t0 + 1, 2000), `${where}: a tap just after the along chip (x ${Math.round(after)}) plays or pauses`);
          const left = () => state.hits.some((h) => h.m === 'DELETE' && h.v === '/presence/along');
          check(!left(), `${where}: nothing so far left listening along`);
          await page.touchscreen.tap((m.drawn.leave.left + m.drawn.leave.right) / 2, m.bar.top + 2);
          check(await until(left), `${where}: a tap above Leave at the bar's top edge asks to stop listening along (${state.hits.filter((h) => h.m !== 'GET').map((h) => h.m + ' ' + h.v).join(', ') || 'nothing asked'})`);
          check(state.errors.length === 0, `${where}: no page errors (${state.errors.join('; ')})`);
          await context.close(); continue;
        }
        if (withJob) {
          // Gone offline (the node out of reach, the app's own server) with the chip still drawn for
          // the job: offline leaves it out, and the gap it left is shared as without one
          await page.evaluate(() => document.body.classList.add('offline')); await sleep(100);
          const off = await measure(page);
          check(!off.drawn.job && off.dead.length === 0 && off.far.length === 0, `${where}, then offline: the chip is left out, no point dead (${off.dead.join(' ') || 'none'}), each to the nearer (${off.far.join(' ') || 'all'})`);
          await context.close(); continue;
        }
        // Real taps: the left edge and just past the song open Now playing; just before Play plays or pauses
        const t0 = playPause(state);
        const npHidden = () => page.evaluate(() => document.querySelector('#nowplaying').hidden);
        const close = async () => { await page.keyboard.press('Escape'); return until(npHidden, 2000); };
        for (const [x, what] of [[3, 'the bar\'s left edge'], [m.drawn.np.right + 2, 'just past the song']]) {
          await page.touchscreen.tap(x, mid);
          check(await until(async () => !(await npHidden()), 2000) && playPause(state) === t0, `${where}: a tap on ${what} (x ${Math.round(x)}) opens Now playing, no play/pause`);
          check(await close(), `${where}: Escape closes Now playing again`);
        }
        await page.touchscreen.tap(m.drawn.toggle.left - 2, mid);
        check(await until(() => playPause(state) === t0 + 1, 2000) && await page.evaluate(() => document.querySelector('#nowplaying').hidden), `${where}: a tap just before Play (x ${Math.round(m.drawn.toggle.left - 2)}) plays or pauses`);
        // The focus ring is drawn inside the bar
        // (a key pressed last: focus by script is drawn as by the keyboard)
        await page.keyboard.press('Shift');
        const ring = await page.evaluate(() => { const np = document.querySelector('#np'); np.focus(); const cs = getComputedStyle(np); const r = np.getBoundingClientRect(); const b = document.querySelector('.player').getBoundingClientRect(); const o = parseFloat(cs.outlineOffset) + parseFloat(cs.outlineWidth); return { vis: np.matches(':focus-visible'), inside: r.left - o >= b.left && r.right + o <= b.right && r.top - o >= b.top && r.bottom + o <= b.bottom, o }; });
        check(ring.vis && ring.inside, `${where}: the song's focus ring is inside the bar (reaches ${ring.o} px out, focus-visible ${ring.vis})`);
        check(state.errors.length === 0, `${where}: no page errors (${state.errors.join('; ')})`);
        await context.close();
      }
    }
  }
  // With a mouse: the song still takes the bar's edges and its half of the gap; Play has no reach
  {
    const { context, page } = await fx.create(browser, 360, { fixtures: fixtures(false) });
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage(() => {}); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
    await context.addInitScript(() => localStorage.setItem('pm.zone', 'linn'));
    await fx.go(page, '/');
    await page.waitForFunction(() => /Wonderful Tonight/.test(document.querySelector('#np .np-title')?.textContent || ''));
    const m = await measure(page);
    const tgReach = await page.evaluate(() => getComputedStyle(document.querySelector('#toggle'), '::before').content);
    check(!m.coarse && m.run >= Math.floor(m.drawn.np.right + 4) && tgReach === 'none', `mouse 360 px: the song takes the bar from its left edge to half the gap (${m.run} px), Play has no reach (::before ${tgReach})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
