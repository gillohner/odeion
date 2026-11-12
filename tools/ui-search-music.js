// Music-focused Pubky search in the page (TASK-music-search-pinakes-1112). Real candidate assets on
// the fixture origin (tools/ui-fixture.js), /v1/search answered here as the node streams it; at 390
// and 1440 px:
//   * words ("daft punk", and the short "lol"): a Kind filter beside Source and Site, which narrows the
//     page to that kind; counts by kind from Pubky in the section heads;
//   * an ISRC: the exact hit is the Top result, with "Also on …", and the page doesn't say "No exact
//     match"; a pasted Spotify page opens its item page, as before, without a search;
//   * concerts near the node come as the Concerts section;
//   * one /v1/search request per search, and nothing wider than the screen.
//   node tools/ui-search-music.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'search-music-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (page, fn, arg = null, timeout = 8000) => { const end = Date.now() + timeout; for (;;) { const v = await page.evaluate(fn, arg).catch(() => null); if (v) return v; if (Date.now() > end) return null; await sleep(50); } };

const ISRC = 'USQX91300108';
const LINK = 'https://open.spotify.com/track/2Foc5Q5nqNiosCNqttzHof';
const hit = (kind, title, subtitle, n, site, more = {}) => ({ kind, title, subtitle, id: `u-${kind}${n}`, page: `https://example.com/${kind}/${n}`, keys: [`g:${kind}${n}`], site, ...more });
const getLucky = hit('song', 'Get Lucky (feat. Pharrell Williams and Nile Rodgers)', 'Daft Punk', 9, 'Spotify', { page: LINK, exact: true, also: ['Deezer', 'TIDAL', 'Apple Music'] });
const ANSWERS = {
  'daft punk': [
    { source: 'library', status: 'ok', ms: 3, hits: [{ kind: 'song', title: 'Around the World', subtitle: 'Daft Punk', id: 'lib-1', track_id: 1, keys: ['isrc:GBDUW0000059'], year: 1997, quality: 'lossless', format: 'FLAC 16/44.1' }] },
    { source: 'pubky', status: 'ok', ms: 80, counts: [['artist', 9], ['album', 5], ['song', 23], ['concert', 2]], hits: [
      hit('artist', 'Daft Punk', 'Spotify', 1, 'Spotify'), hit('album', 'Random Access Memories', 'Daft Punk', 1, 'Spotify'), hit('album', 'Discovery', 'Daft Punk', 2, 'Apple Music'), hit('album', 'Homework', 'Daft Punk', 3, 'Deezer'),
      hit('song', 'Get Lucky', 'Daft Punk', 1, 'Spotify'), hit('song', 'One More Time', 'Daft Punk', 2, 'Deezer'), hit('song', 'Harder, Better, Faster, Stronger', 'Daft Punk', 3, 'TIDAL'),
      hit('concert', 'Daft Punk Tribute @ Kaufleuten', '2026-12-05', 1, 'Songkick'), hit('podcast', 'The Daft Punk Story', 'Podcast', 1, 'Apple Podcasts'),
    ] },
  ],
  lol: [
    { source: 'library', status: 'ok', ms: 2, hits: [] },
    { source: 'pubky', status: 'ok', ms: 60, counts: [['song', 2], ['artist', 1]], hits: [hit('song', 'LOL', 'Basshunter', 4, 'Spotify'), hit('song', 'Lola', 'The Kinks', 5, 'Deezer'), hit('artist', 'Lolo Zouaï', 'Spotify', 2, 'Spotify')] },
  ],
  [ISRC]: [{ source: 'library', status: 'ok', ms: 2, hits: [] }, { source: 'pubky', status: 'ok', ms: 40, hits: [getLucky] }],
  'noah kahan': [
    { source: 'library', status: 'ok', ms: 2, hits: [] },
    { source: 'pubky', status: 'ok', ms: 70, counts: [['concert', 3], ['artist', 1]], hits: [hit('concert', 'Noah Kahan @ Hallenstadion', '2026-12-01', 2, 'Songkick'), hit('concert', 'Noah Kahan @ Volkshaus', '2026-12-03', 3, 'Songkick'), hit('artist', 'Noah Kahan', 'Spotify', 3, 'Spotify')] },
  ],
};

