// Artist release sections in a real browser, against synthetic catalogues (no service, no account):
// the node serves the app; /v1/item/artist/* is answered here, the way the node answers it.
//   * a TIDAL-like catalogue of 75 mixed releases (the owner's /artist/tidal-4805908 report: one
//     "Albums 60" grid of everything): Albums, then EPs & singles, then Other releases, each newest
//     first with its own count, the first 12 and View all;
//   * View all: a paged grid (48 a page, Previous/Next), its own path, Back to the artist; the
//     sections page on their own (paging one never moves another); a deep link to page 2;
//   * a 130-album catalogue (three pages), releases with no type ("Releases"), compilations;
//   * blank, loading (the page asks again), an error with Retry, a partial list, a list cut at the
//     bound, and a list still on its way after every re-ask (Retry);
//   * desktop (1280), tablet (768) and phone (390): no sideways scroll, screenshots.
//   npm install playwright@1.49.1 && npx playwright install chromium   (anywhere; run from there)
//   node tools/ui-release-sections.js <api token> [http://127.0.0.1:7788] [screenshot dir] [--quick]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2]; const BASE = process.argv[3] || 'http://127.0.0.1:7788'; const OUT = process.argv[4] && !process.argv[4].startsWith('--') ? process.argv[4] : null;
const QUICK = process.argv.includes('--quick');
const problems = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// Polls in the page (the node's Content-Security-Policy forbids Playwright's waitForFunction)
const waitFor = async (page, fn, arg = null, timeout = 15000) => { const end = Date.now() + timeout; for (;;) { const v = await page.evaluate(fn, arg).catch(() => null); if (v) return v; if (Date.now() > end) return null; await sleep(100); } };
const IMG = '/icon.svg';

// ---- synthetic catalogues (what the node answers: newest first, each with the service's type) ----
const rel = (i, type, date, extra = {}) => ({ id: `tidal-${9000000 + i}`, title: extra.title || `Release ${i}`, year: Number(date.slice(0, 4)), image: IMG, page: `https://tidal.com/album/${9000000 + i}`, ...(type ? { release_type: type } : {}), released: date, ...(extra.compilation ? { compilation: true } : {}), ...(extra.secondary ? { secondary_types: extra.secondary } : {}) });
const newest = (list) => [...list].sort((a, b) => (b.released || '').localeCompare(a.released || ''));
// 75 releases, TIDAL's types: 15 ALBUM, 23 EP, 30 SINGLE, 7 with none (titles that would fool a guess)
const mixed75 = newest(Array.from({ length: 75 }, (_, i) => {
  const k = (i * 37) % 75; const date = `${1990 + Math.floor(k / 12)}-${String(k % 12 + 1).padStart(2, '0')}-${String(1 + k % 28).padStart(2, '0')}`;
  const type = i % 5 === 0 ? 'album' : i % 5 === 1 ? 'ep' : i % 5 < 4 ? 'single' : i % 10 === 4 ? 'ep' : null;
  const title = { 0: 'Night Drive', 5: 'Night Drive', 10: 'Night Drive (Deluxe Edition)', 2: 'Night Drive (Remix)', 3: 'Album', 1: 'Greatest Hits' }[i];
  return rel(i, type, date, { title });
}));
const many = (n, type, from, extra = {}) => Array.from({ length: n }, (_, i) => rel(from + i, type, `${2025 - Math.floor(i / 4)}-${String(12 - (i % 4) * 3).padStart(2, '0')}-01`, extra));
const big = newest([...many(130, 'album', 1000), ...many(3, 'single', 2000)]);
// Provider text is text: a secondary type carrying markup must show as written, never as HTML
const EVIL = '<b data-evil="1">x</b>';
// Compilations, untyped releases, a type the page doesn't know (Other) and a live album (MusicBrainz's secondary type, still an album)
const comps = newest([...many(4, 'album', 3000), ...many(3, 'album', 3100, { compilation: true, secondary: ['Compilation'] }), ...many(2, null, 3200), ...many(1, 'broadcast', 3300), ...many(1, 'album', 3400, { secondary: ['Live'] }), ...many(1, 'album', 3500, { secondary: [EVIL] })]);
const untyped = newest(many(5, null, 4000));
const CATALOGUES = {
  'tidal-4805908': { albums: mixed75, catalogue: { source: 'tidal', complete: true } },
  'tidal-4805920': { albums: big, catalogue: { source: 'tidal', complete: true } },
  'spotify-4Z8W4fKeB5YxbusRsdQVPb': { albums: comps, catalogue: { source: 'spotify', complete: true, total: 9 } },
  'deezer-555': { albums: untyped, catalogue: { source: 'deezer', complete: true, total: 5 } },
  'deezer-556': { albums: [], catalogue: { source: 'deezer', complete: true, total: 0 } },
  'tidal-4805921': { albums: mixed75.slice(0, 40), catalogue: { source: 'tidal', complete: false, error: 'tidal: /artists/4805921/relationships/albums answered 500 Internal Server Error' } },
  'tidal-4805922': { albums: newest(many(500, 'single', 5000)), catalogue: { source: 'tidal', complete: false, truncated: true } },
  // Incomplete with neither an error nor the bound (a service that didn't say how much it listed)
  'tidal-4805943': { albums: mixed75.slice(0, 10), catalogue: { source: 'tidal', complete: false } },
  // Nothing arrived: View all says why and shows no "Page 1 of 1 · 0 releases"
  'tidal-4805944': { albums: [], catalogue: { source: 'tidal', complete: false, error: 'tidal: /artists/4805944/relationships/albums answered 503 Service Unavailable' } },
};
// Sections by the rule the page follows: a compilation or no type → Other; album → Albums; EP or single → EPs & singles
const sectionOf = (a) => (a.compilation ? 'other' : a.release_type === 'album' ? 'albums' : a.release_type === 'ep' || a.release_type === 'single' ? 'eps' : 'other');
const expected = (albums) => ({ albums: albums.filter((a) => sectionOf(a) === 'albums'), eps: albums.filter((a) => sectionOf(a) === 'eps'), other: albums.filter((a) => sectionOf(a) === 'other') });

