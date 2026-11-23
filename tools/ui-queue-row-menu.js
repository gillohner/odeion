// A Queue row's ⋯ is the song's own menu at every width, as on Songs: a library song's has Add to
// playlist…, Add to favourites and Go to album (its Add to playlist adds that song by its track id),
// a streamed song's has Go to song page. Desktop rows show the ⋯ (before, only phones did, and a
// queued song had to be found again on Songs to be added to a playlist); their Move up / Move down /
// Remove stay buttons on the row, not menu items. Phones keep those three first in the ⋯. A library
// song the node can't answer for still gets a menu (Go to song page). A ⋯ whose queue is drawn again
// while its menu still reads opens its menu on the same song's new ⋯, with Move up / Move down for
// where the song is now (before, nothing opened); none opens once that song has gone (a toast says
// so), nor once another page was opened (its answer in while that page still loads, the Queue's
// rows still in it). Two ⋯ tapped
// quickly open only the last one's menu (the
// first's, slower, opened over it before), and the first ⋯ stops showing its progress. A ⋯ waits
// at most 1.5 s for the song's own items (then it opens with Go to song page, and a phone's
// Move/Remove). An edit picked from the menu is for its
// song wherever it is by then (the queue changed while the menu was open: before, the song now at
// the old place was removed), and for none once it has gone; on a computer too, for a song with no
// page (its ⋯ then has Move/Remove). On a node zone: the zone the page shows changes while the menu
// is open (the page follows the host to another device): Play next, Add to queue and Start radio send
// nothing (before, they added to or replaced the other device's queue); the queue changes while a
// phone's menu is open: Move/Remove ask the node for that song's own queue entry. Start radio picked
// before the page follows to another device turns the radio on for the queue it replaced, and the
// toast names that device (before, that queue was replaced and its radio left off without a word).
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
  for (const want of ['Add to collection…', 'Add to Bookmarks', 'Go to album']) check(lib.includes(want), `${width}: a library song's ⋯ has ${want} (${lib.join(' | ')})`);
  check(phone === (lib[0] === 'Move up'), `${width}: Move up is ${phone ? 'first in' : 'not in'} the ⋯`);
  check(lib.includes('Remove from queue') === phone, `${width}: Remove from queue is ${phone ? '' : 'not '}in the ⋯`);

  // Add to playlist… › the playlist: that song goes in by its track id
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to collection…' }).click();
  await page.locator('dialog [data-ppi="0"]').click();
  await page.waitForTimeout(300);
  const add = state.hits.find((h) => h.m === 'POST' && h.v === '/playlists/c1/items');
  check(add && JSON.parse(add.body).track_ids?.join(',') === '2', `${width}: Add to collection sends Song 2 (${add && add.body})`);

  const streamed = await openMenu(page, 3, phone);
  check(streamed.includes('Go to song page') && !streamed.includes('Add to collection…'), `${width}: a streamed song's ⋯ has Go to song page (${streamed.join(' | ')})`);
  await close(page);

  const unknown = await openMenu(page, 2, phone);
  check(unknown.includes('Go to song page') && !unknown.includes('Add to collection…'), `${width}: a library song the node didn't answer for still has a menu: Go to song page (${unknown.join(' | ')})`);
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

  // A song whose menu reads slowly: the ⋯ opens within 1.5 s without the song's own items, with Go
  // to song page (and a phone's Move/Remove); before, a desktop's ⋯ waited as long as the reads did
  {
    let go; hold = { wait: new Promise((r) => { go = r; }) };
    const t0 = Date.now();
    await tapMore(page, 1, phone);
    await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor({ timeout: 4000 }).catch(() => {});
    const took = Date.now() - t0; const items = await menuItems(page);
    check(took < 2500 && items.includes('Go to song page') && items.includes('Remove from queue') === phone, `${width}: the ⋯ opens within 1.5 s while the song's menu reads (${took} ms: ${items.join(' | ')})`);
    check(await qmore(page, 1).getAttribute('aria-busy') !== 'true', `${width}: and stops showing its progress`);
    go(); hold = null; await page.waitForTimeout(600);
    check((await menuItems(page)).join('|') === items.join('|'), `${width}: the song's late menu doesn't replace the open one`);
    await close(page);
  }

  // The page drawn again while the menu still reads (Song 1 removed, Song 2 moves up to the first
  // row): the menu opens on Song 2's new ⋯, Move up gone with it (before, no menu opened at all;
  // before that, one at the window's corner with Move/Remove for the row's old place)
  const atMore = (n) => page.evaluate((k) => {
    const m = document.querySelector('.popover.menu:not([hidden])'); const b = document.querySelectorAll('.list.queue .list-row [data-qmore]')[k];
    if (!m || !b) return null; const r = b.getBoundingClientRect(); const t = m.getBoundingClientRect().top;
    return Math.abs(t - Math.min(window.innerHeight - m.offsetHeight - 8, r.bottom + 6)) <= 1;
  }, n);
  {
    let go; hold = { wait: new Promise((r) => { go = r; }) };
    await tapMore(page, 1, phone);
    await page.waitForTimeout(300);
    await removeRow(page, 0);
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 3);
    go(); hold = null;
    await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor({ timeout: 2500 }).catch(() => {});
    const its = await menuItems(page);
    check(await title(page) === 'Song 2' && its.includes('Add to collection…'), `${width}: a ⋯ whose queue was drawn again while its menu read opens the song's menu (${await title(page)}: ${its.join(' | ')})`);
    check(!its.includes('Move up') && its.includes('Move down') === phone, `${width}: with Move up / Move down for where the song is now (${its.join(' | ')})`);
    if (!phone) check(await atMore(0), `${width}: at the song's new ⋯`);
    await page.screenshot({ path: path.join(shots, `queue-menu-redrawn-${width}.png`) });
    await close(page);
  }
  // The same, its read slower than 1.5 s (Song 2 moved down meanwhile, then back): the menu without
  // the song's own items opens on its new ⋯, with Move up and Move down
  {
    let go; hold = { wait: new Promise((r) => { go = r; }) };
    const t0 = Date.now();
    await tapMore(page, 0, phone);
    await page.waitForTimeout(300);
    await page.evaluate(() => { const b = document.querySelector('[data-qdown="0"]'); b.onclick.call(b, new MouseEvent('click')); });
    await page.waitForFunction(() => (document.querySelector('.list.queue .list-row .name') || {}).textContent === 'Song 3');
    await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor({ timeout: 2500 }).catch(() => {});
    const took = Date.now() - t0; const its = await menuItems(page);
    check(took < 2500 && await title(page) === 'Song 2' && its.includes('Go to song page'), `${width}: a slow ⋯ whose queue was drawn again opens within 1.5 s (${took} ms, ${await title(page)}: ${its.join(' | ')})`);
    check(its.includes('Move up') === phone && its.includes('Move down') === phone, `${width}: its Move up / Move down for the song's new place (${its.join(' | ')})`);
    if (!phone) check(await atMore(1), `${width}: at the song's new ⋯`);
    go(); hold = null; await page.waitForTimeout(400);
    await close(page);
    await page.evaluate(() => { const b = document.querySelector('[data-qup="1"]'); b.onclick.call(b, new MouseEvent('click')); });
    await page.waitForFunction(() => (document.querySelector('.list.queue .list-row .name') || {}).textContent === 'Song 2');
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
  // The song itself removed while its menu reads: no menu opens, and a toast says why
  if (!phone) {
    let go; hold = { wait: new Promise((r) => { go = r; }) };
    await tapMore(page, 0, phone);
    await page.waitForTimeout(300);
    await removeRow(page, 0);
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 2);
    go(); hold = null; await page.waitForTimeout(1800);
    check(await page.locator('.popover.menu:not([hidden])').count() === 0, `${width}: a ⋯ whose song left the queue while its menu read opens no menu`);
    check(await page.getByText('Song 2 isn’t in the queue any more').count() > 0, `${width}: and says the song isn’t in the queue any more`);
  }
  // Another page opened while a ⋯'s menu reads, its answer in while that page still loads (the
  // Queue's rows still drawn): no menu opens over it. Before, the late answer opened the Queue song's
  // menu over the page being opened.
  {
    // (the ⋯'s answer is held in the page and let go in the same step as the click on Sources, so
    // it is handled within the page's 120 ms loading placeholder however slow the runner: an answer
    // sent from here could come after it, and the case would then fail on a correct app)
    await page.addInitScript(() => {
      const f = window.fetch;
      window.fetch = function (u, o) {
        const held = window.versionsHeld;
        if (!held || !/\/library\/tracks\/2\/versions/.test(String((u && u.url) || u))) return f.call(this, u, o);
        window.versionsHeld = null;
        return held.then(() => new Response('[]', { status: 200, headers: { 'content-type': 'application/json' } }));
      };
    });
    await fx.go(page, '/queue');
    await page.locator('.list.queue .list-row').nth(1).waitFor();
    await page.evaluate(() => { window.versionsHeld = new Promise((r) => { window.releaseVersions = r; }); });
    // (Sources waits for its list before it draws: the Queue's rows stay until the page's 120 ms
    // loading placeholder)
    let goSources; const sourcesHeld = new Promise((r) => { goSources = r; });
    await context.route('**/v1/sources', async (r) => { await sourcesHeld; r.fallback(); });
    await tapMore(page, 1, phone);
    await page.waitForTimeout(300);
    check(await page.locator('.popover.menu:not([hidden])').count() === 0, `${width}: no menu yet while the ⋯ reads`);
    // (the case holds only while its answer is handled with the Queue's rows still drawn: after the
    // placeholder the ⋯ is gone, and no menu could open without the fix either. Whether its ⋯ is
    // still in the page is read the moment it stops showing its progress, before any timer can run)
    const more = await qmore(page, 1).elementHandle();
    check(await more.getAttribute('aria-busy') === 'true', `${width}: the ⋯ shows its progress while it reads`);
    await more.evaluate((b) => { window.qmoreSettled = null; new MutationObserver((_, o) => { if (b.getAttribute('aria-busy') !== 'true') { window.qmoreSettled = b.isConnected; o.disconnect(); } }).observe(b, { attributes: true, attributeFilter: ['aria-busy'] }); });
    // (the ⋯'s reads before it, sent from here, all in: the held one is the last it waits for)
    await page.waitForFunction(() => window.versionsHeld === null, null, { timeout: 5000 });
    await page.evaluate(() => { document.querySelector('.nav-item[data-view="sources"]').click(); window.releaseVersions(); });
    await page.waitForFunction(() => window.qmoreSettled !== null, null, { timeout: 3000 }).catch(() => {});
    check(await page.evaluate(() => window.qmoreSettled) === true, `${width}: the ⋯'s answer is handled while the Queue's rows are still in the page (${await page.evaluate(() => window.qmoreSettled)})`);
    await page.waitForTimeout(1800);
    check(await page.locator('.popover.menu:not([hidden])').count() === 0, `${width}: a ⋯ whose answer comes in after another page was opened opens no menu (${await title(page)})`);
    await page.screenshot({ path: path.join(shots, `queue-menu-other-page-${width}.png`) });
    goSources(); await context.unroute('**/v1/sources');
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

// A node zone, Kitchen (z1), and the Living room (z2); the queue's entries have the node's ids
async function runZone(browser, width) {
  const phone = width < 600;
  const items = tracks.map((t, k) => ({ id: 101 + k, uri: `track:${t.id}`, track_id: t.id, title: t.title, artist: 'Sefa', album: 'Journey', duration_ms: 60000 }));
  const items2 = [{ ...items[0], id: 201 }];
  const Z = (id, name, q) => ({ id, name, output: 'upnp:' + id, transport: 'paused', now_playing: { item: q[0], position_ms: 1000, duration_ms: 60000 }, queue: q, queue_index: 0, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 0, volume: { percent: 40 } });
  const z1 = Z('z1', 'Kitchen', items); const z2 = Z('z2', 'Living room', items2);
  const zf = { ...fixtures, '/zones': { body: [z1, z2] }, '/zones/z1': { body: z1 }, '/zones/z2': { body: z2 }, '/zones/z1/queue': { body: items }, '/zones/z2/queue': { body: items2 },
    '/library/tracks/2': { body: { ...tracks[1], canonical: { kind: 'recording', mbid: fx.track.mbid } } }, // (matched: it has Start radio)
    '/outputs': { body: [{ id: 'upnp:z1', name: 'Kitchen', kind: 'upnp', caps: {} }, { id: 'upnp:z2', name: 'Living room', kind: 'upnp', caps: {} }] } };
  const { context, page, state } = await fx.create(browser, width, { fixtures: zf, zone: 'z1', tracks, collections: [fx.summary], ...(phone ? { mobile: true } : {}) });
  let ws = null;
  await context.routeWebSocket(/\/v1\/events/, (w) => { ws = w; w.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') w.send(JSON.stringify({ type: 'pong', id: j.id })); }); w.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [z1, z2] })); });
  let seedHold = null;
  await context.route(/\/v1\/zones\/z[12]\/(queue(\/\d+)?|radio|play|transport)$/, async (r) => {
    // (answered here, so noted here: the fixture's own log never sees them)
    const q = r.request(); if (q.method() === 'GET') return r.fallback();
    const v = new URL(q.url()).pathname.slice(3);
    state.hits.push({ v, m: q.method(), q: '', body: q.postData(), at: Date.now() });
    if (seedHold && q.method() === 'POST' && v === '/zones/z1/queue') await seedHold.wait;
    return r.fulfill({ status: 200, contentType: 'application/json', body: '{}' }).catch(() => {});
  });
  await fx.go(page, '/queue');
  await page.locator('.list.queue .list-row').nth(2).waitFor();
  const sent = (n) => state.hits.slice(n).filter((h) => h.m !== 'GET').map((h) => `${h.m} ${h.v}`);

  // The page follows the host to the Living room while a song's menu is open: its items send nothing
  for (const what of ['Play next', 'Add to queue', 'Start radio']) {
    await openMenu(page, 1, phone);
    const its = await menuItems(page);
    check(its.includes(what), `${width}: Kitchen: Song 2's ⋯ has ${what} (${its.join(' | ')})`);
    zf['/presence'] = { body: [{ mine: true, key: 'host', name: 'Host', zone: { id: 'z2', name: 'Living room' }, your_zone: 'z1' }] };
    zf['/me/presence'] = { body: { visible: 'hidden' } };
    ws.send(JSON.stringify({ type: 'zone_removed', seq: 3, zone: 'z9' })); // (another zone: asks /presence again)
    await page.waitForFunction(() => /Living room/.test((document.querySelector('.page-head') || {}).textContent || ''));
    check(await page.locator('.popover.menu:not([hidden])').count() === 1, `${width}: the menu is still open after the page followed to the Living room`);
    const n = state.hits.length;
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: what }).first().click();
    await page.waitForTimeout(500);
    check(sent(n).length === 0, `${width}: ${what} for a Kitchen song sends nothing to the Living room (${sent(n).join(', ')})`);
    check(await page.getByText('Nothing changed: the Queue now shows Living room').count() > 0, `${width}: and says why`);
    // back to the Kitchen for the next item
    await close(page);
    delete zf['/presence']; delete zf['/me/presence'];
    await page.evaluate(() => { localStorage.setItem('pm.zone', 'z1'); });
    await page.reload(); await page.locator('.list.queue .list-row').nth(1).waitFor();
    await page.waitForFunction(() => /Kitchen/.test((document.querySelector('.page-head') || {}).textContent || ''));
  }
  // The queue changes while a phone's menu is open (another device removes Song 1, the socket says
  // so): Remove asks the node to remove Song 3's own entry (103), not the one at its old place
  if (phone) {
    await openMenu(page, 2, phone);
    check(await title(page) === 'Song 3', `${width}: Kitchen: Song 3's menu is open`);
    const q2 = items.slice(1); zf['/zones/z1/queue'].body = q2;
    ws.send(JSON.stringify({ type: 'zone', seq: 2, state: { ...z1, queue: q2, queue_version: 2 } }));
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 2);
    const n = state.hits.length;
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from queue' }).click();
    await page.waitForTimeout(500);
    check(sent(n).join(',') === 'DELETE /zones/z1/queue/103', `${width}: Remove after the queue changed asks for Song 3's own entry (${sent(n).join(', ')})`);
    // (Song 1 comes back first meanwhile: Move up moves Song 3 from its new place, the third, to the second)
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 2);
    await openMenu(page, 1, phone);
    check(await title(page) === 'Song 3', `${width}: Kitchen: Song 3's menu is open again`);
    zf['/zones/z1/queue'].body = items;
    ws.send(JSON.stringify({ type: 'zone', seq: 3, state: { ...z1, queue: items, queue_version: 3 } }));
    await page.waitForFunction(() => document.querySelectorAll('.list.queue .list-row').length === 3);
    const m = state.hits.length;
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Move up' }).click();
    await page.waitForTimeout(500);
    const put = state.hits.slice(m).filter((h) => h.m !== 'GET');
    check(put.map((h) => `${h.m} ${h.v} ${h.body}`).join(',') === 'PUT /zones/z1/queue/103 {"to_index":1}', `${width}: Move up after the queue changed moves Song 3's own entry from where it is now (${put.map((h) => `${h.m} ${h.v} ${h.body}`).join(', ')})`);
  }
  // Start radio picked in the Kitchen; the page follows the host to the Living room while the song
  // is on its way: the Kitchen's queue was replaced, so its radio goes on too, and the toast says where
  {
    await page.waitForFunction(() => /Kitchen/.test((document.querySelector('.page-head') || {}).textContent || ''));
    let go; seedHold = { wait: new Promise((r) => { go = r; }) };
    await openMenu(page, 1, phone);
    const n = state.hits.length;
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Start radio' }).click();
    for (let t = 0; t < 40 && !sent(n).includes('POST /zones/z1/queue'); t++) await page.waitForTimeout(50);
    zf['/presence'] = { body: [{ mine: true, key: 'host', name: 'Host', zone: { id: 'z2', name: 'Living room' }, your_zone: 'z1' }] };
    zf['/me/presence'] = { body: { visible: 'hidden' } };
    ws.send(JSON.stringify({ type: 'zone_removed', seq: 9, zone: 'z9' }));
    await page.waitForFunction(() => /Living room/.test((document.querySelector('.page-head') || {}).textContent || ''));
    go(); seedHold = null; await page.waitForTimeout(800);
    const radio = state.hits.slice(n).filter((h) => /\/radio$/.test(h.v)).map((h) => `${h.m} ${h.v} ${h.body}`);
    check(sent(n)[0] === 'POST /zones/z1/queue' && radio.join(',') === 'PUT /zones/z1/radio {"on":true}', `${width}: Start radio, then the page follows to the Living room: the Kitchen's radio goes on (${sent(n).join(', ')}; ${radio.join(', ')})`);
    check(await page.getByText('Radio from “Song 2” on Kitchen: similar music follows').count() > 0, `${width}: and the toast names the Kitchen`);
  }
  check(state.errors.length === 0, `${width}: no page errors ${state.errors.join('; ')}`);
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768, 412]) await run(browser, w); await runNoPage(browser, 1024); for (const w of [1280, 412]) await runZone(browser, w); } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
