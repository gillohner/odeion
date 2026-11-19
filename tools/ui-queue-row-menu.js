// A Queue row's ⋯ is the song's own menu at every width, as on Songs: a library song's has Add to
// playlist…, Add to favourites and Go to album (its Add to playlist adds that song by its track id),
// a streamed song's has Go to song page. Desktop rows show the ⋯ (before, only phones did, and a
// queued song had to be found again on Songs to be added to a playlist); their Move up / Move down /
// Remove stay buttons on the row, not menu items. Phones keep those three first in the ⋯. A library
// song the node can't answer for still gets a menu (Go to song page). A ⋯ whose row is drawn again
// while its menu still reads opens nothing. Two ⋯ tapped quickly open only the last one's menu (the
// first's, slower, opened over it before), and the first ⋯ stops showing its progress. A phone's
// Move/Remove wait at most 1.5 s for the song's own items. An edit picked from the menu is for its
// song wherever it is by then (the queue changed while the menu was open: before, the song now at
// the old place was removed), and for none once it has gone; on a computer too, for a song with no
// page (its ⋯ then has Move/Remove).
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
const title = (page) => page.locator('.popover.menu:not([hidden]) .pop-title').first().textContent().catch(() => null);
const qmore = (page, n) => page.locator('.list.queue .list-row').nth(n).locator('[data-qmore]');
const tapMore = (page, n, touch) => (touch ? qmore(page, n).tap() : qmore(page, n).click());
const names = (page) => page.locator('.list.queue .list-row .name').evaluateAll((ns) => ns.map((n) => n.textContent.trim()));
// (a row's own Remove, pressed by the page as another tab's or device's change would, past the
// menu's click-outside guard: the queue changes while the menu stays open)
const removeRow = (page, n) => page.evaluate((k) => { const b = document.querySelector(`[data-qrm="${k}"]`); b.onclick.call(b, new MouseEvent('click')); }, n);
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

  // Two ⋯ tapped quickly: Song 2's still reads (its versions held), Song 3's opens at once; Song 2's
  // answer then opens nothing over it, and Song 2's ⋯ shows its progress only until the second tap
  {
    let go; hold = { wait: new Promise((r) => { go = r; }) };
    await tapMore(page, 1, phone);
    await page.waitForTimeout(300);
    check(await qmore(page, 1).getAttribute('aria-busy') === 'true', `${width}: a ⋯ shows its progress while its menu reads`);
    check(await page.locator('.popover.menu:not([hidden])').count() === 0, `${width}: no menu yet while it reads`);
    await tapMore(page, 2, phone);
    await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor();
    check(await title(page) === 'Song 3', `${width}: the second ⋯ opens its menu at once (${await title(page)})`);
    check(await qmore(page, 1).getAttribute('aria-busy') !== 'true', `${width}: the first ⋯ stops showing progress once another is tapped`);
    go(); hold = null; await page.waitForTimeout(600);
    check(await title(page) === 'Song 3', `${width}: the slower first ⋯ opens nothing over the second's menu (${await title(page)})`);
    await close(page);
  }

  // A song whose menu reads slowly: a phone's Move/Remove open within 1.5 s (without the song's own
  // items); a desktop's ⋯ waits for the song's menu (its Move/Remove are on the row)
  {
    let go; hold = { wait: new Promise((r) => { go = r; }) };
    const t0 = Date.now();
    await tapMore(page, 1, phone);
    if (phone) {
      await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor({ timeout: 4000 }).catch(() => {});
      const took = Date.now() - t0; const items = await menuItems(page);
      check(took < 2500 && items.includes('Move up') && items.includes('Remove from queue'), `${width}: Move/Remove open within 1.5 s while the song's menu reads (${took} ms: ${items.join(' | ')})`);
      go(); hold = null; await page.waitForTimeout(600);
      check((await menuItems(page)).join('|') === items.join('|'), `${width}: the song's late menu doesn't replace the open one`);
    } else {
      await page.waitForTimeout(2000);
      check(await page.locator('.popover.menu:not([hidden])').count() === 0, `${width}: a desktop ⋯ waits for the song's menu`);
      go(); hold = null;
      const items = await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor().then(() => menuItems(page));
      check(items.includes('Add to playlist…'), `${width}: then the song's own menu opens (${items.join(' | ')})`);
    }
    await close(page);
  }

  // The page drawn again while the menu still reads (Song 1 removed): no menu opens for the ⋯ that
  // went (before, it opened at the window's corner, its Move/Remove for the row's old place)
  {
    let go; hold = { wait: new Promise((r) => { go = r; }) };
    await tapMore(page, 1, phone);
    await page.waitForTimeout(300);
    await removeRow(page, 0);
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 3);
    go(); hold = null; await page.waitForTimeout(phone ? 1800 : 500);
    check(await page.locator('.popover.menu:not([hidden])').count() === 0, `${width}: a ⋯ whose row was drawn again while its menu read opens no menu`);
  }

  // The queue changes while a phone's menu is open (Song 2, before Song 3, removed): its Remove
  // removes Song 3 where it is now (before, the song at Song 3's old place: the streamed one); a
  // song gone from the queue meanwhile is left alone
  if (phone) {
    check((await names(page)).join('|') === 'Song 2|Song 3|Streamed song', `${width}: queue before (${(await names(page)).join(' | ')})`);
    await openMenu(page, 1, phone);
    check(await title(page) === 'Song 3', `${width}: Song 3's menu is open`);
    await removeRow(page, 0);
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 2);
    check(await page.locator('.popover.menu:not([hidden])').count() === 1, `${width}: the menu stays open while the queue changes`);
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from queue' }).click();
    await page.waitForTimeout(400);
    check((await names(page)).join('|') === 'Streamed song', `${width}: Remove from a menu opened before the change removes its own song (${(await names(page)).join(' | ')})`);
    await openMenu(page, 0, phone);
    await removeRow(page, 0);
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 0);
    if (await page.locator('.popover.menu:not([hidden])').count()) {
      await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from queue' }).click();
      await page.waitForTimeout(400);
      check(await page.getByText('that song isn’t in the queue any more').count() > 0, `${width}: Remove for a song gone meanwhile says so and changes nothing`);
    } else check(false, `${width}: the menu stays open while its song is removed`);
  }
  check(state.errors.length === 0, `${width}: no page errors ${state.errors.join('; ')}`);
  await context.close();
}