const pageJson = (id, extra) => ({ kind: 'artist', id, uri: null, target: { uri: `https://example.invalid/${id}` }, title: `Synthetic ${id}`, artist: null, album: null, year: null, image: null, source: 'TIDAL', source_id: 'tidal', section: 'artists', plays_from: { service: 'tidal', kind: 'artist', id, label: 'TIDAL', mode: 'full' }, connect: [], reading: false, pending: false, songs: [], total_ms: 0, albums: [], versions: [], also_on: [], about: [], open_on: [], ...extra });
// Each browser context answers on its own (a fixture that fails once fails once per context)
const makeAnswer = () => {
  const asked = {};
  return (id) => {
    const n = (asked[id] = (asked[id] || 0) + 1);
    if (id === 'tidal-4805923') return n <= 2 ? pageJson(id, { pending: true }) : pageJson(id, CATALOGUES['tidal-4805908']); // loading, then there
    if (id === 'tidal-4805924') return n <= 1 ? pageJson(id, { albums: [], catalogue: { source: 'tidal', complete: false, error: 'tidal: /artists/4805924/relationships/albums answered 502 Bad Gateway' } }) : pageJson(id, CATALOGUES['tidal-4805908']);
    if (id === 'tidal-4805925') return pageJson(id, { pending: true }); // never arrives
    if (/^tidal-48059(40|41|42)$/.test(id)) return pageJson(id, CATALOGUES['tidal-4805908']);
    return pageJson(id, CATALOGUES[id] || { albums: [] });
  };
};
// Answers held back until the check lets them go (`ctx.holds`): the first request for these ids
const HOLD = new Set(['tidal-4805940', 'tidal-4805941', 'tidal-4805942']);
const routeItems = async (ctx) => {
  const answer = makeAnswer(); ctx.reqLog = []; ctx.holds = new Map(); const held = new Set();
  await ctx.route(/\/v1\/item\/artist\/[^/?]+(\/extras)?(\?.*)?$/, (r) => {
    const u = new URL(r.request().url());
    if (u.pathname.endsWith('/extras')) return r.fulfill({ json: {} });
    const id = decodeURIComponent(u.pathname.split('/')[4]);
    ctx.reqLog.push(u.pathname + u.search);
    if (HOLD.has(id) && !held.has(id)) { held.add(id); ctx.holds.set(id, (status, json) => r.fulfill({ status, json })); return undefined; }
    return r.fulfill({ json: answer(id) });
  });
};

