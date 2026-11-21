// On a phone, My rating's stars in an item page's top card are 44 px taps: each star 44 × 44 px, each
// half star 22 × 44 px (they were 12 × 26 px). A press near a half star's top or bottom edge, or at its
// side next to the next star, sets that rating (one PUT, the value the press was on). The glyphs keep
// their place (the stars' middle stays where it was), the stars stay on one row with "My rating", and
// nothing is wider than the screen. Add notes and the notes editor's Save and Cancel are 44 px tall.
// With no rating "N of 5" takes no room (Add notes starts at the row's left); rated, it shows under the
// stars before Add notes. A song page's "Play this version" is a 44 px tap (was 36 px). Desktop
// (1280 px) keeps the 24 × 26 px stars, the reserved "N of 5" width and the 36 px version button.
// Add notes always starts the line under the stars (with "N of 5" before it once rated), so a first
// rating never moves it down; below 340 px the stars are 40 px wide (44 px tall) so all five stay on
// "My rating"'s line; the half stars are square (no dead corners where two meet). At 320, 360, 390,
// 412 and 430 px (360 as the Android app too) every half star is pressed unrated, rated, and with the
// notes editor open: each press is one PUT with that half's value (a press on the rating it already
// has clears it). Before each press the half is measured again and its point probed until it names
// that half (a redraw can still be on its way). Rated, opening the notes editor (which hides Add
// notes) leaves "N of 5" where it was: its line keeps Add notes' 44 px, so it doesn't jump up
// ~12 px; Cancel puts both back.
// The top card's Pubky tags answer 120 ms late, so each save's redraw shows the tags skeleton for a
// moment (the stars up to 42 px lower): a press is measured only once that redraw is done.
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
  const store = { review, puts: [], dels: 0 };
  // The top card's Pubky tags answer a little late, as on a real network: every save redraws the
  // card, and until they come its tags line is a skeleton that puts the stars lower (42 px at 360)
  await page.route(/\/v1\/discover\/uri\?/, async (r) => { await sleep(120); return r.fallback(); });
  await page.route(/\/v1\/reviews(\?.*)?$/, (r) => {
    const m = r.request().method();
    if (m === 'PUT') { const b = JSON.parse(r.request().postData()); store.puts.push(b); store.review = { rating: b.rating, text: b.text || '' }; return r.fulfill({ json: {} }); }
    if (m === 'DELETE') { store.dels++; store.review = null; return r.fulfill({ json: {} }); }
    return r.fulfill({ json: { reviews: store.review ? [store.review] : [] } });
  });
  await fx.go(page, kind === 'album' ? '/album/lib-7' : '/song/lib-1');
  await page.waitForSelector('#stars'); await drawn(page, review ? review.rating : null); await sleep(300);
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
    radius: getComputedStyle(document.querySelector('#stars .half.l')).borderTopLeftRadius,
  };
});
// Where to press half star v: its box measured now, and the point `at` picks in it (its middle by
// default). Each retry measures the half again: any late redraw of the top card (the tags skeleton,
// or anything else) may have moved the stars since the last look, so a point measured earlier would
// name nothing ("hit null"). Asked until the point names half v (bounded); returns the last point
// and what it named, so the press goes where the probe last looked.
const middle = (b) => ({ x: b.l + b.w / 2, y: b.t + b.h / 2 });
const probeRate = async (page, v, at = middle) => {
  let got = { hit: null, x: 0, y: 0 };
  await until(async () => {
    const box = await page.evaluate((v) => { const e = document.querySelector(`#stars [data-rate="${v}"]`); if (!e) return null; const r = e.getBoundingClientRect(); return { l: r.left, t: r.top, w: r.width, h: r.height, r: r.right, b: r.bottom }; }, v);
    if (!box) { got = { hit: null, x: 0, y: 0 }; return false; }
    const { x, y } = at(box);
    const hit = await page.evaluate(([x, y]) => { const e = document.elementFromPoint(x, y); return e && e.dataset ? e.dataset.rate || null : null; }, [x, y]);
    got = { hit, x, y };
    return hit === String(v);
  }, 2000);
  return got;
};
// The page drawn again with rating v (null: none), and done: the top card's tags line has its answer
// (while it is a skeleton the stars sit lower, so a point measured then misses once it lands)
const drawn = (page, v) => page.waitForFunction((v) => { const t = document.getElementById('ratingval'); const p = document.querySelector('#stars [aria-pressed="true"]'); return !document.querySelector('#main .item-hero .tag-skel') && t && (v == null ? t.textContent === '' && !p : !!p && p.dataset.rate === String(v)); }, v);
// Presses every half star once, 1 to 10. before(): put the page in the state wanted first (the
// notes editor open). cur: the rating before the first press. Each press must be one PUT with that
// half's value (or, on the rating already set, one DELETE and no PUT)
async function pressEach(page, store, at, what, cur, before) {
  const bad = [];
  for (let v = 1; v <= 10; v++) {
    if (before) await before();
    const { hit, x, y } = await probeRate(page, v);
    const puts = store.puts.length; const dels = store.dels;
    await page.mouse.click(x, y);
    const clears = cur === v;
    await until(() => (clears ? store.dels > dels : store.puts.length > puts));
    await drawn(page, clears ? null : v).catch(() => {});
    const put = store.puts[store.puts.length - 1];
    const ok = hit === String(v) && (clears ? store.dels === dels + 1 && store.puts.length === puts : store.puts.length === puts + 1 && put.rating === v && store.dels === dels);
    if (!ok) bad.push(`${v}: hit ${hit}, ${store.puts.length - puts} PUT ${put && put.rating}, ${store.dels - dels} DELETE`);
    cur = clears ? null : v;
  }
  check(!bad.length, `${at} ${what}: each of the 10 half stars, pressed, is one PUT with its value (${bad.join('; ') || 'all 10'})`);
  return cur;
}

