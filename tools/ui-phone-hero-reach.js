// On a phone, the small links in an item page's top card and About are 44 px taps, without anything
// drawn moving:
//   * "All about this album/song/artist →" (#iglobal, 14 px tall): its whole paragraph, the "every
//     source, versions and tags" under it too, and 5 px above and below it; a press on that second
//     line opens the song's page for every source. The byline's links above (artist, album) keep
//     their own boxes, and so does the source line's link out above it (the album here has one), and
//     a press in the top half of the gap above it stays off #iglobal;
//   * the tag + (.ptag-add, drawn 40 × 40 px) reaches 2 px past its edge (44 × 44 px), and a press
//     just outside it opens the tag field; the group's name and the chips beside it keep theirs;
//   * About's "Wikipedia" link (17 px tall) reaches 14 px up and down, 8 px right and 3 px left,
//     where More's reach (8 px) stops: More keeps its own reach.
// At 320, 360 and 412 px (360 as the Android app too), album, song and artist pages. Desktop
// (1280 px) has none of these layers. Nothing is wider than the screen.
// Presses use page.mouse.click at exact points (a touchscreen tap snaps onto a nearby target).
// Real candidate assets, isolated responses: node tools/ui-phone-hero-reach.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-hero-reach-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, timeout = 4000) => { const end = Date.now() + timeout; for (;;) { const v = await fn(); if (v) return v; if (Date.now() > end) return v; await sleep(50); } };

const SONG = 'https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69';
const base = { source: 'Your library', source_id: 'library', uri: null, song_uri: SONG, link: null, plays_from: { service: 'library', label: 'Your library' }, albums: [], versions: [], also_on: [], about: [], open_on: [], connect: [], on_pubky: null };
const songs = [1, 2, 3].map((n) => ({ id: `lib-${n}`, track_id: n, title: `Song ${n}`, artist: 'Sefa', duration_ms: 200000, no: n }));
const about_text = { text: 'Sefa is a Dutch hardcore producer known for melodic and classical influences. '.repeat(6), source: 'Wikipedia', url: 'https://en.wikipedia.org/wiki/Sefa', lang: 'en', license: 'CC BY-SA 4.0' };
const about = [{ label: 'Released', value: '2024', kind: 'fact' }];
const fixtures = {
  '/item/album/lib-7': { body: { ...base, kind: 'album', id: 'lib-7', title: 'Journey', section: 'albums', target: { album_id: 7 }, link: 'https://example.com/journey', songs, artist: { id: 'lib-2', name: 'Sefa' }, year: 2024, about, about_text } },
  '/item/song/lib-1': { body: { ...base, kind: 'song', id: 'lib-1', title: 'Journey Through Time', section: 'tracks', target: { track_id: 1 }, songs: [songs[0]], artist: { id: 'lib-2', name: 'Sefa' }, album: { id: 'lib-7', name: 'Journey' }, year: 2024, about, about_text } },
  '/item/artist/lib-2': { body: { ...base, kind: 'artist', id: 'lib-2', title: 'Sefa', section: 'artists', target: { artist_id: 2 }, songs, about, about_text } },
  '/item/album/lib-7/extras': { body: {} }, '/item/song/lib-1/extras': { body: {} }, '/item/artist/lib-2/extras': { body: {} },
  '/reviews': { body: { reviews: [] } }, '/tags': { body: { tags: [], uri: SONG } }, '/comments': { body: { comments: [], indexed: true } },
};
const PAGES = [['album', '/album/lib-7'], ['song', '/song/lib-1'], ['artist', '/artist/lib-2']];