(async () => {
  const browser = await chromium.launch();
  const newCtx = async (viewport) => {
    const ctx = await browser.newContext({ viewport, colorScheme: 'dark' });
    await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); HTMLMediaElement.prototype.play = async function () {}; }, TOKEN);
    await routeItems(ctx);
    return ctx;
  };
  const ctx = await newCtx({ width: 1280, height: 800 });
  const p = await ctx.newPage();
  p.on('pageerror', (e) => problems.push('page error: ' + e.message));
  p.on('console', (m) => { if (m.type() === 'error' && /Content Security Policy/i.test(m.text())) problems.push('CSP: ' + m.text()); });
  const settled = () => waitFor(p, () => !!document.querySelector('#main #ialbums [data-rel], #main #ialbums .relempty, #main #ialbums #irelstatus'), null, 15000);
  const sections = () => p.$$eval('#ialbums [data-rel]', (els) => els.map((s) => ({ key: s.dataset.rel, title: s.querySelector('h3').textContent.trim(), count: (s.querySelector('.relcount') || {}).textContent, tiles: [...s.querySelectorAll('.grid .tile .t')].map((t) => t.textContent), subs: [...s.querySelectorAll('.grid .tile .a')].map((t) => t.textContent), all: !!s.querySelector('[data-relall]') })));
  // The app scrolls inside #main, not the document: a screenshot grows the window to the page's
  // height first (so the whole page is in it), then puts it back
  const shot = async (page, name) => {
    if (!OUT) return;
    const vp = page.viewportSize();
    const h = await page.evaluate(() => { const m = document.querySelector('#main'); return window.innerHeight + Math.max(0, m.scrollHeight - m.clientHeight); });
    await page.setViewportSize({ width: vp.width, height: Math.min(Math.max(h, vp.height), 6000) }); await sleep(400);
    await page.screenshot({ path: `${OUT}/${name}.png` });
    await page.setViewportSize(vp); await sleep(200);
  };

  // ---- the router: every View all path round-trips (window.pmRoutes, the item pages' route table) ----
  await p.goto(BASE + '/artist/tidal-4805908'); await settled();
  const routes = await p.evaluate(() => window.pmRoutes.check());
  const relRoutes = routes.filter((r) => /\/releases\//.test(r));
  check(relRoutes.length === 3 && relRoutes.every((r) => r.startsWith('ok ')), `View all paths round-trip (${relRoutes.join(' | ') || 'none in the table'})`);
  check(routes.every((r) => r.startsWith('ok ')), `and every other route still does (${routes.filter((r) => !r.startsWith('ok ')).join(' | ')})`);
  // ---- the owner's report: 75 mixed releases ----
  await p.goto(BASE + '/artist/tidal-4805908'); await settled();
  let sec = await sections();
  const want = expected(mixed75);
  check(JSON.stringify(sec.map((s) => s.key)) === '["albums","eps","other"]', `Albums, then EPs & singles, then Other releases (${sec.map((s) => s.key)})`);
  check(sec[0] && sec[0].title === 'Albums' && sec[1] && sec[1].title === 'EPs & singles' && sec[2] && sec[2].title === 'Other releases', `the headings (${sec.map((s) => s.title).join(' | ')})`);
  check(sec[0] && sec[0].count === String(want.albums.length) && sec[1].count === String(want.eps.length) && sec[2].count === String(want.other.length), `each section counts its own releases: ${sec.map((s) => s.count)} (want ${want.albums.length}, ${want.eps.length}, ${want.other.length})`);
  check(sec.length === 3 && sec.every((s) => JSON.stringify(s.tiles) === JSON.stringify(want[s.key].slice(0, 12).map((a) => a.title))), 'each shows its newest 12 first');
  check(sec[0] && sec[0].all && sec[1].all && !sec[2].all, 'View all where a section has more than 12');
  check(sec[0] && sec[0].tiles.filter((t) => t === 'Night Drive').length === want.albums.slice(0, 12).filter((a) => a.title === 'Night Drive').length, 'same-titled releases each keep their tile');
  check(sec[1] && sec[1].subs.every((t, k) => t === `${want.eps[k].year} · ${want.eps[k].release_type === 'ep' ? 'EP' : 'Single'}`), `an EP or single says which, after its year (${sec[1] && sec[1].subs.slice(0, 3).join(' | ')})`);
  check(sec[0] && sec[0].subs.every((t, k) => t === String(want.albums[k].year)), 'an album tile shows its year');
  const titles = await p.$$eval('#ialbums [data-rel] .grid .tile .t', (els) => els.map((e) => e.textContent));
  check(!titles.includes('Album') || want.albums.slice(0, 12).some((a) => a.title === 'Album'), 'a single titled "Album" is not an album');
  check(sec.length === 3 && !(await p.$('#ialbums #irelstatus')), 'a complete list says nothing more');
  await shot(p, 'desktop-artist-mixed75');
  // A tile opens its album page
  const firstTile = await p.$('#ialbums [data-rel="eps"] .grid .tile');
  if (firstTile) { await firstTile.click(); await waitFor(p, () => location.pathname.startsWith('/album/')); check(new URL(p.url()).pathname === `/album/${want.eps[0].id}`, `a tile opens its release (${new URL(p.url()).pathname})`); await p.goBack(); await settled(); }

  // ---- View all, paging, Back ----
  await p.click('#ialbums [data-relall="eps"]');
  await waitFor(p, () => location.pathname.endsWith('/releases/eps-singles') && document.querySelector('#main .grid .tile'));
  const viewAll = async () => p.evaluate(() => ({ path: location.pathname + location.search, h1: (document.querySelector('#main h1') || {}).textContent, eyebrow: (document.querySelector('#main .page-head .eyebrow') || {}).textContent, tiles: [...document.querySelectorAll('#main .grid .tile .t')].map((t) => t.textContent), info: (document.querySelector('#main .pager .pageinfo') || {}).textContent, prev: (document.querySelector('#main [data-relpage="prev"]') || {}).disabled, next: (document.querySelector('#main [data-relpage="next"]') || {}).disabled, back: (document.querySelector('#main .back') || {}).textContent }));
  let v = await viewAll();
  check(v.path === '/artist/tidal-4805908/releases/eps-singles', `View all has its own path (${v.path})`);
  check(v.h1 === 'EPs & singles' && /Synthetic tidal-4805908/.test(v.eyebrow || ''), `titled by its section and artist (${v.h1} / ${v.eyebrow})`);
  check(JSON.stringify(v.tiles) === JSON.stringify(want.eps.slice(0, 48).map((a) => a.title)), `page 1: the newest 48 (${v.tiles.length})`);
  check(v.info === `Page 1 of ${Math.ceil(want.eps.length / 48)} · ${want.eps.length} releases` && v.prev === true && v.next === false, `the pager (${v.info}; prev ${v.prev}, next ${v.next})`);
  await shot(p, 'desktop-viewall-eps-page1');
  await p.click('#main [data-relpage="next"]');
  await waitFor(p, () => /page=2/.test(location.search) && document.querySelector('#main .grid .tile'));
  v = await viewAll();
  check(v.path === '/artist/tidal-4805908/releases/eps-singles?page=2' && JSON.stringify(v.tiles) === JSON.stringify(want.eps.slice(48).map((a) => a.title)), `page 2: the rest (${v.path}, ${v.tiles.length})`);
  check(v.prev === false && v.next === true, 'Previous on, Next off at the end');
  await shot(p, 'desktop-viewall-eps-page2');
  await p.click('#main .back');
  await waitFor(p, () => location.pathname === '/artist/tidal-4805908' && document.querySelector('#ialbums [data-rel]'));
  check(new URL(p.url()).pathname === '/artist/tidal-4805908', `Back from page 2 goes straight to the artist (${new URL(p.url()).pathname})`);
  sec = await sections();
  check(sec.length === 3 && sec[1].tiles.length === 12, 'the artist page is as it was');
  // Independent: Albums' View all starts on its own page 1
  await p.click('#ialbums [data-relall="albums"]');
  await waitFor(p, () => location.pathname.endsWith('/releases/albums') && document.querySelector('#main .grid .tile'));
  v = await viewAll();
  check(v.path === '/artist/tidal-4805908/releases/albums' && v.info === `Page 1 of 1 · ${want.albums.length} releases` && v.tiles.length === want.albums.length, `Albums' View all is on its own first page (${v.path}, ${v.info})`);
  check(v.prev === true && v.next === true, 'one page: both pager buttons off');
  await p.goBack(); await settled();
  // The Other section opens too (discoverable)
  check(!!(await p.$('#ialbums [data-rel="other"] .grid .tile')), 'Other releases are listed on the page');
  // A deep link to page 2 (a reload, a shared link): no history, Back goes to the artist
  const p2 = await ctx.newPage();
  await p2.goto(BASE + '/artist/tidal-4805908/releases/eps-singles?page=2');
  await waitFor(p2, () => document.querySelector('#main .grid .tile'));
  const d = await p2.evaluate(() => ({ h1: (document.querySelector('#main h1') || {}).textContent, info: (document.querySelector('#main .pager .pageinfo') || {}).textContent, n: document.querySelectorAll('#main .grid .tile').length, back: (document.querySelector('#main .back') || {}).textContent }));
  check(d.h1 === 'EPs & singles' && /^Page 2 of 2/.test(d.info || '') && d.n === want.eps.length - 48, `a deep link opens page 2 (${d.h1}, ${d.info}, ${d.n})`);
  check(/Synthetic tidal-4805908/.test(d.back || ''), `its Back names the artist (${(d.back || '').trim()})`);
  await p2.click('#main .back');
  await waitFor(p2, () => location.pathname === '/artist/tidal-4805908' && document.querySelector('#ialbums [data-rel]'));
  check(new URL(p2.url()).pathname === '/artist/tidal-4805908', `…and goes to the artist (${new URL(p2.url()).pathname})`);
  // A section that doesn't exist, or a page past the end
  await p2.goto(BASE + '/artist/tidal-4805908/releases/eps-singles?page=9');
  await waitFor(p2, () => document.querySelector('#main .pager .pageinfo'));
  check(/^Page 2 of 2/.test(await p2.textContent('#main .pager .pageinfo').catch(() => '')), 'a page past the end shows the last page');
  await p2.close();

  // ---- 130 albums: three pages ----
  // Each page change waits for the pager to name the new page: the view draws a loading
  // placeholder first and the pager only once the artist answers.
  await p.goto(BASE + '/artist/tidal-4805920'); await settled();
  sec = await sections();
  check(JSON.stringify(sec.map((s) => [s.key, s.count])) === '[["albums","130"],["eps","3"]]', `130 albums and 3 singles, no empty section (${JSON.stringify(sec.map((s) => [s.key, s.count]))})`);
  await p.click('#ialbums [data-relall="albums"]');
  await waitFor(p, () => document.querySelector('#main .pager .pageinfo'));
  const seenTitles = [];
  for (let k = 1; k <= 3; k++) {
    v = await viewAll(); seenTitles.push(...v.tiles);
    check(v.info === `Page ${k} of 3 · 130 releases`, `page ${k} of 3 (${v.info}, ${v.tiles.length} tiles)`);
    if (k < 3) { await p.click('#main [data-relpage="next"]'); await waitFor(p, (n) => new URLSearchParams(location.search).get('page') === String(n) && (document.querySelector('#main .pager .pageinfo') || {}).textContent?.startsWith(`Page ${n} of 3`) && document.querySelector('#main .grid .tile'), k + 1); }
  }
  check(seenTitles.length === 130 && new Set(seenTitles).size === 130, `every album exactly once across the pages (${seenTitles.length})`);
  check(JSON.stringify(seenTitles) === JSON.stringify(expected(big).albums.map((a) => a.title)), 'newest first across the pages');
  await p.click('#main [data-relpage="prev"]'); await waitFor(p, () => new URLSearchParams(location.search).get('page') === '2' && (document.querySelector('#main .pager .pageinfo') || {}).textContent?.startsWith('Page 2 of 3'));
  check((await viewAll()).info === 'Page 2 of 3 · 130 releases', 'Previous goes back a page');
  await p.click('#main .back'); await settled();
  check(new URL(p.url()).pathname === '/artist/tidal-4805920', 'Back after paging both ways: the artist');

  // ---- compilations, untyped, blank ----
  await p.goto(BASE + '/artist/spotify-4Z8W4fKeB5YxbusRsdQVPb'); await settled();
  sec = await sections();
  const other = sec.find((s) => s.key === 'other');
  check(other && other.count === '6' && other.subs.filter((t) => /Compilation$/.test(t)).length === 3, `compilations, untyped releases and a type the page doesn't know are Other, compilations say so (${other && other.subs.join(' | ')})`);
  const albumsSec = sec.find((s) => s.key === 'albums');
  check(albumsSec && albumsSec.count === '6' && albumsSec.subs.filter((t) => / · Live$/.test(t)).length === 1, `a live album is an album that says Live (${albumsSec && albumsSec.subs.join(' | ')})`);
  const evil = await p.evaluate(() => ({ node: !!document.querySelector('[data-evil]'), literal: [...document.querySelectorAll('#ialbums .tile .a')].some((e) => e.textContent.includes('<b data-evil="1">x</b>')) }));
  check(!evil.node && evil.literal, `provider text with markup is shown as text, no element injected (node ${evil.node}, literal ${evil.literal})`);
  await p.goto(BASE + '/artist/deezer-555'); await settled();
  sec = await sections();
  check(sec.length === 1 && sec[0].title === 'Releases' && sec[0].count === '5', `nothing typed: one "Releases" section, no made-up Albums (${JSON.stringify(sec.map((s) => [s.title, s.count]))})`);
  await p.goto(BASE + '/artist/deezer-556'); await settled();
  check(/No releases found/.test(await p.textContent('#ialbums').catch(() => '')) && !(await p.$('#ialbums [data-rel]')), 'blank: says no releases were found');
  await shot(p, 'desktop-blank');

  // ---- loading, error and Retry, partial, truncated ----
  await p.goto(BASE + '/artist/tidal-4805923');
  const loading = await waitFor(p, () => { const st = document.querySelector('#ialbums #irelstatus'); return st && document.querySelector('#ialbums .skeleton') ? st.textContent : null; }, null, 8000);
  check(!!loading && /Getting the releases from TIDAL/.test(loading), `loading: placeholders and what it waits for (${loading})`);
  await shot(p, 'desktop-loading');
  const arrived = await waitFor(p, () => document.querySelectorAll('#ialbums [data-rel]').length === 3, null, 15000);
  check(!!arrived && !(await p.$('#ialbums .skeleton')), 'the page asks again and the sections arrive');
  await p.goto(BASE + '/artist/tidal-4805924'); await settled();
  const err = await p.textContent('#ialbums #irelstatus').catch(() => '');
  check(/TIDAL didn’t list the releases/.test(err) && /502 Bad Gateway/.test(err) && !(await p.$('#ialbums [data-rel]')), `error: says so, with TIDAL's answer (${err.trim().slice(0, 120)})`);
  check(!/No releases found/.test(await p.textContent('#ialbums')), 'an error is not "no releases"');
  await shot(p, 'desktop-error');
  await p.click('#ialbums #irelretry');
  check(!!(await waitFor(p, () => document.querySelectorAll('#ialbums [data-rel]').length === 3, null, 8000)), 'Retry asks again and shows the sections');
  const asks = ctx.reqLog.filter((x) => x.startsWith('/v1/item/artist/tidal-4805924'));
  check(asks.length >= 2 && /[?&]refresh=1\b/.test(asks[1]) && !/refresh/.test(asks[0]), `Retry asks the node to fetch again, not for its cache (${asks.join(' | ')})`);
  check(!asks.slice(2).some((x) => /refresh/.test(x)), 'only the Retry itself skips the cache');
  await p.goto(BASE + '/artist/tidal-4805921'); await settled();
  const part = await p.textContent('#ialbums #irelstatus').catch(() => '');
  sec = await sections();
  check(/Only 40 releases arrived/.test(part) && !!(await p.$('#ialbums #irelretry')), `partial: says only part arrived, with Retry (${part.trim().slice(0, 100)})`);
  check(sec.length > 0 && sec.every((s) => /\+$/.test(s.count)), `partial counts are "at least" (${sec.map((s) => s.count)})`);
  await shot(p, 'desktop-partial');
  await p.goto(BASE + '/artist/tidal-4805922'); await settled();
  const cut = await p.textContent('#ialbums #irelstatus').catch(() => '');
  sec = await sections();
  check(/more than 500/.test(cut) && sec[0] && sec[0].count === '500+', `cut at the bound: says TIDAL lists more (${cut.trim().slice(0, 100)}; ${sec.map((s) => s.count)})`);
  await p.goto(BASE + '/artist/tidal-4805943'); await settled();
  const vague = await p.textContent('#ialbums #irelstatus').catch(() => '');
  check(/may not be every release/.test(vague), `an incomplete list without a reason still says it may not be everything (${vague.trim().slice(0, 100)})`);

  await p.goto(BASE + '/artist/tidal-4805944/releases/albums');
  const ve = await waitFor(p, () => { const st = document.querySelector('#main #irelstatus'); return st ? { text: st.textContent, pager: !!document.querySelector('#main .pager'), retry: !!document.querySelector('#main #irelretry'), back: (document.querySelector('#main .back') || {}).textContent } : null; });
  check(!!ve && /didn’t list the releases/.test(ve.text) && /503/.test(ve.text) && ve.retry && !ve.pager, `View all with nothing arrived: the error and Retry, no empty pager (${ve && JSON.stringify(ve).slice(0, 160)})`);

  // ---- answers that arrive late never land on a page they don't belong to ----
  const lateCheck = async (what, id, gone) => {
    const leak = await p.evaluate((k) => ({ text: document.querySelector('#main').textContent.includes('late failure'), back: !!document.querySelector(`[data-back="item:artist:${k}"]`) }), id);
    check(!leak.text && !leak.back && gone(), `${what} (text ${leak.text}, its Back ${leak.back}, at ${new URL(p.url()).pathname})`);
  };
  // View all, left for another page before its answer (a failure) arrives
  await p.goto(BASE + '/artist/tidal-4805940/releases/albums');
  for (let k = 0; k < 50 && !ctx.holds.has('tidal-4805940'); k++) await sleep(100);
  await p.click('.nav-item[data-view="queue"]'); await waitFor(p, () => location.pathname === '/queue'); await sleep(1200);
  ctx.holds.get('tidal-4805940')(502, { error: 'late failure 4805940' }); await sleep(800);
  await lateCheck('View all left before a late failure: the failure is not shown on the next page', 'tidal-4805940', () => new URL(p.url()).pathname === '/queue');
  // The artist page, left the same way
  await p.goto(BASE + '/artist/tidal-4805941');
  for (let k = 0; k < 50 && !ctx.holds.has('tidal-4805941'); k++) await sleep(100);
  await p.click('.nav-item[data-view="queue"]'); await waitFor(p, () => location.pathname === '/queue'); await sleep(1200);
  ctx.holds.get('tidal-4805941')(502, { error: 'late failure 4805941' }); await sleep(800);
  await lateCheck('an artist page left before a late failure: not shown on the next page', 'tidal-4805941', () => new URL(p.url()).pathname === '/queue');
  // The same View all opened again: the newer answer stays, the older failure arriving later is dropped
  await p.goto(BASE + '/artist/tidal-4805942/releases/eps-singles');
  for (let k = 0; k < 50 && !ctx.holds.has('tidal-4805942'); k++) await sleep(100);
  await p.click('.nav-item[data-view="queue"]'); await waitFor(p, () => location.pathname === '/queue'); await sleep(1200);
  await p.goBack(); await waitFor(p, () => document.querySelectorAll('#main .grid .tile').length === 48, null, 8000);
  ctx.holds.get('tidal-4805942')(502, { error: 'late failure 4805942' }); await sleep(800);
  const stillGrid = await p.evaluate(() => document.querySelectorAll('#main .grid .tile').length);
  check(stillGrid === 48 && !(await p.evaluate(() => document.querySelector('#main').textContent.includes('late failure'))), `the same View all reopened keeps its newer answer (${stillGrid} tiles)`);

  if (!QUICK) {
    // Still on its way after every re-ask: say so, and offer Retry instead of spinning forever
    await p.goto(BASE + '/artist/tidal-4805925');
    const stuck = await waitFor(p, () => document.querySelector('#ialbums #irelretry') && document.querySelector('#ialbums #irelstatus').textContent, null, 60000);
    check(!!stuck && /still/i.test(stuck), `a list that never arrives ends with Retry (${(stuck || '').trim().slice(0, 100)})`);
  }

  // ---- tablet and phone: same sections, no sideways scroll ----
  for (const [name, viewport] of [['tablet', { width: 768, height: 1024 }], ['phone', { width: 390, height: 844 }]]) {
    const c = await newCtx(viewport); const q = await c.newPage();
    q.on('pageerror', (e) => problems.push(`${name} page error: ` + e.message));
    await q.goto(BASE + '/artist/tidal-4805908');
    await waitFor(q, () => document.querySelectorAll('#ialbums [data-rel]').length === 3);
    const lay = await q.evaluate(() => { const m = document.querySelector('#main'); const tiles = [...document.querySelectorAll('#ialbums .grid .tile')]; return { over: m.scrollWidth - m.clientWidth, docOver: document.documentElement.scrollWidth - window.innerWidth, inside: tiles.every((t) => { const r = t.getBoundingClientRect(); return r.left >= 0 && r.right <= window.innerWidth + 0.5; }), heads: [...document.querySelectorAll('#ialbums [data-rel] h3')].map((h) => h.textContent), allBtn: [...document.querySelectorAll('#ialbums [data-relall]')].map((b) => { const r = b.getBoundingClientRect(); return Math.min(r.width, r.height); }) }; });
    check(lay.over <= 0 && lay.docOver <= 0 && lay.inside, `${name}: no sideways scroll, tiles inside the screen (main ${lay.over}, doc ${lay.docOver})`);
    check(lay.heads.join('|') === 'Albums|EPs & singles|Other releases', `${name}: the three sections (${lay.heads.join(' | ')})`);
    check(lay.allBtn.every((x) => x >= 24), `${name}: View all is big enough to touch (${lay.allBtn.map((x) => Math.round(x))})`);
    await shot(q, `${name}-artist-mixed75`);
    await q.click('#ialbums [data-relall="eps"]');
    await waitFor(q, () => document.querySelector('#main .pager .pageinfo'));
    const lv = await q.evaluate(() => ({ over: document.querySelector('#main').scrollWidth - document.querySelector('#main').clientWidth, n: document.querySelectorAll('#main .grid .tile').length }));
    check(lv.over <= 0 && lv.n === 48, `${name}: View all pages without sideways scroll (${lv.n} tiles)`);
    await shot(q, `${name}-viewall-eps`);
    await q.click('#main .back');
    check(!!(await waitFor(q, () => location.pathname === '/artist/tidal-4805908' && document.querySelector('#ialbums [data-rel]'))), `${name}: Back to the artist`);
    // This context's own first answer for the fixture is the error; Retry brings the sections
    await q.goto(BASE + '/artist/tidal-4805924');
    const shown = await waitFor(q, () => document.querySelector('#ialbums #irelretry') || document.querySelector('#ialbums [data-rel]') ? true : null);
    const eb = await q.evaluate(() => { const st = document.querySelector('#ialbums #irelstatus'); const rb = document.querySelector('#irelretry'); const r = rb ? rb.getBoundingClientRect() : { width: 0, height: 0 }; return { text: st ? st.textContent : '', sections: document.querySelectorAll('#ialbums [data-rel]').length, over: document.querySelector('#main').scrollWidth - document.querySelector('#main').clientWidth, retry: Math.min(r.width, r.height) }; });
    check(!!shown && /didn’t list the releases/.test(eb.text) && /502 Bad Gateway/.test(eb.text) && eb.sections === 0, `${name}: the error itself is shown (${eb.text.trim().slice(0, 80)})`);
    check(eb.over <= 0 && eb.retry >= 24, `${name}: the error and Retry fit the screen (Retry ${Math.round(eb.retry)} px, sideways ${eb.over})`);
    await shot(q, `${name}-error`);
    await q.click('#ialbums #irelretry');
    const back = await waitFor(q, () => document.querySelectorAll('#ialbums [data-rel]').length === 3 ? { over: document.querySelector('#main').scrollWidth - document.querySelector('#main').clientWidth, status: !!document.querySelector('#ialbums #irelstatus') } : null, null, 8000);
    check(!!back && back.over <= 0 && !back.status, `${name}: Retry brings the sections, no error left, no sideways scroll`);
    check(c.reqLog.filter((x) => x.startsWith('/v1/item/artist/tidal-4805924')).length === 2, `${name}: one answer per ask in this context (${c.reqLog.filter((x) => x.includes('4805924')).length})`);
    await c.close();
  }
  await browser.close();
  console.log(problems.length ? `\n${problems.length} problem(s):\n- ${problems.join('\n- ')}` : '\nall release-section checks passed');
  process.exit(problems.length ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
