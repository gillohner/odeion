// Empty pages that send you somewhere must send you where the thing is. Songs and Albums of an
// empty library said "Add the folder with your music in Settings (Library folders)" with Open
// settings, and Confirm matches with nothing to confirm had Open Settings, but folders and Identify
// moved to Your library › Node library: Settings has neither, so both were dead ends. Now the owner's
// Songs and Albums offer Add music (its chooser opens, 1 click), a listener reads where the music
// comes from with no button, and Open Identify opens Node library with the Identify card on screen
// and its title focused (1 click; before: a dead end, then 2 clicks and a scroll).
// At 1280, 1024, 768 and 360 px.
//   node tools/ui-empty-ways-in.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'empty-ways-in-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const stats = { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0, total_duration_ms: 0 };
const empty = { '/library/stats': { body: stats }, '/info': { body: { name: 'Fixture node', offline: false, library: { tracks: 0, albums: 0, artists: 0, files: 0, missing_files: 0 } } } };
const idSt = { phase: 'new', progress: { running: false, total: 0, done: 0, identified: 0, suggested: 0, unknown: 0, errors: 0 }, counts: { total: 4, identified: 0, suggested: 0, suggested_missing: 0, unknown: 0, unchecked: 4, skipped: 0, outdated: 0, by: { tags: 0, album: 0, fingerprint: 0, you: 0, title_artist: 0, auto: 0 } }, acoustid: false, acoustid_own: false, acoustid_state: 'missing', write_ids: false, auto_match: true };
const nodeLib = {
  '/library/suggestions/albums': { body: { albums: [], songs: 0, total_albums: 0, offset: 0, next: null } },
  '/library/identify': { body: idSt },
  '/library/folders': { body: [['/music', true]] },
  '/library/webdav': { body: [] },
  '/library/drives': { body: { providers: [], roots: [] } },
  '/library/hidden': { body: Array.from({ length: 14 }, (_, k) => ({ path: `/music/Band/Album/${k + 1} Song.flac`, hidden_at: 1793631888 })) },
  '/library/stats': { body: { ...stats, tracks: 4, files: 4 } },
};
const emptyBox = (page) => page.evaluate(() => { const box = document.querySelector('.main .empty'); return box && { title: box.querySelector('h3')?.textContent, text: box.querySelector('p')?.textContent || '', buttons: [...box.querySelectorAll('button')].map((b) => b.textContent.trim()) }; });

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 360]) {
      // An empty library, the owner: Songs and Albums offer Add music, and it opens Add music's chooser
      for (const [view, title] of [['/tracks', 'No songs yet'], ['/albums', 'No albums yet']]) {
        const T = `${width} owner ${view}`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: empty, tracks: [] });
        try {
          await fx.go(page, view);
          const shown = await page.waitForSelector('.main .empty', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${T}: the empty state shows`);
          if (!shown) continue;
          const e = await emptyBox(page);
          check(e.title === title, `${T}: the title is "${title}" (${e.title})`);
          check(!/Settings/.test(e.text), `${T}: it no longer sends you to Settings (${e.text})`);
          check(/Add the folder with your music, or upload files/.test(e.text), `${T}: it says how music gets in (${e.text})`);
          check(e.buttons.length === 1 && e.buttons[0] === 'Add music', `${T}: one button, Add music (${e.buttons.join(', ')})`);
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${T}: nothing wider than the screen (${o})`);
          await page.screenshot({ path: path.join(SHOTS, `owner${view.replace('/', '-')}-${width}.png`) });
          await page.locator('.main .empty button').click();
          const dlg = await page.waitForFunction(() => { const d = document.querySelector('dialog[open]'); return d && /Add music/.test(d.textContent) && /Add a folder or network share/.test(d.textContent); }, null, { timeout: 8000 }).then(() => true, () => false);
          check(dlg, `${T}: Add music opens the chooser with "Add a folder or network share" (${await page.evaluate(() => location.pathname)})`);
          await page.screenshot({ path: path.join(SHOTS, `owner${view.replace('/', '-')}-chooser-${width}.png`) });
          check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // A listener can't add to the node's library: a plain line, no button
      {
        const T = `${width} listener /tracks`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: empty, tracks: [], role: 'listener' });
        try {
          await fx.go(page, '/tracks');
          const shown = await page.waitForSelector('.main .empty', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${T}: the empty state shows`);
          if (shown) {
            const e = await emptyBox(page);
            check(e.text === 'Songs show up here once the node’s owner adds music.', `${T}: it says where the music comes from (${e.text})`);
            check(!e.buttons.length, `${T}: no button (${e.buttons.join(', ')})`);
            await page.screenshot({ path: path.join(SHOTS, `listener-tracks-${width}.png`) });
          }
          check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Confirm matches with nothing to confirm: Open Identify lands on the Identify card
      {
        const T = `${width} /identify`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: nodeLib });
        try {
          await fx.go(page, '/identify');
          const shown = await page.waitForSelector('.main .empty', { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${T}: Nothing to confirm shows`);
          if (!shown) continue;
          const e = await emptyBox(page);
          check(e.title === 'Nothing to confirm', `${T}: the title (${e.title})`);
          check(/Your library › Node library/.test(e.text), `${T}: it says where Identify is (${e.text})`);
          check(e.buttons.length === 1 && e.buttons[0] === 'Open Identify', `${T}: one button, Open Identify (${e.buttons.join(', ')})`);
          await page.screenshot({ path: path.join(SHOTS, `identify-empty-${width}.png`) });
          await page.locator('.main .empty button').click();
          const landed = await page.waitForFunction(() => location.pathname === '/library/node' && document.querySelector('#idcard'), null, { timeout: 8000 }).then(() => true, () => false);
          check(landed, `${T}: Open Identify opens Node library (${await page.evaluate(() => location.pathname)})`);
          if (!landed) continue;
          await page.waitForTimeout(400);
          const at = await page.evaluate(() => {
            const h = document.querySelector('#idcard').closest('.set-block').querySelector('.set-name'); const r = h.getBoundingClientRect();
            const hit = document.elementFromPoint(r.left + 20, r.top + r.height / 2); // nothing on top of it (the top bar, a phone's sticky Library chips, the player)
            const grp = document.querySelector('#idcard').closest('.set-group').querySelector('.set-h').getBoundingClientRect();
            return { title: h.textContent.trim(), seen: !!hit && (hit === h || h.contains(hit)), top: Math.round(r.top), heading: grp.top >= 0 && grp.bottom <= r.top, focused: document.activeElement === h };
          });
          check(at.seen, `${T}: the Identify card's title "${at.title}" is on screen and not covered (top ${at.top})`);
          check(at.heading, `${T}: the IDENTIFY heading above it is on screen too`);
          check(at.focused, `${T}: its title has the focus`);
          await page.screenshot({ path: path.join(SHOTS, `identify-landing-${width}.png`) });
          // Node library reached another way, in the app, doesn't jump to the card
          await page.goBack(); await page.waitForSelector('.main .empty');
          await page.goForward(); await page.waitForSelector('#idcard'); await page.waitForTimeout(400);
          const back = await page.evaluate(() => ({ y: scrollY + (document.querySelector('.main')?.scrollTop || 0), focus: document.activeElement && document.activeElement.className }));
          check(back.y < 50 && back.focus !== 'set-name', `${T}: Back, then Forward to Node library, stays at the top, focus left alone (${JSON.stringify(back)})`);
          check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
      // Open Identify, then away before Node library has loaded: opening it later doesn't jump
      {
        const T = `${width} /identify left early`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: nodeLib, delay: (v) => (v === '/library/folders' ? 1500 : 0) });
        try {
          await fx.go(page, '/identify');
          await page.waitForSelector('.main .empty');
          await page.locator('.main .empty button').click();
          await page.waitForFunction(() => location.pathname === '/library/node');
          await page.goBack(); await page.waitForSelector('.main .empty'); await page.waitForTimeout(2000); // (the first ask has answered by now, to a page that is gone)
          await page.goForward(); await page.waitForSelector('#idcard'); await page.waitForTimeout(400);
          const got = await page.evaluate(() => ({ y: scrollY + (document.querySelector('.main')?.scrollTop || 0), focus: document.activeElement && document.activeElement.className }));
          check(got.y < 50 && got.focus !== 'set-name', `${T}: Node library opened later stays at the top, focus left alone (${JSON.stringify(got)})`);
          check(!state.errors.length, `${T}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