async function open(browser, width, route, android) {
  const { page, context, state } = await fx.create(browser, width, width < 600 ? { mobile: true, fixtures: { ...fixtures } } : { fixtures: { ...fixtures } });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  await fx.go(page, route);
  await page.waitForSelector('#iplay'); await sleep(300);
  return { page, context, state };
}
// What a press at (x, y) lands on, as the element's selector-ish name; scrolled so the point is in view
const probe = (page, pts) => page.evaluate((pts) => {
  const name = (t) => { if (!t) return 'nothing'; const a = t.closest('#iglobal, #iartist, #ialbum, .ptag-add, .wikilink, #iblurbmore, .ptag, .tgroup-name, .tagfaces'); return a ? (a.id ? `#${a.id}` : `.${a.className.split(' ').pop()}`) : (t.id ? `#${t.id}` : t.tagName.toLowerCase()); };
  return pts.map(([x, y]) => name(document.elementFromPoint(x, y)));
}, pts);
const rect = (page, sel) => page.evaluate((sel) => { const e = document.querySelector(sel); if (!e || !e.getClientRects().length) return null; const b = e.getBoundingClientRect(); return { l: b.left, t: b.top, r: b.right, b: b.bottom, w: b.width, h: b.height }; }, sel);
// The reach through the middle of a control, measured: how far up/down and left/right a press still lands on it
const reach = (page, sel) => page.evaluate((sel) => {
  const e = document.querySelector(sel); const b = e.getBoundingClientRect();
  const cx = b.left + b.width / 2; const cy = b.top + b.height / 2;
  const on = (x, y) => { const t = document.elementFromPoint(x, y); return !!t && (t === e || e.contains(t)); };
  let up = 0; let dn = 0; let l = 0; let r = 0;
  while (up < 40 && on(cx, cy - up - 0.5)) up += 0.5; while (dn < 40 && on(cx, cy + dn + 0.5)) dn += 0.5;
  while (l < 80 && on(cx - l - 0.5, cy)) l += 0.5; while (r < 80 && on(cx + r + 0.5, cy)) r += 0.5;
  return { h: up + dn, w: l + r, top: cy - up, bottom: cy + dn, left: cx - l, right: cx + r };
}, sel);

