// A read that failed is said, with Try again in its place, and never looks like "nothing here".
// Real candidate assets on the fixture origin (tools/ui-fixture.js); a read answers 500, then works:
//   * the Queue of a node zone whose queue could not be read says so: not "The queue is empty", and
//     no "Shuffle my music" over a queue that may be full; Try again shows the songs;
//   * Sources without any list says so above the page (Your library stays): not a node with no
//     services; Try again shows them;
//   * Spotify's devices that could not be read are not "No Spotify device online";
//   * a smart list, a person's page, a shared collection, an album page and Downloads keep Back or
//     their head and get Try again (they had the words alone, and no way on but leaving the page);
//   * every one is announced (role=alert), Try again draws the page, and its button is 44 px high
//     on a phone.
//   node tools/ui-failed-reads.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'failed-reads-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const t0 = Date.now(); for (;;) { const v = await fn().catch(() => null); if (v) return v; if (Date.now() - t0 > ms) return v; await sleep(40); } };
const DOWN = { status: 500, body: { error: 'the node is busy' } };

const item = (n) => ({ id: n, uri: `track:${n}`, track_id: n, title: `Song ${n}`, artist: 'Sefa', album: 'Journey', duration_ms: 240000 });
const zone = { id: 'kitchen', name: 'Kitchen', output: 'upnp:kitchen', transport: 'stopped', queue: [item(1), item(2)], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: null };
const outputs = [{ id: 'upnp:kitchen', name: 'kitchen', kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }];

// What the failed notice on the page says and offers
const notice = (page) => page.evaluate(() => {
  const n = document.querySelector('#main [data-failed]'); if (!n) return null;
  const b = n.querySelector('[data-retry]'); const r = b ? b.getBoundingClientRect() : null;
  return { text: n.textContent.replace(/\s+/g, ' ').trim(), alert: n.getAttribute('role') === 'alert', retry: b ? b.textContent.trim() : '', h: r ? Math.round(r.height) : 0, back: !!document.querySelector('#main [data-back]'), head: !!document.querySelector('#main .page-head h2') };
});
const pageText = (page) => page.evaluate(() => document.getElementById('main').innerText);

