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
//     where More's reach (8 px) stops: More keeps its own reach. On a right-to-left page (dir=rtl)
//     the same, mirrored: 3 px on More's side (its right), 8 px on the other;
//   * with own and public tags over several rows (two groups on the album, so own chips have a move
//     handle too), every chip's link, ×, move handle and faces, the group names and "+N more" keep
//     every point of their own box next to the enlarged +, folded and shown whole.
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
// Own and public tags over several rows (tagged pages only)
const P = Array.from({ length: 6 }, (_, i) => `pppp${String.fromCharCode(97 + i)}`.padEnd(52, 'y'));
const tagged = {
  ...fixtures,
  '/tags': { body: { uri: SONG, tags: [{ label: 'frenchcore', synced: true }, { label: 'melodic', synced: true }, { label: 'not-yet-pushed', synced: false }] } },
  '/discover/uri': { body: { uri: SONG, indexed: true, taggers: 9, tags: [['frenchcore', 7], ['hardcore', 5], ['melodic', 3], ['uptempo', 2], ['classical', 2], ['dutch', 1], ['festival anthem', 1]], faces: { frenchcore: P.slice(0, 5), hardcore: P.slice(1, 6), melodic: P.slice(2, 5), uptempo: P.slice(0, 2), classical: [P[3]], dutch: [P[4]], 'festival anthem': [P[5]] } } },
  '/profiles': { body: [] },
};
const PAGES = [['album', '/album/lib-7'], ['song', '/song/lib-1'], ['artist', '/artist/lib-2']];

