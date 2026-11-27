// A collection's web link (its card) and an import's unmatched line (Find on MusicBrainz) line up
// with the songs above and below them: the card's picture stands where a song's Play stands and is
// as wide, so the title starts at the same x and the row is as tall. Before, the picture was 44 px
// plus a margin: the card's title sat 18 px further in and its row 5 px taller. Also with the
// page's own picture loaded, and on a touch screen (Play is 40 px there). On a phone neither row
// shows Play or a picture, and both titles still start together. In a list whose songs show covers
// (songs a service plays), the card keeps a cover's size, where the covers are, after an empty
// slot as wide as Play, so it lines up there too. And in the other arrangements: while selecting
// (a narrow window hides Play then, so it hides the card's Play-sized picture too), in the desktop
// layout chosen on a window under 600 px (songs keep Play there, so the card keeps its picture),
// and in the phone layout chosen on a wide window (neither shows Play or a picture).
// Isolated API fixtures; no node needed.
//   node tools/ui-card-row-align.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'card-row-align-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// The fixture's collection (a library song, a web link) plus an unmatched import line
const SEARCH = 'https://musicbrainz.org/search?type=recording&query=Nobody+Missing';
const items = [...fx.detail.playlist.items, { uri: SEARCH }];
const detail = { ...fx.detail, summary: { ...fx.summary, items: 3 }, playlist: { ...fx.detail.playlist, items, items_count: 3 }, tracks: [...fx.detail.tracks, null], entries: [...fx.detail.entries, { kind: 'link', uri: SEARCH }] };
const COVER = 'http://odeion-fixture.test/cover.png';
// The same song played from a service, which gives it a cover
const served = { index: 0, uri: fx.SONG, title: fx.track.title, artist: 'Sefa, Dr. Peacock', image: COVER, duration_ms: 240000, stream_uri: 'jamendo:track:1', page: 'https://www.jamendo.com/track/1', plays_from: { service: 'jamendo', label: 'Jamendo' }, sources: [] };
const pf = { order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: [
  { index: 0, uri: fx.SONG, title: fx.track.title, artist: 'Sefa, Dr. Peacock', track_id: 1, plays_from: { service: 'library', label: 'Your library' }, sources: [] },
  { index: 1, uri: fx.LINK, plays_from: null, sources: [] },
  { index: 2, uri: SEARCH, plays_from: null, sources: [] },
] };
const IMG = 'http://odeion-fixture.test/og.png';
const card = (image) => ({ uri: fx.LINK, status: 'found', kind: 'Article', name: 'Liner notes', site: 'example.com', description: 'How it was made', ...(image ? { image: IMG } : {}) });
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');

// lead: what starts the row after its number (a song's Play, a card's picture or its empty slot);
// pic: the picture (a song's cover, a card's picture), when it isn't the lead
const rows = () => [...document.querySelectorAll('#plsongs .list > .list-row')].map((r) => {
  const box = r.getBoundingClientRect(); const name = r.querySelector('.grow .name').getBoundingClientRect();
  const geo = (el) => { if (!el || !el.getClientRects().length) return null; const b = el.getBoundingClientRect(); return { x: Math.round(b.x), w: Math.round(b.width), h: Math.round(b.height), mid: Math.round(b.y + b.height / 2 - box.y) }; };
  const lead = r.querySelector(':scope > [data-splay], :scope > .playslot, :scope > .thumb');
  const pic = r.querySelector(':scope > .thumbwrap, :scope > .playslot + .thumb');
  const sub = r.querySelector('.grow .sub'); const lh = sub && parseFloat(getComputedStyle(sub).lineHeight);
  return { wraps: !!(sub && lh && sub.getBoundingClientRect().height > lh * 1.5), kind: r.matches('[data-search]') ? 'search' : r.classList.contains('card-row') ? 'card' : 'song', img: !!r.querySelector(':scope > img.thumb'), nameX: Math.round(name.x), h: Math.round(box.height), lead: geo(lead), pic: geo(pic) };
});
// (the middle is compared only while the song's words take one line each, as the card's do)
const same = (a, b, mid = true) => (a && b ? a.x === b.x && a.w === b.w && (!mid || Math.abs(a.mid - b.mid) <= 1) : a === b);
const say = (g) => (g ? `${g.x} ${g.w}×${g.h} mid ${g.mid}` : 'none');

