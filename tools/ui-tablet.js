// Tablets get the desktop layout (TASK-tablet-desktop-layout-and-loaders-1104, part A; owner,
// 2026-11-04): two layouts, phone under 600 px and desktop from 600 px. Real candidate assets on
// the fixture origin (tools/ui-fixture.js): no node, no network. At 600, 700, 820, 1024, 1180 and
// 1366 px, portrait and landscape, light and dark, with a long zone name playing:
//   * the desktop layout: the sidebar with every desktop place (reachable when the window is short),
//     no bottom tabs;
//   * the player bar: no two of its parts overlap (as drawn, after any clipping), nothing leaves the
//     bar, also with a files job showing (on phones too, nothing cut at the edge), and the title, artist and zone name each keep at least 8
//     characters when cut;
//   * every desktop feature reachable: the queue, the volume (the bar's slider or its button, never
//     the Sound panel),
//     shuffle and repeat (bar or Now playing), the sound pill and the zone picker;
//   * the album page: the cover beside the words, and "Songs · count · Select" on one baseline;
//   * a touch screen keeps 40 px targets in the desktop layout.
// Also: 390 px is still the phone (tabs, mini player), and the old Layout choices map across
// (Compact = Phone, Comfortable = Automatic). Screenshots of each width go to the shots folder.
//   node tools/ui-tablet.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'tablet-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const artists = [{ id: 31, name: 'Calvin Russell' }];
const TITLES = ['Crossroad', 'Nothing Can Save Me', 'Rats and Roaches', 'This Is Your World', 'Baby I Love You', 'Little Stars'];
const tracks = Array.from({ length: 16 }, (_, i) => ({ ...fx.track, id: 100 + i, title: TITLES[i % 6] + (i > 5 ? ` (live ${i})` : ''), artists, album: { id: 7, title: 'Crossroad: Unplugged Live' }, track_no: i + 1, disc_no: 1, duration_ms: 200000 + i * 7000 }));
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: {}, title: 'Crossroad: Unplugged Live', artist: { name: 'Calvin Russell', id: 'lib-31' }, album: null, year: 1997, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: tracks.map((t, i) => ({ no: i + 1, title: t.title, artist: 'Calvin Russell', duration_ms: t.duration_ms, uri: 'track:' + t.id, track_id: t.id, id: 'lib-' + t.id, page: 'lib-' + t.id })),
  total_ms: tracks.reduce((a, t) => a + t.duration_ms, 0), albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
// The owner's zone (a long UPnP name), playing a song with a long title: what crowded the bar
const outputs = [{ id: 'upnp:fixture', name: 'Linn Klimax DS/3', kind: 'upnp', caps: { hw_volume: { kind: 'db', min: -80, max: 0, step: 1 } } }];
const zone = { id: 'linn-klimax-ds3-upnp-av', name: 'Linn Klimax DS/3 (UPnP-AV)', output: 'upnp:fixture', volume_db: -30, transport: 'playing', queue: [{ id: 1, uri: 'track:100', title: tracks[1].title }], queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 22, muted: false },
  now_playing: { item: { id: 1, uri: 'track:100', title: 'Nothing Can Save Me (live at the Crossroad sessions)', artist: 'Calvin Russell', album: 'Crossroad: Unplugged Live', duration_ms: 254000 }, index: 0, position_ms: 61000, duration_ms: 254000, source_format: fmt, output_format: fmt,
    signal_path: { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }, { name: 'output', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } } };

// A files job beside the sound pill (its chip has words and a bar): the right group's busiest state
const showJob = (page) => page.evaluate(() => { const c = document.querySelector('#jobchip'); c.hidden = false; c.innerHTML = '<svg class="ic"><use href="#i-upload"/></svg><span class="jc-text">1234/5678</span><span class="jc-bar"><span style="width:40%"></span></span>'; });
async function open(browser, w, h, opts = {}) {
  if (opts.touch) return openTouch(browser, w, h);
  const run = await fx.create(browser, w, { offline: !!opts.offline, fixtures: { '/zones': { body: [zone] }, ...(opts.layout ? {} : { '/outputs': { body: outputs } }), '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } } });
  await run.page.setViewportSize({ width: w, height: h });
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  // The layout index.html's inline script picks, before app.js runs (the first data-shell set)
  await run.context.addInitScript(() => { new MutationObserver((ms, o) => { const e = document.documentElement; if (e && e.dataset.shell) { window.__firstShell = e.dataset.shell; o.disconnect(); } }).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ['data-shell'] }); });
  await run.context.addInitScript(([z, theme, layout]) => { localStorage.setItem('pm.zone', z); if (theme) localStorage.setItem('pm.theme', theme); if (layout) localStorage.setItem('pm.layout', layout); }, [zone.id, opts.theme || '', opts.layout || '']);
  return run;
}
// A touch screen (pointer: coarse) at a tablet's width: the same fixture in a touch context
async function openTouch(browser, w, h) {
  const ctxOpts = { viewport: { width: w, height: h }, hasTouch: true, isMobile: true, reducedMotion: 'reduce' };
  const orig = browser.newContext.bind(browser);
  const b2 = Object.create(browser); b2.newContext = (o) => orig({ ...o, ...ctxOpts });
  const run = await fx.create(b2, w, { fixtures: { '/zones': { body: [zone] }, '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } } });
  await run.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage(() => {}); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  await run.context.addInitScript(([z]) => { localStorage.setItem('pm.zone', z); }, [zone.id]);
  return run;
}

// The player bar's parts: no two overlap, each stays inside the bar, cut text keeps 8 characters
const barReport = () => {
  const shown = (el) => { const r = el.getBoundingClientRect(); if (r.width < 1 || r.height < 1) return false; const st = getComputedStyle(el); return st.display !== 'none' && st.visibility !== 'hidden' && Number(st.opacity) > 0.05 && !el.closest('[hidden]'); };
  const bar = document.querySelector('.player'); const br = bar.getBoundingClientRect();
  const parts = [...bar.querySelectorAll('button, input, .time, .np-title, .np-sub, .qsrc, .qdot, .qtext, .qbadge, #zonename, .jc-text, .jc-bar, .ic')].filter(shown);
  // What shows of a part: its box cut by every ancestor in the bar that clips (overflow other than visible)
  const seen = (el) => { let r = el.getBoundingClientRect(); let { left, right, top, bottom } = r; for (let a = el.parentElement; a && a !== bar; a = a.parentElement) { const st = getComputedStyle(a); if (st.overflowX !== 'visible' || st.overflowY !== 'visible') { const c = a.getBoundingClientRect(); left = Math.max(left, c.left); right = Math.min(right, c.right); top = Math.max(top, c.top); bottom = Math.min(bottom, c.bottom); } } return { left, right, top, bottom }; };
  const name = (e) => `${e.tagName.toLowerCase()}${e.id ? '#' + e.id : ''}${typeof e.className === 'string' && e.className ? '.' + e.className.split(' ')[0] : ''}`;
  const overlaps = []; const outside = [];
  for (let i = 0; i < parts.length; i++) {
    const a = seen(parts[i]); if (a.right - a.left < 1 || a.bottom - a.top < 1) continue;
    if (a.left < br.left - 1 || a.right > br.right + 1 || a.top < br.top - 1 || a.bottom > br.bottom + 1) outside.push(name(parts[i]));
    for (let j = i + 1; j < parts.length; j++) {
      if (parts[i].contains(parts[j]) || parts[j].contains(parts[i])) continue;
      const b = seen(parts[j]);
      const w = Math.min(a.right, b.right) - Math.max(a.left, b.left); const h = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
      if (w > 1 && h > 1) overlaps.push(`${name(parts[i])} × ${name(parts[j])} (${Math.round(w)}×${Math.round(h)})`);
    }
  }
  // Characters a cut text still shows: the width left after clipping and the ellipsis, over the
  // text's average character width
  const ctx2d = document.createElement('canvas').getContext('2d');
  const kept = (el) => {
    if (!el || !shown(el)) return null; const t = el.textContent.trim(); if (!t) return null;
    const v = seen(el); const visW = Math.min(el.clientWidth, v.right - v.left); const full = el.scrollWidth;
    if (full <= visW + 1) return t.length;
    const st = getComputedStyle(el); ctx2d.font = `${st.fontWeight} ${st.fontSize} ${st.fontFamily}`;
    return Math.max(0, Math.floor((visW - ctx2d.measureText('…').width) / (full / t.length)));
  };
  // The quality shows whole: the badge uncut, or the whole sound line
  const whole = (el) => { if (!el || !shown(el)) return false; const r = el.getBoundingClientRect(); const v = seen(el); return v.right - v.left >= r.width - 1 && el.scrollWidth <= el.clientWidth + 1; };
  const quality = whole(bar.querySelector('#quality .qbadge')) || whole(bar.querySelector('#quality .qtext'));
  return { overlaps, outside, quality, title: kept(bar.querySelector('.np-title')), sub: kept(bar.querySelector('.np-sub')), zone: kept(bar.querySelector('#zonename')) };
};
// Baselines of the Songs head's title, count and button (a zero-height probe sits on each one's baseline)
const songsHead = () => {
  const head = document.querySelector('#isongs .section-head'); if (!head) return null;
  const probe = (el) => { const p = document.createElement('span'); p.style.cssText = 'display:inline-block;width:0;height:0;vertical-align:baseline;align-self:baseline'; el.appendChild(p); const y = p.getBoundingClientRect().top; p.remove(); return y; };
  const h3 = head.querySelector('h3'); const count = head.querySelector('h3 + .muted'); const sel = head.querySelector('[data-selstart]');
  return { title: probe(h3), count: count ? probe(count) : null, select: sel ? probe(sel) : null, countLeft: count ? count.getBoundingClientRect().left - h3.getBoundingClientRect().right : null };
};
const visible = (page, sel) => page.evaluate((s) => { const e = document.querySelector(s); if (!e) return false; const r = e.getBoundingClientRect(); const st = getComputedStyle(e); return r.width > 0 && r.height > 0 && st.display !== 'none' && st.visibility !== 'hidden' && !e.closest('[hidden]'); }, sel);

(async () => {
  const browser = await chromium.launch();
  const widths = [600, 700, 820, 1024, 1101, 1180, 1280, 1366, 1600, 1920];
  for (const w of widths) {
    for (const [orient, h] of [['portrait', Math.round(w * 4 / 3)], ['landscape', Math.round(w * 3 / 4)]]) {
      const tag = `${w}×${h}`;
      const { page, context, state } = await open(browser, w, h);
      await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs [data-selstart]'); await page.waitForTimeout(400);
      // The layout
      check(await page.evaluate(() => document.documentElement.dataset.shell) === 'desktop', `${tag}: the desktop layout`);
      check(!(await visible(page, '.bottomnav')), `${tag}: no bottom tabs`);
      const nav = await page.evaluate(() => [...document.querySelectorAll('#sidebar .nav-item:not(.nav-offline)')].map((b) => { b.scrollIntoView({ block: 'nearest' }); const r = b.getBoundingClientRect(); return { v: b.dataset.view, ok: r.width > 0 && r.height >= 32 && r.top >= 0 && r.bottom <= innerHeight && getComputedStyle(b).display !== 'none' }; }));
      check(nav.length === 13 && nav.every((n) => n.ok), `${tag}: the sidebar has every desktop place, each reachable (${nav.filter((n) => !n.ok).map((n) => n.v).join(', ') || nav.length})`);
      // The player bar
      const bar = await page.evaluate(barReport);
      check(!bar.overlaps.length, `${tag}: nothing in the player bar overlaps${bar.overlaps.length ? ': ' + bar.overlaps.slice(0, 4).join('; ') : ''}`);
      check(!bar.outside.length, `${tag}: everything stays in the player bar${bar.outside.length ? ': ' + bar.outside.join(', ') : ''}`);
      check(bar.title >= 8 && bar.sub >= 8 && bar.zone >= 8, `${tag}: title, artist and zone keep 8+ characters (${bar.title}, ${bar.sub}, ${bar.zone})`);
      // Every desktop feature, reachable
      check(await visible(page, '#sidebar .nav-item[data-view="queue"]'), `${tag}: the queue is in the sidebar`);
      check(bar.quality, `${tag}: the sound pill shows the quality whole (its badge, or the whole line)`);
      const volIn = (await visible(page, '.player .volume')) ? 'bar' : (await visible(page, '#volbtn')) ? 'button' : null;
      await page.click('#quality'); await page.waitForTimeout(200);
      const panelVol = await visible(page, '#qpanel input[type="range"]');
      check(!!volIn && !panelVol, `${tag}: the volume is in the bar, as its slider or its button, and not in the Sound panel (${volIn || 'not in the bar'}${panelVol ? ', and in the Sound panel' : ''})`);
      check(await visible(page, '#qpanel'), `${tag}: the sound pill opens the Sound panel (signal path)`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(150);
      await page.click('#zonebtn'); await page.waitForTimeout(150);
      check(await visible(page, '#zonepop'), `${tag}: the zone picker opens`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(150);
      const modesInBar = await visible(page, '#shufflebtn') && await visible(page, '#repeatbtn');
      if (!modesInBar) {
        await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden])');
        check(await visible(page, '#nowplaying [data-npt="shuffle"]') && await visible(page, '#nowplaying [data-npt="repeat"]'), `${tag}: shuffle and repeat are in Now playing`);
        if (!volIn) check(await visible(page, '#nowplaying #npvol'), `${tag}: Now playing has the volume`);
        await page.keyboard.press('Escape'); await page.waitForTimeout(200);
      } else check(true, `${tag}: shuffle and repeat are in the bar`);
      // The album page
      const hero = await page.evaluate(() => { const art = document.querySelector('.hero .art'); const h1 = document.querySelector('.hero h1'); const a = art.getBoundingClientRect(); const t = h1.getBoundingClientRect(); return { beside: t.left >= a.right, artW: Math.round(a.width) }; });
      check(hero.beside, `${tag}: the album's words sit beside its cover (cover ${hero.artW} px)`);
      const sh = await page.evaluate(songsHead);
      check(sh && Math.abs(sh.title - sh.count) <= 1.5 && Math.abs(sh.title - sh.select) <= 1.5 && sh.countLeft >= 0 && sh.countLeft <= 20, `${tag}: Songs, its count and Select on one baseline, the count beside the title (${sh && [sh.title, sh.count, sh.select].map(Math.round).join('/')}, gap ${sh && Math.round(sh.countLeft)})`);
      check(!state.errors.length, `${tag}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      check(overflow <= 0, `${tag}: no sideways scroll (${overflow})`);
      await showJob(page); await page.waitForTimeout(100);
      const busy = await page.evaluate(barReport);
      check(!busy.overlaps.length && !busy.outside.length, `${tag}: with a files job showing, still nothing overlaps or leaves the bar${busy.overlaps.length || busy.outside.length ? ': ' + [...busy.overlaps, ...busy.outside].slice(0, 4).join('; ') : ''}`);
      check(busy.quality && busy.title >= 8 && busy.sub >= 8 && busy.zone >= 8, `${tag}: with a files job, the quality whole and title, artist and zone 8+ characters (${busy.quality}, ${busy.title}, ${busy.sub}, ${busy.zone})`);
      if (orient === 'portrait' || w === 1366) {
        for (const theme of ['light', 'dark']) { await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme); await page.screenshot({ path: path.join(SHOTS, `album-${w}x${h}-${theme}.png`) }); }
      }
      await context.close();
    }
  }
  // Touch screens (44 px transport buttons, 40 px targets): the bar at every width, with and without a files job
  for (const w of widths) {
    const h = Math.round(w * 3 / 4); const { page, context } = await open(browser, w, h, { touch: true });
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs [data-selstart]'); await page.waitForTimeout(300);
    for (const job of [false, true]) {
      if (job) { await showJob(page); await page.waitForTimeout(100); }
      const b = await page.evaluate(barReport);
      check(!b.overlaps.length && !b.outside.length && b.quality && b.title >= 8 && b.sub >= 8 && b.zone >= 8, `touch ${w}×${h}${job ? ' with a files job' : ''}: the bar fits (${b.quality}, ${b.title}, ${b.sub}, ${b.zone})${b.overlaps.length || b.outside.length ? ': ' + [...b.overlaps, ...b.outside].slice(0, 4).join('; ') : ''}`);
    }
    const t = await page.evaluate(() => [...document.querySelectorAll('.player button, #sidebar .nav-item, .topbar .menu-btn')].filter((e) => e.getBoundingClientRect().width > 0 && getComputedStyle(e).display !== 'none' && !e.closest('[hidden]')).filter((e) => e.getBoundingClientRect().height < 40).map((e) => e.id || e.className.split(' ')[0]));
    check(!t.length, `touch ${w}: the bar's, sidebar's and top bar's buttons are 40 px or more${t.length ? ' (not: ' + t.join(', ') + ')' : ''}`);
    await context.close();
  }
  // A touch screen at a tablet's width: the desktop layout with 40 px targets
  {
    const { page, context } = await open(browser, 820, 1180, { touch: true });
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs [data-selstart]'); await page.waitForTimeout(300);
    const t = await page.evaluate(() => ({ shell: document.documentElement.dataset.shell, coarse: matchMedia('(pointer: coarse)').matches, select: document.querySelector('[data-selstart]').getBoundingClientRect().height, more: document.querySelector('#isongs [data-smore]').getBoundingClientRect().height, play: document.querySelector('.hero .actions > .btn').getBoundingClientRect().height }));
    check(t.coarse && t.shell === 'desktop' && t.select >= 40 && t.more >= 40 && t.play >= 40, `touch 820: the desktop layout keeps 40 px targets (${JSON.stringify(t)})`);
    const sh = await page.evaluate(songsHead);
    check(sh && Math.abs(sh.title - sh.select) <= 1.5, `touch 820: Select on the Songs baseline (${sh && [sh.title, sh.select].map(Math.round).join('/')})`);
    await page.screenshot({ path: path.join(SHOTS, 'album-820x1180-touch.png') });
    await context.close();
  }
  // Phones are still the phone, their mini player untouched by the desktop bar's rules
  for (const [w, h, touch] of [[360, 780, true], [375, 812, true], [390, 844, false], [390, 844, true]]) {
    const { page, context } = await open(browser, w, h, { touch });
    await fx.go(page, '/album/lib-7'); await page.waitForTimeout(800);
    const bar = await page.evaluate(barReport);
    check(!bar.overlaps.length && !bar.outside.length, `${w}${touch ? ' touch' : ''}: the mini player's parts don't overlap or leave it${bar.overlaps.length || bar.outside.length ? ': ' + [...bar.overlaps, ...bar.outside].slice(0, 4).join('; ') : ''}`);
    // With a files job's chip too (#269 review): nothing overlaps, and nothing is cut at the edge of
    // its group (the group clips what doesn't fit, so overlaps alone don't show it)
    await showJob(page); await page.waitForTimeout(150);
    const jb = await page.evaluate(barReport);
    // (every part inside the group, not only its children: a shrunk button would clip its own icon)
    const cut = await page.evaluate(() => { const g = document.querySelector('.player-right').getBoundingClientRect(); return [...document.querySelectorAll('.player-right *')].filter((e) => { const b = e.getBoundingClientRect(); return b.width > 0 && getComputedStyle(e).visibility !== 'hidden'; }).filter((e) => { const b = e.getBoundingClientRect(); return b.right > g.right + 0.5 || b.left < g.left - 0.5; }).map((e) => e.id || e.getAttribute('class') || e.tagName); });
    // A pill without a quality badge (nothing known of the format) isn't drawn empty beside the chip
    const bare = await page.evaluate(() => { const b = document.querySelector('#quality .qbadge'); if (!b) return null; const keep = b.outerHTML; b.remove(); const q = document.querySelector('#quality'); const shown = q.getBoundingClientRect().width > 0; q.querySelector('.qshort').insertAdjacentHTML('beforeend', keep); return shown; });
    check(bare === false, `${w}${touch ? ' touch' : ''} with a files job: a pill without a quality badge isn't drawn empty (${bare})`);
    const job = await visible(page, '#jobchip'); const zone = await visible(page, '#zonebtn');
    // (the phone's bar names no zone since TASK-phone-ui-simplify-1108: Now playing does)
    const titleW = await page.evaluate(() => document.querySelector('#np .np-title').getBoundingClientRect().width / document.querySelector('.player').getBoundingClientRect().width);
    check(job && !zone && titleW >= 0.5 && !jb.overlaps.length && !jb.outside.length && !cut.length, `${w}${touch ? ' touch' : ''} with a files job: the chip shows, the title keeps ${Math.round(titleW * 100)} % of the bar, nothing overlaps or is cut${jb.overlaps.length || jb.outside.length || cut.length ? ': ' + [...jb.overlaps, ...jb.outside, ...cut.map((c) => `${c} cut`)].slice(0, 4).join('; ') : ''}`);
    await page.screenshot({ path: path.join(SHOTS, `mini-job-${w}${touch ? '-touch' : ''}.png`), clip: { x: 0, y: h - 170, width: w, height: 170 } });
    await page.evaluate(() => { document.querySelector('#jobchip').hidden = true; });
    if (w !== 390 || touch) { await context.close(); continue; }
    const p = await page.evaluate(() => ({ shell: document.documentElement.dataset.shell, tabs: [...document.querySelectorAll('.bottomnav .bnav')].filter((b) => b.getBoundingClientRect().width > 0).map((b) => b.dataset.bnav).join(',') }));
    check(p.shell === 'phone' && p.tabs === 'home,search,library,zones', `390: the phone, with its four tabs (${p.shell}: ${p.tabs})`);
    check(!(await visible(page, '#sidebar')), '390: no sidebar');
    await page.screenshot({ path: path.join(SHOTS, 'album-390x844-light.png') });
    await context.close();
  }
  // Desktop forced on a 500 px window: the bar has no overlaps and the page doesn't scroll sideways
  {
    const { page, context } = await open(browser, 500, 800, { layout: 'desktop' });
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs [data-selstart]'); await page.waitForTimeout(300);
    for (const job of [false, true]) {
      if (job) { await showJob(page); await page.waitForTimeout(100); }
      const b = await page.evaluate(barReport); const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      check(!b.overlaps.length && !b.outside.length && b.quality && over <= 0, `Desktop at 500${job ? ' with a files job' : ''}: the bar fits, no sideways scroll (${over})${b.overlaps.length || b.outside.length ? ': ' + [...b.overlaps, ...b.outside].slice(0, 4).join('; ') : ''}`);
    }
    await context.close();
  }
  // The offline app (no node): no volume button (the Sound panel needs the node), so the slider stays
  // in the bar down to 760 px (#269 review: offline at 1100-1279 px had no volume)
  for (const w of [760, 1024, 1180, 1279]) {
    const { page, context } = await open(browser, w, 800, { offline: true });
    await fx.go(page, '/downloads'); await page.waitForFunction(() => document.body.classList.contains('offline')); await page.waitForTimeout(400);
    const b = await page.evaluate(barReport); const vol = await visible(page, '.player-right .volume input');
    check(vol && !b.overlaps.length && !b.outside.length, `offline at ${w}: the volume slider is in the bar and the bar fits${b.overlaps.length || b.outside.length ? ': ' + [...b.overlaps, ...b.outside].slice(0, 4).join('; ') : ''}`);
    await context.close();
  }
  // A playlist's Delete follows the window (#269 review): beside Play in a wide desktop window, in
  // ⋯ otherwise, also after a resize without reopening the page
  for (const widths of [[1280, 800, 1280], [800, 1280]]) {
    const { page, context, state } = await open(browser, widths[0], 800);
    await fx.go(page, '/playlist/c1'); await page.waitForSelector('#plplay'); await page.waitForTimeout(300);
    const menuHasDelete = async () => {
      await page.click('#plmore'); await page.waitForTimeout(250);
      const items = await page.evaluate(() => [...document.querySelectorAll('.popover.menu:not([hidden]) .pop-item')].map((b) => b.textContent.trim()));
      await page.keyboard.press('Escape'); await page.waitForTimeout(200);
      return items.some((t) => /^Delete playlist/.test(t));
    };
    for (const w of widths) {
      await page.setViewportSize({ width: w, height: 800 }); await page.waitForTimeout(300);
      const beside = await visible(page, '#pldel'); const inMenu = await menuHasDelete();
      const wide = w > 1024;
      check(beside === wide && inMenu === !wide, `playlist opened at ${widths[0]}, at ${w}: Delete ${wide ? 'beside Play, not in ⋯' : 'in ⋯, not beside Play'} (beside ${beside}, in ⋯ ${inMenu})`);
    }
    check(!state.errors.length, `playlist resize: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
    await context.close();
  }
  // Settings › Appearance › Layout: Automatic / Phone / Desktop; the old choices map across
  // Comfortable (the tablet rail from 600 px, a phone below) becomes Automatic: the same at every width
  for (const [was, now, w, kept] of [['compact', 'phone', 1024, 'phone'], ['comfortable', 'desktop', 1024, null], ['comfortable', 'phone', 500, null], ['desktop', 'desktop', 500, 'desktop'], ['desktop', 'phone', 390, 'desktop'], ['phone', 'phone', 1366, 'phone']]) {
    const { page, context, state } = await open(browser, w, 800, { layout: was });
    await fx.go(page, '/settings/appearance'); await page.waitForSelector('[data-layout-pick]', { state: 'attached' }).catch(() => {});
    const s = await page.evaluate(() => ({ first: window.__firstShell, shell: document.documentElement.dataset.shell, kept: localStorage.getItem('pm.layout'), picks: [...document.querySelectorAll('[data-layout-pick]')].map((b) => `${b.dataset.layoutPick}${b.classList.contains('on') ? '*' : ''}`).join(',') }));
    check(!state.errors.length && s.first === now && s.shell === now && s.kept === kept && s.picks === `auto,phone,desktop`.replace(kept || 'auto', (kept || 'auto') + '*'), `Layout "${was}" at ${w}: ${now} (${JSON.stringify(s)})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