const open = async (browser, w, q) => {
  const { page, context, state } = await fx.create(browser, w, { touch: w < 1025 });
  const asked = [];
  await context.route(/\/v1\/search\?/, (r) => {
    const term = new URL(r.request().url()).searchParams.get('q'); asked.push(term);
    const answers = ANSWERS[term] || ANSWERS[term.toLowerCase()] || [{ source: 'library', status: 'ok', ms: 1, hits: [] }];
    r.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': answers.map((a) => a.source).join(',') }, body: answers.map((a) => JSON.stringify(a)).join('\n') + '\n' });
  });
  await context.route(/\/v1\/search\/tags\?/, (r) => r.fulfill({ contentType: 'application/json', body: JSON.stringify({ mine: [], others: [] }) }));
  await page.setViewportSize({ width: w, height: 900 });
  await fx.go(page, `/search?q=${encodeURIComponent(q)}`);
  await page.waitForSelector('#sresults');
  await until(page, () => !document.querySelector('#sresults .skeleton') && document.querySelector('#sresults [data-skind], #sresults #stop'));
  await sleep(300);
  return { page, context, state, asked };
};
const read = (page) => page.evaluate(() => {
  const q = (s) => document.querySelector(s); const all = (s) => [...document.querySelectorAll(s)];
  return {
    kinds: all('#sresults [data-skind]').map((s) => s.dataset.skind), heads: Object.fromEntries(all('#sresults [data-skind]').map((s) => [s.dataset.skind, s.querySelector('.section-head').innerText.replace(/\s+/g, ' ').trim()])),
    facetGroups: all('[data-facet-group]').map((g) => g.dataset.facetGroup), kindChips: all('[data-facet^="kind:"]').map((b) => b.innerText.replace(/\s+/g, ' ').trim()),
    top: q('#stop') ? q('#stop .name').innerText.trim() : null, also: q('#stopalso') ? q('#stopalso').innerText.trim() : null, nomatch: !!q('#snomatch'),
    wide: document.documentElement.scrollWidth - innerWidth, filters: !!q('details.sfilters'),
  };
});
const openFilters = async (page) => { if (await page.$('details.sfilters:not([open]) > summary')) { await page.click('details.sfilters > summary'); await sleep(150); } };

