// Now playing on a phone is one screen (TASK-phone-nowplaying-integrated-1105; owner, 2026-11-05:
// "bottom spacing and the detail page of a song on android look shit with bad submenu navigation
// … could be more integrated everything"). Real candidate assets on the fixture origin
// (tools/ui-fixture.js) with a phone's viewport rules and the app's bridge, a Linn playing with a
// queue, at 360x800, 390x844 and 412x915, dark and light:
//   * Now playing takes the whole screen: the app's tabs and the mini player are hidden, nothing of
//     it sits under another bar, and the page is the screen's width;
//   * the first screen is the player: cover, title, seek, transport and volume all on it, at least
//     12 px under the top bar, and one 16 px side inset for the cover, title, seek and chips;
//   * under it Up next, Lyrics, Details, Sources and Tags as sections in the same scroll, with one
//     row of chips (buttons) that stays at the top with the song and Play once the transport scrolls
//     away; a chip scrolls its section to just under the row and marks itself current;
//   * no second row of tabs, no sheet mode and no close button of its own;
//   * synced lyrics scroll their own box, never the page; a redraw while reading further down keeps
//     the place and the focus; scrolling by hand moves the current chip; the chips row is as tall
//     stuck as not;
//   * a swipe down closes it from the player, not from a scrolled section; focus goes back to the
//     mini player;
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
// Synced lyrics: their follower scrolls their own box, never the page (review: it scrolled the one screen)
const lyrics = { synced: true, source: 'file', lines: Array.from({ length: 40 }, (_, i) => ({ ms: i * 2000, text: `Line ${i + 1} of the song` })) };
const fixtures = { '/library/tracks/1/lyrics': { body: lyrics }, '/zones': { body: [zone] }, '/outputs': { body: [{ id: 'upnp:linn', name: 'Linn', kind: 'upnp', caps: { hw_volume: 'percent' } }] }, '/zones/linn/queue': { body: queue }, '/library/albums': { body: albums } };

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
      const lib = await page.evaluate(() => {
        const r = (s) => document.querySelector(s).getBoundingClientRect(); const cs = (s) => getComputedStyle(document.querySelector(s));
        const head = document.querySelector('#main > .page-head'); const chips = head && head.nextElementSibling;
        return { player: r('.player'), nav: r('.bottomnav'), vh: innerHeight, pbg: cs('.player').backgroundColor, nbg: cs('.bottomnav').backgroundColor, pradius: cs('.player').borderTopLeftRadius, gap: head && chips && chips.matches('.chips') ? Math.round(chips.getBoundingClientRect().top - head.getBoundingClientRect().bottom) : null };
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
        return { o: { top: o.top, bottom: o.bottom, left: o.left, right: o.right }, vh: innerHeight, vw: document.documentElement.clientWidth, sw: document.documentElement.scrollWidth, iw: innerWidth, nav: vis('.bottomnav'), player: vis('.player'), npTop: r('.np-top'), big: r('.np-big'), title: r('.np-titles h1'), seek: r('#npseek'), transport: r('.np-transport'), vol: r('.np-vol'), chipsRow: r('.np-jump'), oldTabs: vis('#nowplaying .np-tabs'), sheet: vis('#nowplaying .np-sheet'), close: vis('#npsheetclose'), sections: [...ov.querySelectorAll('.np-sec')].map((s) => s.id), chips: [...ov.querySelectorAll('[data-npjump]')].map((c) => ({ k: c.dataset.npjump, tag: c.tagName })) };
      });
      check(!top.nav && !top.player && top.o.top === 0 && Math.abs(top.o.bottom - top.vh) <= 0.5 && top.iw === top.vw && top.sw <= top.vw, `${at}: Now playing takes the whole screen (tabs ${top.nav ? 'shown' : 'hidden'}, mini player ${top.player ? 'shown' : 'hidden'}, ${top.o.top}..${Math.round(top.o.bottom)} of ${top.vh}, width ${top.sw} of ${top.vw})`);
      check(top.transport && top.vol && top.vol.bottom <= top.vh && top.big.top - top.npTop.bottom >= 12, `${at}: the player is the first screen (volume ends at ${top.vol && Math.round(top.vol.bottom)} of ${top.vh}; ${Math.round(top.big.top - top.npTop.bottom)} px under the top bar)`);
      const lefts = [top.big.left, top.title.left, top.seek.left].map(Math.round);
      check(lefts.every((l) => l === 16) && Math.round(top.big.right) === w - 16, `${at}: one 16 px side inset for the cover, title and seek (${lefts.join(', ')}; cover ends ${Math.round(top.big.right)})`);
      check(!top.oldTabs && !top.sheet && !top.close, `${at}: no second row of tabs, sheet or close button of its own`);
      check(top.sections.join(',') === 'npsec-queue,npsec-lyrics,npsec-details,npsec-sources,npsec-tags' && top.chips.every((c) => c.tag === 'BUTTON') && top.chips.length === 5, `${at}: Up next, Lyrics, Details, Sources and Tags in the same scroll, with five chip buttons (${top.sections.join(' ')})`);
      if (theme === 'dark') await page.screenshot({ path: path.join(SHOTS, `nowplaying-${w}.png`) });
      await sleep(1600);
      const ly = await page.evaluate(() => ({ top: document.querySelector('#nowplaying').scrollTop, lines: !!document.querySelector('#nplyrics .lyric-line.on'), barH: document.querySelector('.np-jump').getBoundingClientRect().height }));
      check(ly.top === 0 && ly.lines, `${at}: synced lyrics follow in their own box and the page stays on the player (page at ${ly.top}, a current line ${ly.lines})`);
      // A chip: its section just under the chips row, the row stuck with the song and Play
      await page.click('[data-npjump="details"]'); await sleep(900);
      const jumped = await page.evaluate(() => {
        const ov = document.querySelector('#nowplaying'); const bar = ov.querySelector('.np-jump').getBoundingClientRect(); const sec = ov.querySelector('#npsec-details').getBoundingClientRect();
        const strip = ov.querySelector('.np-jump-strip'); const play = ov.querySelector('.np-jump-play');
        return { end: ov.scrollTop + ov.clientHeight >= ov.scrollHeight - 2, barTop: Math.round(bar.top), barBottom: Math.round(bar.bottom), secTop: Math.round(sec.top), stuck: ov.querySelector('.np-jump').classList.contains('stuck'), strip: getComputedStyle(strip).display !== 'none', play: getComputedStyle(play).display !== 'none', cur: ov.querySelector('[data-npjump][aria-current="true"]')?.dataset.npjump, scroll: ov.scrollTop, focus: document.activeElement && document.activeElement.id };
      });
      check(jumped.barTop === 0 && jumped.stuck && jumped.strip && jumped.play, `${at}: the chips row stays at the top with the song and Play (${jumped.barTop}, stuck ${jumped.stuck})`);
      // (where the page ends first, as far as it goes: the part is then below the row, not under it)
      check(jumped.secTop >= jumped.barBottom && (jumped.end || jumped.secTop - jumped.barBottom <= 24), `${at}: Details scrolls to just under the chips row (${jumped.secTop - jumped.barBottom} px under it${jumped.end ? ', the page ends there' : ''})`);
      check(jumped.cur === 'details' && jumped.focus === 'npsech-details', `${at}: the Details chip is current and its heading has the focus (${jumped.cur}, ${jumped.focus})`);
      if (theme === 'dark') await page.screenshot({ path: path.join(SHOTS, `nowplaying-details-${w}.png`) });
      const stuckH = await page.evaluate(() => document.querySelector('.np-jump').getBoundingClientRect().height);
      check(Math.abs(stuckH - ly.barH) <= 0.5, `${at}: the chips row is as tall stuck as not (${ly.barH} → ${stuckH})`);
      // Further down, Play in the chips row: the redraw after it keeps the place and the focus there
      await page.click('[data-npjump="tags"]'); await sleep(900);
      const before = await page.evaluate(() => document.querySelector('#nowplaying').scrollTop);
      await page.click('.np-jump-play'); await sleep(900);
      const after = await page.evaluate(() => ({ top: document.querySelector('#nowplaying').scrollTop, focus: !!document.activeElement && document.activeElement.matches('.np-jump [data-npt="toggle"]') }));
      check(Math.abs(after.top - before) <= 4 && after.focus, `${at}: a redraw further down keeps the place and the focus on the row's Play (${before} → ${after.top}, focus ${after.focus})`);
      // Scrolling by hand: the part at the top is the current chip
      await page.mouse.move(w / 2, h / 2); await page.mouse.wheel(0, -1);
      await page.evaluate(() => { const ov = document.querySelector('#nowplaying'); const sec = ov.querySelector('#npsec-lyrics'); ov.scrollTop = sec.offsetTop + 4 - ov.querySelector('.np-jump').offsetHeight; ov.dispatchEvent(new Event('scroll')); }); await sleep(400);
      const hand = await page.evaluate(() => document.querySelector('[data-npjump][aria-current="true"]')?.dataset.npjump);
      check(hand === 'lyrics', `${at}: scrolling by hand to Lyrics makes it the current chip (${hand})`);
      // A swipe down on a scrolled section doesn't close it; from the player it does, and focus goes back
      const swipe = (sel) => page.$eval(sel, (el) => {
        const b = el.getBoundingClientRect(); const x = b.left + b.width / 2; const y = Math.max(b.top + 20, 40);
        const t = (yy) => new Touch({ identifier: 1, target: el, clientX: x, clientY: yy });
        el.dispatchEvent(new TouchEvent('touchstart', { touches: [t(y)], changedTouches: [t(y)], bubbles: true, cancelable: true }));
        el.dispatchEvent(new TouchEvent('touchend', { touches: [], changedTouches: [t(y + 200)], bubbles: true, cancelable: true }));
      });
      await swipe('#npsec-details'); await sleep(300);
      check(!(await page.$eval('#nowplaying', (e) => e.hidden)), `${at}: a swipe down on a scrolled section leaves Now playing open`);
      await page.$eval('#nowplaying', (ov) => { ov.scrollTop = 0; }); await sleep(300);
      await swipe('.np-big'); await sleep(400);
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
    const t = await page.evaluate(() => { const ov = document.querySelector('#nowplaying'); const p = ov.querySelector('.np-player').getBoundingClientRect(); const sh = ov.querySelector('.np-sheet:not([hidden])'); const r = sh && sh.getBoundingClientRect(); return { open: !ov.hidden, sheet: !!r && r.width > 0, beside: !!r && p.right <= r.left + 1 && r.top < p.bottom, one: !!ov.querySelector('.np-jump') }; });
    check(t.open && t.sheet && t.beside && !t.one, `1024: a tablet keeps the player beside the open sheet (open ${t.open}, sheet ${t.sheet}, beside ${t.beside}, one screen ${t.one})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