(async () => {
  const browser = await chromium.launch();
  for (const [width, android] of [[320, false], [360, false], [390, false], [412, false], [430, false], [360, true]]) {
    const at = `${width}${android ? ' Android' : ''}`;
    const sw = width < 340 ? 40 : 44;
    // ---- unrated album: sizes, place, presses at the edges, every half ----
    {
      const { page, context, state, store } = await open(browser, width, 'album', null, android);
      const m = await measure(page);
      check(m.stars.length === 5 && m.stars.every((s) => s.w >= sw && s.h >= 44), `${at}: every star is ${sw} × 44 px (${m.stars.map((s) => `${s.w}×${s.h}`).join(' ')})`);
      check(m.halves.every((h) => h.w >= sw / 2 && h.h >= 44), `${at}: every half star is ${sw / 2} × 44 px or more (${m.halves[0].w}×${m.halves[0].h})`);
      check(m.radius === '0px', `${at}: the half stars are square, no dead corners between two (${m.radius})`);
      check(m.stars.every((s, i) => i === 0 || Math.abs(s.l - m.stars[i - 1].r) < 0.5), `${at}: the stars sit side by side, no gap between two taps`);
      check(m.stars.every((s) => Math.abs(s.t - m.stars[0].t) < 1) && Math.abs(m.label.t + m.label.h / 2 - (m.stars[0].t + m.stars[0].h / 2)) < 3, `${at}: the five stars and "My rating" are one row`);
      // The glyphs stay where the 26 px stars put them at 360 px: their middle 29 px under the actions
      const mid = m.glyphMid - m.actsBottom;
      check(Math.abs(mid - 29) <= 1.5, `${at}: the stars' middle is where it was at 360 px (${mid.toFixed(1)} px under the actions, was 29)`);
      check(m.valText === '' && m.val === null, `${at}: with no rating "N of 5" takes no room`);
      check(m.notes && m.notes.h >= 44 && m.notes.l - m.row.l <= 1 && Math.abs(m.notes.t - (m.stars[0].b + 8)) < 1, `${at}: Add notes is a 44 px tap at the row's left, on its own line 8 px under the stars (${m.notes && `${m.notes.w}×${m.notes.h} at +${(m.notes.l - m.row.l).toFixed(0)}, ${(m.notes.t - m.stars[0].b).toFixed(1)} under`})`);
      // Its box starts on the line under the stars (1 px above that line is a star, not Add notes)
      const up = await page.evaluate(([x, y]) => { const e = document.elementFromPoint(x, y); return e && (e.dataset.rate || e.id || e.tagName.toLowerCase()); }, [m.notes.l + m.notes.w / 2, m.stars[0].b - 1]);
      check(up !== 'notesbtn', `${at}: 1 px above Add notes' line is not Add notes (${up})`);
      check(m.wide <= 0 && m.stars[4].r <= width - 8, `${at}: nothing wider than the screen (page ${m.wide}, last star ends at ${m.stars[4].r.toFixed(0)})`);
      await page.screenshot({ path: path.join(SHOTS, `album-unrated-${width}${android ? '-android' : ''}.png`) });
      // Add notes opens the editor where Add notes was, 8 px under the stars
      await page.click('#notesbtn');
      const ed0 = await page.evaluate(() => document.getElementById('notesedit').getBoundingClientRect().top);
      check(Math.abs(ed0 - (m.stars[0].b + 8)) < 1, `${at}: with no rating the notes editor opens 8 px under the stars, where Add notes was (${(ed0 - m.stars[0].b).toFixed(1)})`);
      await page.click('#notescancel');
      // A press 2 px inside the 3rd star's right half, at its top edge, its bottom edge, and next to star 4
      for (const [what, pick] of [['top edge', (b) => ({ x: b.l + b.w / 2, y: b.t + 2 })], ['bottom edge', (b) => ({ x: b.l + b.w / 2, y: b.b - 2 })], ['side next to star 4', (b) => ({ x: b.r - 2, y: b.t + b.h / 2 })]]) {
        const before = store.puts.length;
        const { hit, x, y } = await probeRate(page, 6, pick);
        await page.mouse.click(x, y);
        await until(() => store.puts.length > before);
        const put = store.puts[store.puts.length - 1];
        check(hit === '6' && store.puts.length === before + 1 && put && put.rating === 6, `${at}: a press at 3 stars' ${what} sets 3 stars (hit ${hit}, PUT ${put && put.rating})`);
        await drawn(page, 6);
        if (what === 'top edge') {
          // A first rating leaves Add notes where it was ("3 of 5" goes in front of it)
          const r = await measure(page);
          check(r.notes && Math.abs(r.notes.t - m.notes.t) < 0.5 && r.val && Math.abs(r.val.t + r.val.h / 2 - (r.notes.t + r.notes.h / 2)) < 3 && r.notes.l >= r.val.r, `${at}: a first rating doesn't move Add notes down ("${r.valText}" before it, top ${r.notes && r.notes.t.toFixed(1)}, was ${m.notes.t.toFixed(1)})`);
        }
        // Back to unrated for the next press (the same value again clears it)
        const again = await probeRate(page, 6, pick); await page.mouse.click(again.x, again.y); await until(() => !store.review);
        await drawn(page, null);
      }
      // Every half, unrated: set it, then clear it again
      const bad = [];
      for (let v = 1; v <= 10; v++) {
        for (const clears of [false, true]) {
          const { hit, x, y } = await probeRate(page, v);
          const puts = store.puts.length; const dels = store.dels;
          await page.mouse.click(x, y);
          await until(() => (clears ? store.dels > dels : store.puts.length > puts));
          await drawn(page, clears ? null : v).catch(() => {});
          const put = store.puts[store.puts.length - 1];
          const ok = hit === String(v) && (clears ? store.dels === dels + 1 && store.puts.length === puts : store.puts.length === puts + 1 && put.rating === v);
          if (!ok) bad.push(`${v}${clears ? ' again' : ''}: hit ${hit}, ${store.puts.length - puts} PUT ${put && put.rating}, ${store.dels - dels} DELETE`);
        }
      }
      check(!bad.length, `${at} unrated: each of the 10 half stars, pressed, is one PUT with its value; again, one DELETE (${bad.join('; ') || 'all 10'})`);
      check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
      await context.close();
    }
    // ---- rated song: N of 5 under the stars before Add notes; every half; the notes editor; the version button ----
    {
      const { page, context, state, store } = await open(browser, width, 'song', { rating: 7, text: '' }, android);
      const m = await measure(page);
      const under = m.val && m.val.t >= m.stars[0].b - 1 && m.val.l - m.row.l <= 1 && m.notes.l >= m.val.r && Math.abs(m.val.t + m.val.h / 2 - (m.notes.t + m.notes.h / 2)) < 3;
      check(/3½ of 5/.test(m.valText) && under, `${at} rated: "${m.valText}" shows under the stars, Add notes after it`);
      check(m.notes && Math.abs(m.notes.t - (m.stars[0].b + 8)) < 1, `${at} rated: Add notes' line is 8 px under the stars, as unrated (${m.notes && (m.notes.t - m.stars[0].b).toFixed(1)})`);
      check(m.stars.every((s) => Math.abs(s.t - m.stars[0].t) < 1) && Math.abs(m.label.t + m.label.h / 2 - (m.stars[0].t + m.stars[0].h / 2)) < 3, `${at} rated: the stars stay one row with "My rating"`);
      // A press 1 px above each star's bottom edge is that star's (Add notes' touch reach, 8 px past
      // its box, stops at the stars' edge), on each half
      const low = await page.evaluate(() => [...document.querySelectorAll('#stars .half')].map((h) => { const b = h.getBoundingClientRect(); const e = document.elementFromPoint(b.left + b.width / 2, b.bottom - 1); return e === h; }));
      check(low.every(Boolean), `${at} rated: a press at any star's bottom edge is that star's, not Add notes' (${low.map((x) => (x ? 1 : 0)).join('')})`);
      check(m.version && m.version.w >= 44 && m.version.h >= 44, `${at}: Play this version is a 44 px tap (${m.version && `${m.version.w}×${m.version.h}`})`);
      check(m.wide <= 0, `${at} rated: nothing wider than the screen (${m.wide})`);
      await page.screenshot({ path: path.join(SHOTS, `song-rated-${width}${android ? '-android' : ''}.png`) });
      await page.click('#notesbtn');
      // Opening the editor hides Add notes: "N of 5" stays where it was (its line keeps its 44 px)
      const mo = await measure(page);
      check(mo.val && Math.abs(mo.val.t + mo.val.h / 2 - (m.val.t + m.val.h / 2)) < 0.5, `${at} rated: opening the notes editor leaves "${m.valText}" where it was (moved ${mo.val && (mo.val.t + mo.val.h / 2 - (m.val.t + m.val.h / 2)).toFixed(1)} px)`);
      await page.screenshot({ path: path.join(SHOTS, `song-rated-notes-open-${width}${android ? '-android' : ''}.png`) });
      const ed = await page.evaluate(() => ['notessave', 'notescancel'].map((id) => { const b = document.getElementById(id).getBoundingClientRect(); return { id, h: b.height }; }));
      check(ed.every((b) => b.h >= 44), `${at}: the notes' Save and Cancel are 44 px taps (${ed.map((b) => `${b.id} ${b.h}`).join(', ')})`);
      await page.click('#notescancel');
      // Every half, rated (3½ stars first; never the value it already has, so each is a PUT)
      const cur = await pressEach(page, store, at, 'rated', 7);
      check(cur === 10, `${at} rated: it ends on 5 stars (${cur})`);
      check(state.errors.length === 0, `${at} song: no page errors (${state.errors.join(' | ')})`);
      await context.close();
    }
    // ---- notes saved, the editor open before each press ----
    {
      const { page, context, state, store } = await open(browser, width, 'song', { rating: 4, text: 'Great drop at 2:10' }, android);
      const openEditor = async () => { await page.waitForSelector('#notesbtn:not([hidden])'); await page.click('#notesbtn'); await page.waitForSelector('#notesedit:not([hidden])'); };
      const m0 = await measure(page);
      await openEditor();
      const m1 = await measure(page);
      check(m0.val && m1.val && Math.abs(m1.val.t + m1.val.h / 2 - (m0.val.t + m0.val.h / 2)) < 0.5, `${at} notes: opening the notes editor leaves "${m0.valText}" where it was (moved ${m1.val && (m1.val.t + m1.val.h / 2 - (m0.val.t + m0.val.h / 2)).toFixed(1)} px)`);
      await page.click('#notescancel');
      const m2 = await measure(page);
      check(m2.val && m2.notes && Math.abs(m2.val.t - m0.val.t) < 0.5 && Math.abs(m2.notes.t - m0.notes.t) < 0.5, `${at} notes: Cancel puts "${m0.valText}" and Edit notes back where they were`);
      await openEditor();
      await page.screenshot({ path: path.join(SHOTS, `song-notes-open-${width}${android ? '-android' : ''}.png`) });
      const first = store.puts.length;
      await pressEach(page, store, at, 'notes editor open', 4, async () => { if (await page.evaluate(() => document.getElementById('notesedit').hidden)) await openEditor(); });
      check(store.puts.slice(first).every((p) => p.text === 'Great drop at 2:10'), `${at} notes open: every press keeps the saved notes`);
      check(state.errors.length === 0, `${at} notes: no page errors (${state.errors.join(' | ')})`);
      await context.close();
    }
  }
  // ---- desktop: as before ----
  {
    const { page, context } = await open(browser, 1280, 'song', null, false);
    const m = await measure(page);
    check(m.stars.every((s) => s.w === 24 && s.h === 26) && Math.abs(m.stars[1].l - m.stars[0].r - 2) < 0.5, `1280: the stars are 24 × 26 px, 2 px apart, as before`);
    check(m.val && m.val.w > 0 && m.valMin !== '0px', `1280: "N of 5" keeps its reserved width (${m.valMin})`);
    check(m.radius !== '0px', `1280: the half stars keep their rounded ends (${m.radius})`);
    check(m.notes && Math.abs(m.notes.t + m.notes.h / 2 - (m.stars[0].t + m.stars[0].h / 2)) < 3, `1280: Add notes stays beside the stars`);
    check(m.version && m.version.w === 36 && m.version.h === 36, `1280: Play this version stays 36 px (${m.version && m.version.w})`);
    await context.close();
  }
  await browser.close();
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
