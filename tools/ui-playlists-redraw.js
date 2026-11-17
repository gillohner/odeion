// Playlists drawn again where it stands. Before, a library change (a scan, an import, an edit)
// drew Playlists from the start: a half-typed new playlist name or a pasted shared link was gone,
// the keyboard dropped to the page and the list jumped to the top. A playlist file imported while
// the person stayed on Playlists (typing a name, a menu open) didn't show up in the list until the
// next visit. Now the page is drawn in place (redrawing/keepPlace): the boxes keep what is typed
// and the caret, focus and scroll stay, and a finished import puts its row in the list at once.
// Create empties the name box and leaves the keyboard in it for the next one. At 1280, 1024, 768 px.
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
        check(shown, `${width}: the imported playlist shows in the list while the person stays on Playlists`);
        check(new URL(page.url()).pathname === '/playlists' && l.name === 'Road' && l.focus === 'plname', `${width}: they stay, typing (${new URL(page.url()).pathname}, "${l.name}", focus ${l.focus})`);
        check(open.includes('Open playlist'), `${width}: the toast still offers ${JSON.stringify(open)}`);
        await page.screenshot({ path: path.join(SHOTS, `import-shown-${width}.png`) });

        // 4. Create: the new row is there, the box is emptied and keeps the keyboard.
        await page.fill('#plname', 'Sunday');
        await page.locator('#plname').press('Enter');
        const made = await until(() => page.evaluate(() => !!document.querySelector('#main [data-pl="new1"]')), 3000);
        await sleep(150); l = await look();
        check(made && l.name === '' && l.focus === 'plname', `${width}: Create draws the new row, empties the box and keeps the keyboard there (made ${made}, "${l.name}", ${l.focus})`);
        // Enter on the Create button itself: the keyboard goes to the box; a click leaves focus be
        let n = await rows(); await page.fill('#plname', 'Monday'); await page.focus('#plcreate'); await page.keyboard.press('Enter');
        await until(async () => (await rows()) === n + 1, 3000); await sleep(150); l = await look();
        check(l.name === '' && l.focus === 'plname', `${width}: Enter on Create empties the box and puts the keyboard in it ("${l.name}", ${l.focus})`);
        n = await rows(); await page.fill('#plname', 'Tuesday'); await page.click('#plcreate');
        await until(async () => (await rows()) === n + 1, 3000); await sleep(150); l = await look();
        check(l.name === '' && l.focus !== 'plname', `${width}: a click on Create empties the box and doesn't pull a phone keyboard up ("${l.name}", ${l.focus})`);

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
        check(early, `${width}: Playlists is drawn from its first answer while a newer one is still coming`);
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
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
