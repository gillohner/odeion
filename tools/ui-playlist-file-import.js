// Importing a playlist file (M3U, CSV or a Roon playlist export) from Your library › Import's Choose
// a playlist file or Playlists' Import. Before, nothing on the Playlists page showed the upload was
// under way, Your library › Import said only "Importing…" in a small line, and once it was in, the new
// playlist was one or two more clicks away (Open Playlists, then its row). Now the button itself says
// Importing… (busy, no second file meanwhile), and the new playlist opens with what matched in a
// toast. Someone who went to another page or is typing meanwhile stays there (toast with Open playlist),
// and so does someone who left a name in Playlists' box or started a Move from Roon check meanwhile;
// a page drawn again meanwhile still shows the import under way, and the busy button opens no picker
// and takes no second file (on the button drawn again either). A
// failed import says why (in the line under the button on Your library › Import, in a toast on
// Playlists) and the button can be used again at once, for the same file too. At 1280, 1024, 768 px.
//   node tools/ui-playlist-file-import.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'playlist-file-import-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const FILE = { name: 'trip.m3u', mimeType: 'audio/x-mpegurl', buffer: Buffer.from('#EXTM3U\n/music/a.flac\n/music/b.flac\nhttps://example.com/x\n') };
const ANSWER = { id: 'c1', name: 'Music and notes', parsed: 3, imported: 1, kept: 1, unresolved: [{ title: 'Lost song' }] };
const PLACES = [{ at: '/library/import', input: '#libplimport', word: 'Choose a playlist file' }, { at: '/playlists', input: '#plimport', word: 'Import M3U, CSV or Roon export' }];
const sleep = (ms) => new Promise((res) => setTimeout(res, ms));
const until = async (fn, ms = 4000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      for (const pl of PLACES) {
        const tag = `${width} ${pl.at}`;
        const { page, context, state } = await fx.create(browser, width, {});
        const net = { status: 200, wait: 900, body: ANSWER }; let sent = 0;
        let roon = 0;
        await page.route('**/v1/import/roon*', async (r) => { roon++; await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ kind: 'tracks', name: null, rows: 3, matched: 2, unmatched: 1, not_identified: 0, favourites: 0, tags_added: 0, playlist: null, tags_found: [], misses: [], dry: true }) }); });
        await page.route('**/v1/playlists/import/file*', async (r) => {
          sent++; await sleep(net.wait);
          if (net.status === 200) state.collections = [fx.summary];
          await r.fulfill(typeof net.body === 'string' ? { status: net.status, contentType: 'text/html', body: net.body } : { status: net.status, contentType: 'application/json', body: JSON.stringify(net.body) });
        });
        try {
          await fx.go(page, pl.at);
          await page.waitForSelector(pl.input, { state: 'attached', timeout: 8000 });
          const lab = page.locator(`label.file-btn:has(${pl.input})`);
          // 1. Choosing a file: the button says Importing… until the node answers, and takes no other file.
          net.wait = 2500;
          await page.focus(pl.input);
          await page.locator(pl.input).setInputFiles(FILE);
          await sleep(300);
          const busyLook = (q) => page.evaluate((x) => { const i = document.querySelector(x); const l = i && i.closest('label'); const o = document.querySelector(x === '#plimport' ? '#plimportout' : '#libplout'); return l ? { text: l.textContent.trim(), busy: l.getAttribute('aria-busy'), off: i.getAttribute('aria-disabled'), spin: l.classList.contains('is-spinning'), focus: document.activeElement === i, said: o ? o.textContent : '' } : null; }, q);
          const mid = await busyLook(pl.input);
          check(!!mid && /^Importing…$/.test(mid.text) && mid.busy === 'true' && mid.off === 'true' && mid.spin, `${tag}: while it imports the button says "${mid && mid.text}" (busy ${mid && mid.busy}, aria-disabled ${mid && mid.off}, spinner ${mid && mid.spin})`);
          check(!!mid && mid.focus && mid.said === 'Importing trip.m3u…', `${tag}: the keyboard stays on the button (${mid && mid.focus}) and a screen reader hears "${mid && mid.said}"`);
          const picker = await Promise.all([page.waitForEvent('filechooser', { timeout: 700 }).then(() => true, () => false), lab.click({ force: true }), page.keyboard.press('Enter')]).then(([x]) => x);
          check(!picker, `${tag}: a click on the busy button opens no file picker`);
          // A second file given to the busy button is dropped, and the button is emptied again.
          await page.locator(pl.input).setInputFiles(FILE); await sleep(150);
          const second = await page.evaluate((q) => document.querySelector(q).value, pl.input);
          check(sent === 1 && second === '', `${tag}: a second file on the busy button is not sent (${sent} requests, input "${second}")`);
          await page.screenshot({ path: path.join(SHOTS, `importing-${pl.at.replace(/\//g, '_')}-${width}.png`) });
          // The page drawn again meanwhile (to Songs and Back) still shows the import under way.
          await page.click('[data-view="tracks"]'); await until(() => page.evaluate(() => location.pathname === '/tracks'), 2000);
          await page.goBack(); await page.waitForSelector(pl.input, { state: 'attached' }); await sleep(200);
          const again = await busyLook(pl.input);
          check(!!again && /^Importing…$/.test(again.text) && again.busy === 'true' && again.off === 'true', `${tag}: the page drawn again meanwhile still says "${again && again.text}" (busy ${again && again.busy})`);
          await page.locator(pl.input).setInputFiles(FILE); await sleep(150);
          const third = await page.evaluate((q) => document.querySelector(q).value, pl.input);
          check(sent === 1 && third === '', `${tag}: a file on the button drawn again is not sent either (${sent} requests, input "${third}")`);
          // 2. The new playlist opens, with what matched said once in a toast. One click: the choice itself.
          const opened = await until(() => page.evaluate(() => location.pathname === '/playlist/c1' && /Music and notes/.test(document.querySelector('#main h1')?.textContent || '')));
          const t = await page.evaluate(() => document.querySelector('#toast')?.textContent || '');
          check(opened, `${tag}: the new playlist opens once it is imported (now ${new URL(page.url()).pathname})`);
          check(/Imported “Music and notes”: 1 of 3 matched · 1 kept as link · not matched: Lost song/.test(t), `${tag}: a toast says what matched ("${t}")`);
          check(sent === 1, `${tag}: the file is sent once (${sent})`);
          await page.screenshot({ path: path.join(SHOTS, `opened-${pl.at.replace(/\//g, '_')}-${width}.png`) });
          // 3. A failed import says why; the button is back and takes the same file again.
          await fx.go(page, pl.at); await page.waitForSelector(pl.input, { state: 'attached' });
          // (Playlists gets a proxy's HTML 502: the message says what the node answered)
          const WHY = pl.at === '/playlists' ? 'The node answered 502' : 'That is not a playlist file';
          net.status = pl.at === '/playlists' ? 502 : 400; net.body = pl.at === '/playlists' ? '<html>Bad gateway</html>' : { error: WHY }; net.wait = 300;
          await page.focus(pl.input);
          await page.locator(pl.input).setInputFiles(FILE);
          const said = await until(() => page.evaluate(({ at, why }) => (at === '/library/import' ? document.querySelector('#libplout')?.textContent : document.querySelector('#toast')?.textContent || '').includes(why), { at: pl.at, why: WHY }));
          await sleep(100);
          const after = await page.evaluate((q) => { const i = document.querySelector(q); const l = i.closest('label'); return { text: l.textContent.trim(), busy: l.getAttribute('aria-busy'), off: i.getAttribute('aria-disabled'), value: i.value, path: location.pathname, focus: document.activeElement === i }; }, pl.input);
          check(said && after.path === pl.at, `${tag}: a failed import says why and stays on the page (${after.path})`);
          check(after.text === pl.word && after.busy == null && after.off == null && after.value === '' && after.focus, `${tag}: after a failure the button reads "${after.text}" again, takes a file (off ${after.off}, value "${after.value}") and keeps the keyboard (${after.focus})`);
          await page.screenshot({ path: path.join(SHOTS, `failed-${pl.at.replace(/\//g, '_')}-${width}.png`) });
          const was = sent; await page.locator(pl.input).setInputFiles(FILE);
          check(await until(() => sent === was + 1, 3000), `${tag}: the same file can be chosen again right after a failure`);
          await sleep(500);
          // 4. Gone to another page meanwhile: stay there; the toast offers Open.
          await fx.go(page, pl.at); await page.waitForSelector(pl.input, { state: 'attached' });
          net.status = 200; net.body = ANSWER; net.wait = 1200;
          await page.locator(pl.input).setInputFiles(FILE);
          await sleep(200);
          await page.evaluate(() => document.querySelector('#nav [data-view="tracks"], [data-view="tracks"]')?.click());
          await until(() => page.evaluate(() => location.pathname === '/tracks'), 2000);
          await sleep(1500);
          const away = await page.evaluate(() => ({ path: location.pathname, toast: document.querySelector('#toast')?.textContent || '', open: [...document.querySelectorAll('#toast .toast-act')].map((b) => b.textContent) }));
          check(away.path === '/tracks' && /Imported “Music and notes”/.test(away.toast) && away.open.includes('Open playlist'), `${tag}: someone who moved on stays on ${away.path}; the toast says it is in, with ${JSON.stringify(away.open)}`);
          if (away.open.includes('Open playlist')) {
            await page.click('#toast .toast-act');
            check(await until(() => page.evaluate(() => location.pathname === '/playlist/c1')), `${tag}: the toast's Open opens the new playlist`);
          }
          // 5. Typing in the page meanwhile (Playlists' name box): stay; the toast offers Open playlist.
          if (pl.at === '/playlists') {
            await fx.go(page, pl.at); await page.waitForSelector(pl.input, { state: 'attached' });
            net.wait = 900; await page.locator(pl.input).setInputFiles(FILE); await sleep(200);
            await page.locator('#plname').fill('Road');
            await sleep(1300);
            const typing = await page.evaluate(() => ({ path: location.pathname, name: document.querySelector('#plname')?.value, open: [...document.querySelectorAll('#toast .toast-act')].map((b) => b.textContent) }));
            check(typing.path === '/playlists' && typing.name === 'Road' && typing.open.includes('Open playlist'), `${tag}: someone typing a name meanwhile stays (${typing.path}, "${typing.name}"), with ${JSON.stringify(typing.open)}`);
            // 6. A name typed and left (the box lost the keyboard): stay, the name is kept.
            await fx.go(page, pl.at); await page.waitForSelector(pl.input, { state: 'attached' });
            net.wait = 900; await page.locator(pl.input).setInputFiles(FILE); await sleep(200);
            await page.locator('#plname').fill('Road'); await page.evaluate(() => document.activeElement && document.activeElement.blur());
            await sleep(1300);
            const leftName = await page.evaluate(() => ({ path: location.pathname, name: document.querySelector('#plname')?.value, focus: document.activeElement === document.body, open: [...document.querySelectorAll('#toast .toast-act')].map((b) => b.textContent) }));
            check(leftName.path === '/playlists' && leftName.name === 'Road' && leftName.focus && leftName.open.includes('Open playlist'), `${tag}: a name typed and left meanwhile keeps them there (${leftName.path}, "${leftName.name}"), with ${JSON.stringify(leftName.open)}`);
          } else {
            // 6. A Move from Roon check started meanwhile: stay, with its results; the toast offers Open playlist.
            await fx.go(page, pl.at); await page.waitForSelector(pl.input, { state: 'attached' });
            net.wait = 900; await page.locator(pl.input).setInputFiles(FILE); await sleep(200);
            const r0 = roon; await page.locator('#roonfiles').setInputFiles({ name: 'roon-tracks.xlsx', mimeType: 'application/octet-stream', buffer: Buffer.from('PK fixture') });
            await page.evaluate(() => document.activeElement && document.activeElement.blur());
            await sleep(1300);
            const moved = await page.evaluate(() => ({ path: location.pathname, roon: document.querySelector('#roonout')?.textContent || '', open: [...document.querySelectorAll('#toast .toast-act')].map((b) => b.textContent) }));
            check(roon === r0 + 1 && moved.path === pl.at && /2 of 3 matched this library/.test(moved.roon) && moved.open.includes('Open playlist'), `${tag}: a Move from Roon check started meanwhile keeps them there with its results (${moved.path}, ${roon - r0} checks), with ${JSON.stringify(moved.open)}`);
            // 7. Nothing else done, the keyboard on the page's body: the new playlist still opens.
            await fx.go(page, pl.at); await page.waitForSelector(pl.input, { state: 'attached' });
            net.wait = 600; await page.locator(pl.input).setInputFiles(FILE); await sleep(100);
            await page.evaluate(() => document.activeElement && document.activeElement.blur());
            check(await until(() => page.evaluate(() => location.pathname === '/playlist/c1')), `${tag}: with nothing else started, the new playlist opens`);
          }
          check(!state.errors.length, `${tag}: no page errors (${state.errors.join('; ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
