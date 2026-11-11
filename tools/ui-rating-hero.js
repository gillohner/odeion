// My rating sits in the item page's top card (TASK-rating-in-hero-1112; owner: "move rating to top
// card"). Real candidate assets on the fixture origin (tools/ui-fixture.js), the node's /v1/reviews
// answered by a small in-test store; an album, a song and an artist at 360 and 412 (phone) and 1385 px:
//   * the stars, "N of 5" and the notes are inside .item-hero, directly under the action row, above
//     the songs; there is no #ireview section and only one rating row;
//   * unrated: empty stars under "My rating"; one click on a half star sends one PUT and shows it;
//   * long notes are clamped to three lines with More/Less; short notes have no More;
//   * the notes editor opens in the hero and saves; Cancel brings the notes back;
//   * nothing is wider than the screen.
//   node tools/ui-rating-hero.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'rating-hero-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (page, fn, arg = null, timeout = 8000) => { const end = Date.now() + timeout; for (;;) { const v = await page.evaluate(fn, arg).catch(() => null); if (v) return v; if (Date.now() > end) return null; await sleep(50); } };

const LONG = 'Bought on vinyl in Austin after the show at the Continental Club. Side two is the one: the slow blues before the title song, then the long guitar outro. '.repeat(4).trim();
const songs = Array.from({ length: 8 }, (_, i) => ({ no: i + 1, title: `Song ${i + 1}`, artist: 'Calvin Russell', artist_id: 'lib-31', duration_ms: 200000 + i * 5000, uri: null, track_id: 100 + i, id: `lib-${100 + i}`, page: `lib-${100 + i}` }));
const base = { uri: null, source: 'library', source_id: '31', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library', image: null, tags: [], about: [], about_text: null, image_credit: null };
const pages = {
  album: { ...base, kind: 'album', id: 'lib-7', target: { album_id: 7 }, title: 'Crossroad', artist: { name: 'Calvin Russell', id: 'lib-31' }, album: null, year: 1997, songs },
  song: { ...base, kind: 'song', id: 'lib-100', target: { track_id: 100 }, title: 'Song 1', artist: { name: 'Calvin Russell', id: 'lib-31' }, album: { title: 'Crossroad', id: 'lib-7' }, year: 1997, songs: [], albums: [{ id: 'lib-7', title: 'Crossroad', artist: 'Calvin Russell', year: 1997, image: null }] },
  artist: { ...base, kind: 'artist', id: 'lib-31', target: { artist_id: 31 }, title: 'Calvin Russell', artist: null, album: null, year: null, songs, albums: [{ id: 'lib-7', title: 'Crossroad', artist: 'Calvin Russell', year: 1997, image: null }] },
};
const fixtures = {};
for (const p of Object.values(pages)) { fixtures[`/item/${p.kind}/${p.id}`] = { body: p }; fixtures[`/item/${p.kind}/${p.id}/extras`] = { body: { collections: [], related: [], concerts: [], similar: [] } }; }
fixtures['/library/artists/31/similar'] = { body: { artists: [] } };
for (const s of songs) fixtures[`/library/tracks/${s.track_id}`] = { body: { ...fx.track, id: s.track_id, title: s.title } };

// One page with its own review store: GET answers what is saved, PUT and DELETE change it
const open = async (browser, w, key, review) => {
  const p = pages[key];
  const { page, context, state } = await fx.create(browser, w, { fixtures, touch: w < 1025 });
  const store = { review, puts: [], deletes: 0 };
  await page.route(/\/v1\/reviews(\?.*)?$/, (r) => {
    const m = r.request().method();
    if (m === 'PUT') { const b = JSON.parse(r.request().postData()); store.puts.push(b); store.review = { rating: b.rating, text: b.text || '' }; return r.fulfill({ json: {} }); }
    if (m === 'DELETE') { store.deletes++; store.review = null; return r.fulfill({ json: {} }); }
    return r.fulfill({ json: { reviews: store.review ? [store.review] : [] } });
  });
  await page.setViewportSize({ width: w, height: 900 });
  await fx.go(page, `/${p.kind}/${p.id}`); await page.waitForSelector('#main .item-hero'); await page.waitForSelector('#stars', { timeout: 8000 }).catch(() => {}); await sleep(400);
  return { page, context, state, store };
};
const layout = (page) => page.evaluate(() => {
  const q = (s) => document.querySelector(s); const box = (el) => (el ? el.getBoundingClientRect() : null);
  const hero = q('#main .item-hero'); const row = q('.reviewrow'); const acts = q('#main .item-hero .actions'); const music = q('#isongs') || q('#ialbums');
  const rb = box(row); const ab = box(acts); const mb = box(music); const hb = box(hero);
  return {
    inHero: !!(hero && q('#stars') && hero.contains(q('#stars')) && hero.contains(q('#ratingval')) && hero.contains(q('#notesbtn')) && hero.contains(q('#notesedit'))),
    section: !!q('#ireview'), rows: document.querySelectorAll('.reviewrow').length, stars: document.querySelectorAll('#stars').length,
    afterActions: !!(row && acts && acts.nextElementSibling === row), under: rb && ab ? Math.round(rb.top - ab.bottom) : null,
    aboveMusic: rb && mb ? rb.bottom <= mb.top + 1 : null, insideHero: rb && hb ? rb.bottom <= hb.bottom + 1 && rb.left >= hb.left - 1 && rb.right <= hb.right + 1 : null,
    wide: document.documentElement.scrollWidth - innerWidth, rowRight: rb ? Math.round(rb.right) : null, vw: innerWidth,
    label: row ? row.querySelector('.eyebrow').textContent : null, val: q('#ratingval') ? q('#ratingval').textContent : null,
    fills: [...document.querySelectorAll('#stars .star')].map((x) => x.className.replace('star ', '')).join(','),
    half: Math.min(...[...document.querySelectorAll('#stars .half')].map((b) => b.getBoundingClientRect().width)),
  };
});

(async () => {
  const browser = await chromium.launch();
  for (const w of [360, 412, 1385]) {
    for (const key of ['album', 'song', 'artist']) {
      const at = `${key} at ${w}`;
      // ---- unrated: where the row is, then one click rates ----
      {
        const { page, context, state, store } = await open(browser, w, key, null);
        const r = await layout(page);
        check(r.inHero && !r.section && r.rows === 1 && r.stars === 1, `${at}: the rating is in the top card, once, and #ireview is gone (in hero ${r.inHero}, #ireview ${r.section}, rows ${r.rows})`);
        check(r.afterActions && r.under != null && r.under >= 0 && r.under <= 24 && r.insideHero, `${at}: directly under the action row (${r.under} px below it)`);
        if (r.aboveMusic != null) check(r.aboveMusic, `${at}: above the songs and albums`);
        check(r.wide <= 0 && r.rowRight <= r.vw, `${at}: nothing wider than the screen (page ${r.wide}, row ends at ${r.rowRight} of ${r.vw})`);
        check(r.label === 'My rating' && r.val === '' && r.fills === 'f0,f0,f0,f0,f0', `${at}: unrated shows empty stars under "My rating" (${r.fills}, "${r.val}")`);
        check(r.half >= 12, `${at}: half-star targets keep their size (${r.half} px)`);
        if (key === 'album') await page.screenshot({ path: path.join(SHOTS, `album-unrated-${w}.png`) });
        if (r.stars) {
          await page.click('[data-rate="7"]');
          const shown = await until(page, () => (document.querySelector('#ratingval') || {}).textContent === '3½ of 5');
          await sleep(300);
          const r2 = await layout(page);
          check(!!shown && store.puts.length === 1 && store.puts[0].rating === 7 && r2.fills === 'f2,f2,f2,f1,f0' && r2.inHero, `${at}: one click saves 3½ once and the hero shows it (${store.puts.length} PUT, ${r2.fills})`);
          // The same value again clears it
          await page.click('[data-rate="7"]');
          // (the pointer still rests on the star, whose hover preview fills it: the saved value is the text)
          const cleared = await until(page, () => (document.querySelector('#ratingval') || {}).textContent === '');
          check(!!cleared && store.deletes === 1, `${at}: the same star again clears the rating (${store.deletes} DELETE)`);
        }
        check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
        await context.close();
      }
      // ---- rated with long notes: clamped with More, the editor opens in the hero ----
      {
        const { page, context, state, store } = await open(browser, w, key, { rating: 9, text: LONG });
        const n = () => page.evaluate(() => {
          const q = (s) => document.querySelector(s); const hero = q('#main .item-hero'); const notes = q('#notes'); const more = q('#notesmore'); const ed = q('#notesedit');
          const lh = notes ? parseFloat(getComputedStyle(notes).lineHeight) || 24 : 0;
          return { inHero: !!(notes && hero.contains(notes) && hero.contains(ed)), lines: notes && !notes.hidden ? Math.round(notes.clientHeight / lh) : 0, cut: notes ? notes.scrollHeight > notes.clientHeight + 2 : null,
            more: more ? { hidden: more.hidden, text: more.textContent, open: more.getAttribute('aria-expanded'), inHero: hero.contains(more) } : null, notesHidden: notes ? notes.hidden || !notes.getClientRects().length : null,
            editor: ed ? !ed.hidden : null, text: q('#notestext') ? q('#notestext').value : null, btn: q('#notesbtn').textContent, btnHidden: q('#notesbtn').hidden, heroH: Math.round(hero.getBoundingClientRect().height), wide: document.documentElement.scrollWidth - innerWidth, val: q('#ratingval').textContent };
        });
        const a = await n();
        check(a.inHero && a.val === '4½ of 5' && a.btn === 'Edit notes', `${at}: saved notes and the rating are in the top card ("${a.val}", ${a.btn})`);
        check(a.lines <= 3 && a.cut && a.more && !a.more.hidden && a.more.text === 'More' && a.more.open === 'false' && a.more.inHero, `${at}: long notes are clamped to three lines with More (${a.lines} lines)`);
        check(a.wide <= 0, `${at}: long notes don't widen the page (${a.wide})`);
        await page.screenshot({ path: path.join(SHOTS, `${key}-notes-${w}.png`) });
        await page.click('#notesmore'); const b = await n();
        check(b.lines > 3 && !b.cut && b.more.text === 'Less' && b.more.open === 'true' && b.heroH > a.heroH, `${at}: More shows all the notes (${b.lines} lines) and says Less`);
        await page.click('#notesmore'); const c = await n();
        check(c.lines <= 3 && c.more.text === 'More' && c.heroH === a.heroH, `${at}: Less clamps them again`);
        await page.click('#notesbtn'); const d = await n();
        check(d.editor && d.notesHidden && d.more.hidden && d.btnHidden && d.text === LONG && d.wide <= 0, `${at}: the notes editor opens in the top card with the saved text`);
        if (key === 'album') await page.screenshot({ path: path.join(SHOTS, `album-editor-${w}.png`) });
        await page.click('#notescancel'); const e = await n();
        check(!e.editor && !e.notesHidden && !e.more.hidden && !e.btnHidden && e.lines <= 3, `${at}: Cancel brings the clamped notes and More back`);
        await page.click('#notesbtn'); await page.fill('#notestext', 'Short note'); await page.click('#notessave');
        const saved = await until(page, () => (document.querySelector('#notes') || {}).textContent === 'Short note');
        await sleep(300); const f = await n();
        check(!!saved && store.puts.length === 1 && store.puts[0].text === 'Short note' && store.puts[0].rating === 9 && f.inHero, `${at}: Save keeps the rating and saves the notes once (${store.puts.length} PUT)`);
        check(f.more && f.more.hidden && !f.cut, `${at}: short notes have no More`);
        check(!state.errors.length, `${at}: no page errors with notes${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
        await context.close();
      }
    }
  }
  // A listener's page has no rating row (as before)
  {
    const l = await fx.create(browser, 1385, { fixtures, role: 'listener' });
    await fx.go(l.page, '/album/lib-7'); await l.page.waitForSelector('#main .item-hero'); await sleep(600);
    const none = await l.page.evaluate(() => !document.querySelector('.reviewrow') && !document.querySelector('#ireview'));
    check(none && !l.state.hits.some((h) => h.v === '/reviews'), 'a listener gets no rating row and no /reviews request');
    await l.context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
