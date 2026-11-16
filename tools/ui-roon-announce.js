// Your library › Import › Move from Roon, for a screen reader. Its whole results area (#roonout) was
// a polite live region and was redrawn for every file, so a check of three files read "Checking 1 of
// 3…", "Checking 2 of 3…", "Checking 3 of 3…" and then every file's name, counts, Roon tags and
// "not moved, and why" again. Now only a small status line is live: what started, then one short
// line with the outcome (how many matched, nothing changed yet, a failure, Imported). The visible
// results are unchanged. At 1280, 1024 and 768 px.
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
  return { kind: n[2], name: n[2] === 'playlist' ? 'Mix' : null, rows: n[0], matched: n[1], unmatched: n[0] - n[1], not_identified: 0, favourites: 0, tags_added: 0, playlist: null, tags_found: [['live', 2]], misses: [{ what: 'Unknown song', reason: 'not in this library' }], dry };
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
const said = (page) => page.evaluate(() => window.__said.filter((x) => x.id !== 'toasts' && !/toast/.test(x.id)));
const reset = (page) => page.evaluate(() => { window.__said = []; });

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      const { page, context, state } = await fx.create(browser, width, {});
      const net = { fail: null, slow: 0 };
      await page.route('**/v1/import/roon*', async (r) => {
        const q = new URL(r.request().url()).searchParams;
        await new Promise((res) => setTimeout(res, 150 + net.slow));
        if (net.fail && q.get('file') === net.fail) return r.fulfill({ status: 502, contentType: 'application/json', body: JSON.stringify({ error: 'The node is busy' }) });
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
        net.fail = null;

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
        check(/^Nothing could be imported\. The results say why\.$/.test(end), `${width}: an import where every file fails says nothing could be imported (${end})`);
        net.fail = null;

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

        check(!state.errors.length, `${width}: no page errors (${state.errors.join(' | ')})`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