async function open(browser, width, route, android, fx0 = fixtures) {
  const { page, context, state } = await fx.create(browser, width, width < 600 ? { mobile: true, fixtures: { ...fx0 } } : { fixtures: { ...fx0 } });
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
          // The address changes before the page's request reaches the fixture's router: wait for both
          const asked = () => state.hits.slice(before).some((h) => h.v.startsWith('/item/song/') && !h.v.endsWith('/lib-1') && !h.v.includes('/lib-1/'));
          const went = await until(async () => (await page.evaluate(() => location.pathname !== '/song/lib-1')) && asked());
          check(went && asked(), `${w}: pressing "every source, versions and tags" opens the song for every source (${await page.evaluate(() => location.pathname)})`);
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
        // Right to left: More is on Wikipedia's right; the 3 px side faces it, the 8 px side the other way
        if (kind === 'artist') {
          await page.evaluate(() => { document.documentElement.dir = 'rtl'; document.querySelector('.blurb-foot').scrollIntoView({ block: 'center' }); }); await sleep(100);
          const wr = await rect(page, '.wikilink'); const mr = await rect(page, '#iblurbmore');
          const rwr = await reach(page, '.wikilink'); const rmr = await reach(page, '#iblurbmore');
          check(wr.r <= mr.l, `${w} rtl: More is on Wikipedia's right (${wr.r.toFixed(1)} ≤ ${mr.l.toFixed(1)})`);
          check(rwr.h >= 44 && rwr.w >= 44, `${w} rtl: Wikipedia is a 44 px tap (${rwr.w} × ${rwr.h})`);
          check(rmr.h >= 44 && rmr.left <= mr.l - 7, `${w} rtl: More keeps its reach (left edge -${(mr.l - rmr.left).toFixed(1)})`);
          check(rwr.right <= rmr.left + 0.5 && Math.abs(rwr.right - wr.r - 3) <= 1 && Math.abs(wr.l - rwr.left - 8) <= 1, `${w} rtl: Wikipedia reaches 3 px towards More, 8 px away from it, and stops where More's reach starts (${(rwr.right - wr.r).toFixed(1)} / ${(wr.l - rwr.left).toFixed(1)}, ${rwr.right.toFixed(1)} ≤ ${rmr.left.toFixed(1)})`);
          await page.evaluate(() => { document.documentElement.dir = ''; });
        }
      }
      const wide = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      check(wide <= 0, `${w}: nothing is wider than the screen (${wide})`);
      check(!state.errors.length, `${w}: no page errors (${state.errors.join('; ')})`);
      if (kind === 'song' || width === 360) { await page.evaluate(() => scrollTo(0, 0)); await page.screenshot({ path: path.join(SHOTS, `${width}${android ? '-android' : ''}-${kind}.png`) }); }
      await context.close();
    }
  }
  // ---- tags over several rows: every chip part next to the enlarged + keeps its own box ----
  for (const [width, android] of [[320, false], [360, false], [412, false], [360, true]]) {
    const at = `${width}${android ? ' Android' : ''}`;
    for (const [kind, route] of PAGES.slice(0, 2)) {
      const { page, context, state } = await open(browser, width, route, android, tagged);
      const w = `${at} ${kind} tagged`;
      await until(() => page.evaluate(() => document.querySelectorAll('.item-hero .tagfaces').length > 0 && !document.querySelector('.item-hero .tag-skel')));
      // Each part of the hero's tags, scrolled into view and probed 1 px inside its edge middles, at its
      // middle, and near its corners (30 % in, inside a rounded or round shape)
      const parts = () => page.evaluate(async () => {
        const els = [...document.querySelectorAll('#htags :is(.chiplink, .chipx, .tagfaces, .tagitem > button.ptag, .tgroup-name, [data-htag-more])')].filter((e) => e.getClientRects().length && e.getBoundingClientRect().width);
        const bad = [];
        for (const e of els) {
          e.scrollIntoView({ block: 'center', behavior: 'instant' }); await new Promise(requestAnimationFrame);
          const b = e.getBoundingClientRect(); const ix = Math.min(b.width, b.height) * 0.3;
          const pts = [[b.left + 1, b.top + b.height / 2], [b.right - 1, b.top + b.height / 2], [b.left + b.width / 2, b.top + 1], [b.left + b.width / 2, b.bottom - 1], [b.left + b.width / 2, b.top + b.height / 2],
            [b.left + ix, b.top + ix], [b.right - ix, b.top + ix], [b.left + ix, b.bottom - ix], [b.right - ix, b.bottom - ix]];
          for (const [x, y] of pts) { const t = document.elementFromPoint(x, y); if (!t || !(t === e || e.contains(t))) bad.push(`${e.className.split(' ')[0]}${e.dataset.label ? ` ${e.dataset.label}` : ''} at ${x.toFixed(0)},${y.toFixed(0)} → ${t ? (t.closest('.ptag-add') ? 'the +' : t.className || t.tagName) : 'nothing'}`); }
        }
        const shown = [...document.querySelectorAll('#htags .tgroup > *')].filter((e) => !e.hidden && e.getClientRects().length && !e.classList.contains('tgroup-name'));
        document.querySelectorAll('#htags .ptag-add').forEach((e, i) => { e.dataset.probeplus = String(i); });
        return { n: els.length, bad, rows: new Set(shown.map((e) => Math.round(e.getBoundingClientRect().top + scrollY))).size, moves: document.querySelectorAll('#htags [data-htag-move]').length, plus: document.querySelectorAll('#htags .ptag-add').length };
      });
      for (const shown of ['folded', 'shown whole']) {
        if (shown === 'shown whole') { for (let i = 0; i < 4; i++) { const more = page.locator('.item-hero [data-htag-more]:visible'); if (!(await more.count())) break; await more.first().click(); await sleep(150); } }
        const r = await parts();
        check(r.rows >= (shown === 'folded' ? 2 : 3) && r.plus >= 1 && (kind !== 'album' || r.moves > 0), `${w} ${shown}: tags over ${r.rows} rows, ${r.plus} +, ${r.moves} move handles, ${r.n} parts`);
        check(!r.bad.length, `${w} ${shown}: every chip link, ×, move handle, faces and name keeps its whole box (${r.bad.slice(0, 4).join('; ') || 'all'})`);
        for (let i = 0; i < r.plus; i++) {
          const sel = `#htags [data-probeplus="${i}"]`;
          await page.evaluate((sel) => document.querySelector(sel).scrollIntoView({ block: 'center', behavior: 'instant' }), sel); await sleep(50);
          const rp = await reach(page, sel);
          check(rp.w >= 44 && rp.h >= 44 && rp.w <= 45 && rp.h <= 45, `${w} ${shown}: group ${i + 1}'s + is a 44 × 44 px tap (${rp.w} × ${rp.h})`);
        }
      }
      const wide = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      check(wide <= 0, `${w}: nothing is wider than the screen (${wide})`);
      check(!state.errors.length, `${w}: no page errors (${state.errors.join('; ')})`);
      if (width === 360 && !android) { await page.evaluate(() => document.querySelector('#htags').scrollIntoView({ block: 'center' })); await page.screenshot({ path: path.join(SHOTS, `${width}-${kind}-tagged.png`) }); }
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
