// Every click shows progress and fires once; navigation answers at once (TASK-tablet-desktop-layout-
// and-loaders-1104, part B; owner, 2026-11-04: "every click shows loader and fires once"). Real candidate
// assets on the fixture origin (tools/ui-fixture.js), answers delayed on purpose:
//   * busy(): any handler that returns a promise marks its control busy within 200 ms (aria-busy,
//     aria-disabled), a spinner after 150 ms, three quick clicks run it once, and the control comes back
//     afterwards; a failure shows its message; focus stays; a change still runs; data-repeat opts out;
//     the same click reaching a row's own handler is not dropped; `return false` still cancels.
//   * real controls with every change delayed 500 ms: the heart on a song row and Add to favourites in
//     its menu, pressed three times, send one request.
//   * navigation: the bar along the top at once, the page's skeleton before its answer, "Still
//     loading…" after 2 s, an error with Try again (which retries), and A → B pressed in quick
//     succession ends on B with nothing of A drawn. Playlists → Genres and an album → Artists pass on
//     the page's own state.view checks too; tag A → A + B (Back then Forward) is the case only the
//     navigation generation catches (an album page covers its tiles at once, so album → album can't
//     be pressed that fast).
// Screenshots of the loading, error and busy states go to the shots folder.
//   node tools/ui-busy.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'busy-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const albums = [{ ...fx.track.album, id: 7, title: 'Album Seven', artist: 'Sefa', artists: fx.artists, year: 2020, tracks: 1 }, { ...fx.track.album, id: 8, title: 'Album Eight', artist: 'Sefa', artists: fx.artists, year: 2021, tracks: 1 }];
const page = (id, title) => ({ kind: 'album', id: `lib-${id}`, uri: null, target: {}, title, artist: { name: 'Sefa', id: 'lib-11' }, album: null, year: 2020, image: null, source: 'library', source_id: String(id), plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false,
  songs: [{ no: 1, title: `${title}, song one`, artist: 'Sefa', duration_ms: 200000, uri: 'track:1', track_id: 1, id: 'lib-1', page: 'lib-1' }], total_ms: 200000, albums: [], versions: [], also_on: [], about: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library' });
const extras = { body: { collections: [], related: [], concerts: [], similar: [] } };
const FIXTURES = { '/library/albums': { body: albums }, '/item/album/lib-7': { body: page(7, 'Album Seven') }, '/item/album/lib-8': { body: page(8, 'Album Eight') }, '/item/album/lib-7/extras': extras, '/item/album/lib-8/extras': extras };

(async () => {
  const browser = await chromium.launch();
  const slow = {}; // path → ms for reads; every change waits 500 ms
  const run = await fx.create(browser, 1280, { fixtures: FIXTURES, delay: (v, m) => (m !== 'GET' ? 500 : slow[v] || 0) });
  const { page: p, state } = run;
  await fx.go(p, '/tracks'); await p.waitForSelector('.track');
  const center = async (sel) => { const b = await p.locator(sel).first().boundingBox(); return [b.x + b.width / 2, b.y + b.height / 2]; };
  const press = async (sel, n = 3, gap = 60) => { const [x, y] = await center(sel); for (let i = 0; i < n; i++) { await p.mouse.click(x, y); if (i < n - 1) await sleep(gap); } };

  // ---- busy() on handlers assigned anywhere ----
  await p.evaluate(() => {
    const add = (html) => { const d = document.createElement('div'); d.innerHTML = html; d.style.cssText = 'position:fixed;left:20px;top:80px;z-index:999;display:flex;gap:8px;background:var(--bg);padding:8px'; document.body.append(d); return d; };
    window.__n = { slow: 0, fail: 0, change: 0, rep: 0, row: 0, inner: 0 };
    const wait = (ms) => new Promise((r) => setTimeout(r, ms));
    add('<button class="btn" id="tslow"><svg class="ic"><use href="#i-heart"/></svg> Slow</button><button class="btn" id="tfail">Fails</button><label><input type="checkbox" id="tchg"> Change</label><button class="btn" id="trep" data-repeat>Step</button><div id="trow" class="list-row" style="padding:6px">Row <button class="btn sm" id="tinner">Inner</button></div><a href="#nope" id="tfalse" class="btn">False</a>');
    document.getElementById('tslow').onclick = async () => { window.__n.slow++; await wait(500); };
    document.getElementById('tfail').onclick = async () => { window.__n.fail++; await wait(200); throw new Error('The fixture said no'); };
    document.getElementById('tchg').onchange = async () => { window.__n.change++; await wait(500); };
    document.getElementById('trep').onclick = async () => { window.__n.rep++; await wait(500); };
    document.getElementById('trow').onclick = async () => { window.__n.row++; await wait(500); };
    document.getElementById('tinner').onclick = async () => { window.__n.inner++; await wait(500); };
    document.getElementById('tfalse').onclick = () => false;
  });
  const attrs = (sel) => p.evaluate((s) => { const e = document.querySelector(s); return { busy: e.getAttribute('aria-busy'), dis: e.getAttribute('aria-disabled'), cls: e.className, focus: document.activeElement === e }; }, sel);
  {
    const [x, y] = await center('#tslow');
    const t0 = Date.now(); await p.mouse.click(x, y); const a = await attrs('#tslow'); const dt = Date.now() - t0;
    check(a.busy === 'true' && a.dis === 'true' && /is-busy/.test(a.cls), `a pressed control is busy at once (aria-busy, aria-disabled; ${dt} ms)`);
    await p.mouse.click(x, y); await sleep(60); await p.mouse.click(x, y);
    await sleep(150); check(/is-spinning/.test((await attrs('#tslow')).cls), 'its icon is a spinner after 150 ms');
    await p.screenshot({ path: path.join(SHOTS, 'busy-control.png'), clip: { x: 0, y: 60, width: 640, height: 80 } });
    await sleep(500); const b = await attrs('#tslow');
    check(b.busy === null && b.dis === null && !/is-busy|is-spinning/.test(b.cls), 'and comes back when done');
    check(await p.evaluate(() => window.__n.slow) === 1, 'three quick clicks run it once');
    await p.mouse.click(x, y); await sleep(600); check(await p.evaluate(() => window.__n.slow) === 2, 'a click after it is done runs it again');
  }
  {
    await p.focus('#tslow'); await p.keyboard.press('Enter'); await sleep(50); const a = await attrs('#tslow');
    check(a.busy === 'true' && a.focus, 'pressed with the keyboard: busy, and focus stays on it'); await sleep(600);
  }
  {
    await press('#tfail', 2); await sleep(400);
    const t = await p.evaluate(() => { const e = document.querySelector('#toast'); return { text: e.textContent, cls: e.className }; });
    check(/fixture said no/.test(t.text) && /error/.test(t.cls), `a failure nobody caught shows its message ("${t.text}")`);
    const a = await attrs('#tfail'); check(a.busy === null && a.dis === null, 'and the control is back');
    check(await p.evaluate(() => window.__n.fail) === 1, 'two quick clicks on it ran it once');
  }
  {
    await press('#tchg', 2, 80); await sleep(700);
    check(await p.evaluate(() => window.__n.change) === 2, 'a change still runs while the last one is on its way (each carries a value)');
    await press('#trep', 3, 60); await sleep(700);
    check(await p.evaluate(() => window.__n.rep) === 3, 'data-repeat opts out (each press runs)');
    await press('#tinner', 1); await sleep(50); await press('#tinner', 1); await sleep(700);
    const n = await p.evaluate(() => window.__n);
    check(n.inner === 1 && n.row === 1, `one click reaches the control's and its row's handler once each (control ${n.inner}, row ${n.row})`);
    // The row's handler takes longer than the control's own: the control stays busy until both are done
    await p.evaluate(() => { const wait = (ms) => new Promise((r) => setTimeout(r, ms)); window.__n.row = 0; window.__n.inner = 0; document.getElementById('tinner').onclick = async () => { window.__n.inner++; await wait(150); }; document.getElementById('trow').onclick = async () => { window.__n.row++; await wait(900); }; });
    await press('#tinner', 1); await sleep(400);
    check((await attrs('#tinner')).busy === 'true', "a control stays busy while its row's slower handler for the same click runs");
    await press('#tinner', 1); await sleep(800);
    const n2 = await p.evaluate(() => window.__n);
    check(n2.inner === 1 && n2.row === 1 && (await attrs('#tinner')).busy === null, `a second click meanwhile runs neither, and it is back afterwards (control ${n2.inner}, row ${n2.row})`);
    await p.click('#tfalse'); check(await p.evaluate(() => location.hash) === '', 'a handler returning false still cancels the link');
    await p.evaluate(() => document.getElementById('trow').parentElement.remove());
  }

  // ---- real controls, every change delayed 500 ms ----
  const puts = () => state.hits.filter((h) => h.m === 'PUT' && h.v === '/favourites').length;
  {
    await p.hover('.track'); const before = puts();
    await press('.track [data-act="fav"]', 3, 60);
    await sleep(80); const a = await attrs('.track [data-act="fav"]');
    check(a.busy === 'true', 'the heart on a song row is busy while its change is on its way');
    await sleep(900);
    check(puts() - before === 1, `three quick presses on the heart send one request (${puts() - before})`);
    check((await attrs('.track [data-act="fav"]')).busy === null, 'and the heart comes back');
  }
  {
    // Add to favourites from the row's menu, twice in quick succession: one request (api() shares it)
    state.favs = []; await fx.go(p, '/tracks'); await p.waitForSelector('.track'); await p.hover('.track');
    const before = puts();
    for (let i = 0; i < 2; i++) { await p.click('.track [data-act="more"]', { force: true }); await p.locator('.pop-item', { hasText: 'Add to favourites' }).click(); }
    await sleep(80);
    const m = await attrs('.track [data-act="more"]');
    check(m.busy === 'true' && m.dis === null, "the row's ⋯ shows its menu's action on its way, and still opens its menu");
    await sleep(900);
    check(puts() - before === 1, `the same change asked twice while on its way is sent once (${puts() - before})`);
  }

  {
    // A switch turned off, on and off again while each change is on its way: three changes, each sent
    // (only the same change asked again while it is the latest one is shared)
    let found = false;
    for (const tab of ['nodehome', 'home', 'playback', 'about']) { await fx.go(p, `/settings/${tab}`); if (await p.waitForSelector('#firststeps', { timeout: 2500 }).then(() => true, () => false)) { found = true; break; } }
    check(found, 'Settings has the First steps switch');
    if (found) {
      const n0 = state.hits.length; const shown = await p.isChecked('#firststeps');
      for (let i = 0; i < 3; i++) { await p.click('#firststeps'); await sleep(80); }
      await sleep(900);
      const sent = state.hits.slice(n0).filter((h) => h.m === 'PUT' && h.v === '/node/settings').map((h) => JSON.parse(h.body).first_steps_hidden);
      // hidden = !checked: from shown, the three presses ask hidden = shown, !shown, shown
      check(JSON.stringify(sent) === JSON.stringify([shown, !shown, shown]), `three presses on a switch while each change is on its way send all three, in order (${JSON.stringify(sent)})`);
      const at = state.hits.slice(n0).filter((h) => h.m === 'PUT' && h.v === '/node/settings').map((h) => h.at);
      check(at.length === 3 && at[1] - at[0] >= 450 && at[2] - at[1] >= 450, `and one after another, so the node applies them in that order (${at.slice(1).map((t, i) => t - at[i]).join(', ')} ms apart)`);
    }
  }
  // ---- navigation ----
  const nav = (v) => `.sidebar .nav-item[data-view="${v}"]`;
  {
    slow['/playlists'] = 2600;
    const t0 = Date.now(); await p.click(nav('playlists'));
    const at = await p.evaluate(() => ({ bar: !!document.querySelector('#navprog:not([hidden])'), on: document.querySelector('.sidebar .nav-item[data-view="playlists"]').classList.contains('on') }));
    check(at.bar, `the bar along the top shows at once (${Date.now() - t0} ms)`);
    check(at.on, 'the place pressed is marked at once');
    await sleep(250);
    check(await p.locator('#main .navskel').isVisible(), 'the page shows its skeleton before its answer');
    check(await p.evaluate(() => { const s = document.querySelector('#navstatus'); return !!s && s.getAttribute('role') === 'status' && s.textContent === 'Loading…' && !document.querySelector('#main').contains(s); }), 'and says "Loading…" in a status region outside the page');
    await p.screenshot({ path: path.join(SHOTS, 'loading.png') });
    await sleep(2000);
    check(await p.locator('#main .slow-note').isVisible(), '"Still loading…" after 2 s');
    await p.screenshot({ path: path.join(SHOTS, 'loading-slow.png') });
    await p.waitForSelector('#main .navskel', { state: 'detached', timeout: 3000 }).catch(() => {});
    await sleep(100);
    check(await p.evaluate(() => !document.querySelector('#main .navskel') && !!document.querySelector('#navprog[hidden]')), 'the answer replaces the skeleton and the bar goes');
    check((await attrs(nav('playlists'))).busy === null, 'the place pressed is not left busy');
    await p.screenshot({ path: path.join(SHOTS, 'empty.png') });
    delete slow['/playlists'];
  }
  {
    // A → B pressed in quick succession ends on B, and nothing of A is drawn after B was pressed:
    // Playlists then Genres, and an album then Artists
    const seen = (re) => p.evaluate((src) => { window.__sawA = false; const r = new RegExp(src); new MutationObserver(() => { if (r.test(document.querySelector('#main').textContent)) window.__sawA = true; }).observe(document.querySelector('#main'), { childList: true, subtree: true, characterData: true }); }, re);
    FIXTURES['/playlists'] = { body: [fx.summary] }; FIXTURES['/library/genres'] = { body: [{ name: 'Rock', tracks: 3 }] };
    for (const [first, then, from, to, aWords, bView] of [[nav('playlists'), nav('genres'), '/playlists', '/library/genres', 'Music and notes', 'genres'], ['[data-album="7"]', nav('artists'), '/item/album/lib-7', '/library/artists', 'Album Seven', 'artists']]) {
      await p.click(nav('albums')); await p.waitForSelector('[data-album="7"]'); await sleep(300);
      slow[from] = 1500; slow[to] = 300; await seen(aWords);
      const n0 = state.hits.filter((h) => h.v === from).length;
      await p.click(first); await sleep(60); await p.click(then);
      await sleep(2200);
      const end = await p.evaluate(() => ({ text: document.querySelector('#main').textContent, path: location.pathname, sawA: window.__sawA }));
      check(end.path.includes(bView) && !end.sawA && !end.text.includes(aWords), `${from} then ${to}: ends on ${end.path}, nothing of the first drawn`);
      check(state.hits.filter((h) => h.v === from).length - n0 === 1, `${from} was asked for once`);
      check((await p.evaluate(() => document.querySelectorAll('.nav-item[aria-busy="true"], #main [aria-busy="true"]').length)) === 0, 'nothing is left busy');
      delete slow[from]; delete slow[to];
    }
    delete FIXTURES['/playlists']; delete FIXTURES['/library/genres'];
  }
  {
    // The same page with another argument, which the page's own checks (state.view) don't tell apart:
    // tag A, then A + B; Back (A, slow) and at once Forward (A + B, fast) ends on A + B with A's answer
    // never drawn over it
    const tagSlow = { 'slowtag': 0, 'slowtag,fasttag': 0 };
    await run.context.route('**/v1/home/tag?*', async (r) => {
      const labels = new URL(r.request().url()).searchParams.get('labels'); state.hits.push({ v: '/home/tag', m: 'GET', q: labels });
      await sleep(tagSlow[labels] || 0);
      const item = (t) => ({ kind: 'other', title: t, name: t, description: 'A fixture page', site: 'example.com', uri: `https://example.com/${encodeURIComponent(t)}` });
      r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(labels === 'slowtag' ? { items: [item('Only on tag A')], tags: [['fasttag', 3]] } : { items: [item('On tags A and B')], tags: [] }) });
    });
    await fx.go(p, '/tag/slowtag'); await p.waitForSelector('[data-tag-add="fasttag"]');
    await p.click('[data-tag-add="fasttag"]'); await p.waitForFunction(() => /On tags A and B/.test(document.querySelector('#main').textContent));
    tagSlow['slowtag'] = 1500; tagSlow['slowtag,fasttag'] = 200;
    await p.evaluate(() => { history.back(); setTimeout(() => history.forward(), 60); });
    await sleep(400);
    await p.evaluate(() => { window.__sawA = false; new MutationObserver(() => { if (/Only on tag A/.test(document.querySelector('#main').textContent)) window.__sawA = true; }).observe(document.querySelector('#main'), { childList: true, subtree: true, characterData: true }); });
    await sleep(1800);
    const end = await p.evaluate(() => ({ text: document.querySelector('#main').textContent, path: decodeURIComponent(location.pathname), sawA: window.__sawA }));
    check(end.path === '/tag/slowtag,fasttag' && /On tags A and B/.test(end.text) && !end.sawA && !/Only on tag A/.test(end.text), `Back then Forward at once ends on ${end.path} with its own answer, nothing of A drawn`);
  }
  {
    // Back to a scrolled page, then another place before it is drawn: the new place starts at the top
    state.tracks = Array.from({ length: 80 }, (_, i) => ({ ...fx.track, id: 1000 + i, title: `Song ${i}` }));
    await p.click(nav('tracks')); await p.waitForSelector('.track:nth-child(60)', { state: 'attached' }); await sleep(300);
    await p.evaluate(() => { document.querySelector('#main').scrollTop = 1500; }); await sleep(100);
    await p.click(nav('genres')); await sleep(500);
    slow['/library/tracks'] = 1200; FIXTURES['/library/genres'] = { body: Array.from({ length: 60 }, (_, i) => ({ name: `Genre ${i}`, tracks: 3 })) };
    await p.evaluate(() => history.back()); await sleep(60); await p.click(nav('favourites'), { force: true });
    await sleep(1800);
    const top = await p.evaluate(() => ({ s: document.querySelector('#main').scrollTop, path: location.pathname }));
    check(top.path.includes('favourites') && top.s === 0, `Back then another place at once: it opens at the top (${top.path}, scrolled ${top.s})`);
    delete slow['/library/tracks']; delete FIXTURES['/library/genres']; delete state.tracks;
  }
  {
    // A page whose answer fails: the error, and Try again asks again
    FIXTURES['/playlists'] = { body: { error: 'the node answered 503 Service Unavailable' }, status: 503 }; state.fixtures = FIXTURES;
    await p.click(nav('genres')); await sleep(200); await p.click(nav('playlists')); await p.waitForSelector('#navretry').catch(() => {});
    check(/try again later|isn.t answering/i.test(await p.textContent('#main .notice')), 'a page that fails says so');
    await p.screenshot({ path: path.join(SHOTS, 'error.png') });
    const n0 = state.hits.filter((h) => h.v === '/playlists').length;
    delete FIXTURES['/playlists']; await p.click('#navretry'); await sleep(600);
    check(state.hits.filter((h) => h.v === '/playlists').length > n0 && !(await p.$('#navretry')), 'Try again asks again and draws the page');
  }
  {
    // Next pressed twice skips twice, on a node zone (each press sends its own request)
    const zone = { id: 'z1', name: 'Living room', output: 'upnp:fx', transport: 'playing', queue: [{ id: 1, uri: 'track:1', title: 'One' }, { id: 2, uri: 'track:1', title: 'Two' }, { id: 3, uri: 'track:1', title: 'Three' }], queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, sync: 'single', volume: { percent: 20, muted: false }, now_playing: { item: { id: 1, uri: 'track:1', title: 'One', artist: 'Sefa', duration_ms: 200000 }, index: 0, position_ms: 1000, duration_ms: 200000 } };
    const r2 = await fx.create(browser, 1280, { fixtures: { '/zones': { body: [zone] }, '/zones/z1/transport': { body: zone } }, delay: (v, m) => (m !== 'GET' ? 500 : 0) });
    await r2.context.addInitScript(() => localStorage.setItem('pm.zone', 'z1'));
    await r2.context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
    await fx.go(r2.page, '/albums'); await r2.page.waitForTimeout(800);
    const nx = await r2.page.locator('#next').boundingBox(); const n0 = r2.state.hits.length;
    await r2.page.mouse.click(nx.x + nx.width / 2, nx.y + nx.height / 2); await sleep(80); await r2.page.mouse.click(nx.x + nx.width / 2, nx.y + nx.height / 2);
    const nb = await r2.page.evaluate(() => { const b = document.querySelector('#next'); return { busy: b.getAttribute('aria-busy'), dis: b.getAttribute('aria-disabled') }; });
    check(nb.busy === 'true' && nb.dis === null, 'Next shows its skip on its way and still takes presses');
    await sleep(900);
    const sent = r2.state.hits.slice(n0).filter((h) => h.m === 'POST' && h.v === '/zones/z1/transport').map((h) => JSON.parse(h.body).action);
    check(sent.length === 2 && sent.every((a) => a === 'next'), `Next pressed twice skips twice (${JSON.stringify(sent)})`);
    const at = r2.state.hits.slice(n0).filter((h) => h.m === 'POST' && h.v === '/zones/z1/transport').map((h) => h.at);
    check(at.length === 2 && at[1] - at[0] >= 450, `the second skip is sent after the first is answered, so answers land in order (${at[1] - at[0]} ms apart)`);
    check(r2.state.errors.length === 0, `no page errors on the zone${r2.state.errors.length ? ': ' + r2.state.errors.join(' | ') : ''}`);
    await r2.context.close();
  }
  check(run.state.errors.length === 0, `no page errors${run.state.errors.length ? ': ' + run.state.errors.join(' | ') : ''}`);
  await run.context.close(); await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