(async () => {
  const browser = await chromium.launch();
  for (const [width, android] of [[360, false], [412, false], [360, true], [320, false]]) {
    const at = `${width}${android ? ' Android' : ''}`;
    for (const [kind, route] of PAGES) {
      const { page, context, state } = await open(browser, width, route, android);
      const w = `${at} ${kind}`;
      // ---- All about this … → ----
      const p = await rect(page, '.to-global'); const g = await rect(page, '#iglobal');
      check(!!p && !!g, `${w}: "All about this ${kind}" is there`);
      if (p && g) {
        const rg = await reach(page, '#iglobal');
        check(rg.h >= 44, `${w}: "All about this ${kind}" is a 44 px tap through its middle (${rg.h} px, drawn ${g.h.toFixed(1)})`);
        const [second, above, below, wellAbove] = await probe(page, [[p.l + 20, p.b - 6], [g.l + 10, p.t - 4], [p.l + 10, p.b + 4], [g.l + 10, p.t - 6]]);
        check(second === '#iglobal', `${w}: a press on its second line ("every source…") opens it (${second})`);
        check(above === '#iglobal' && below === '#iglobal', `${w}: 4 px above and below its paragraph open it too (${above}, ${below})`);
        check(wellAbove !== '#iglobal', `${w}: 6 px above its paragraph is not it (${wellAbove})`);
        // The byline's links keep every point of their own box
        for (const sel of ['#iartist', '#ialbum', '#ialso .chiplink']) {
          const b = await rect(page, sel); if (!b) continue;
          const hits = await probe(page, [[b.l + 2, b.t + 2], [b.r - 2, b.b - 2], [b.l + b.w / 2, b.b - 2]]);
          check(hits.every((h) => h !== '#iglobal'), `${w}: ${sel} above keeps its whole box (${hits.join(' ')})`);
        }
        if (kind === 'album') check(!!(await rect(page, '#ialso .chiplink')), `${w}: the album's source line with its link out is drawn above it`);
        // Press the second line: the song's page for every source opens
        if (kind === 'song') {
          const before = state.hits.length;
          await page.mouse.click(p.l + 20, p.b - 6);
          const went = await until(() => page.evaluate(() => location.pathname !== '/song/lib-1'));
          check(went && state.hits.slice(before).some((h) => h.v.startsWith('/item/song/') && !h.v.endsWith('/lib-1') && !h.v.includes('/lib-1/')), `${w}: pressing "every source, versions and tags" opens the song for every source (${await page.evaluate(() => location.pathname)})`);
          if (went) { await page.goBack(); await until(() => page.evaluate(() => location.pathname === '/song/lib-1' && !!document.querySelector('.wikilink') && !!document.querySelector('.item-hero .ptag-add'))); await sleep(200); }
        }
      }
      // ---- the tag + ----
      const plus = await rect(page, '.item-hero .ptag-add');
      check(!!plus, `${w}: the tag + is there`);
      if (plus) {
        const rp = await reach(page, '.item-hero .ptag-add');
        check(rp.w >= 44 && rp.h >= 44, `${w}: the tag + is a 44 × 44 px tap (${rp.w} × ${rp.h}, drawn ${plus.w} × ${plus.h})`);
        check(rp.w <= 45 && rp.h <= 45, `${w}: … and no more, the 8 px between chips mostly stays a gap (${rp.w} × ${rp.h})`);
        check(Math.abs(plus.w - 40) < 0.5 && Math.abs(plus.h - 40) < 0.5, `${w}: the tag + is still drawn 40 × 40 (${plus.w} × ${plus.h})`);
        const name = await rect(page, '.item-hero .tgroup-name');
        const [nm] = await probe(page, [[name.r - 1, name.t + name.h / 2]]);
        check(nm !== '.ptag-add', `${w}: the group's name beside it stays its own (${nm})`);
        await page.mouse.click(plus.l - 1.5, plus.t + plus.h / 2);
        const field = await until(() => page.evaluate(() => !!document.querySelector('.item-hero input[data-htag-add]')));
        check(field, `${w}: a press 1.5 px left of the + opens the tag field`);
        await page.keyboard.press('Escape'); await sleep(150);
      }
      // ---- About: More and Wikipedia ----
      await page.evaluate(() => document.querySelector('.blurb-foot').scrollIntoView({ block: 'center' })); await sleep(100);
      const wk = await rect(page, '.wikilink'); const more = await rect(page, '#iblurbmore');
      check(!!wk && !!more, `${w}: More and Wikipedia are there`);
      if (wk && more) {
        const rw = await reach(page, '.wikilink');
        check(rw.h >= 44 && rw.w >= 44, `${w}: Wikipedia is a 44 px tap (${rw.w} × ${rw.h}, drawn ${wk.w.toFixed(0)} × ${wk.h.toFixed(1)})`);
        const rm = await reach(page, '#iblurbmore');
        check(rm.h >= 44 && rm.right >= more.r + 7, `${w}: More keeps its reach (${rm.w} × ${rm.h}, right edge +${(rm.right - more.r).toFixed(1)})`);
        check(rw.left >= rm.right - 0.5, `${w}: Wikipedia's reach starts where More's stops (${rw.left.toFixed(1)} ≥ ${rm.right.toFixed(1)})`);
      }
      const wide = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      check(wide <= 0, `${w}: nothing is wider than the screen (${wide})`);
      check(!state.errors.length, `${w}: no page errors (${state.errors.join('; ')})`);
      if (kind === 'song' || width === 360) { await page.evaluate(() => scrollTo(0, 0)); await page.screenshot({ path: path.join(SHOTS, `${width}${android ? '-android' : ''}-${kind}.png`) }); }
      await context.close();
    }
  }
  // ---- desktop: none of these layers ----
  {
    const { page, context } = await open(browser, 1280, '/song/lib-1', false);
    const layers = await page.evaluate(() => ['#iglobal', '.item-hero .ptag-add', '.wikilink'].map((s) => { const e = document.querySelector(s); return e ? getComputedStyle(e, '::before').content : 'missing'; }));
    check(layers.every((c) => c === 'none' || c === 'normal'), `1280: no reach layers on desktop (${layers.join(', ')})`);
    const g = await rect(page, '#iglobal'); const p = await rect(page, '.to-global');
    const [second] = await probe(page, [[p.r - 10, g.t + g.h / 2]]);
    check(second !== '#iglobal', `1280: the explanation beside the link stays text (${second})`);
    await context.close();
  }
  await browser.close();
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
