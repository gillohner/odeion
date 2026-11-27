// A collection's songs keep their titles readable in a narrow window. A song row's Move up, Move
// down and Remove (shown on hover) kept ~110 px even while hidden, so at 601-768 px the title had
// 39-186 px: "Jo…" at 601, and the artist and album wrapped onto two or three lines (a taller row
// than the link cards'). Up to 820 px they now give their room to the title (the row's ⋯ menu
// offers them, and its Remove from collection still removes the song), and artist and album stay
// on one line (a focused artist or album link's ring not cut). From 821 px they are in the row as before, and not in the menu (one way to do it).
// Isolated API fixtures; no node needed.
//   node tools/ui-plsongs-room.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'plsongs-room-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

// The fixture's collection (a library song, a web link) plus an unmatched import line
const SEARCH = 'https://musicbrainz.org/search?type=recording&query=Nobody+Missing';
const items = [...fx.detail.playlist.items, { uri: SEARCH }];
const detail = { ...fx.detail, summary: { ...fx.summary, items: 3 }, playlist: { ...fx.detail.playlist, items, items_count: 3 }, tracks: [...fx.detail.tracks, null], entries: [...fx.detail.entries, { kind: 'link', uri: SEARCH }] };
const pf = { order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: [
  { index: 0, uri: fx.SONG, title: fx.track.title, artist: 'Sefa, Dr. Peacock', track_id: 1, plays_from: { service: 'library', label: 'Your library' }, sources: [] },
  { index: 1, uri: fx.LINK, plays_from: null, sources: [] },
  { index: 2, uri: SEARCH, plays_from: null, sources: [] },
] };
const card = { uri: fx.LINK, status: 'found', kind: 'Article', name: 'Liner notes', site: 'example.com', description: 'How it was made' };

// narrow: the row's Move up / Move down / Remove give their room (the ⋯ menu has them)
const CASES = [
  { width: 601, narrow: true, title: 140 }, { width: 640, narrow: true, title: 180 }, { width: 700, narrow: true, title: 240 },
  { width: 768, narrow: true, title: 240 }, { width: 768, narrow: true, touch: true, title: 230 }, { width: 820, narrow: true, title: 240 },
  { width: 821, title: 230 }, { width: 1024, title: 230 }, { width: 1280, title: 230 },
  { width: 412, narrow: true, mobile: true, title: 100 },
];

