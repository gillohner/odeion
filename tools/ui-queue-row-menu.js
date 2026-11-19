// A Queue row's ⋯ is the song's own menu at every width, as on Songs: a library song's has Add to
// playlist…, Add to favourites and Go to album (its Add to playlist adds that song by its track id),
// a streamed song's has Go to song page. Desktop rows show the ⋯ (before, only phones did, and a
// queued song had to be found again on Songs to be added to a playlist); their Move up / Move down /
// Remove stay buttons on the row, not menu items. Phones keep those three first in the ⋯. A library
// song the node can't answer for still gets a menu (Go to song page). A ⋯ whose row is drawn again
// while its menu still reads opens nothing.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-queue-row-menu.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/queue-row-menu-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
// (Song 2 has an ISRC: its menu also reads the song's other versions, which a case below holds)
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, title: `Song ${id}`, album: { id: 7, title: 'Journey' }, ...(id === 2 ? { isrc: 'GBAYE0000001' } : {}) }));
const fixtures = {};
for (const t of tracks.slice(0, 2)) fixtures[`/library/tracks/${t.id}`] = { body: t }; // (Song 3: the node doesn't answer)
const saved = [
  ...tracks.map((t) => ({ id: t.id, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 })),
  { id: 9, uri: 'tidal:track:534050211', title: 'Streamed song', artist: 'Someone', duration_ms: 60000, stream: 'https://example.com/s.flac' },
];
const menuItems = (page) => page.locator('.popover.menu:not([hidden]) .pop-item').evaluateAll((bs) => bs.map((b) => b.textContent.trim()));
const openMenu = async (page, n, touch) => {
  const b = page.locator('.list.queue .list-row').nth(n).locator('[data-qmore]');
  if (touch) await b.tap(); else await b.click();
  await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor();
  return menuItems(page);
};
const close = async (page) => { await page.keyboard.press('Escape'); await page.locator('.popover.menu:not([hidden])').waitFor({ state: 'detached' }).catch(() => {}); await page.waitForTimeout(100); };

async function run(browser, width) {
  const phone = width < 600;
  const { context, page, state } = await fx.create(browser, width, { fixtures, tracks, collections: [fx.summary], ...(phone ? { mobile: true } : {}) });
  await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(saved), JSON.stringify({ index: 0, id: saved[0].id, pos: 0 })]);
  let hold = null;
  await context.route('**/v1/library/tracks/2/versions', async (r) => { if (hold) await hold.wait; r.fulfill({ status: 200, contentType: 'application/json', body: '[]' }); });
  await fx.go(page, '/queue');
  await page.locator('.list.queue .list-row').nth(3).waitFor();
  const row = page.locator('.list.queue .list-row').nth(1);
  if (!phone) await row.hover();
  check(await row.locator('[data-qmore]').isVisible(), `${width}: a queue row shows its ⋯`);
  check(phone !== await row.locator('[data-qup]').isVisible(), `${width}: Move up is ${phone ? 'not ' : ''}a button on the row`);
  await page.screenshot({ path: path.join(shots, `queue-${width}.png`) });

  const lib = await openMenu(page, 1, phone);
  await page.screenshot({ path: path.join(shots, `queue-menu-${width}.png`) });
  for (const want of ['Add to playlist…', 'Add to favourites', 'Go to album']) check(lib.includes(want), `${width}: a library song's ⋯ has ${want} (${lib.join(' | ')})`);
  check(phone === (lib[0] === 'Move up'), `${width}: Move up is ${phone ? 'first in' : 'not in'} the ⋯`);
  check(lib.includes('Remove from queue') === phone, `${width}: Remove from queue is ${phone ? '' : 'not '}in the ⋯`);

  // Add to playlist… › the playlist: that song goes in by its track id
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to playlist…' }).click();
  await page.locator('dialog [data-ppi="0"]').click();
  await page.waitForTimeout(300);
  const add = state.hits.find((h) => h.m === 'POST' && h.v === '/playlists/c1/items');
  check(add && JSON.parse(add.body).track_ids?.join(',') === '2', `${width}: Add to playlist sends Song 2 (${add && add.body})`);

  const streamed = await openMenu(page, 3, phone);
  check(streamed.includes('Go to song page') && !streamed.includes('Add to playlist…'), `${width}: a streamed song's ⋯ has Go to song page (${streamed.join(' | ')})`);
  await close(page);

  const unknown = await openMenu(page, 2, phone);
  check(unknown.includes('Go to song page') && !unknown.includes('Add to playlist…'), `${width}: a library song the node didn't answer for still has a menu: Go to song page (${unknown.join(' | ')})`);
  await close(page);

  // The page drawn again while the menu still reads (Song 1 removed): no menu opens for the ⋯ that
  // went (before, it opened at the window's corner, its Move/Remove for the row's old place)
  if (!phone) {
    let go; hold = { wait: new Promise((r) => { go = r; }) };
    await page.locator('.list.queue .list-row').nth(1).locator('[data-qmore]').click();
    await page.waitForTimeout(300);
    await page.locator('.list.queue .list-row').nth(0).hover();
    await page.locator('[data-qrm="0"]').click();
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 3);
    go(); hold = null; await page.waitForTimeout(500);
    check(await page.locator('.popover.menu:not([hidden])').count() === 0, `${width}: a ⋯ whose row was drawn again while its menu read opens no menu`);
  }

  check(state.errors.length === 0, `${width}: no page errors ${state.errors.join('; ')}`);
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768, 412]) await run(browser, w); } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