const CASES = [
  { width: 1280 }, { width: 1024 }, { width: 768 },
  { width: 1024, image: true },
  { width: 1024, touch: true }, { width: 768, touch: true },
  { width: 700 }, { width: 700, touch: true },
  { width: 412, mobile: true }, { width: 360, mobile: true },
  { width: 1280, covers: true }, { width: 1024, covers: true }, { width: 768, covers: true }, { width: 700, covers: true },
  { width: 1024, covers: true, image: true }, { width: 1024, covers: true, touch: true }, { width: 412, covers: true, mobile: true },
  // Selecting: up to 720 px a row shows its tick box instead of Play
  { width: 1024, select: true }, { width: 720, select: true }, { width: 700, select: true }, { width: 700, select: true, touch: true },
  { width: 700, select: true, covers: true }, { width: 1024, select: true, covers: true },
  // The desktop layout on a 480-600 px window (Settings, Layout: Desktop; Automatic at exactly 600)
  { width: 560, layout: 'desktop' }, { width: 560, layout: 'desktop', image: true }, { width: 560, layout: 'desktop', covers: true }, { width: 560, layout: 'desktop', select: true }, { width: 560, layout: 'desktop', touch: true }, { width: 600 },
  // The phone layout on a wide window (Settings, Layout: Phone)
  { width: 700, layout: 'phone' }, { width: 1024, layout: 'phone' }, { width: 1024, layout: 'phone', covers: true }, { width: 1024, layout: 'phone', image: true },
];
// No Play on the row: the phone layout, or selecting in a window up to 720 px
const noPlay = (c) => !!(c.mobile || c.layout === 'phone' || (c.select && c.width <= 720));
// No song cover or card picture: a collection up to 600 px wide (on a wider window the phone layout
// keeps the covers)
const noPic = (c) => c.width <= 600;

