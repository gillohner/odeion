// On a phone, My rating's stars in an item page's top card are 44 px taps: each star 44 × 44 px, each
// half star 22 × 44 px (they were 12 × 26 px). A press near a half star's top or bottom edge, or at its
// side next to the next star, sets that rating (one PUT, the value the press was on). The glyphs keep
// their place (the stars' middle stays where it was), the stars stay on one row with "My rating", and
// nothing is wider than the screen. Add notes and the notes editor's Save and Cancel are 44 px tall.
// With no rating "N of 5" takes no room (Add notes starts at the row's left); rated, it shows under the
// stars before Add notes. A song page's "Play this version" is a 44 px tap (was 36 px). Desktop
// (1280 px) keeps the 24 × 26 px stars, the reserved "N of 5" width and the 36 px version button.
// Presses use page.mouse.click at exact points (a touchscreen tap snaps onto a nearby target).
// Real candidate assets, isolated responses: node tools/ui-phone-rating-taps.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-rating-taps-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, timeout = 4000) => { const end = Date.now() + timeout; for (;;) { const v = await fn(); if (v) return v; if (Date.now() > end) return v; await sleep(50); } };

const base = { source: 'Your library', source_id: 'library', uri: null, song_uri: null, link: null, plays_from: { service: 'library', label: 'Your library' }, albums: [], versions: [], also_on: [], about: [], open_on: [], connect: [], on_pubky: null };
const songs = [1, 2, 3].map((n) => ({ id: `lib-${n}`, track_id: n, title: `Song ${n}`, artist: 'Sefa', duration_ms: 200000, no: n }));
const fixtures = {
  '/item/album/lib-7': { body: { ...base, kind: 'album', id: 'lib-7', title: 'Journey', section: 'albums', target: { album_id: 7 }, songs, artist: { id: 'lib-2', name: 'Sefa' }, year: 2024 } },
  '/item/song/lib-1': { body: { ...base, kind: 'song', id: 'lib-1', title: 'Journey Through Time', section: 'tracks', target: { track_id: 1 }, songs: [songs[0]], artist: { id: 'lib-2', name: 'Sefa' }, album: { id: 'lib-7', name: 'Journey' }, versions: [{ source: 'library', label: 'Journey Through Time · Live · 2023', page: 'lib-9' }] } },
  '/item/album/lib-7/extras': { body: {} }, '/item/song/lib-1/extras': { body: {} },
};

async function open(browser, width, kind, review, android) {
  const { page, context, state } = await fx.create(browser, width, width < 600 ? { mobile: true, fixtures } : { fixtures });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  const store = { review, puts: [] };
  await page.route(/\/v1\/reviews(\?.*)?$/, (r) => {
    const m = r.request().method();
    if (m === 'PUT') { const b = JSON.parse(r.request().postData()); store.puts.push(b); store.review = { rating: b.rating, text: b.text || '' }; return r.fulfill({ json: {} }); }
    if (m === 'DELETE') { store.review = null; return r.fulfill({ json: {} }); }
    return r.fulfill({ json: { reviews: store.review ? [store.review] : [] } });
  });
  await fx.go(page, kind === 'album' ? '/album/lib-7' : '/song/lib-1');
  await page.waitForSelector('#stars'); await sleep(300);
  return { page, context, state, store };
}
const measure = (page) => page.evaluate(() => {
  const r = (e) => { if (!e || !e.getClientRects().length) return null; const b = e.getBoundingClientRect(); return { l: b.left, t: b.top, w: b.width, h: b.height, r: b.right, b: b.bottom }; };
  const row = document.querySelector('.reviewrow');
  const glyph = document.querySelector('#stars .star .glyph');
  return {
    row: r(row), label: r(row.querySelector('.eyebrow')),
    stars: [...document.querySelectorAll('#stars .star')].map(r), halves: [...document.querySelectorAll('#stars .half')].map(r),
    val: r(document.getElementById('ratingval')), valText: document.getElementById('ratingval').textContent,
    valMin: getComputedStyle(document.getElementById('ratingval')).minWidth,
    notes: r(document.getElementById('notesbtn')), glyphMid: glyph ? (() => { const b = glyph.getBoundingClientRect(); return b.top + b.height / 2; })() : null,
    actsBottom: document.querySelector('#main .item-hero .actions').getBoundingClientRect().bottom,
    version: r(document.querySelector('#iversions [data-vplay]')),
    wide: document.documentElement.scrollWidth - document.documentElement.clientWidth,
  };
});

