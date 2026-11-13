// Now playing on a phone (TASK-phone-nowplaying-integrated-1105, revised by TASK-phone-ui-simplify-1108;
// owner, 2026-11-08: the one long scroll "feels weird"). Real candidate assets on the fixture origin
// (tools/ui-fixture.js) with a phone's viewport rules and the app's bridge, a Linn playing with a
// queue, at 360x800, 390x844 and 412x915, dark and light:
//   * Now playing takes the whole screen: the app's tabs and the mini player are hidden, nothing of
//     it sits under another bar, and the page is the screen's width;
//   * the player is one screen that doesn't scroll: cover, title, seek, transport, volume and three
//     buttons (Up next, Lyrics, More), one 16 px side inset for the cover, title and seek;
//   * no five-tab row, side sheet, close button or long scroll of sections;
//   * Lyrics opens as a sheet; synced lyrics scroll their own box; a redraw keeps the sheet, its
//     place and the focus;
//   * a swipe down inside the sheet's list doesn't close it, on its head it does; a swipe down from
//     the player closes Now playing and focus goes back to the mini player;
//   * on the library page the mini player and the tabs share an edge (one surface), the tabs reach
//     the screen's bottom, and the filter row and the quality chips are at least 12 px apart.
//   node tools/ui-nowplaying-phone.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'nowplaying-phone-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const TITLES = ['Come On', 'Flowing River', 'Illusions', 'Bad Dreams', 'Shine', 'Gone', 'Nights', 'Days'];
const queue = TITLES.map((title, i) => ({ id: i + 1, uri: `track:${i + 1}`, title, artist: 'Dr. Peacock, Sefa' }));
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [{ name: 'decode', in_format: fmt, out_format: fmt, quality: 'bit_perfect', note: null }] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = { id: 'linn', name: 'Linn Klimax DS3:UPnP AV', output: 'upnp:linn', transport: 'playing', queue, queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: np };
const albums = Array.from({ length: 8 }, (_, i) => ({ id: 70 + i, title: `Album ${i}`, album_artist: 'Sefa', year: 2017, tracks: 10 }));
// Mixed qualities, so the Quality row narrows and shows (it hides when every album is the same)
const formats = albums.map((a, i) => ({ album_id: a.id, label: i % 2 ? '24/96 FLAC' : 'FLAC', hi_res: !!(i % 2), lossless: true, dsd: false }));
// Synced lyrics: their follower scrolls their own box, never the page (review: it scrolled the one screen)
const lyrics = { synced: true, source: 'file', lines: Array.from({ length: 40 }, (_, i) => ({ ms: i * 2000, text: `Line ${i + 1} of the song` })) };
const fixtures = { '/library/albums/formats': { body: formats }, '/library/tracks/1/lyrics': { body: lyrics }, '/zones': { body: [zone] }, '/outputs': { body: [{ id: 'upnp:linn', name: 'Linn', kind: 'upnp', caps: { hw_volume: 'percent' } }] }, '/zones/linn/queue': { body: queue }, '/library/albums': { body: albums } };

