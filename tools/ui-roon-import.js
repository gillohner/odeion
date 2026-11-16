// Your library › Import: the file buttons and Move from Roon. Every file button hid its input with
// display:none, so the keyboard could not reach Choose a playlist file (or Playlists' Import, or
// Restore's Choose a backup file). Move from Roon showed the browser's own "Choose Files / No file
// chosen" control and wanted a Check press before Import. Now each file button is in the Tab order
// (Enter opens the picker, the button shows the focus), Move from Roon has a Choose Roon export files
// button, and choosing files or changing an option checks them right away (only the latest check
// draws; the same files chosen again are checked again; a failed check has Check again); Import
// sends the real run, and after it only new files offer Import. At 1280, 1024 and 768 px.
//   node tools/ui-roon-import.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'roon-import-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// A dry run's answer (roon_import.rs): with tags on, one more row matches, so the drawn line tells
// which check drew it.
const answer = (q) => {
  const tags = q.get('tags') === 'true'; const dry = q.get('dry_run') === 'true';
  return { kind: 'tracks', name: null, rows: 3, matched: tags ? 2 : 1, unmatched: tags ? 1 : 2, not_identified: 0, favourites: 1, tags_added: tags ? 2 : 0, playlist: null, tags_found: [['live', 2]], misses: [{ what: 'Unknown song', reason: 'not in this library' }], dry };
};
const FILE = { name: 'roon-tracks.xlsx', mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', buffer: Buffer.from('PK fixture') };

// Tab from the top until the element `id` has the focus (or give up).
const tabTo = async (page, id) => {
  await page.evaluate(() => { document.activeElement && document.activeElement.blur(); window.scrollTo(0, 0); });
  for (let i = 0; i < 80; i++) {
    await page.keyboard.press('Tab');
    if (await page.evaluate((x) => document.activeElement && document.activeElement.id === x, id)) return true;
  }
  return false;
};
const ringOn = (page, id) => page.evaluate((x) => { const l = document.getElementById(x).closest('label'); const cs = l && getComputedStyle(l); return !!cs && cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0; }, id);

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      const { page, context, state } = await fx.create(browser, width, {});
      const calls = []; const net = { fail: false, slow: 0 };
      // Listen for the file picker from the start: Playwright turns its interception on without
      // waiting, so a listener added together with the Enter press can lose that race on a slow
      // runner and the picker that did open is never seen.
      const choosers = []; page.on('filechooser', (c) => choosers.push(c));
      await page.route('**/v1/import/roon*', async (r) => {
        const q = new URL(r.request().url()).searchParams; calls.push({ tags: q.get('tags'), dry: q.get('dry_run'), fav: q.get('favourites') });
        if (calls.length === 1) await new Promise((res) => setTimeout(res, 1200)); // the first check is slow
        if (q.get('dry_run') === 'false') await new Promise((res) => setTimeout(res, 500)); // and the import takes a moment
        if (net.slow) await new Promise((res) => setTimeout(res, net.slow));
        if (net.fail) return r.fulfill({ status: 502, contentType: 'application/json', body: JSON.stringify({ error: 'The node is busy' }) });
        await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(answer(q)) });
      });
      try {
        await fx.go(page, '/library/import');
        const drawn = await page.waitForSelector('#roonfiles', { state: 'attached', timeout: 8000 }).then(() => true, () => false);
        check(drawn, `${width}: Your library › Import is drawn`);
        if (!drawn) continue;
        const look = await page.evaluate(() => {
          const lab = document.querySelector('label.file-btn:has(#roonfiles)');
          const inp = document.querySelector('#roonfiles'); const r = inp.getBoundingClientRect();
          return { btn: !!lab && !!lab.offsetParent && /Choose Roon export files/.test(lab.textContent), native: r.width > 2 && r.height > 2 && getComputedStyle(inp).opacity !== '0', check: !!document.querySelector('#rooncheck'), go: document.querySelector('#roongo').disabled };
        });
        check(look.btn, `${width}: Move from Roon has a Choose Roon export files button`);
        check(!look.native, `${width}: the browser's own file control is not shown`);
        check(!look.check, `${width}: there is no separate Check button`);
        check(look.go, `${width}: Import waits until files are checked`);
        await page.screenshot({ path: path.join(SHOTS, `import-${width}.png`), fullPage: true });

        // The keyboard reaches both file buttons on this page; the button shows the focus.
        const plKey = await tabTo(page, 'libplimport');
        check(plKey, `${width}: Tab reaches Choose a playlist file`);
        if (plKey) check(await ringOn(page, 'libplimport'), `${width}: Choose a playlist file shows the keyboard's focus`);
        const roonKey = await tabTo(page, 'roonfiles'); let chosen = false;
        check(roonKey, `${width}: Tab reaches Choose Roon export files`);
        if (roonKey) {
          check(await ringOn(page, 'roonfiles'), `${width}: Choose Roon export files shows the keyboard's focus`);
          await page.screenshot({ path: path.join(SHOTS, `roon-focus-${width}.png`) });
          // Enter opens the picker; choosing a file checks it without another press.
          choosers.length = 0;
          await page.keyboard.press('Enter');
          for (let i = 0; i < 30 && !choosers.length; i++) await page.waitForTimeout(100);
          const chooser = choosers[0] || null;
          const why = chooser ? '' : await page.evaluate(() => ` (focus on #${document.activeElement && document.activeElement.id})`);
          check(!!chooser, `${width}: Enter on it opens the file picker${why}`);
          if (chooser) { await chooser.setFiles(FILE); chosen = true; }
        }
        if (!chosen) await page.locator('#roonfiles').setInputFiles(FILE).catch(() => {});
        await page.waitForTimeout(150);
        check(calls.length === 1 && calls[0].dry === 'true', `${width}: choosing a file checks it right away, nothing changed (${JSON.stringify(calls)})`);
        const busy = await page.evaluate(() => { const o = document.querySelector('#roonout'); return { text: o.textContent, go: document.querySelector('#roongo').disabled }; });
        check(/Checking/.test(busy.text) && busy.go, `${width}: while it checks, it says so and Import waits (${busy.text})`);

        // An option changed during the slow first check: the second check is the one drawn.
        await page.locator('#roontags').setChecked(true);
        await page.waitForFunction(() => /Checked: nothing changed yet/.test(document.querySelector('#roonout').textContent), null, { timeout: 5000 }).catch(() => {});
        await page.waitForTimeout(1400); // the first, slow answer comes in after the second
        const after = await page.evaluate(() => ({ text: document.querySelector('#roonout').textContent, go: document.querySelector('#roongo').disabled }));
        check(calls.length === 2 && calls[1].dry === 'true' && calls[1].tags === 'true', `${width}: changing an option checks again with it (${JSON.stringify(calls)})`);
        check(/2 of 3 matched/.test(after.text) && !/1 of 3 matched/.test(after.text), `${width}: the latest check is the one shown (${after.text.slice(0, 120)})`);
        check(!after.go, `${width}: Import is ready once something matched`);
        await page.screenshot({ path: path.join(SHOTS, `checked-${width}.png`), fullPage: true });

        // Import: the real run, with the options as checked; the controls wait during it. A double press sends it once.
        const last = calls.length;
        await page.locator('#roongo').dblclick({ timeout: 2000 }).catch(() => {});
        const during = await page.evaluate(() => ['#roonfiles', '#roonfav', '#roonpl', '#roontags'].every((q) => document.querySelector(q).disabled));
        await page.waitForFunction(() => /Imported\./.test(document.querySelector('#roonout').textContent), null, { timeout: 5000 }).catch(() => {});
        const done = await page.evaluate(() => ({ text: document.querySelector('#roonout').textContent, go: document.querySelector('#roongo').disabled, free: ['#roonfiles', '#roonfav', '#roonpl', '#roontags'].every((q) => !document.querySelector(q).disabled) }));
        check(calls.length === last + 1 && calls[last].dry === 'false' && calls[last].tags === 'true', `${width}: Import sends the real run with the chosen options (${JSON.stringify(calls[last])})`);
        check(during, `${width}: the files and options can't change during the import`);
        check(/Imported\./.test(done.text) && done.go && done.free, `${width}: it says Imported, Import is done, the options are free again`);

        // After the import, an option changed doesn't offer the same files' Import again.
        let n = calls.length;
        await page.locator('#roontags').setChecked(false); await page.waitForTimeout(300);
        check(calls.length === n && await page.locator('#roongo').isDisabled(), `${width}: after an import, changing an option doesn't offer Import again (${calls.length - n} calls)`);

        // Choosing the same file again checks it again, with the last results kept (dimmed) while it does.
        n = calls.length; net.slow = 600;
        await page.locator('#roonfiles').setInputFiles(FILE);
        await page.waitForTimeout(200);
        const kept = await page.evaluate(() => ({ stale: document.querySelectorAll('#roonout .roonfile.stale').length, text: document.querySelector('#roonout').textContent }));
        check(calls.length === n + 1 && calls[n].dry === 'true', `${width}: choosing the same file again checks it again (${calls.length - n} calls)`);
        check(kept.stale > 0 && /Checking/.test(kept.text), `${width}: while it checks again, the last results stay, dimmed (${kept.stale})`);
        await page.waitForFunction(() => /Checked: nothing changed yet/.test(document.querySelector('#roonout').textContent), null, { timeout: 5000 }).catch(() => {});
        check(!(await page.locator('#roongo').isDisabled()), `${width}: newly chosen files offer Import again`);
        net.slow = 0;

        // A check that fails has Check again; once the node answers, Import is ready.
        net.fail = true; n = calls.length;
        await page.locator('#roonfiles').setInputFiles(FILE);
        await page.waitForSelector('#roonout .joberr', { timeout: 5000 }).catch(() => {});
        const bad = await page.evaluate(() => ({ err: (document.querySelector('#roonout .joberr') || {}).textContent || '', again: !!document.querySelector('#roonout [data-roon-again]'), go: document.querySelector('#roongo').disabled, nothing: /Nothing in these files matched/.test(document.querySelector('#roonout').textContent) }));
        check(/busy/.test(bad.err) && bad.again && bad.go, `${width}: a failed check says why, has Check again, and Import waits (${bad.err})`);
        check(!bad.nothing, `${width}: a failed check doesn't say that nothing matched`);
        await page.screenshot({ path: path.join(SHOTS, `failed-${width}.png`), fullPage: true });
        net.fail = false; n = calls.length;
        await page.locator('#roonout [data-roon-again]').click({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => /matched this library/.test(document.querySelector('#roonout').textContent), null, { timeout: 5000 }).catch(() => {});
        check(calls.length === n + 1 && calls[n].dry === 'true' && !(await page.locator('#roongo').isDisabled()), `${width}: Check again checks once more and Import is ready (${calls.length - n} calls)`);

        const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
        check(o <= 0, `${width}: nothing wider than the screen (${o})`);

        // Playlists' own Import button is in the Tab order too.
        await fx.go(page, '/playlists');
        await page.waitForSelector('#plimport', { state: 'attached', timeout: 8000 }).catch(() => {});
        check(await tabTo(page, 'plimport'), `${width}: Tab reaches Playlists' Import M3U, CSV or Roon export`);
        // And Settings › Backup's Choose a backup file.
        await fx.go(page, '/settings/backup');
        const rs = await page.waitForSelector('#rsfile', { state: 'attached', timeout: 8000 }).then(() => true, () => false);
        check(rs && await tabTo(page, 'rsfile'), `${width}: Tab reaches Settings › Backup's Choose a backup file`);
        check(!state.errors.length, `${width}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
