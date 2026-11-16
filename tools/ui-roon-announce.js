// Your library › Import › Move from Roon, for a screen reader. Its whole results area (#roonout) was
// a polite live region and was redrawn for every file, so a check of three files read "Checking 1 of
// 3…", "Checking 2 of 3…", "Checking 3 of 3…" and then every file's name, counts, Roon tags and
// "not moved, and why" again. Now only a small status line is live: what started, then one short
// line with the outcome (how many matched, nothing changed yet, a failure, Imported). The visible
// results are unchanged. Import sends only the files whose check went through (and says the others
// were left out), says "Nothing was imported" when nothing matched, and a check left behind by leaving
// Import says nothing on the page drawn again, nor frees its controls during a newer import. At 1280,
// 1024 and 768 px.
//   node tools/ui-roon-announce.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'roon-announce-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// A dry run's answer (roon_import.rs) per file: tracks 1 of 3 matched, albums 2 of 4, the playlist 0 of 2.
const answer = (q) => {
  const file = q.get('file'); const dry = q.get('dry_run') === 'true';
  const n = { 'tracks.xlsx': [3, 1, 'tracks'], 'albums.xlsx': [4, 2, 'albums'], 'mix.xlsx': [2, 0, 'playlist'] }[file] || [1, 0, 'tracks'];
  return { kind: n[2], name: n[2] === 'playlist' ? 'Mix' : null, rows: n[0], matched: !dry && answer.zero ? 0 : n[1], unmatched: n[0] - n[1], not_identified: 0, favourites: 0, tags_added: 0, playlist: null, tags_found: [['live', 2]], misses: [{ what: 'Unknown song', reason: 'not in this library' }], dry };
};
const file = (name) => ({ name, mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', buffer: Buffer.from('PK fixture') });
const THREE = [file('tracks.xlsx'), file('albums.xlsx'), file('mix.xlsx')];

// Records what a screen reader would be told: every change to text inside a live region
// (aria-live other than off, or role status/alert/log), as the region's text after the change.
const LISTEN = () => {
  window.__said = [];
  const live = (n) => { for (let e = n && (n.nodeType === 1 ? n : n.parentElement); e; e = e.parentElement) { const a = e.getAttribute('aria-live'); if (a && a !== 'off') return e; if (/^(status|alert|log)$/.test(e.getAttribute('role') || '')) return e; } return null; };
  new MutationObserver((ms) => {
    const seen = new Set();
    for (const m of ms) { const r = live(m.target); if (r && !seen.has(r)) { seen.add(r); const t = r.textContent.replace(/\s+/g, ' ').trim(); if (t) window.__said.push({ id: r.id, text: t }); } }
  }).observe(document.documentElement, { subtree: true, childList: true, characterData: true });
};
// Leave Import for Songs and come Back, inside the app (the page is drawn again; nothing is reloaded).
const away = async (page) => {
  await page.click('[data-view="tracks"]'); await page.waitForFunction(() => location.pathname === '/tracks', null, { timeout: 4000 }).catch(() => {});
  await page.goBack(); await page.waitForFunction(() => location.pathname === '/library/import' && !!document.querySelector('#roonfiles'), null, { timeout: 4000 }).catch(() => {});
};
const said = (page) => page.evaluate(() => window.__said.filter((x) => x.id !== 'toasts' && !/toast/.test(x.id)));
const reset = (page) => page.evaluate(() => { window.__said = []; });

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      const { page, context, state } = await fx.create(browser, width, {});
      const net = { fail: null, slow: 0, importSlow: 0 }; const sent = [];
      await page.route('**/v1/import/roon*', async (r) => {
        const q = new URL(r.request().url()).searchParams; sent.push({ file: q.get('file'), dry: q.get('dry_run') });
        await new Promise((res) => setTimeout(res, 150 + net.slow + (q.get('dry_run') === 'false' ? net.importSlow : 0)));
        if (net.fail && q.get('file') === net.fail) return r.fulfill({ status: 502, contentType: 'application/json', body: JSON.stringify({ error: 'The node is busy' }) });
        if (net.odd && q.get('file') === net.odd) return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...answer(q), tags_found: null }) });
        await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(answer(q)) });
      });
      try {
        await fx.go(page, '/library/import');
        const drawn = await page.waitForSelector('#roonfiles', { state: 'attached', timeout: 8000 }).then(() => true, () => false);
        check(drawn, `${width}: Your library › Import is drawn`);
        if (!drawn) continue;
        await page.evaluate(LISTEN);
        const region = await page.evaluate(() => {
          const out = document.querySelector('#roonout');
          let e = out; let liveOut = false; for (; e; e = e.parentElement) { const a = e.getAttribute('aria-live'); if ((a && a !== 'off') || /^(status|alert|log)$/.test(e.getAttribute('role') || '')) liveOut = true; }
          const say = document.querySelector('#roonsay'); const r = say && say.getBoundingClientRect();
          return { liveOut, say: !!say && (say.getAttribute('role') === 'status' || say.getAttribute('aria-live') === 'polite'), hidden: !!r && r.width <= 1 && r.height <= 1, outside: !!say && !out.contains(say) };
        });
        check(!region.liveOut, `${width}: the results area is not read aloud as it is redrawn`);
        check(region.say && region.hidden && region.outside, `${width}: a visually hidden status line, outside the results, is what is read (${JSON.stringify(region)})`);

        // A check of three files: one "Checking 3 files…", then one line with the outcome.
        await page.locator('#roonfiles').setInputFiles(THREE);
        await page.waitForFunction(() => /Checked: nothing changed yet/.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(100);
        let s = await said(page);
        const texts = s.map((x) => x.text);
        check(s.length > 0 && s.length <= 2, `${width}: checking 3 files is said in at most 2 lines, not per file (${s.length}: ${JSON.stringify(texts).slice(0, 300)})`);
        check(texts.every((t) => t.length <= 120), `${width}: each line is short, not the list of files (${Math.max(0, ...texts.map((t) => t.length))} characters)`);
        check(!texts.some((t) => /of 3…|tracks\.xlsx|not moved/.test(t)), `${width}: no per-file progress or file list is read`);
        check(/^Checking 3 files…$/.test(texts[0] || ''), `${width}: it says what started (${texts[0]})`);
        check(/Checked: 3 of 9 matched this library\. Nothing changed yet; Import is ready\./.test(texts[texts.length - 1] || ''), `${width}: then how many matched, that nothing changed and that Import is ready (${texts[texts.length - 1]})`);
        const vis = await page.evaluate(() => ({ files: document.querySelectorAll('#roonout .roonfile').length, go: document.querySelector('#roongo').disabled }));
        check(vis.files === 3 && !vis.go, `${width}: the visible results are still there, one per file, and Import is ready (${vis.files})`);
        await page.screenshot({ path: path.join(SHOTS, `checked-${width}.png`), fullPage: true });

        // A check where one file fails.
        await reset(page); net.fail = 'albums.xlsx';
        await page.locator('#roonfiles').setInputFiles(THREE);
        await page.waitForSelector('#roonout [data-roon-again]', { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(100);
        s = await said(page);
        let end = (s[s.length - 1] || {}).text || '';
        check(s.length <= 2 && /^Not every file could be checked; in the others, 1 of 5 matched this library\. Nothing changed yet\. Import is ready for those\. Check again is above the results\.$/.test(end), `${width}: a failed check says so in one line (${s.length}: ${end})`);
        // Import after it sends only the files that checked out, and says the other one was left out.
        net.fail = null; await reset(page); let n0 = sent.length;
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => /Imported\./.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(100);
        s = await said(page); end = (s[s.length - 1] || {}).text || '';
        const partial = sent.slice(n0);
        check(partial.length === 2 && partial.every((x) => x.dry === 'false') && !partial.some((x) => x.file === 'albums.xlsx'), `${width}: Import after a partly failed check sends only the files that checked out (${JSON.stringify(partial)})`);
        check(/^Importing 2 files…$/.test((s[0] || {}).text || '') && /^Imported: 1 of 5 matched this library\. The file whose check failed was left out\. Check again is above the results\.$/.test(end), `${width}: it says the file whose check failed was left out (${JSON.stringify(s.map((x) => x.text))})`);
        const leftVis = await page.evaluate(() => (document.querySelector('#roonout [data-roonstate]') || {}).textContent || '');
        check(/^Imported\. The file whose check failed was left out\. Check it again$/.test(leftVis), `${width}: and shows it, with Check it again (${leftVis})`);
        // Check it again checks just the file left out; its Import sends just that file.
        n0 = sent.length;
        await page.locator('#roonout [data-roon-again]').click({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => !document.querySelector('#roongo').disabled, null, { timeout: 8000 }).catch(() => {});
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => /Imported\./.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        const retry = sent.slice(n0);
        check(retry.length === 2 && retry.every((x) => x.file === 'albums.xlsx') && retry[0].dry === 'true' && retry[1].dry === 'false', `${width}: Check it again checks and imports only the file left out (${JSON.stringify(retry)})`);

        // A file whose answer can't be read (a 200 without its tag list) counts as failed: Import leaves it out.
        await reset(page); net.odd = 'albums.xlsx';
        await page.locator('#roonfiles').setInputFiles(THREE);
        await page.waitForSelector('#roonout [data-roon-again]', { timeout: 8000 }).catch(() => {});
        await page.waitForFunction(() => !document.querySelector('#roongo').disabled, null, { timeout: 8000 }).catch(() => {});
        net.odd = null; n0 = sent.length;
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => /Imported\./.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        const odd = sent.slice(n0);
        check(odd.length === 2 && !odd.some((x) => x.file === 'albums.xlsx'), `${width}: a file whose check answer could not be read is not imported (${JSON.stringify(odd)})`);

        // Nothing matched.
        await reset(page);
        await page.locator('#roonfiles').setInputFiles([file('mix.xlsx')]);
        await page.waitForFunction(() => /Nothing in these files matched/.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(100);
        s = await said(page); end = (s[s.length - 1] || {}).text || '';
        check(/^Checking…$/.test((s[0] || {}).text || '') && /^Checked: nothing in these files matched this library yet\. Scan your music folders first, then choose the files again\.$/.test(end), `${width}: one file: "Checking…", then that nothing matched (${JSON.stringify(s.map((x) => x.text))})`);

        // Import: what started, then Imported with the count.
        await page.locator('#roonfiles').setInputFiles(THREE);
        await page.waitForFunction(() => !document.querySelector('#roongo').disabled, null, { timeout: 8000 }).catch(() => {});
        await reset(page);
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => /Imported\./.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(100);
        s = await said(page); end = (s[s.length - 1] || {}).text || '';
        check(s.length <= 2 && /^Importing 3 files…$/.test((s[0] || {}).text || '') && /^Imported: 3 of 9 matched this library\.$/.test(end), `${width}: Import says it started and then Imported with the count (${JSON.stringify(s.map((x) => x.text))})`);
        await page.screenshot({ path: path.join(SHOTS, `imported-${width}.png`), fullPage: true });

        // An import where one file fails: said in one line, with what did get in.
        await page.locator('#roonfiles').setInputFiles(THREE);
        await page.waitForFunction(() => !document.querySelector('#roongo').disabled, null, { timeout: 8000 }).catch(() => {});
        await reset(page); net.fail = 'albums.xlsx';
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => /Imported\./.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(100);
        s = await said(page); end = (s[s.length - 1] || {}).text || '';
        check(s.length <= 2 && /^Imported what matched \(1 of 5 matched this library\), but not every file could be imported\. The results say which\.$/.test(end), `${width}: an import with a failed file says so in one line (${s.length}: ${end})`);
        net.fail = null;

        // An import where every file fails doesn't say Imported.
        await page.locator('#roonfiles').setInputFiles([file('tracks.xlsx')]);
        await page.waitForFunction(() => !document.querySelector('#roongo').disabled, null, { timeout: 8000 }).catch(() => {});
        await reset(page); net.fail = 'tracks.xlsx';
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {});
        await page.waitForSelector('#roonout .joberr', { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(100);
        s = await said(page); end = (s[s.length - 1] || {}).text || '';
        check(/^Nothing was imported\. The results say why\.$/.test(end), `${width}: an import where every file fails says nothing was imported (${end})`);
        net.fail = null;

        // An import where nothing matched after all (the library changed since the check): "Nothing was imported".
        await page.locator('#roonfiles').setInputFiles([file('tracks.xlsx')]);
        await page.waitForFunction(() => !document.querySelector('#roongo').disabled, null, { timeout: 8000 }).catch(() => {});
        await reset(page); answer.zero = true;
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {});
        await page.waitForFunction(() => /Nothing was imported/.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        await page.waitForTimeout(100);
        s = await said(page); end = (s[s.length - 1] || {}).text || '';
        const zeroVis = await page.evaluate(() => (document.querySelector('#roonout [data-roonstate]') || {}).textContent || '');
        check(/^Nothing was imported: nothing in these files matched this library\.$/.test(end) && zeroVis === 'Nothing was imported.', `${width}: an import where nothing matched says nothing was imported (${end} | ${zeroVis})`);
        answer.zero = false;

        // An option changed during a check: the new check is said again, then one outcome, the latest.
        net.slow = 400;
        await page.locator('#roonfiles').setInputFiles(THREE);
        await page.waitForTimeout(200);
        await reset(page);
        await page.locator('#roonpl').setChecked(false);
        await page.waitForFunction(() => /Checked: nothing changed yet/.test(document.querySelector('#roonout').textContent), null, { timeout: 12000 }).catch(() => {});
        await page.waitForTimeout(1500); // the first check's late answers come in, and must say nothing
        s = await said(page);
        check(s.length === 2 && s[0].text === 'Checking 3 files…' && /^Checked: 3 of 9 matched this library\./.test(s[1].text), `${width}: an option changed mid-check: "Checking 3 files…" is said again, then one outcome (${JSON.stringify(s.map((x) => x.text))})`);
        net.slow = 0;

        // A check left behind by leaving Import (and coming back) says nothing over the page drawn again.
        net.slow = 1500;
        await page.locator('#roonfiles').setInputFiles(THREE);
        await page.waitForTimeout(200);
        await away(page);
        await page.waitForSelector('#roonsay', { state: 'attached', timeout: 8000 }).catch(() => {});
        await page.evaluate(LISTEN);
        await page.waitForTimeout(5500); // the old check's three answers come in
        s = await said(page);
        const back = await page.evaluate(() => ({ say: document.querySelector('#roonsay').textContent, go: document.querySelector('#roongo').disabled }));
        check(!s.length && back.say === '' && back.go, `${width}: a check left behind says nothing on the page drawn again (${JSON.stringify(s.map((x) => x.text))}, "${back.say}")`);
        net.slow = 0;

        // An import left behind doesn't free the controls of a newer import when it ends.
        net.importSlow = 2500;
        await page.locator('#roonfiles').setInputFiles([file('tracks.xlsx')]);
        await page.waitForFunction(() => !document.querySelector('#roongo').disabled, null, { timeout: 8000 }).catch(() => {});
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {}); const t0 = Date.now();
        await page.waitForTimeout(200);
        await away(page);
        await page.waitForSelector('#roonfiles', { state: 'attached', timeout: 8000 }).catch(() => {});
        await page.locator('#roonfiles').setInputFiles([file('tracks.xlsx')]);
        await page.waitForFunction(() => !document.querySelector('#roongo').disabled, null, { timeout: 8000 }).catch(() => {});
        await page.locator('#roongo').click({ timeout: 2000 }).catch(() => {});
        await page.waitForTimeout(Math.max(0, t0 + 3000 - Date.now())); // the first import has ended; the second still runs
        const held = await page.evaluate(() => ({ off: ['#roonfiles', '#roonfav', '#roonpl', '#roontags'].every((q) => document.querySelector(q).disabled), busy: /Importing/.test(document.querySelector('#roonout').textContent) }));
        check(held.off && held.busy, `${width}: an import left behind doesn't free a newer import's files and options (${JSON.stringify(held)})`);
        await page.waitForFunction(() => /Imported\./.test(document.querySelector('#roonout').textContent), null, { timeout: 8000 }).catch(() => {});
        net.importSlow = 0;

        check(!state.errors.length, `${width}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