(async () => {
  const browser = await chromium.launch();
  for (const [w, h] of [[360, 800], [390, 844], [412, 915]]) {
    for (const theme of ['dark', 'light']) {
      const at = `${w} ${theme}`;
      const { page, context, state } = await fx.create(browser, w, { mobile: true, fixtures });
      await page.setViewportSize({ width: w, height: h });
      await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
      await context.addInitScript((t) => { localStorage.setItem('pm.zone', 'linn'); localStorage.setItem('pm.theme', t); window.pmDesktop = { platform: 'android', version: '0.0.0' }; }, theme);
      // The library page: the mini player on the tabs, the tabs to the bottom, the header's gap
      await fx.go(page, '/albums'); await page.waitForSelector('#main .grid .tile'); await sleep(400);
      // The gap is measured to the first chip: on a touch phone the row's box also holds the chips'
      // tap reach (padding), which is not a visible gap
      const lib = await page.evaluate(() => {
        const r = (s) => document.querySelector(s).getBoundingClientRect(); const cs = (s) => getComputedStyle(document.querySelector(s));
        const head = document.querySelector('#main > .page-head'); const chips = head && head.nextElementSibling;
        return { player: r('.player'), nav: r('.bottomnav'), vh: innerHeight, pbg: cs('.player').backgroundColor, nbg: cs('.bottomnav').backgroundColor, pradius: cs('.player').borderTopLeftRadius, gap: head && chips && chips.matches('.chips') && chips.querySelector('.chip') ? Math.round(chips.querySelector('.chip').getBoundingClientRect().top - head.getBoundingClientRect().bottom) : null };
      });
      check(Math.abs(lib.player.bottom - lib.nav.top) <= 0.5 && lib.player.left === 0 && lib.player.right === w && lib.pbg === lib.nbg && lib.pradius === '0px', `${at}: the mini player sits on the tabs as one surface (gap ${Math.round(lib.nav.top - lib.player.bottom)} px, ${lib.player.left}..${lib.player.right}, ${lib.pbg} / ${lib.nbg})`);
      check(Math.abs(lib.nav.bottom - lib.vh) <= 0.5, `${at}: the tabs reach the screen's bottom (${Math.round(lib.nav.bottom)} of ${lib.vh})`);
      check(lib.gap != null && lib.gap >= 12, `${at}: the filter row and the quality chips are ${lib.gap} px apart`);
      if (theme === 'dark') await page.screenshot({ path: path.join(SHOTS, `library-${w}.png`) });
      // Now playing, its first screen
      await page.click('#np'); await page.waitForSelector('#nowplaying:not([hidden]) .np-transport'); await sleep(500);
      const top = await page.evaluate(() => {
        const ov = document.querySelector('#nowplaying'); const o = ov.getBoundingClientRect(); const r = (s) => { const e = ov.querySelector(s); return e ? e.getBoundingClientRect() : null; };
        const vis = (s) => { const e = document.querySelector(s); return !!e && getComputedStyle(e).display !== 'none' && e.getClientRects().length > 0; };
        const pg = ov.querySelector('.np-page');
        return { o: { top: o.top, bottom: o.bottom, left: o.left, right: o.right }, vh: innerHeight, vw: document.documentElement.clientWidth, sw: document.documentElement.scrollWidth, iw: innerWidth, nav: vis('.bottomnav'), player: vis('.player'), npTop: r('.np-top'), big: r('.np-big'), title: r('.np-titles h1'), seek: r('#npseek'), transport: r('.np-transport'), vol: r('.np-vol'), dock: r('.np-dock'), over: Math.max(ov.scrollHeight - ov.clientHeight, pg.scrollHeight - pg.clientHeight), oldTabs: vis('#nowplaying .np-tabs'), sideSheet: vis('#nowplaying .np-sheet'), sheet: vis('#nowplaying .np-bsheet'), close: vis('#npsheetclose'), sections: ov.querySelectorAll('.np-sec, .np-jump').length, dockBtns: [...ov.querySelectorAll('.np-dock [data-npsheet]')].map((c) => `${c.tagName}:${c.textContent.trim()}`) };
      });
      check(!top.nav && !top.player && top.o.top === 0 && Math.abs(top.o.bottom - top.vh) <= 0.5 && top.iw === top.vw && top.sw <= top.vw, `${at}: Now playing takes the whole screen (tabs ${top.nav ? 'shown' : 'hidden'}, mini player ${top.player ? 'shown' : 'hidden'}, ${top.o.top}..${Math.round(top.o.bottom)} of ${top.vh}, width ${top.sw} of ${top.vw})`);
      check(top.transport && top.vol && top.dock && top.dock.bottom <= top.vh && top.over <= 1 && top.big.top - top.npTop.bottom >= 4, `${at}: the player is one screen that doesn't scroll (buttons end at ${top.dock && Math.round(top.dock.bottom)} of ${top.vh}, overflow ${top.over} px)`);
      const lefts = [top.big.left, top.title.left, top.seek.left].map(Math.round);
      check(lefts.every((l) => l === 16) && Math.round(top.big.right) === w - 16, `${at}: one 16 px side inset for the cover, title and seek (${lefts.join(', ')}; cover ends ${Math.round(top.big.right)})`);
      check(!top.oldTabs && !top.sideSheet && !top.sheet && !top.close && !top.sections, `${at}: no five-tab row, side sheet, close button or long scroll of sections on the first screen`);
      check(top.dockBtns.join(',') === 'BUTTON:Up next,BUTTON:Lyrics,BUTTON:More', `${at}: Up next, Lyrics and More as three buttons under the player (${top.dockBtns.join(' ')})`);
      if (theme === 'dark') await page.screenshot({ path: path.join(SHOTS, `nowplaying-${w}.png`) });
      // Lyrics: a sheet; synced lines follow in their own box
      await page.click('.np-dock [data-npsheet="lyrics"]'); await page.waitForSelector('.np-bsheet:not([hidden]) .lyric-lines'); await sleep(1600);
      const ly = await page.evaluate(() => { const box = document.querySelector('#nplyrics .lyric-lines'); const on = document.querySelector('#nplyrics .lyric-line.on'); const b = box && box.getBoundingClientRect(); const o = on && on.getBoundingClientRect(); const body = document.querySelector('.np-bsheet .np-sheet-body'); return { lines: !!on, boxTop: box ? box.scrollTop : 0, inBox: !!(b && o && o.top >= b.top - 1 && o.bottom <= b.bottom + 1), body: body.scrollTop, tab: document.querySelector('.np-stabs [aria-selected="true"]')?.textContent.trim() }; });
      check(ly.lines && ly.boxTop > 0 && ly.inBox && ly.tab === 'Lyrics', `${at}: Lyrics opens as a sheet and synced lines follow in their own box (scrolled ${ly.boxTop}, the current line in view ${ly.inBox})`);
      if (theme === 'dark') await page.screenshot({ path: path.join(SHOTS, `nowplaying-lyrics-${w}.png`) });
      // More: Details first; a redraw keeps the sheet's place and the focus on its Play
      await page.click('.np-stabs [data-npsheet="more"]'); await page.waitForSelector('.np-bsheet[data-tab="more"] #nppart-details'); await sleep(500);
      await page.$eval('.np-bsheet .np-sheet-body', (b) => { b.scrollTop = 120; });
      const before = await page.$eval('.np-bsheet .np-sheet-body', (b) => b.scrollTop);
      await page.click('.np-bsheet .np-strip [data-npt="toggle"]'); await sleep(900);
      const after = await page.evaluate(() => ({ top: document.querySelector('.np-bsheet .np-sheet-body').scrollTop, focus: !!document.activeElement && document.activeElement.matches('.np-strip [data-npt="toggle"]'), tab: document.querySelector('.np-bsheet').dataset.tab }));
      check(after.tab === 'more' && Math.abs(after.top - before) <= 4 && after.focus, `${at}: a redraw keeps the sheet, its place and the focus on its Play (${before} → ${after.top}, focus ${after.focus})`);
      if (theme === 'dark') await page.screenshot({ path: path.join(SHOTS, `nowplaying-more-${w}.png`) });
      // A finger's swipe (Pointer Events): down on the sheet's head closes it, down on the player closes Now playing
      const swipe = (sel, dy) => page.$eval(sel, (el, d) => {
        const b = el.getBoundingClientRect(); const x = b.left + b.width / 2; const y = b.top + Math.min(20, b.height / 2);
        const t = document.elementFromPoint(x, y) || el;
        const ev = (type, yy) => t.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 3, pointerType: 'touch', isPrimary: true, clientX: x, clientY: yy }));
        ev('pointerdown', y); for (let i = 1; i <= 5; i++) ev('pointermove', y + (d * i) / 5); ev('pointerup', y + d);
      }, dy);
      await swipe('.np-bsheet .np-sheet-body', 200); await sleep(300);
      check(!(await page.$eval('.np-bsheet', (e) => e.hidden)), `${at}: a swipe down inside the sheet's list doesn't close it (only its head does)`);
      await swipe('.np-bsheet .np-sheet-head', 200); await sleep(400);
      check(await page.$eval('.np-bsheet', (e) => e.hidden) && !(await page.$eval('#nowplaying', (e) => e.hidden)), `${at}: a swipe down on the sheet's head closes it, Now playing stays`);
      await swipe('.np-big', 200); await sleep(400);
      const closed = await page.evaluate(() => ({ hidden: document.querySelector('#nowplaying').hidden, focus: document.activeElement && document.activeElement.id, nav: getComputedStyle(document.querySelector('.bottomnav')).display }));
      check(closed.hidden && closed.focus === 'np' && closed.nav !== 'none', `${at}: a swipe down from the player closes it, the tabs come back and focus is on the mini player (${closed.focus})`);
      check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
      await context.close();
    }
  }
  // A tablet keeps the player beside the open sheet
  {
    const { page, context } = await fx.create(browser, 1024, { fixtures });
    await page.setViewportSize({ width: 1024, height: 768 });
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
    await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn'); });
    await fx.go(page, '/albums'); await page.waitForSelector('#np'); await sleep(600);
    await page.click('#npart'); await page.waitForSelector('#nowplaying:not([hidden]) .np-player', { timeout: 5000 });
    const t = await page.evaluate(() => { const ov = document.querySelector('#nowplaying'); const p = ov.querySelector('.np-player').getBoundingClientRect(); const sh = ov.querySelector('.np-sheet:not([hidden])'); const r = sh && sh.getBoundingClientRect(); return { open: !ov.hidden, sheet: !!r && r.width > 0, beside: !!r && p.right <= r.left + 1 && r.top < p.bottom, one: !!ov.querySelector('.np-dock, .np-bsheet') }; });
    check(t.open && t.sheet && t.beside && !t.one, `1024: a tablet keeps the player beside the open sheet (open ${t.open}, sheet ${t.sheet}, beside ${t.beside}, one screen ${t.one})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