(async () => {
  const browser = await chromium.launch();
  for (const w of [390, 1440]) {
    // ---- words: the Kind filter ----
    {
      const { page, context, state, asked } = await open(browser, w, 'daft punk');
      await openFilters(page);
      const r = await read(page);
      check(r.facetGroups.includes('kind') && r.facetGroups.indexOf('kind') === 0, `${w} daft punk: a Kind filter leads the filters (${r.facetGroups.join()})`);
      check(['Songs', 'Albums', 'Artists', 'Concerts', 'Podcasts'].every((l) => r.kindChips.some((c) => c.startsWith(l))), `${w} daft punk: it lists the kinds found (${r.kindChips.join(' | ')})`);
      check(['artist', 'album', 'song', 'podcast', 'concert'].every((k) => r.kinds.includes(k)), `${w} daft punk: every kind has its section (${r.kinds.join()})`);
      check(/of 23 on Pubky/.test(r.heads.song || '') && /of 5 on Pubky/.test(r.heads.album || ''), `${w} daft punk: the heads count the music on Pubky (${r.heads.song} | ${r.heads.album})`);
      check(r.wide <= 0, `${w} daft punk: nothing wider than the screen (${r.wide})`);
      await page.screenshot({ path: path.join(SHOTS, `daft-punk-${w}.png`) });
      await page.click('[data-facet="kind:album"]'); await sleep(250);
      await openFilters(page);
      const a = await read(page);
      check(a.kinds.join() === 'album' && a.wide <= 0, `${w} daft punk: Albums narrows the page to albums (${a.kinds.join()})`);
      const pressed = await page.evaluate(() => { const b = document.querySelector('[data-facet="kind:album"]'); return b && b.getAttribute('aria-pressed'); });
      check(pressed === 'true', `${w} daft punk: the picked kind is pressed (${pressed})`);
      await page.screenshot({ path: path.join(SHOTS, `daft-punk-albums-${w}.png`) });
      await page.click('[data-facet="kind:album"]'); await sleep(250);
      const back = await read(page);
      check(back.kinds.length >= 5, `${w} daft punk: pressing it again shows every kind (${back.kinds.join()})`);
      check(asked.length === 1, `${w} daft punk: one search request, filters ask nothing (${asked.length})`);
      check(!state.errors.length, `${w} daft punk: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
      await context.close();
    }
    // ---- a short query ----
    {
      const { page, context, asked } = await open(browser, w, 'lol');
      const r = await read(page);
      check(r.kinds.includes('song') && r.kinds.includes('artist') && r.wide <= 0 && asked.length === 1, `${w} lol: a short query lists its songs and artists (${r.kinds.join()}, ${asked.length} request)`);
      await page.screenshot({ path: path.join(SHOTS, `lol-${w}.png`) });
      await context.close();
    }
    // ---- an ISRC and a pasted link: the exact hit on top with Also on ----
    for (const [name, q] of [['isrc', ISRC]]) {
      const { page, context, state, asked } = await open(browser, w, q);
      const r = await read(page);
      check(r.top === getLucky.title, `${w} ${name}: the exact hit is the Top result (${r.top})`);
      check(r.also === 'Also on Deezer · TIDAL · Apple Music', `${w} ${name}: with the other sites it is on (${r.also})`);
      check(!r.nomatch, `${w} ${name}: the page doesn't say "No exact match"`);
      check(r.wide <= 0 && asked.length === 1 && asked[0] === q, `${w} ${name}: one request with the term as typed, nothing wider than the screen (${asked.length}, ${r.wide})`);
      check(!state.errors.length, `${w} ${name}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
      await page.screenshot({ path: path.join(SHOTS, `${name}-${w}.png`) });
      await context.close();
    }
    // ---- a pasted service page opens its item page, as before (its Also on is there); no search ----
    {
      const { page, context, state } = await fx.create(browser, w, { touch: w < 1025 });
      const asked = [];
      await context.route(/\/v1\/search\?/, (r) => { asked.push(r.request().url()); r.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': 'library' }, body: JSON.stringify({ source: 'library', status: 'ok', ms: 1, hits: [] }) + '\n' }); });
      await page.setViewportSize({ width: w, height: 900 });
      await fx.go(page, '/'); await page.waitForSelector('#search');
      await page.fill('#search', LINK); await page.press('#search', 'Enter');
      const at = await until(page, () => (/^\/(song|item)\//.test(location.pathname) ? location.pathname : null));
      await sleep(400);
      check(at === '/song/spotify-2Foc5Q5nqNiosCNqttzHof' && asked.length === 0, `${w} spotify-link: a pasted Spotify page opens its item page without a search (${at}, ${asked.length} requests)`);
      check(state.hits.some((h) => h.v === '/item/song/spotify-2Foc5Q5nqNiosCNqttzHof'), `${w} spotify-link: the node is asked for that item`);
      await page.screenshot({ path: path.join(SHOTS, `spotify-link-${w}.png`) });
      await context.close();
    }
    // ---- concerts near the node ----
    {
      const { page, context } = await open(browser, w, 'noah kahan');
      const r = await read(page);
      const rows = await page.evaluate(() => [...document.querySelectorAll('[data-skind="concert"] .list-row')].map((x) => x.innerText.replace(/\s+/g, ' ').trim()));
      check(r.kinds.includes('concert') && rows.length === 2 && /Hallenstadion/.test(rows[0]) && /2026-12-01/.test(rows[0]), `${w} concerts: the Concerts section lists them with their dates (${rows.join(' | ')})`);
      check(r.wide <= 0, `${w} concerts: nothing wider than the screen (${r.wide})`);
      await page.screenshot({ path: path.join(SHOTS, `concerts-${w}.png`) });
      await context.close();
    }
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
