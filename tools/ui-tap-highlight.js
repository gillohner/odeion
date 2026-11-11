// Round buttons stay round when pressed and when focused (TASK-round-button-tap-highlight-1105;
// owner, Android: "a square overlay during the click/hover/touch"). Real candidate assets on the
// fixture origin (tools/ui-fixture.js), the album page with a song playing, at 390 px with touch and at
// 1280 px with a mouse:
//   * the page paints no tap highlight (-webkit-tap-highlight-color transparent);
//   * the round controls (play, previous, next, a song row's button, the account button) keep their
//     50 %/999 px radius while focused from the keyboard (a guard: the global :focus-visible rule set a
//     6 px radius, which these buttons' own rule outranked by order but others may not);
//   * on a touch screen a tap leaves no hover background behind on an icon button.
//   node tools/ui-tap-highlight.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'tap-highlight-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// Round things a thumb lands on: on a phone the mini player's play/pause and Now playing's
// transport and heart (the bar has no previous/next there), on a desktop the bar's
const ROUND = (phone) => phone
  ? [['#nowplaying [data-npt="prev"]', 'Now playing previous'], ['#nowplaying [data-npt="next"]', 'Now playing next'], ['#nowplaying [data-npt="play"], #nowplaying .icon-btn.play', 'Now playing play/pause'], ['#nowplaying .icon-btn', 'a Now playing icon button'], ['.me-btn, #mebtn, .avatar', 'the account button']]
  : [['#toggle', 'play/pause'], ['#prev', 'previous'], ['#next', 'next'], ['#isongs .srow .icon-btn', "a song row's button"], ['.me-btn, #mebtn, .avatar', 'the account button']];
const TAB_FROM = (phone) => (phone ? ['#nowplaying [data-npt="prev"]', '#nowplaying [data-npt="play"], #nowplaying .icon-btn.play'] : ['#prev', '#toggle']);
const TAP = (phone) => (phone ? '#nowplaying [data-npt="next"]' : '#next');
const isRound = (r) => /^(50%|999px|9999px)$/.test(r) || /^50% 50%/.test(r);
// An album page with songs (the fixture origin's bare item has none)
const song = (id, no) => ({ no, title: `Song ${no}`, artist: 'Sefa', artist_id: 'lib-11', duration_ms: 200000, uri: null, track_id: id, id: `lib-${id}`, page: `lib-${id}` });
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [song(101, 1), song(102, 2), song(103, 3)], total_ms: 600000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
// A node zone playing (the bar and Now playing have their controls then), as ui-tablet sets it up
const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const outputs = [{ id: 'upnp:fixture', name: 'Fixture renderer', kind: 'upnp', caps: { hw_volume: { kind: 'db', min: -80, max: 0, step: 1 } } }];
const zone = { id: 'fixture-upnp', name: 'Fixture renderer', output: 'upnp:fixture', volume_db: -30, transport: 'playing', queue: [{ id: 1, uri: 'track:101', title: 'Song 1' }], queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 22, muted: false },
  now_playing: { item: { id: 1, uri: 'track:101', title: 'Song 1', artist: 'Sefa', album: 'Journey', duration_ms: 200000 }, index: 0, position_ms: 61000, duration_ms: 200000, source_format: fmt, output_format: fmt,
    signal_path: { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }, { name: 'output', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } } };
const fixtures = { '/zones': { body: [zone] }, '/outputs': { body: outputs }, '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } };
for (const id of [101, 102, 103]) fixtures[`/library/tracks/${id}`] = { body: { ...fx.track, id, title: `Song ${id - 100}` } };
const open = async (browser, w, touch) => {
  const run = await fx.create(browser, w, { touch, fixtures });
  await run.page.setViewportSize({ width: w, height: w < 600 ? 844 : 900 });
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  await run.context.addInitScript((z) => { localStorage.setItem('pm.zone', z); }, zone.id);
  return run;
};

(async () => {
  const browser = await chromium.launch();
  for (const [w, touch] of [[390, true], [1280, false]]) {
    const { page, context, state } = await open(browser, w, touch);
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs .srow'); await sleep(300);
    const at = `${w}${touch ? ' touch' : ''}`;
    const phone = w < 600;
    const tap = await page.evaluate(() => [document.documentElement, document.querySelector('#toggle')].map((el) => getComputedStyle(el).webkitTapHighlightColor));
    check(tap.every((t) => /rgba\(0, 0, 0, 0\)|transparent/.test(t)), `${at}: no painted tap highlight on the page or a button (${tap.join(', ')})`);
    if (phone) {
      // The mini player's play/pause first (it hides once Now playing opens), then Now playing
      const mini = await page.$eval('#toggle', (b) => ({ shown: b.getClientRects().length > 0, radius: getComputedStyle(b).borderRadius }));
      check(mini.shown && isRound(mini.radius), `${at}: the mini player's play/pause is round (${mini.radius})`);
      await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden])'); await sleep(300);
    }
    // Focus from the keyboard: the control's own radius stays, and the ring's radius is not a square's
    for (const [sel, name] of ROUND(phone)) {
      // (a phone's Now playing covers the page, which is out of reach meanwhile: the account button is
      // probed with Now playing closed, then it opens again for the rest)
      const behind = phone && sel.startsWith('.me-btn');
      if (behind) { await page.keyboard.press('Escape'); await page.waitForSelector('#nowplaying', { state: 'hidden' }); await sleep(200); }
      // Focus from the keyboard for real: land on the control, step back and forward with Tab
      const found = await page.evaluate((sel) => { const el = [...document.querySelectorAll(sel)].find((e) => e.getClientRects().length); if (!el) return null; el.dataset.tapProbe = '1'; return getComputedStyle(el).borderRadius; }, sel);
      if (found == null) { check(false, `${at}: ${name} is on the page (${sel})`); continue; }
      if (!/^(50%|999px|9999px)/.test(found)) { console.log(`skip ${at}: ${name} isn't round here (${found})`); await page.evaluate(() => { const el = document.querySelector('[data-tap-probe]'); if (el) delete el.dataset.tapProbe; }); continue; }
      // (Now playing redraws on its own clock: a probe whose control was replaced meanwhile is tried again)
      let r = null;
      for (let i = 0; i < 3 && !(r && r.focused); i++) {
        if (i) await page.evaluate((sel) => { const el = [...document.querySelectorAll(sel)].find((e) => e.getClientRects().length); if (el) el.dataset.tapProbe = '1'; }, sel);
        if (!(await page.$('[data-tap-probe]'))) continue;
        await page.focus('[data-tap-probe]').catch(() => {}); await page.keyboard.press('Shift+Tab'); await page.keyboard.press('Tab'); await sleep(30);
        r = await page.evaluate((before) => { const el = document.querySelector('[data-tap-probe]'); if (!el) return null; const cs = getComputedStyle(el); const r = { before, after: cs.borderRadius, outline: cs.outlineStyle, focused: document.activeElement === el, fv: el.matches(':focus-visible') }; delete el.dataset.tapProbe; return r; }, found);
      }
      r = r || { before: found, after: '?', outline: 'none', focused: false, fv: false };
      check(r.focused && r.fv && r.outline !== 'none' && r.after === r.before, `${at}: ${name} keeps its radius with a keyboard focus ring (${r.before} → ${r.after}${r.focused ? '' : ', focus went elsewhere'}${r.fv ? '' : ', not focus-visible'})`);
      if (behind) { await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden])'); await sleep(300); }
    }
    // Keyboard focus on the play button really draws the ring (Tab from the previous button)
    const [from, to] = TAB_FROM(phone);
    let ring = null;
    for (let i = 0; i < 3 && !(ring && ring.isPlay); i++) {
      await page.focus(from).catch(() => {}); await page.keyboard.press('Tab'); await sleep(50);
      ring = await page.evaluate((to) => { const el = document.activeElement; if (!el || el === document.body) return null; const cs = getComputedStyle(el); return { isPlay: el.matches(to), fv: el.matches(':focus-visible'), outline: cs.outlineStyle, radius: cs.borderRadius }; }, to);
    }
    ring = ring || { isPlay: false, fv: false, outline: 'none', radius: '?' };
    check(ring.isPlay && ring.fv && ring.outline !== 'none' && isRound(ring.radius), `${at}: Tab to play/pause draws a focus ring on a round button (${JSON.stringify(ring)})`);
    await page.screenshot({ path: path.join(SHOTS, `focus-${w}.png`), clip: { x: 0, y: w < 600 ? 844 - 180 : 900 - 110, width: w, height: w < 600 ? 180 : 110 } });
    if (touch) {
      // A tap leaves no hover background behind
      const before = await page.$eval(TAP(phone), (b) => getComputedStyle(b).backgroundColor);
      await page.tap(TAP(phone)); await sleep(400);
      const after = await page.$eval(TAP(phone), (b) => getComputedStyle(b).backgroundColor);
      check(after === before, `${at}: a tap on Next leaves no hover background (${before} → ${after})`);
    }
    check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
