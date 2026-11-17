// Artists: each row ends in a chevron. It pointed down, the way a row that opens in place looks,
// though the row opens the artist's page. It now points right in the muted grey, the same mark as
// Search's rows that open a page here (a person, your own playlist), and the row still opens the
// artist by a click and by Enter. The same down chevron ended the item pages' "In collections"
// rows, each of which opens that collection's page; they now point right the same way.
// At 1280, 1024 and 768 px, and on a phone at 360 and 412 px.
// Real candidate assets, isolated API fixtures; no node or network.
//   node tools/ui-artist-row-chevron.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'artist-row-chevron-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

const artists = [{ id: 11, name: 'Sefa', albums: 2, tracks: 9 }, { id: 22, name: 'Dr. Peacock', albums: 1, tracks: 4 }, { id: 33, name: 'A very long artist name that has to share its row with the chevron at the end', albums: 1, tracks: 1 }];
const COLL = 'yxk3r6cjn8b9xq5ppq3pq1nwj4ijgsqgd9sbrmtmq3qhkey5h7xo';
const inColl = { count: 2, items: [{ uri: `pubky://${COLL}/pub/social/v1/posts/0035WKVKQDHSM`, author: COLL, id: '0035WKVKQDHSM', name: 'Night drives', mine: false }, { uri: `pubky://${COLL}/pub/social/v1/posts/00351MT54YS9G`, author: COLL, id: '00351MT54YS9G', mine: false }] };
// Sefa's page (the item answer's shape as in ui-phone-item-hero-row.js), and its collections
const sefa = { kind: 'artist', id: 'lib-11', title: 'Sefa', uri: null, source: 'library', source_id: '11', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'artists', target: { artist_id: 11 }, image: null, tags: [], songs: [] };
const fixtures = { '/library/artists': { body: artists }, '/item/artist/lib-11': { body: sefa }, '/item/artist/lib-11/extras': { body: { collections: inColl, related: [], concerts: [], similar: [] } } };
// What a chevron at the end of each row looks like
const chevrons = (page, rows) => page.$$eval(rows, (rs) => rs.map((r) => {
  const ic = r.lastElementChild; const cs = ic && getComputedStyle(ic);
  const box = ic && ic.getBoundingClientRect();
  const muted = getComputedStyle(document.documentElement).getPropertyValue('--muted').trim();
  const probe = document.createElement('span'); probe.style.color = muted; document.body.append(probe); const mutedRgb = getComputedStyle(probe).color; probe.remove();
  return { isIcon: !!ic && ic.matches('svg.ic'), href: ic && ic.querySelector('use') && ic.querySelector('use').getAttribute('href'), right: !!ic && ic.matches('.list-row .ic.right'), transform: cs && cs.transform, color: cs && cs.color, mutedRgb, visible: !!box && box.width > 0 && box.height > 0, inRow: !!box && box.right <= r.getBoundingClientRect().right + 0.5 };
}));
// rotate(-90deg) is matrix(0, -1, 1, 0, 0, 0): the down chevron turned to point right
const m = (t) => (/^matrix\(([^)]+)\)$/.exec(t || '') || [, ''])[1].split(',').map(Number);
const pointsRight = (g) => { const v = m(g.transform); return v.length === 6 && Math.abs(v[0]) < 1e-6 && Math.abs(v[1] + 1) < 1e-6 && Math.abs(v[2] - 1) < 1e-6 && Math.abs(v[3]) < 1e-6; };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, phone] of [[1280, false], [1024, false], [768, false], [360, true], [412, true]]) {
      const tag = phone ? `phone-${width}` : `${width}`;
      const { page, context, state } = await fx.create(browser, width, phone ? { fixtures, mobile: true } : { fixtures });
      try {
        await fx.go(page, '/artists');
        const drawn = await page.waitForFunction(() => document.querySelectorAll('#artistlist [data-artist]').length === 3, null, { timeout: 8000 }).then(() => true, () => false);
        check(drawn, `${tag}: three artist rows drawn`);
        if (!drawn) continue;
        if (phone) check(await page.evaluate(() => document.documentElement.dataset.shell === 'phone'), `${tag}: phone shell`);
        const got = await chevrons(page, '#artistlist [data-artist]');
        check(got.every((g) => g.isIcon && g.href === '#i-chevron' && g.visible), `${tag}: every row ends in a visible chevron (${got.map((g) => g.href).join()})`);
        check(got.every(pointsRight), `${tag}: every chevron points right (${got.map((g) => g.transform).join(' | ')})`);
        check(got.every((g) => g.right), `${tag}: the same mark as Search's rows that open a page (.list-row .ic.right)`);
        check(got.every((g) => g.color === g.mutedRgb), `${tag}: chevrons in the muted grey (${got[0].color} vs ${got[0].mutedRgb})`);
        check(got.every((g) => g.inRow), `${tag}: the chevron stays inside its row, a long name too`);
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${tag}: no horizontal overflow`);
        await page.screenshot({ path: path.join(SHOTS, `artists-${tag}.png`) });
        // The row still opens the artist: a click on the chevron itself, and Enter on the row
        await page.locator('#artistlist [data-artist="22"] > .ic:last-child').click();
        const opened = await page.waitForFunction(() => location.pathname === '/artist/lib-22', null, { timeout: 5000 }).then(() => true, () => false);
        check(opened, `${tag}: a click on the chevron opens Dr. Peacock's page (${await page.evaluate(() => location.pathname)})`);
        if (!phone) {
          await page.goBack();
          const back = await page.waitForFunction(() => document.querySelectorAll('#artistlist [data-artist][tabindex]').length === 3, null, { timeout: 8000 }).then(() => true, () => false);
          check(back, `${tag}: Back shows the artists again`);
          if (back) {
            await page.focus('#artistlist [data-artist="11"]'); await page.keyboard.press('Enter');
            const entered = await page.waitForFunction(() => location.pathname === '/artist/lib-11', null, { timeout: 5000 }).then(() => true, () => false);
            check(entered, `${tag}: Enter on Sefa's row opens Sefa's page`);
          }
        }
        // Sefa's page: the In collections rows end in the same right-pointing chevron and open the collection
        {
          await fx.go(page, '/artist/lib-11');
          const coll = await page.waitForFunction(() => document.querySelectorAll('#icollections [data-icoll]').length === 2, null, { timeout: 8000 }).then(() => true, () => false);
          check(coll, `${tag}: Sefa's page lists two collections`);
          if (coll) {
            const c = await chevrons(page, '#icollections [data-icoll]');
            check(c.every((g) => g.isIcon && g.href === '#i-chevron' && g.visible && g.inRow), `${tag}: every collection row ends in a visible chevron`);
            check(c.every(pointsRight) && c.every((g) => g.right), `${tag}: collection chevrons point right (${c.map((g) => g.transform).join(' | ')})`);
            check(c.every((g) => g.color === g.mutedRgb), `${tag}: collection chevrons in the muted grey (${c[0].color})`);
            check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${tag}: no horizontal overflow on the artist page`);
            await page.locator('#icollections').scrollIntoViewIfNeeded();
            await page.screenshot({ path: path.join(SHOTS, `collections-${tag}.png`) });
            await page.locator('#icollections [data-icoll="0"] > .ic:last-child').click();
            const shared = await page.waitForFunction((u) => location.pathname === u, `/shared/${COLL}/0035WKVKQDHSM`, { timeout: 5000 }).then(() => true, () => false);
            check(shared, `${tag}: a click on the chevron opens that collection (${await page.evaluate(() => location.pathname)})`);
          }
        }
        check(state.errors.length === 0, `${tag}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