// A computer's queue of songs with no song page (nothing else for their ⋯): Move up / Move down /
// Remove are its menu. The queue changes while it is open: they act on its song where it is now.
async function runNoPage(browser, width) {
  const { context, page, state } = await fx.create(browser, width, { tracks, collections: [fx.summary] });
  const plain = ['A', 'B', 'C'].map((n, k) => ({ id: 20 + k, uri: `x-other:${n}`, title: `Plain ${n}`, artist: 'Someone', duration_ms: 60000, stream: `https://example.com/${n}.flac` }));
  await context.addInitScript(([q, a]) => { localStorage.setItem('pm.browserQueue', q); localStorage.setItem('pm.browserAt', a); }, [JSON.stringify(plain), JSON.stringify({ index: 0, id: plain[0].id, pos: 0 })]);
  await fx.go(page, '/queue');
  await page.locator('.list.queue .list-row').nth(2).waitFor();
  const items = await openMenu(page, 1, false);
  check(items.join('|') === 'Move up|Move down|Remove from queue', `${width}: a song with no page has Move/Remove in its ⋯ (${items.join(' | ')})`);
  await removeRow(page, 0);
  await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 2);
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Move up' }).click();
  await page.waitForTimeout(400);
  check(await page.getByText('it’s first in the queue now').count() > 0 && (await names(page)).join('|') === 'Plain B|Plain C', `${width}: Move up for a song that became first says so and moves nothing (${(await names(page)).join(' | ')})`);
  await close(page);
  await openMenu(page, 1, false);
  await removeRow(page, 0);
  await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 1);
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from queue' }).click();
  await page.waitForTimeout(400);
  check((await names(page)).length === 0, `${width}: Remove from a menu opened before the change removes its own song (${(await names(page)).join(' | ')})`);
  check(state.errors.length === 0, `${width}: no page errors ${state.errors.join('; ')}`);
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768, 412]) await run(browser, w); await runNoPage(browser, 1024); } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
