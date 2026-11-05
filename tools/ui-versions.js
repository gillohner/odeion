// A song kept in several copies is one row with a quiet "2 versions" that opens a chooser
// (TASK-hide-missing-tracks-1104 points 6-7; owner: "maybe have some indication or version chooser,
// very subtle"). Real candidate assets on the fixture origin (tools/ui-fixture.js), an album page whose
// first song has two copies (#268's `versions` on the song row) and whose second has one:
//   * one marker, on the right row, in the secondary text colour, inside its row with nothing
//     overlapping, at 390, 820 and 1440 px (touch at 390 and 820);
//   * the chooser lists each copy (where, format, size), the default first and said so;
//   * picking the other copy plays that copy (its track id) from this row on; the row's own Play still
//     plays the default; a song with one copy has no marker.
//   node tools/ui-versions.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'versions-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// The track that started playing: the first play the page reports to the node after n0
const played = (st, n0) => { const h = st.hits.slice(n0).find((x) => x.m === 'POST' && x.v === '/library/history'); try { return h ? JSON.parse(h.body).track_id : null; } catch { return null; } };

const versions = [
  { track_id: 201, source: 'homeserver', container: 'flac', format: '16/44.1 FLAC', rate_hz: 44100, bits: 16, size_bytes: 28000000, lossless: true, best: false },
  { track_id: 101, source: 'library', container: 'flac', format: '24/96 FLAC', rate_hz: 96000, bits: 24, size_bytes: 41200000, lossless: true, best: true },
];
const song = (id, no, title, extra = {}) => ({ no, title, artist: 'Sefa', artist_id: 'lib-11', duration_ms: 200000, uri: null, track_id: id, id: `lib-${id}`, page: `lib-${id}`, ...extra });
const albumPage = { kind: 'album', id: 'lib-7', uri: null, target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [song(101, 1, 'Journey Through Time', { versions }), song(102, 2, 'Second Song')], total_ms: 400000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' };
const trackFor = (id) => ({ ...fx.track, id, title: id === 102 ? 'Second Song' : 'Journey Through Time' });
const fixtures = { '/item/album/lib-7': { body: albumPage }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } };
for (const id of [101, 102, 201]) fixtures[`/library/tracks/${id}`] = { body: trackFor(id) };

(async () => {
  const browser = await chromium.launch();
  for (const [w, h, touch] of [[390, 844, true], [820, 1180, true], [1440, 900, false]]) {
    const run = await fx.create(browser, w, { fixtures, touch });
    const { page, state: st } = run;
    await page.setViewportSize({ width: w, height: h });
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs .srow'); await sleep(300);
    const at = `${w}px`;
    const r = await page.evaluate(() => {
      const marks = [...document.querySelectorAll('#isongs .vmark')];
      const m = marks[0]; if (!m) return { n: 0 };
      const row = m.closest('.srow'); const rr = row.getBoundingClientRect(); const mr = m.getBoundingClientRect();
      const sub = m.closest('.sub'); const parts = [...row.children].filter((c) => c.getBoundingClientRect().width > 0);
      let overlap = 0; for (let i = 0; i < parts.length; i++) for (let j = i + 1; j < parts.length; j++) { const a = parts[i].getBoundingClientRect(); const b = parts[j].getBoundingClientRect(); if (Math.min(a.right, b.right) - Math.max(a.left, b.left) > 1 && Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > 1) overlap++; }
      const muted = getComputedStyle(document.documentElement).getPropertyValue('--muted').trim();
      const probe = document.createElement('span'); probe.style.color = muted; document.body.append(probe); const mutedRgb = getComputedStyle(probe).color; probe.remove();
      return { n: marks.length, row: row.dataset.srow, text: m.textContent.trim(), inside: mr.left >= rr.left - 1 && mr.right <= rr.right + 1 && mr.top >= rr.top - 1 && mr.bottom <= rr.bottom + 1, overlap, color: getComputedStyle(m).color, mutedRgb, size: parseFloat(getComputedStyle(m).fontSize), subSize: parseFloat(getComputedStyle(sub).fontSize), seen: getComputedStyle(m).display !== 'none' && mr.width > 0 };
    });
    check(r.n === 1 && r.row === '0', `${at}: one marker, on the song with two copies (${r.n})`);
    check(r.text === '2 versions' && r.seen, `${at}: it says "2 versions" ("${r.text}")`);
    check(r.color === r.mutedRgb && r.size <= r.subSize, `${at}: quiet: the secondary text colour and size (${r.color} vs ${r.mutedRgb})`);
    check(r.inside && r.overlap === 0, `${at}: inside its row, nothing in the row overlaps (${r.overlap})`);
    await page.screenshot({ path: path.join(SHOTS, `album-${w}.png`) });
    // The chooser
    await page.click('#isongs .vmark'); await sleep(300);
    const items = await page.evaluate(() => [...document.querySelectorAll('.popover.menu:not([hidden]) .pop-item')].map((b) => b.textContent.trim()));
    check(items.length === 2 && /^Your library · 24\/96 FLAC · 41\.2 MB · plays by default$/.test(items[0]) && /^Your homeserver · 16\/44\.1 FLAC · 28\.0 MB$/.test(items[1]), `${at}: the chooser lists both copies, the default first: ${items.join(' | ')}`);
    await page.screenshot({ path: path.join(SHOTS, `chooser-${w}.png`) });
    const n0 = st.hits.length;
    await page.locator('.popover.menu .pop-item', { hasText: 'Your homeserver' }).click(); await sleep(800);
    const t = played(st, n0);
    check(t === 201, `${at}: picking the homeserver copy plays that copy (track ${t})`);
    await run.context.close();
  }
  // The row's own Play still plays the default copy
  {
    const run = await fx.create(browser, 1440, { fixtures }); const { page, state } = run;
    await fx.go(page, '/album/lib-7'); await page.waitForSelector('#isongs .srow'); await page.hover('#isongs .srow');
    const n0 = state.hits.length; await page.click('#isongs [data-splay="0"]'); await sleep(800);
    const t = played(state, n0);
    check(t === 101, `the row's Play plays the default copy (track ${t})`);
    check(state.errors.length === 0, `no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
    await run.context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