const look = () => {
  const r = document.querySelector('#plsongs .list > .srow:not(.card-row)');
  const name = r.querySelector('.grow .name'); const sub = r.querySelector('.grow .sub');
  const acts = r.querySelector('.q-actions');
  const lh = parseFloat(getComputedStyle(sub).lineHeight);
  return {
    acts: !!acts && getComputedStyle(acts).display !== 'none',
    actsShown: !!acts && getComputedStyle(acts).display !== 'none' && getComputedStyle(acts).opacity === '1',
    title: Math.round(r.querySelector('.grow').getBoundingClientRect().width),
    whole: name.scrollWidth <= name.clientWidth + 1 && Math.round(name.getBoundingClientRect().height) <= Math.ceil(parseFloat(getComputedStyle(name).lineHeight)) + 1,
    subLines: Math.round(sub.getBoundingClientRect().height / lh),
    rowH: Math.round(r.getBoundingClientRect().height),
    overflow: document.documentElement.scrollWidth > innerWidth,
  };
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const c of CASES) {
      const tag = `${c.width}${c.touch ? ' touch' : ''}${c.mobile ? ' phone' : ''}`;
      const { page, context } = await fx.create(browser, c.width, { touch: c.touch, mobile: c.mobile, fixtures: { '/playlists/c1': { body: detail }, '/playlists/c1/play-from': { body: pf }, '/cards': { body: card } } });
      const writes = [];
      await context.route(/\/v1\/playlists\/c1\/items\/\d+$/, (r) => { if (r.request().method() !== 'DELETE') return r.fallback(); writes.push(`DELETE ${r.request().url().split('/').pop()}`); return r.fulfill({ status: 200, contentType: 'application/json', body: '{}' }); });
      try {
        await fx.go(page, '/playlist/c1');
        await page.locator('#plsongs .card-row .name', { hasText: 'Liner notes' }).waitFor();
        await page.waitForFunction(() => ![...document.querySelectorAll('#plsongs .card-sub')].some((e) => /Reading the page/.test(e.textContent)));
        const song = page.locator('#plsongs .list > .srow:not(.card-row)').first();
        await song.waitFor();
        await sleep(100);
        const a = await page.evaluate(look);
        check(a.acts === !c.narrow, `${tag}: Move up, Move down and Remove ${c.narrow ? 'give their room to the title' : 'are in the row'} (${a.acts ? 'in the row' : 'not in the row'})`);
        check(a.title >= c.title, `${tag}: the title has ${a.title} px (at least ${c.title})`);
        // (at 601 px the title has 147 px: cut with "…", but no longer one letter)
        if (!c.mobile && c.width >= 640) check(a.whole, `${tag}: "${fx.track.title}" shows whole, no "…"`);
        check(a.subLines === 1, `${tag}: artist and album take one line (${a.subLines})`);
        // (one line each: 63 px, 65 on a touch screen; the wrapped artist and album made it 80-132)
        if (!c.mobile) check(a.rowH <= 66, `${tag}: the song's row is one line of title and one of artist and album tall (${a.rowH} px)`);
        check(!a.overflow, `${tag}: no sideways overflow`);
        await page.screenshot({ path: path.join(SHOTS, `plsongs-${tag.replace(/ /g, '-')}.png`) });
        // The artist link is a Tab stop on a desktop layout: its focus ring (2 px, 2 px out) isn't
        // cut by the one-line artist and album clip
        if (!c.mobile) {
          const ring = await page.evaluate(() => {
            const sub = document.querySelector('#plsongs .list > .srow:not(.card-row) .grow .sub'); const a = sub.querySelector('a.lnk');
            const s = sub.getBoundingClientRect(); const b = a.getBoundingClientRect();
            return { clips: getComputedStyle(sub).overflowX !== 'visible', left: Math.round(b.left - s.left), top: Math.round(b.top - s.top), bottom: Math.round(s.bottom - b.bottom) };
          });
          check(!ring.clips || (ring.left >= 4 && ring.top >= 4 && ring.bottom >= 4), `${tag}: a focused artist link's ring ${ring.clips ? 'has room in its line' : 'isn\'t clipped (the line doesn\'t clip)'} (left ${ring.left}, top ${ring.top}, bottom ${ring.bottom} px; 4 needed)`);
        }
        if (!c.narrow && !c.touch) {
          await song.locator('.grow .name').hover();
          check(await until(() => page.evaluate(() => { const x = document.querySelector('#plsongs .list > .srow:not(.card-row) .q-actions'); return getComputedStyle(x).opacity === '1'; }), 2000), `${tag}: on hover the row shows Move up, Move down and Remove`);
        }
        // The ⋯ menu: Move down and Remove from collection while the row's buttons are hidden (the
        // first song has no Move up), none of them while they are in the row
        await song.locator('[data-smore]').click();
        await until(() => page.locator('.popover.menu:not([hidden]) .pop-item').first().isVisible().catch(() => false), 3000);
        const menu = await page.$$eval('.popover.menu:not([hidden]) .pop-item', (xs) => xs.map((x) => x.textContent.trim()));
        const has = (w) => menu.includes(w);
        if (c.narrow) {
          check(has('Move down') && has('Remove from collection') && !has('Move up'), `${tag}: the ⋯ menu offers Move down and Remove from collection (${menu.filter((m) => /^(Move|Remove from c)/.test(m)).join(', ') || 'neither'})`);
          await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from collection' }).click();
          await until(async () => writes.length > 0, 3000);
          check(writes.join() === 'DELETE 0', `${tag}: its Remove from collection removes the song (${writes.join() || 'nothing sent'})`);
        } else {
          check(!has('Move down') && !has('Move up') && !has('Remove from collection'), `${tag}: the ⋯ menu doesn't repeat the row's Move and Remove (${menu.join(', ')})`);
          await page.keyboard.press('Escape');
        }
      } catch (e) { check(false, `${tag}: ${e.message}`); }
      await context.close();
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAILED` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