(async () => {
  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const at = `${width}px`; const phone = width < 600;
    // ---- the queue of a node zone -------------------------------------------------------------
    {
      const { context, page, state } = await fx.create(browser, width, { mobile: phone, zone: 'kitchen', fixtures: { '/zones': { body: [zone] }, '/outputs': { body: outputs }, '/zones/kitchen/queue': DOWN } });
      await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
      await fx.go(page, '/queue');
      const n = await until(() => notice(page));
      const text = await pageText(page);
      check(!!n && /Couldn.t read the queue/.test(n.text) && /the node is busy/.test(n.text) && n.alert && n.retry === 'Try again', `${at} queue: a queue that could not be read says so, with Try again (${n ? n.text : 'no notice'})`);
      check(!/The queue is empty/.test(text) && !(await page.$('#qmine')), `${at} queue: it is not called empty, and nothing offers to shuffle over it`);
      check(!!n && n.head, `${at} queue: the page keeps its head`);
      if (phone) check(!!n && n.h >= 44, `${at} queue: Try again is ${n ? n.h : 0} px high`);
      await page.screenshot({ path: path.join(SHOTS, `queue-failed-${width}.png`) });
      state.fixtures['/zones/kitchen/queue'] = { body: zone.queue };
      await page.click('#main [data-failed] [data-retry]', { timeout: 1500 }).catch(() => {});
      const rows = await until(() => page.evaluate(() => document.querySelectorAll('#main [data-qrow]').length === 2 && !document.querySelector('#main [data-failed]')));
      check(rows, `${at} queue: Try again shows the songs`);
      check(!state.errors.length, `${at} queue: no page error (${state.errors.join(' | ') || 'none'})`);
      await context.close();
    }
    // ---- an empty queue is still an empty queue -----------------------------------------------
    {
      const empty = { ...zone, queue: [] };
      const { context, page, state } = await fx.create(browser, width, { mobile: phone, zone: 'kitchen', fixtures: { '/zones': { body: [empty] }, '/outputs': { body: outputs }, '/zones/kitchen/queue': { body: [] } } });
      await fx.go(page, '/queue');
      const said = await until(async () => /The queue is empty/.test(await pageText(page)));
      check(said && !(await notice(page)), `${at} queue: a queue that was read and is empty says "The queue is empty"`);
      check(!state.errors.length, `${at} empty queue: no page error (${state.errors.join(' | ') || 'none'})`);
      await context.close();
    }
    // ---- Sources ------------------------------------------------------------------------------
    {
      const { context, page, state } = await fx.create(browser, width, { mobile: phone, fixtures: { '/sources': DOWN } });
      await fx.go(page, '/sources');
      const n = await until(() => notice(page));
      check(!!n && /Couldn.t read the services/.test(n.text) && n.alert && n.retry === 'Try again' && n.head, `${at} Sources: no list is said, under the page's head, with Try again (${n ? n.text : 'no notice'})`);
      const tabs0 = await page.evaluate(() => [...document.querySelectorAll('#main [data-srctab]')].map((b) => b.dataset.srctab));
      check(tabs0.join() === 'library', `${at} Sources: Your library is still there, and no service is drawn from a list that was not read (${tabs0.join() || 'no tabs'})`);
      await page.screenshot({ path: path.join(SHOTS, `sources-failed-${width}.png`) });
      state.fixtures['/sources'] = { body: [{ id: 'tidal', name: 'TIDAL', description: '', status: 'needs_login', enabled: true }] };
      await page.click('#main [data-failed] [data-retry]', { timeout: 1500 }).catch(() => {});
      const tabs = await until(() => page.evaluate(() => !!document.querySelector('#main [data-srctab="tidal"]') && !document.querySelector('#main [data-failed]')));
      check(tabs, `${at} Sources: Try again shows the services`);
      check(!state.errors.length, `${at} Sources: no page error (${state.errors.join(' | ') || 'none'})`);
      await context.close();
    }
    // ---- pages that had the words alone -------------------------------------------------------
    for (const [name, address, read, healed, ready, keeps, more] of [
      ['a smart list', '/smart/fresh', '/smart/fresh/tracks', { body: { playlist: { id: 'fresh', name: 'Fresh' }, tracks: [] } }, '#main .page-head h2, #main .hero', 'back'],
      ['a person', '/person/zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o', '/profiles/zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o', null, null, 'back'],
      ['a shared list', '/shared/zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o', '/shared/zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o/playlists', { body: [] }, '#main .page-head h2, #main .empty, #main h2', 'back'],
      ['an album', '/album/lib-7', '/item/album/lib-7', null, null, 'back'],
      ['Downloads', '/downloads', '/offline', null, null, 'head', { offline: true }], // (the app's own page, offline)
    ]) {
      const { context, page, state } = await fx.create(browser, width, { mobile: phone, ...(more || {}), fixtures: { [read]: DOWN } });
      await fx.go(page, address);
      const n = await until(() => notice(page));
      const asked = state.hits.filter((h) => h.v === read).length;
      check(!!n && /the node is busy/.test(n.text) && n.alert && n.retry === 'Try again' && (keeps === 'back' ? n.back : n.head), `${at} ${name}: says what went wrong, announced, with Try again and ${keeps === 'back' ? 'Back' : 'its head'} (${n ? n.text : `no notice; asked ${asked}×`})`);
      if (phone && n) check(n.h >= 44, `${at} ${name}: Try again is ${n.h} px high`);
      if (width === 1280 && name === 'an album') await page.screenshot({ path: path.join(SHOTS, 'album-failed-1280.png') });
      if (n) {
        // Try again asks again (still failing: the notice is back, after one more read)
        await page.click('#main [data-failed] [data-retry]', { timeout: 1500 }).catch(() => {});
        const again = await until(async () => state.hits.filter((h) => h.v === read).length > asked && !!(await notice(page)));
        check(again, `${at} ${name}: Try again asks again (${state.hits.filter((h) => h.v === read).length - asked} more)`);
        if (healed) {
          state.fixtures[read] = healed;
          await page.click('#main [data-failed] [data-retry]', { timeout: 1500 }).catch(() => {});
          const drawn = await until(() => page.evaluate((r) => !document.querySelector('#main [data-failed]') && !!document.querySelector(r), ready));
          check(drawn, `${at} ${name}: once it answers, Try again draws the page`);
        }
      }
      check(!state.errors.length, `${at} ${name}: no page error (${state.errors.join(' | ') || 'none'})`);
      await context.close();
    }
    // ---- a page whose code gives up: the same notice, with its old handle ---------------------
    {
      const { context, page, state } = await fx.create(browser, width, { mobile: phone, fixtures: { '/library/albums': DOWN } });
      await fx.go(page, '/albums');
      const n = await until(() => notice(page));
      check(!!n && n.alert && n.retry === 'Try again' && !!(await page.$('#main [data-failed] #navretry')), `${at} Albums: the page's own failure is the same notice (${n ? n.text : 'no notice'})`);
      state.fixtures['/library/albums'] = { body: [] };
      await page.click('#navretry', { timeout: 1500 }).catch(() => {});
      check(await until(() => page.evaluate(() => !document.querySelector('#main [data-failed]'))), `${at} Albums: Try again draws the page`);
      check(!state.errors.length, `${at} Albums: no page error (${state.errors.join(' | ') || 'none'})`);
      await context.close();
    }
  }
  // ---- Spotify's devices ----------------------------------------------------------------------
  {
    const spotify = { id: 'spotify', name: 'Spotify', description: '', status: 'ready', enabled: true };
    const { context, page, state } = await fx.create(browser, 1280, { fixtures: { '/sources': { body: [spotify] }, '/sources/spotify/status': { body: { signed_in: true } }, '/sources/spotify/signin': { body: { ways: [], in_use: { way: 'app' }, details: { status: { signed_in: true } } } }, '/sources/spotify/collection': { body: { items: [] } }, '/sources/spotify/playlists': { body: [] }, '/sources/spotify/devices': DOWN } });
    await fx.go(page, '/sources/spotify');
    const got = await until(() => page.evaluate(() => { const n = document.querySelector('#main [data-failed]'); return n ? n.textContent.replace(/\s+/g, ' ').trim() : (document.querySelector('#spotfav') ? 'drawn without a notice' : null); }), 6000);
    check(got !== null, 'Spotify: the signed-in page is drawn');
    if (got !== null) {
      check(/Couldn.t read your Spotify devices/.test(got), `Spotify: devices that could not be read say so (${got})`);
      check(!/No Spotify device online/.test(await pageText(page)), 'Spotify: not "No Spotify device online"');
      state.fixtures['/sources/spotify/devices'] = { body: [{ name: 'Phone', type: 'Smartphone', is_active: true }] };
      await page.click('#spdevretry', { timeout: 1500 }).catch(() => {});
      check(await until(async () => /Phone/.test(await pageText(page)) && !(await notice(page))), 'Spotify: Try again shows the devices');
    }
    check(!state.errors.length, `Spotify: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