(async () => {
  const browser = await chromium.launch();
  try {
    for (const c of CASES) {
      const tag = `${c.width}${c.image ? ' picture' : ''}${c.touch ? ' touch' : ''}${c.mobile ? ' phone' : ''}${c.covers ? ' covers' : ''}${c.layout ? ` ${c.layout}-layout` : ''}${c.select ? ' selecting' : ''}`;
      const { page, context, state } = await fx.create(browser, c.width, { touch: c.touch, mobile: c.mobile, fixtures: { '/playlists/c1': { body: detail }, '/playlists/c1/play-from': { body: c.covers ? { ...pf, items: [served, ...pf.items.slice(1)] } : pf }, '/cards': { body: card(c.image) } } });
      try {
        if (c.layout) await context.addInitScript((v) => localStorage.setItem('pm.layout', v), c.layout);
        await context.route(IMG, (r) => r.fulfill({ status: 200, contentType: 'image/png', body: PNG }));
        await context.route(COVER, (r) => r.fulfill({ status: 200, contentType: 'image/png', body: PNG }));
        await fx.go(page, '/playlist/c1');
        await page.locator('#plsongs .card-row .name', { hasText: 'Liner notes' }).waitFor();
        await page.locator('#plsongs [data-search] .name', { hasText: 'Nobody Missing' }).waitFor();
        // (where the picture is shown it loads; a hidden one may never load, so a timeout is a FAIL)
        const phoneLayout = c.mobile || c.layout === 'phone';
        if (c.image && !phoneLayout) check(await page.waitForFunction(() => { const i = document.querySelector('#plsongs .card-row:not([data-search]) > img.thumb'); return i && i.complete && i.naturalWidth > 0; }).then(() => true, () => false), `${tag}: the link's own picture loads`);
        // The cards have read their pages (the link's words below its title are final)
        await page.waitForFunction(() => ![...document.querySelectorAll('#plsongs .card-sub')].some((e) => /Reading the page/.test(e.textContent)));
        if (c.covers && !noPic(c)) await page.waitForFunction(() => { const i = document.querySelector('#plsongs .srow:not(.card-row) > .thumbwrap > img.thumb'); return i && i.complete; });
        const shell = await page.evaluate(() => document.documentElement.dataset.shell);
        check(shell === (c.mobile || c.layout === 'phone' ? 'phone' : 'desktop'), `${tag}: the ${shell} layout`);
        if (c.select) {
          await page.locator('#plsongs [data-selstart]').click();
          await page.locator('#plsongs .card-row .selbox input').first().waitFor();
        }
        const [song, link, search] = await page.evaluate(rows);
        check(song.kind === 'song' && link.kind === 'card' && search.kind === 'search', `${tag}: song, link card, unmatched line (${song.kind}, ${link.kind}, ${search.kind})`);
        if (!noPic(c)) check(!!song.pic === !!c.covers, `${tag}: the song ${c.covers ? 'shows' : 'has no'} cover (${say(song.pic)})`);
        check(!song.lead === noPlay(c), `${tag}: the song ${noPlay(c) ? 'has no' : 'shows'} Play (${say(song.lead)})`);
        if (c.image && !phoneLayout) check(link.img, `${tag}: the link's own picture is shown`);
        for (const r of [link, search]) {
          const what = `${tag}: ${r.kind === 'card' ? 'the link' : 'the unmatched line'}`;
          check(Math.abs(r.nameX - song.nameX) <= 1, `${what}'s title starts where the song's does (${r.nameX} vs ${song.nameX})`);
          // At 700 px the song's artist and album take two lines (its own room, beside its source
          // chip): its row is taller there for that, so only the titles' start is compared
          if (!song.wraps) check(Math.abs(r.h - song.h) <= 2, `${what}'s row is as tall as the song's (${r.h} vs ${song.h})`);
          if (phoneLayout) {
            // No Play and no Play-sized picture; on a wide window the songs' covers stay, and the
            // card's picture with them
            const pics = c.covers && !noPic(c);
            check(!r.lead && !song.lead, `${what}: no Play and no Play-sized picture in the phone layout (${say(r.lead)} vs ${say(song.lead)})`);
            check(pics ? !!r.pic && same(r.pic, song.pic, !song.wraps) && r.pic.h === song.pic.h : !r.pic && !song.pic, `${what}'s picture ${pics ? 'is where the song\'s cover is' : 'is not shown'} (${say(r.pic)} vs ${say(song.pic)})`);
            continue;
          }
          if (noPlay(c)) {
            // Selecting in a narrow window: no Play and no Play-sized picture or slot; a cover stays
            check(!r.lead && !song.lead, `${what}: no Play-sized picture or slot while the songs have no Play (${say(r.lead)} vs ${say(song.lead)})`);
            check(c.covers ? !!r.pic && same(r.pic, song.pic, !song.wraps) && r.pic.h === song.pic.h : !r.pic && !song.pic, `${what}'s picture ${c.covers ? 'is where the song\'s cover is' : 'is not shown'} (${say(r.pic)} vs ${say(song.pic)})`);
            continue;
          }
          if (c.covers && noPic(c)) {
            // Under 600 px the covers go: the slot stays as wide as Play, the card's picture goes
            check(r.lead && song.lead && r.lead.x === song.lead.x && r.lead.w === song.lead.w && !r.pic && !song.pic, `${what}: an empty slot as wide as Play, and no picture (${say(r.lead)} vs ${say(song.lead)}, ${say(r.pic)})`);
          } else if (c.covers) {
            check(r.lead && song.lead && r.lead.x === song.lead.x && r.lead.w === song.lead.w, `${what}: an empty slot as wide as Play first (${say(r.lead)} vs ${say(song.lead)})`);
            check(!!r.pic && same(r.pic, song.pic, !song.wraps) && r.pic.h === song.pic.h, `${what}'s picture is where the song's cover is, as big (${say(r.pic)} vs ${say(song.pic)})`);
          } else {
            check(!r.pic && same(r.lead, song.lead, !song.wraps) && r.lead.h === song.lead.h, `${what}'s picture stands in Play's place, as big (${say(r.lead)} vs ${say(song.lead)})`);
          }
        }
        const sw = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
        check(sw <= 0, `${tag}: nothing wider than the screen (${sw})`);
        check(!state.errors.length, `${tag}: no page errors ${state.errors.join(' | ')}`);
        await page.locator('#plsongs').screenshot({ path: path.join(SHOTS, `collection-${tag.replace(/ /g, '-')}.png`) });
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
