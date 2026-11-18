// A Back whose name is cut with … ("Late night drives through the city…") had no tooltip, so a
// mouse on a desktop or tablet could not read where it goes back to (the button's own text still
// names it whole for a screen reader). Now hovering a cut Back shows the whole name; a Back whose
// name fits shows none, and the check is made at hover time, so a window made wide enough drops it.
// The same for an item page's own name at the end of its breadcrumb ("Sefa › Late night drives…",
// cut at 32ch): hovering it shows the whole name, one that fits shows none.
// Real UI in Chromium against fixtures (no node).
//   node tools/ui-back-tooltip.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'back-tooltip-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const LONG = 'Late night drives through the city with the windows down and nowhere to be, everything we played that summer on the coast road';
const album = { uri: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [],
  kind: 'album', id: 'lib-7', target: { album_id: 7 }, title: 'Journey', artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020,
  songs: [{ no: 1, title: fx.track.title, artist: 'Sefa', artist_id: 'lib-11', duration_ms: 240000, uri: null, track_id: 1, id: 'lib-1', page: 'lib-1' }], about: [], about_text: null, image_credit: null };
// (cut at every width from 768 to 1280; LONG alone fits once the window is very wide)
const LONGER = `${LONG}, and the next morning on the ferry back`.repeat(2);
const withName = (name) => ({
  '/item/album/lib-7': { body: album },
  '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
  '/playlists/c1': { body: { ...fx.detail, summary: { ...fx.summary, name }, playlist: { ...fx.detail.playlist, name } } },
});
const BACK = '#main .pagebar > .back';
const look = (page) => page.locator(BACK).first().evaluate((b) => {
  const l = b.querySelector('.back-l');
  return { text: l.textContent, cut: l.scrollWidth > l.clientWidth, title: b.getAttribute('title') };
});
// The mouse comes onto Back from the page's top left corner, as a person's would.
const hover = async (page) => { await page.mouse.move(2, 2); await page.locator(BACK).first().hover(); };
// From the playlist, its first song's album: the album page's Back names the playlist.
async function openAlbum(page) {
  await fx.go(page, '/playlist/c1');
  await page.waitForSelector('#main .srow a[href*="/album/"]');
  await page.locator('#main .srow a[href*="/album/"]').first().click();
  await page.waitForSelector(BACK);
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [768, 1024, 1280]) {
      const { page, context, state } = await fx.create(browser, width, { fixtures: withName(LONGER) });
      try {
        await openAlbum(page);
        const before = await look(page);
        check(before.text === LONGER && before.cut, `${width} album from a long-named playlist: Back reads the whole name, cut with … on screen`);
        check(before.title === null, `${width}: no tooltip before the mouse comes`);
        await hover(page);
        const on = await look(page);
        check(on.title === LONGER, `${width}: hovering the cut Back shows the whole name (${on.title === null ? 'no tooltip' : on.title.slice(0, 24) + '…'})`);
        await page.screenshot({ path: path.join(SHOTS, `after-album-${width}.png`) });
        await page.locator(BACK).first().click();
        await page.waitForFunction(() => location.pathname.startsWith('/playlist/'));
        check(true, `${width}: the cut Back still goes back (${await page.evaluate(() => location.pathname)})`);
        check(!state.errors.length, `${width}: no page errors (${state.errors.join('; ')})`);
        await fx.cleanCopy(page);
      } finally { await context.close(); }
    }
    // A window made wide enough to show the whole name: the next hover drops the tooltip.
    {
      const { page, context } = await fx.create(browser, 768, { fixtures: withName(LONG) });
      try {
        await openAlbum(page);
        await hover(page);
        check((await look(page)).title === LONG, '768: cut Back has the tooltip');
        await page.setViewportSize({ width: 2400, height: 900 });
        await page.waitForFunction((s) => { const l = document.querySelector(s + ' .back-l'); return l && l.scrollWidth <= l.clientWidth; }, BACK);
        await hover(page);
        const wide = await look(page);
        check(!wide.cut && wide.title === null, `768 → 2400: the whole name fits, hover shows no tooltip (${wide.title})`);
      } finally { await context.close(); }
    }
    // A short name fits: no tooltip on hover (it would only repeat the button's text).
    for (const width of [768, 1280]) {
      const { page, context } = await fx.create(browser, width, { fixtures: withName('Road trip') });
      try {
        await openAlbum(page);
        await hover(page);
        const s = await look(page);
        check(s.text === 'Road trip' && !s.cut && s.title === null, `${width}: a Back whose name fits has no tooltip (${s.title})`);
      } finally { await context.close(); }
    }
    // An album page's breadcrumb (Sefa › album title): its last crumb, the album's own name, is cut
    // at 32ch. Opened straight (no Back history needed), at each desktop width.
    const CRUMB = '#main .pagebar .crumbs li[aria-current]';
    const lookCrumb = (page) => page.locator(CRUMB).first().evaluate((c) => ({ text: c.textContent, cut: c.scrollWidth > c.clientWidth, title: c.getAttribute('title') }));
    const hoverCrumb = async (page) => { await page.mouse.move(2, 2); await page.locator(CRUMB).first().hover(); };
    const named = (title) => ({ '/item/album/lib-7': { body: { ...album, title } }, '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } } });
    for (const width of [768, 1024, 1280]) {
      const { page, context, state } = await fx.create(browser, width, { fixtures: named(LONG) });
      try {
        await fx.go(page, '/album/lib-7');
        await page.waitForSelector(CRUMB);
        const before = await lookCrumb(page);
        check(before.text === LONG && before.cut, `${width} album with a long name: its crumb reads the whole name, cut with … on screen`);
        check(before.title === null, `${width} crumb: no tooltip before the mouse comes`);
        await hoverCrumb(page);
        const on = await lookCrumb(page);
        check(on.title === LONG, `${width}: hovering the cut crumb shows the whole name (${on.title === null ? 'no tooltip' : on.title.slice(0, 24) + '…'})`);
        await page.screenshot({ path: path.join(SHOTS, `after-crumb-${width}.png`) });
        check(!state.errors.length, `${width} crumb: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    for (const width of [768, 1280]) {
      const { page, context } = await fx.create(browser, width, { fixtures: named('Journey') });
      try {
        await fx.go(page, '/album/lib-7');
        await page.waitForSelector(CRUMB);
        await hoverCrumb(page);
        const s = await lookCrumb(page);
        check(s.text === 'Journey' && !s.cut && s.title === null, `${width}: a crumb whose name fits has no tooltip (${s.title})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