(async () => {
  const browser = await chromium.launch();
  for (const [width, android] of [[360, false], [412, false], [360, true]]) {
    const at = `${width}${android ? ' Android' : ''}`;
    // ---- unrated album: sizes, place, presses at the edges ----
    {
      const { page, context, state, store } = await open(browser, width, 'album', null, android);
      const m = await measure(page);
      check(m.stars.length === 5 && m.stars.every((s) => s.w >= 44 && s.h >= 44), `${at}: every star is a 44 px tap (${m.stars.map((s) => `${s.w}×${s.h}`).join(' ')})`);
      check(m.halves.every((h) => h.w >= 22 && h.h >= 44), `${at}: every half star is 22 × 44 px or more (${m.halves[0].w}×${m.halves[0].h})`);
      check(m.stars.every((s, i) => i === 0 || Math.abs(s.l - m.stars[i - 1].r) < 0.5), `${at}: the stars sit side by side, no gap between two taps`);
      check(m.stars.every((s) => Math.abs(s.t - m.stars[0].t) < 1) && Math.abs(m.label.t + m.label.h / 2 - (m.stars[0].t + m.stars[0].h / 2)) < 3, `${at}: the five stars and "My rating" are one row`);
      // The glyphs stay where the 26 px stars put them at 360 px: their middle 29 px under the actions
      // (at 412 px Add notes used to share their row, and its 40 px centred them 36 px down)
      const mid = m.glyphMid - m.actsBottom;
      check(Math.abs(mid - 29) <= 1.5, `${at}: the stars' middle is where it was at 360 px (${mid.toFixed(1)} px under the actions, was 29)`);
      check(m.valText === '' && m.val === null, `${at}: with no rating "N of 5" takes no room`);
      check(m.notes && m.notes.h >= 44 && m.notes.l - m.row.l <= 1 && m.notes.t >= m.stars[0].b - 1, `${at}: Add notes is a 44 px tap at the row's left under the stars (${m.notes && `${m.notes.w}×${m.notes.h} at +${(m.notes.l - m.row.l).toFixed(0)}`})`);
      check(m.wide <= 0 && m.stars[4].r <= width - 8, `${at}: nothing wider than the screen (page ${m.wide}, last star ends at ${m.stars[4].r.toFixed(0)})`);
      await page.screenshot({ path: path.join(SHOTS, `album-unrated-${width}${android ? '-android' : ''}.png`) });
      // A press 2 px inside the 3rd star's right half, at its top edge, its bottom edge, and next to star 4
      const s = m.halves[5];
      for (const [what, x, y] of [['top edge', s.l + s.w / 2, s.t + 2], ['bottom edge', s.l + s.w / 2, s.b - 2], ['side next to star 4', s.r - 2, s.t + s.h / 2]]) {
        const before = store.puts.length;
        const hit = await page.evaluate(([x, y]) => { const e = document.elementFromPoint(x, y); return e && e.dataset ? e.dataset.rate || null : null; }, [x, y]);
        await page.mouse.click(x, y);
        await until(() => store.puts.length > before);
        const put = store.puts[store.puts.length - 1];
        check(hit === '6' && store.puts.length === before + 1 && put && put.rating === 6, `${at}: a press at 3 stars' ${what} sets 3 stars (hit ${hit}, PUT ${put && put.rating})`);
        // Back to unrated for the next press (the same value again clears it)
        await page.waitForSelector('#ratingval:not(:empty)'); await page.mouse.click(x, y); await until(() => !store.review);
        await page.waitForFunction(() => document.getElementById('ratingval').textContent === '');
      }
      check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
      await context.close();
    }
    // ---- rated song: N of 5 under the stars before Add notes; the notes editor; the version button ----
    {
      const { page, context, state } = await open(browser, width, 'song', { rating: 7, text: '' }, android);
      const m = await measure(page);
      // Under the stars at the row's left (360 px), or beside the 5th star where it fits (412 px)
      const under = m.val && m.val.t >= m.stars[0].b - 1 && m.val.l - m.row.l <= 1 && m.notes.l >= m.val.r;
      const beside = m.val && m.val.l >= m.stars[4].r && Math.abs(m.val.t + m.val.h / 2 - (m.stars[0].t + m.stars[0].h / 2)) < 3 && m.val.r <= m.row.r + 1;
      check(/3½ of 5/.test(m.valText) && (width < 400 ? under : beside), `${at} rated: "${m.valText}" shows ${width < 400 ? 'under the stars, Add notes after it' : 'beside the stars'}`);
      check(m.stars.every((s) => Math.abs(s.t - m.stars[0].t) < 1), `${at} rated: the stars stay one row`);
      // A press 1 px above each star's bottom edge is that star's (Add notes' touch reach, 8 px past
      // its box, stops at the stars' edge), on each half
      const low = await page.evaluate(() => [...document.querySelectorAll('#stars .half')].map((h) => { const b = h.getBoundingClientRect(); const e = document.elementFromPoint(b.left + b.width / 2, b.bottom - 1); return e === h; }));
      check(low.every(Boolean), `${at} rated: a press at any star's bottom edge is that star's, not Add notes' (${low.map((x) => (x ? 1 : 0)).join('')})`);
      check(m.version && m.version.w >= 44 && m.version.h >= 44, `${at}: Play this version is a 44 px tap (${m.version && `${m.version.w}×${m.version.h}`})`);
      check(m.wide <= 0, `${at} rated: nothing wider than the screen (${m.wide})`);
      await page.screenshot({ path: path.join(SHOTS, `song-rated-${width}${android ? '-android' : ''}.png`) });
      await page.click('#notesbtn');
      const ed = await page.evaluate(() => ['notessave', 'notescancel'].map((id) => { const b = document.getElementById(id).getBoundingClientRect(); return { id, h: b.height }; }));
      check(ed.every((b) => b.h >= 44), `${at}: the notes' Save and Cancel are 44 px taps (${ed.map((b) => `${b.id} ${b.h}`).join(', ')})`);
      check(state.errors.length === 0, `${at} song: no page errors (${state.errors.join(' | ')})`);
      await context.close();
    }
  }
  // ---- desktop: as before ----
  {
    const { page, context } = await open(browser, 1280, 'song', null, false);
    const m = await measure(page);
    check(m.stars.every((s) => s.w === 24 && s.h === 26) && Math.abs(m.stars[1].l - m.stars[0].r - 2) < 0.5, `1280: the stars are 24 × 26 px, 2 px apart, as before`);
    check(m.val && m.val.w > 0 && m.valMin !== '0px', `1280: "N of 5" keeps its reserved width (${m.valMin})`);
    check(m.version && m.version.w === 36 && m.version.h === 36, `1280: Play this version stays 36 px (${m.version && m.version.w})`);
    await context.close();
  }
  await browser.close();
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
