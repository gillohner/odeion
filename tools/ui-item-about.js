// Item pages show the music first; ids, links and credits sit in Details at the foot
// (TASK-item-page-details-to-bottom-1105; owner: "the page shows artist metadata and songs but not less
// relevant metadata directly to user"). Real candidate assets on the fixture origin
// (tools/ui-fixture.js); a Calvin Russell-like artist with a Wikipedia article, an album and a song,
// each with facts of both kinds from the node, at 390 (phone), 820 and 1280 px:
//   * under the hero: the blurb (with More and a small Wikipedia link), one facts line without labels
//     (Origin · Active 1978–2011 · Genres), "In your library: …" as a small line, then the songs;
//   * no raw URL, MusicBrainz id or file path above the songs;
//   * Details is the page's last section, holds the ids, links and credits, and is closed on a phone;
//   * on a phone the songs' heading is within the first screen after the hero;
// and an artist without an article keeps About at the end, whole, as before. On a phone (360x640 to
// 412x915, browser and Android app) the article's opening is 2 lines and More a 44 px tap; 4 lines from 600 px.
//   node tools/ui-item-about.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'item-about-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const MBID = '5ba4c4b7-1d1e-4a3d-9f36-2d4a9d4c0a11';
const WIKI = 'https://en.wikipedia.org/wiki/Calvin_Russell';
const TEXT = 'Calvin Russell was an American singer-songwriter from Austin, Texas, whose gravelly roots rock found its audience in France and Switzerland before it did at home. He recorded more than a dozen albums over three decades, touring Europe most years. ';
const about_text = { text: TEXT.repeat(4), lang: 'en', source: 'Wikipedia', url: WIKI, licence: 'CC BY-SA 4.0', licence_url: 'https://creativecommons.org/licenses/by-sa/4.0/' };
const image_credit = { author: 'Someone', licence: 'FAL', licence_url: 'https://artlibre.org/licence/lal/en/', file_page: 'https://commons.wikimedia.org/wiki/File:Calvin_Russell.jpg', source: 'Wikimedia Commons' };
const ref = (label, value, href) => ({ label, value, href, kind: 'ref' });
const fact = (label, value) => ({ label, value, kind: 'fact' });
const artistAbout = [ref('MusicBrainz', MBID, `https://musicbrainz.org/artist/${MBID}`), fact('In your library', '8 albums · 91 songs'), fact('Origin', 'Austin, Texas, United States'), fact('Active', '1978–2011'), fact('Genres', 'roots rock, blues rock'), ref('Wikipedia', WIKI, WIKI), ref('Wikidata', 'https://www.wikidata.org/wiki/Q490290', 'https://www.wikidata.org/wiki/Q490290')];
const songs = Array.from({ length: 8 }, (_, i) => ({ no: i + 1, title: `Song ${i + 1}`, artist: 'Calvin Russell', artist_id: 'lib-31', duration_ms: 200000 + i * 5000, uri: null, track_id: 100 + i, id: `lib-${100 + i}`, page: `lib-${100 + i}` }));
const base = { uri: null, source: 'library', source_id: '31', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library', image: null, tags: [] };
const pages = {
  artist: { ...base, kind: 'artist', id: 'lib-31', target: { artist_id: 31 }, title: 'Calvin Russell', artist: null, album: null, year: null, songs, about: artistAbout, about_text, image_credit, albums: [{ id: 'lib-7', title: 'Crossroad', artist: 'Calvin Russell', year: 1997, image: null }] },
  album: { ...base, kind: 'album', id: 'lib-7', target: { album_id: 7 }, title: 'Crossroad', artist: { name: 'Calvin Russell', id: 'lib-31' }, album: null, year: 1997, songs, about: [ref('MusicBrainz', MBID, `https://musicbrainz.org/release-group/${MBID}`), fact('Released', '1997-03-01'), fact('Label', 'SPV'), ref('Wikipedia', WIKI, WIKI)], about_text, image_credit: null },
  song: { ...base, kind: 'song', id: 'lib-100', target: { track_id: 100 }, title: 'Song 1', artist: { name: 'Calvin Russell', id: 'lib-31' }, album: { title: 'Crossroad', id: 'lib-7' }, year: 1997, songs: [], albums: [{ id: 'lib-7', title: 'Crossroad', artist: 'Calvin Russell', year: 1997, image: null }], about: [ref('Format', 'FLAC'), ref('File', '/music/calvin/song1.flac'), ref('MusicBrainz', MBID, `https://musicbrainz.org/recording/${MBID}`), ref('Identified by', 'tags')], about_text, image_credit: null },
  short: { ...base, kind: 'artist', id: 'lib-33', target: { artist_id: 33 }, title: 'Short Facts', artist: null, album: null, year: null, songs, about: [fact('In your library', '1 album · 8 songs'), fact('Origin', 'Austin'), fact('Active', '1978–2011')], about_text, image_credit: null },
  plain: { ...base, kind: 'artist', id: 'lib-32', target: { artist_id: 32 }, title: 'No Article', artist: null, album: null, year: null, songs, about: [ref('MusicBrainz', MBID, `https://musicbrainz.org/artist/${MBID}`), fact('In your library', '1 album · 8 songs')], about_text: null, image_credit: null, blurb: null },
};
const fixtures = {};
for (const [k, p] of Object.entries(pages)) { fixtures[`/item/${p.kind}/${p.id}`] = { body: p }; fixtures[`/item/${p.kind}/${p.id}/extras`] = { body: { collections: [], related: [], concerts: [], similar: [] } }; }
fixtures['/library/artists/31/similar'] = { body: { artists: [] } };
fixtures['/library/artists/32/similar'] = { body: { artists: [] } };
fixtures['/library/artists/33/similar'] = { body: { artists: [] } };
for (const s of songs) fixtures[`/library/tracks/${s.track_id}`] = { body: { ...fx.track, id: s.track_id, title: s.title } };
const RAW = new RegExp(`https?://|${MBID}|/music/calvin`);

(async () => {
  const browser = await chromium.launch();
  for (const [w, h] of [[390, 844], [820, 1180], [1280, 900]]) {
    for (const key of ['artist', 'album', 'song']) {
      const p = pages[key];
      const { page, context, state } = await fx.create(browser, w, { fixtures, touch: w < 1025 });
      await page.setViewportSize({ width: w, height: h });
      await fx.go(page, `/${p.kind}/${p.id}`); await page.waitForSelector('#iabout'); await sleep(400);
      const at = `${key} at ${w}`;
      const r = await page.evaluate((songKind) => {
        const q = (s) => document.querySelector(s); const top = (el) => (el ? Math.round(el.getBoundingClientRect().top + scrollY) : null);
        const sections = [...document.querySelectorAll('#main > .section, #main > details.section, #main > [id$="-slot"]')].map((s) => s.id).filter(Boolean);
        const hero = q('.hero'); const about = q('#iabout'); const songsEl = q('#isongs') || q('#ialbums'); const details = q('#idetails');
        // (only what is drawn: a closed Details' content has no box and would count as "above")
        const aboveSongs = songsEl ? [...document.querySelectorAll('#main *')].filter((el) => el.getClientRects().length && el.getBoundingClientRect().top + scrollY < top(songsEl) && el.children.length === 0).map((el) => el.textContent).join(' | ') : '';
        const facts = q('#iabout .facts-line'); const lib = q('#iabout .libline'); const wiki = q('#iabout .wikilink');
        return {
          sections, aboutTop: top(about), songsTop: top(songsEl), heroBottom: hero ? Math.round(hero.getBoundingClientRect().bottom + scrollY) : null, detailsTop: top(details),
          detailsOpen: details ? details.open : null, detailsText: details ? details.textContent : '', facts: facts ? facts.innerText : null, lib: lib ? lib.innerText : null, wiki: wiki ? wiki.getAttribute('href') : null,
          blurbClamped: !!q('#iblurb.clamp'), more: !!q('#iblurbmore') && !q('#iblurbmore').hidden, aboveSongs, last: sections[sections.length - 1], hasSongs: !!songsEl, kind: songKind,
        };
      }, p.kind);
      const musicSection = p.kind === 'song' ? '#ialbums' : '#isongs';
      check(r.aboutTop != null && (p.kind === 'song' ? !r.hasSongs || r.aboutTop < r.songsTop : r.aboutTop < r.songsTop), `${at}: the blurb and facts come before ${musicSection} (${r.aboutTop} < ${r.songsTop})`);
      check(r.last === 'idetails' && r.detailsTop > (r.songsTop || 0), `${at}: Details is the last section (${r.sections.join(' → ')})`);
      check(r.detailsOpen === (w >= 600), `${at}: Details is ${w >= 600 ? 'open' : 'closed'} (${r.detailsOpen})`);
      check(!RAW.test(r.aboveSongs), `${at}: no raw URL, MusicBrainz id or file path above the songs${RAW.test(r.aboveSongs) ? ': ' + (r.aboveSongs.match(RAW) || [])[0] : ''}`);
      check(/MusicBrainz/.test(r.detailsText) && (key !== 'artist' || /Wikidata/.test(r.detailsText) && /Picture/.test(r.detailsText)) && /Text: /.test(r.detailsText), `${at}: Details holds the ids, links and credits`);
      if (key === 'artist') {
        check(r.facts === 'Austin, Texas, United States · Active 1978–2011 · roots rock, blues rock', `${at}: one facts line without labels (${r.facts})`);
        check(r.lib === 'In your library: 8 albums · 91 songs', `${at}: the library line (${r.lib})`);
      }
      if (key === 'album') check(r.facts === '1997-03-01 · SPV', `${at}: the album's facts line (${r.facts})`);
      check(r.wiki === WIKI && r.blurbClamped && r.more, `${at}: the blurb is clamped with More and a Wikipedia link after it`);
      if (w < 600 && r.hasSongs) check(r.songsTop - r.heroBottom <= h, `${at}: the songs start within a screen of the hero (${r.songsTop - r.heroBottom} px)`);
      check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
      await page.screenshot({ path: path.join(SHOTS, `${key}-${w}.png`) });
      await context.close();
    }
  }
  // No article: About stays whole, at the end
  {
    const p = pages.plain;
    const { page, context } = await fx.create(browser, 390, { fixtures, touch: true });
    await page.setViewportSize({ width: 390, height: 844 });
    await fx.go(page, `/${p.kind}/${p.id}`); await page.waitForSelector('#iabout'); await sleep(400);
    const r = await page.evaluate(() => { const ids = [...document.querySelectorAll('#main > .section, #main > details.section')].map((s) => s.id); const a = document.querySelector('#iabout'); return { ids, last: ids[ids.length - 1], text: a.innerText, details: !!document.querySelector('#idetails') }; });
    check(r.last === 'iabout' && !r.details && /MusicBrainz/.test(r.text) && /In your library/.test(r.text), `no article: About is whole and last (${r.ids.join(' → ')})`);
    await context.close();
  }
  // On a phone the article's opening is 2 lines (4 on tablets and computers), so the first Top song
  // comes a screen sooner; More is a 44 px tap that moves nothing, and the Wikipedia link keeps its own
  for (const [w, h, android, key] of [[360, 640, false], [360, 800, false], [360, 800, true], [360, 800, true, 'short'], [412, 915, true], [820, 1180, false], [1280, 900, false]]) {
    const p = pages[key || 'artist']; const phone = w < 600;
    const { page, context, state } = await fx.create(browser, w, { fixtures, mobile: phone, touch: w < 1025 });
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    await page.setViewportSize({ width: w, height: h });
    await fx.go(page, `/${p.kind}/${p.id}`); await page.waitForSelector('#iabout'); await sleep(400);
    const at = `${key ? `${key}-facts artist` : 'artist'} at ${w}x${h}${android ? ' (Android app)' : ''}`;
    const r = await page.evaluate(() => {
      const q = (s) => document.querySelector(s); const bl = q('#iblurb'); const more = q('#iblurbmore'); const wiki = q('#iabout .wikilink');
      const lh = parseFloat(getComputedStyle(bl).lineHeight); const m = more.getBoundingClientRect(); const wk = wiki.getBoundingClientRect();
      const isMore = (x, y) => { const el = document.elementFromPoint(x, y); return !!el && !!el.closest('#iblurbmore'); };
      const cx = m.left + m.width / 2; const cy = m.top + m.height / 2;
      const fold = Math.min(innerHeight, ...[...document.querySelectorAll('footer.player, #bottomnav')].filter((e) => e.getClientRects().length).map((e) => e.getBoundingClientRect().top));
      const row = q('#isongs .list-row, #isongs .song-row');
      return { lines: Math.round(bl.clientHeight / lh), moreH: Math.round(m.height),
        reach: [isMore(cx, cy - 21), isMore(cx, cy + 21), isMore(m.left - 6, cy), isMore(m.right + 6, cy)],
        wikiOwn: (() => { const el = document.elementFromPoint(wk.left + 1, wk.top + wk.height / 2); return !!el && !!el.closest('.wikilink'); })(),
        below: { x: cx, y: cy + 18 }, fold: Math.round(fold), song1Bottom: row ? Math.round(row.getBoundingClientRect().bottom) : null,
        sw: document.documentElement.scrollWidth - innerWidth };
    });
    check(r.lines === (phone ? 2 : 4), `${at}: the article's opening is ${phone ? 2 : 4} lines (${r.lines})`);
    check(r.moreH <= 24, `${at}: More still looks like a small link (${r.moreH} px tall)`);
    check(r.wikiOwn, `${at}: the Wikipedia link still takes its own tap, right up to its left edge`);
    check(r.sw <= 0, `${at}: no sideways scroll (${r.sw})`);
    if (phone) {
      check(r.reach.every(Boolean), `${at}: More is a 44 px tap, 21 px above and below its middle and 6 px to its sides (${r.reach})`);
      // (a one-line facts line at 360x800 is where 2 lines instead of 4 brings the first song out from under the mini player)
      if (w === 412 || key === 'short') check(r.song1Bottom != null && r.song1Bottom <= r.fold, `${at}: the first Top song is whole above the mini player (${r.song1Bottom} <= ${r.fold})`);
      await page.touchscreen.tap(r.below.x, r.below.y); await sleep(200);
      const open = await page.evaluate(() => !document.querySelector('#iblurb').classList.contains('clamp') && document.querySelector('#iblurbmore').getAttribute('aria-expanded') === 'true');
      check(open, `${at}: a tap just under More shows the whole article`);
    }
    check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
    await page.screenshot({ path: path.join(SHOTS, `artist-blurb-${key ? `${key}-` : ''}${w}x${h}${android ? '-android' : ''}.png`) });
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
