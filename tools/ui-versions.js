// A song kept in several copies is one row with a quiet "2 versions" that opens a chooser
// (TASK-hide-missing-tracks-1104 points 6-7; owner: "maybe have some indication or version chooser,
// very subtle"). Real candidate assets on the fixture origin (tools/ui-fixture.js), an album page whose
// second song has two copies (#268's `versions` on the song row, a long artist name) and whose first and
// third have one:
//   * one marker, on the right row, in the secondary text colour and size, inside its row with nothing
//     in the row overlapping, and wholly shown even beside a long artist (the artist shrinks, not the
//     marker), at 390 and 820 px (touch: a target at least 28 px high) and 1440 px;
//   * the chooser lists each copy (where, format, size), the default first and said so, for a listener
//     too;
//   * picking the other copy plays that copy and the songs after it (the queue is that copy, then the
//     third song); the row's own Play afterwards still plays the default.
//   node tools/ui-versions.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'versions-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// The tracks that started playing, in order: what the page reports to the node after n0
const played = (st, n0) => st.hits.slice(n0).filter((x) => x.m === 'POST' && x.v === '/library/history').map((h) => { try { return JSON.parse(h.body).track_id; } catch { return null; } });

const LONG = 'Sefa and the Unusually Long Named Orchestra of Somewhere Far Away';
const versions = [
  { track_id: 202, source: 'homeserver', container: 'flac', format: '16/44.1 FLAC', rate_hz: 44100, bits: 16, size_bytes: 28000000, lossless: true, best: false },
  { track_id: 102, source: 'library', container: 'flac', format: '24/96 FLAC', rate_hz: 96000, bits: 24, size_bytes: 41200000, lossless: true, best: true },
];
const TITLES = { 101: 'Opening Song', 102: 'Second Song', 202: 'Second Song (homeserver copy)', 103: 'Third Song' };
const song = (id, no, extra = {}) => ({ no, title: TITLES[id], artist: 'Sefa', artist_id: 'lib-11', duration_ms: 200000, uri: null, track_id: id, id: `lib-${id}`, page: `lib-${id}`, ...extra });
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [song(101, 1), song(102, 2, { artist: LONG, versions }), song(103, 3)], total_ms: 600000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
const fixtures = { '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } };
for (const id of [101, 102, 202, 103]) fixtures[`/library/tracks/${id}`] = { body: { ...fx.track, id, title: TITLES[id], ...(id === 102 || id === 202 ? { artists: [{ id: 99, name: LONG }] } : {}) } };
const open = async (browser, w, h, opts = {}) => {
  const run = await fx.create(browser, w, { fixtures, ...opts });
  await run.page.setViewportSize({ width: w, height: h });
  await fx.go(run.page, '/album/lib-7'); await run.page.waitForSelector('#isongs .srow'); await sleep(300);
  return run;
};
const menuItems = (page) => page.evaluate(() => [...document.querySelectorAll('.popover.menu:not([hidden]) .pop-item')].map((b) => b.textContent.trim()));
const DEFAULT = /^Your library · 24\/96 FLAC · 41\.2 MB · plays by default$/; const OTHER = /^Your homeserver · 16\/44\.1 FLAC · 28\.0 MB$/;

(async () => {
  const browser = await chromium.launch();
  for (const [w, h, touch] of [[390, 844, true], [820, 1180, true], [1440, 900, false]]) {
    const { page, state: st, context } = await open(browser, w, h, { touch });
    const at = `${w}px`;
    const r = await page.evaluate(() => {
      const marks = [...document.querySelectorAll('#isongs .vmark')];
      const m = marks[0]; if (!m) return { n: 0 };
      // In view, clear of the phone's mini player (a taller top card pushes the songs down)
      m.closest('.srow').scrollIntoView({ block: 'center', behavior: 'instant' });
      // Its words as drawn (on a touch screen its target reaches further, invisibly)
      const row = m.closest('.srow'); const rr = row.getBoundingClientRect(); const rg = document.createRange(); rg.selectNodeContents(m); const mr = rg.getBoundingClientRect(); const box = m.getBoundingClientRect();
      const parts = [...row.children, ...row.querySelectorAll('.grow > *, .sub > *')].filter((c) => c.getBoundingClientRect().width > 0 && !c.contains(m) && !m.contains(c));
      let overlap = 0; for (const c of parts) { const b = c.getBoundingClientRect(); if (Math.min(mr.right, b.right) - Math.max(mr.left, b.left) > 1 && Math.min(mr.bottom, b.bottom) - Math.max(mr.top, b.top) > 1) overlap++; }
      const probe = document.createElement('span'); probe.style.color = getComputedStyle(document.documentElement).getPropertyValue('--muted').trim(); document.body.append(probe); const mutedRgb = getComputedStyle(probe).color; probe.remove();
      const hit = document.elementFromPoint(mr.right - 2, mr.top + mr.height / 2);
      const sub = row.querySelector('.sub');
      const rows = [...document.querySelectorAll('#isongs .srow')].map((x) => Math.round(x.getBoundingClientRect().height));
      return { rows, n: marks.length, row: row.dataset.srow, text: m.textContent.trim(), inside: mr.left >= rr.left - 1 && mr.right <= rr.right + 1 && mr.top >= rr.top - 1 && mr.bottom <= rr.bottom + 1, overlap, whole: !!hit && (hit === m || m.contains(hit)), color: getComputedStyle(m).color, mutedRgb, size: parseFloat(getComputedStyle(m).fontSize), subSize: parseFloat(getComputedStyle(sub).fontSize), height: box.height };
    });
    check(r.n === 1 && r.row === '1', `${at}: one marker, on the song with two copies (${r.n} on row ${r.row})`);
    check(r.text === '2 versions' && r.whole, `${at}: it says "2 versions", wholly shown beside a long artist name`);
    check(r.color === r.mutedRgb && r.size <= r.subSize, `${at}: quiet: the secondary text colour and size (${r.color} vs ${r.mutedRgb})`);
    check(r.inside && r.overlap === 0, `${at}: inside its row, nothing in the row overlaps it (${r.overlap})`);
    if (touch) check(r.height >= 28, `${at}: on a touch screen its target is at least 28 px high (${Math.round(r.height)})`);
    check(r.rows && Math.max(...r.rows) - Math.min(...r.rows) <= 2, `${at}: the row with the marker is as tall as the others (${(r.rows || []).join(', ')})`);
    await page.screenshot({ path: path.join(SHOTS, `album-${w}.png`) });
    // The chooser
    if (r.n) { await page.click('#isongs .vmark'); await sleep(300); }
    const items = await menuItems(page);
    check(items.length === 2 && DEFAULT.test(items[0]) && OTHER.test(items[1]), `${at}: the chooser lists both copies, the default first: ${items.join(' | ')}`);
    await page.screenshot({ path: path.join(SHOTS, `chooser-${w}.png`) });
    const n0 = st.hits.length;
    if (items.length) { await page.locator('.popover.menu .pop-item', { hasText: 'Your homeserver' }).click(); await sleep(800); }
    check(played(st, n0)[0] === 202, `${at}: picking the homeserver copy plays that copy (track ${played(st, n0)[0]})`);
    await context.close();
  }
  // From this row on, and the default stays: the queue after a pick, then the row's own Play
  {
    const { page, state, context } = await open(browser, 1440, 900);
    await page.click('#isongs .vmark'); await page.locator('.popover.menu .pop-item', { hasText: 'Your homeserver' }).click(); await sleep(800);
    await page.click('.sidebar .nav-item[data-view="queue"]'); await sleep(800);
    const q = await page.evaluate(() => document.querySelector('#main').innerText);
    const i1 = q.indexOf(TITLES[202]); const i2 = q.indexOf(TITLES[103]);
    check(i1 >= 0 && i2 > i1 && !q.includes(TITLES[101]), `the queue is the picked copy, then the song after it (${i1}, ${i2})`);
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs .srow'); await page.hover('#isongs [data-srow="1"]');
    const n0 = state.hits.length; await page.click('#isongs [data-splay="1"]'); await sleep(800);
    check(played(state, n0)[0] === 102, `then the row's own Play plays the default copy (track ${played(state, n0)[0]})`);
    check(state.errors.length === 0, `no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
    await context.close();
  }
  // A listener chooses among the same copies (the homeserver's included)
  {
    const { page, context } = await open(browser, 1440, 900, { role: 'listener' });
    const has = await page.$('#isongs .vmark');
    if (has) { await page.click('#isongs .vmark'); await sleep(300); }
    const items = has ? await menuItems(page) : [];
    check(items.length === 2 && DEFAULT.test(items[0]) && OTHER.test(items[1]), `a listener's chooser lists both copies too (${items.length})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
