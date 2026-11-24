// Playlists drawn again where it stands. Before, a library change (a scan, an import, an edit)
// drew Playlists from the start: a half-typed new playlist name or a pasted shared link was gone,
// the keyboard dropped to the page and the list jumped to the top. A playlist file imported while
// the person stayed on Playlists (typing a name, a menu open) didn't show up in the list until the
// next visit. Now the page is drawn in place (redrawing/keepPlace): the boxes keep what is typed
// and the caret, focus and scroll stay, and a finished import puts its row in the list at once.
// Create empties the name box and leaves the keyboard in it for the next one (Enter or a mouse
// click; a tap on a touch screen doesn't pull the on-screen keyboard up: the focus goes to the new
// playlist's row instead, not lost on the page). The keyboard on a playlist's row stays there when the
// list is drawn again. At 1280, 1024, 768 px.
//   node tools/ui-playlists-redraw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlists-redraw-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };
const pl = (i) => ({ ...fx.summary, id: `p${i}`, name: `Mix ${String(i).padStart(2, '0')}`, items: i });
const FILE = { name: 'trip.m3u', mimeType: 'audio/x-mpegurl', buffer: Buffer.from('#EXTM3U\n/music/a.flac\n') };
const IMPORTED = { ...fx.summary, id: 'imp1', name: 'Road trip', items: 1 };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      const { page, context, state } = await fx.create(browser, width, {});
      state.collections = Array.from({ length: 30 }, (_, i) => pl(i + 1));
      let socket = null;
      await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
      const changed = () => socket.send(JSON.stringify({ type: 'library_changed' }));
      // `gets`: how the next lists answer ({ ms, status }), each with the list as it was when asked
      const gets = [];
      await page.route('**/v1/playlists', async (r) => {
        if (r.request().method() === 'GET') {
          const g = gets.shift(); if (!g) return r.fallback();
          const then = JSON.stringify(state.collections); await sleep(g.ms);
          return r.fulfill({ status: g.status || 200, contentType: 'application/json', body: g.status ? JSON.stringify({ error: 'The node did not answer' }) : then });
        }
        if (r.request().method() !== 'POST') return r.fallback();
        const name = JSON.parse(r.request().postData() || '{}').name;
        state.collections = [...state.collections, { ...fx.summary, id: 'new1', name, items: 0 }];
        return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...fx.detail, summary: { ...fx.summary, id: 'new1', name } }) });
      });
      await page.route('**/v1/playlists/import/file*', async (r) => { await sleep(900); state.collections = [...state.collections, IMPORTED]; await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ id: 'imp1', name: 'Road trip', parsed: 1, imported: 1, kept: 0, unresolved: [] }) }); });
      const rows = () => page.locator('#main [data-pl]').count();
      // A new list element: the redraw is done
      const mark = () => page.evaluate(() => { document.querySelector('#main .list').dataset.old = '1'; });
      const redrawn = () => until(() => page.evaluate(() => { const l = document.querySelector('#main .list'); return !!l && !l.dataset.old; }));
      const look = () => page.evaluate(() => { const n = document.querySelector('#plname'); const s = document.querySelector('#plshared'); const a = document.activeElement; return { name: n && n.value, link: s && s.value, focus: a && (a.id || a.dataset.plplay || a.tagName), caret: n ? [n.selectionStart, n.selectionEnd] : null, top: document.querySelector('#main').scrollTop, create: document.querySelector('#plcreate')?.disabled, skeleton: !!window.__skeleton }; });
      try {
        await fx.go(page, '/playlists');
        await page.waitForSelector('#main [data-pl]');
        for (let i = 0; i < 50 && !socket; i++) await sleep(100);
        check(!!socket, `${width}: events connected`);
        await page.evaluate(() => { window.__skeleton = false; new MutationObserver(() => { if (document.querySelector('#main .sk-grid, #main .navskel')) window.__skeleton = true; }).observe(document.querySelector('#main'), { childList: true, subtree: true }); });

        // 1. A name half typed and a pasted link: a library change keeps both, the caret and the focus.
        await page.fill('#plshared', 'pubky://abc');
        await page.fill('#plname', 'Late night');
        await page.locator('#plname').press('ArrowLeft'); await page.locator('#plname').press('ArrowLeft');
        state.collections = [...state.collections, pl(31)];
        await mark(); changed(); await redrawn();
        await until(async () => (await rows()) === 31, 2000);
        let l = await look();
        check((await rows()) === 31, `${width}: a library change draws the new row (${await rows()} rows)`);
        check(l.name === 'Late night' && l.link === 'pubky://abc', `${width}: the typed name and link stay ("${l.name}", "${l.link}")`);
        check(l.focus === 'plname' && l.caret && l.caret[0] === 8 && l.caret[1] === 8, `${width}: the keyboard stays in the name box, caret where it was (${l.focus}, ${JSON.stringify(l.caret)})`);
        check(l.create === false, `${width}: Create stays usable with the kept name (disabled ${l.create})`);
        await page.keyboard.type('s');
        check((await page.locator('#plname').inputValue()) === 'Late nigsht', `${width}: typing goes on at the caret`);
        await page.screenshot({ path: path.join(SHOTS, `typed-kept-${width}.png`) });

        // 2. Scrolled down, keyboard on a row's Play: both stay.
        await page.fill('#plname', ''); await page.fill('#plshared', '');
        const play = page.locator('#main [data-plplay="p25"]');
        await play.focus(); await play.scrollIntoViewIfNeeded();
        const before = await look();
        state.collections = [...state.collections, pl(32)];
        await mark(); changed(); await redrawn(); await sleep(150);
        l = await look();
        check(before.top > 0 && Math.abs(l.top - before.top) <= 1, `${width}: the scroll stays (${before.top} → ${l.top})`);
        check(l.focus === 'p25', `${width}: the keyboard stays on the row's Play (${l.focus})`);
        check(!l.skeleton, `${width}: no skeleton while drawn again`);
        await page.screenshot({ path: path.join(SHOTS, `scroll-kept-${width}.png`) });

        // 3. An import that finishes while the person types a name: they stay, the new row is there.
        await page.evaluate(() => document.querySelector('#main').scrollTo({ top: 0, behavior: 'instant' }));
        await page.locator('#plimport').setInputFiles(FILE); await sleep(150);
        await page.locator('#plname').fill('Road');
        const shown = await until(() => page.evaluate(() => !!document.querySelector('#main [data-pl="imp1"]')), 4000);
        l = await look();
        const open = await page.evaluate(() => [...document.querySelectorAll('#toast .toast-act')].map((b) => b.textContent));
        check(shown, `${width}: the imported collection shows in the list while the person stays on Collections`);
        check(new URL(page.url()).pathname === '/playlists' && l.name === 'Road' && l.focus === 'plname', `${width}: they stay, typing (${new URL(page.url()).pathname}, "${l.name}", focus ${l.focus})`);
        check(open.includes('Open collection'), `${width}: the toast still offers ${JSON.stringify(open)}`);
        await page.screenshot({ path: path.join(SHOTS, `import-shown-${width}.png`) });

        // 3b. The keyboard on a playlist's row: a library change keeps it on that row (before, the new
        // row had no tabindex until the next frame, so the focus fell to the page)
        await page.focus('#main [data-pl="p5"]'); await mark(); changed();
        const kept = await redrawn(); await sleep(150);
        const on = await page.evaluate(() => { const a = document.activeElement; return a && (a.dataset.pl || a.id || a.tagName); });
        check(kept && on === 'p5', `${width}: the keyboard on a collection's row stays on it when the list is drawn again (redrawn ${kept}, ${on})`);
        await page.screenshot({ path: path.join(SHOTS, `keyboard-kept-${width}.png`) });

        // 4. Create: the new row is there, the box is emptied and keeps the keyboard.
        await page.fill('#plname', 'Sunday');
        await page.locator('#plname').press('Enter');
        const made = await until(() => page.evaluate(() => !!document.querySelector('#main [data-pl="new1"]')), 3000);
        await sleep(150); l = await look();
        check(made && l.name === '' && l.focus === 'plname', `${width}: Create draws the new row, empties the box and keeps the keyboard there (made ${made}, "${l.name}", ${l.focus})`);
        // Enter on the Create button itself, or a mouse click on it: the keyboard goes to the box for
        // the next name (before, a click left focus on Create, which then went grey, so it fell to the page)
        let n = await rows(); await page.fill('#plname', 'Monday'); await page.focus('#plcreate'); await page.keyboard.press('Enter');
        await until(async () => (await rows()) === n + 1, 3000); await sleep(150); l = await look();
        check(l.name === '' && l.focus === 'plname', `${width}: Enter on Create empties the box and puts the keyboard in it ("${l.name}", ${l.focus})`);
        n = await rows(); await page.fill('#plname', 'Tuesday'); await page.click('#plcreate');
        await until(async () => (await rows()) === n + 1, 3000); await sleep(150); l = await look();
        check(l.name === '' && l.focus === 'plname' && l.create === true, `${width}: a mouse click on Create empties the box and puts the keyboard in it, Create greyed until the next name ("${l.name}", ${l.focus}, disabled ${l.create})`);
        if (l.focus === 'plname') { await page.keyboard.type('Wednesday'); l = await look(); } // (typed on the page, letters are shortcuts)
        check(l.name === 'Wednesday' && l.create === false, `${width}: the next name is typed straight away after a click ("${l.name}", Create disabled ${l.create})`);
        await page.fill('#plname', '');

        // 5. Opened again from the sidebar: drawn from the start, at the top, nothing typed carried over.
        await page.fill('#plname', 'Carry'); await page.evaluate(() => document.querySelector('#main').scrollTo({ top: 400, behavior: 'instant' }));
        await page.click('[data-view="tracks"]'); await until(() => page.evaluate(() => location.pathname === '/tracks'));
        await page.click('[data-view="playlists"]'); await page.waitForSelector('#main [data-pl]');
        await sleep(200); l = await look();
        check(l.name === '' && l.top === 0, `${width}: opened again it starts fresh ("${l.name}", top ${l.top})`);

        // 6. Opened while the library changes: the first answer is drawn while the newer one comes,
        // and an older answer (or its error) that comes after a newer one was drawn doesn't replace it.
        let extra = 40;
        const open2 = async (waits) => {
          await page.click('[data-view="tracks"]'); await until(() => page.evaluate(() => location.pathname === '/tracks'));
          await page.waitForSelector('#main [data-pl]', { state: 'detached' });
          gets.push(...waits); await page.click('[data-view="playlists"]'); await sleep(100);
          state.collections = [...state.collections, pl(++extra)]; changed();
        };
        await open2([{ ms: 300 }, { ms: 2500 }]);
        const early = await until(() => page.evaluate(() => location.pathname === '/playlists' && !!document.querySelector('#main [data-pl]')), 1500);
        check(early, `${width}: Collections is drawn from its first answer while a newer one is still coming`);
        await until(() => page.evaluate(() => !!document.querySelector('#main [data-pl="p41"]')), 4000);
        check(await page.evaluate(() => !!document.querySelector('#main [data-pl="p41"]')), `${width}: then the newer answer's row is drawn too`);
        for (const old of [{ ms: 1500 }, { ms: 1500, status: 500 }]) {
          await open2([old, { ms: 100 }]);
          const newest = state.collections[state.collections.length - 1].id;
          await until(() => page.evaluate((x) => !!document.querySelector(`#main [data-pl="${x}"]`), newest), 2000);
          await sleep(1700);
          const end = await page.evaluate((x) => ({ row: !!document.querySelector(`#main [data-pl="${x}"]`), notice: document.querySelector('#main .notice.warn')?.textContent || '' }), newest);
          check(end.row && !end.notice, `${width}: an older ${old.status ? 'error' : 'answer'} after the newer list doesn't replace it (row ${end.row}, notice "${end.notice}")`);
        }
        check(!state.errors.length, `${width}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // A tap on Create (a touch screen, 1024 and 768 px tablets): the box empties but isn't focused, so
    // no on-screen keyboard pops up over the list. The focus goes to the new playlist's row (before, it
    // fell to the page when Create went grey: a screen reader lost its place). Someone who put the
    // focus somewhere else while the playlist was being made keeps it there.
    for (const width of [1024, 768]) {
      const { page, context, state } = await fx.create(browser, width, { touch: true });
      state.collections = Array.from({ length: 30 }, (_, i) => pl(i + 1));
      let socket = null;
      await page.routeWebSocket(/\/v1\/events/, (ws) => { socket = ws; ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [] })); });
      let made = 0; let wait = 0;
      await page.route('**/v1/playlists', async (r) => {
        if (r.request().method() !== 'POST') return r.fallback();
        const name = JSON.parse(r.request().postData() || '{}').name; const id = `tap${++made}`;
        if (wait) await sleep(wait);
        state.collections = [...state.collections, { ...fx.summary, id, name, items: 0 }];
        return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...fx.detail, summary: { ...fx.summary, id, name } }) });
      });
      const look = () => page.evaluate(() => { const a = document.activeElement; const r = a && a.closest('[data-pl]'); return { name: document.querySelector('#plname').value, focus: a && (a.id || (r && r.dataset.pl) || a.tagName), row: !!r && r === a, top: document.querySelector('#main').scrollTop }; });
      try {
        await fx.go(page, '/playlists');
        await page.waitForSelector('#main [data-pl]');
        await page.fill('#plname', 'Thursday'); await page.tap('#plcreate');
        const drawn = await until(() => page.evaluate(() => !!document.querySelector('#main [data-pl="tap1"]')), 3000);
        await until(async () => (await look()).focus !== 'BODY', 1500);
        let l = await look();
        check(drawn && l.name === '' && l.focus !== 'plname', `${width} touch: a tap on Create draws the row and empties the box without pulling the keyboard up (made ${drawn}, "${l.name}", ${l.focus})`);
        check(l.focus === 'tap1' && l.row, `${width} touch: the focus goes to the new collection's row, not to the page (${l.focus})`);
        // (#main scrolls smoothly: the row is in view once that ends)
        const seen = await until(() => page.evaluate(() => { const r = document.querySelector('#main [data-pl="tap1"]').getBoundingClientRect(); const m = document.querySelector('#main').getBoundingClientRect(); return r.top >= m.top && r.bottom <= m.bottom; }), 3000);
        check(seen, `${width} touch: the new row, here at the foot of a long list, is scrolled into view (top ${(await look()).top})`);
        await page.screenshot({ path: path.join(SHOTS, `touch-create-${width}.png`) });
        // The node says "library changed" after a create: that redraw (in place) keeps the focus on the row
        for (let i = 0; i < 50 && !socket; i++) await sleep(100);
        await page.evaluate(() => { document.querySelector('#main .list').dataset.old = '1'; });
        socket.send(JSON.stringify({ type: 'library_changed' }));
        const again = await until(() => page.evaluate(() => { const x = document.querySelector('#main .list'); return !!x && !x.dataset.old; }));
        await sleep(150);
        l = await look();
        check(again && l.focus === 'tap1' && l.row, `${width} touch: the redraw "library changed" brings keeps the focus on the new row (redrawn ${again}, ${l.focus})`);
        // While the next one is being made, the person taps into the shared-link box: the focus stays there
        wait = 600;
        await page.fill('#plname', 'Friday'); await page.tap('#plcreate');
        await sleep(100); await page.focus('#plshared');
        await until(() => page.evaluate(() => !!document.querySelector('#main [data-pl="tap2"]')), 3000);
        await sleep(300);
        l = await look();
        check(l.focus === 'plshared', `${width} touch: focus moved elsewhere during the request stays there (${l.focus})`);
        check(!state.errors.length, `${width} touch: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
