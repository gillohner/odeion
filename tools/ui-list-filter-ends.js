// A list shown through a filter must still look like a list: the row that is now its last has no
// line under it (the list's own border is there) and, with keyboard focus, its ring follows the
// list's round corners; the same for the row that is now first. A playlist's "Filter by source"
// chips hide rows (`.filtered`) that used to be the list's first/last child, which left the visible
// end row with a doubled bottom line and square ring corners inside the round list.
// Real UI in Chromium against fixtures (no node), 1280, 1024 and 768 px, plus a 412 px phone
// (its song lists are square, so no round corner to check there; the doubled line is checked the same).
//   node tools/ui-list-filter-ends.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'list-filter-ends-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// Four songs: TIDAL, library, library, TIDAL. "Your library" hides the first and the last row;
// "TIDAL" hides the two in the middle (its ends are the list's real ends).
const SVC = ['tidal', 'library', 'library', 'tidal'];
const uri = (k) => `https://musicbrainz.org/recording/00000000-0000-4000-8000-00000000000${k}`;
const tracks = SVC.map((_, k) => ({ ...fx.track, id: k + 1, title: `Song ${k + 1}`, canonical_uri: uri(k), mbid: uri(k).split('/').pop() }));
const summary = { ...fx.summary, items: 4 };
const detail = { summary, playlist: { ...summary, items: tracks.map((t) => ({ uri: t.canonical_uri })) }, tracks, entries: tracks.map((t) => ({ kind: 'track', uri: t.canonical_uri })), local_tracks: [] };
const svcName = { library: 'Your library', tidal: 'TIDAL' };
const pf = { order: { order: ['library', 'tidal'], services: [{ id: 'library', name: 'Your library', connected: true }, { id: 'tidal', name: 'TIDAL', connected: true }] }, own_order: false,
  items: tracks.map((t, k) => ({ index: k, uri: t.canonical_uri, title: t.title, artist: 'Sefa', track_id: SVC[k] === 'library' ? t.id : null, plays_from: { service: SVC[k], label: svcName[SVC[k]] }, sources: [{ service: SVC[k], label: svcName[SVC[k]], available: true }] })) };
const fixtures = { '/playlists/c1': { body: detail }, '/playlists/c1/play-from': { body: pf }, '/playlists': { body: [summary] } };

// The visible rows' ends: the line under the last, and (when ringed) the corners of the first/last
const ends = () => {
  const list = document.querySelector('#plsongs .list'); const ls = getComputedStyle(list);
  const vis = [...list.children].filter((r) => r.classList.contains('list-row') && r.getClientRects().length);
  const one = (r) => { const s = getComputedStyle(r); return { title: r.querySelector('.name').textContent.trim(), bb: parseFloat(s.borderBottomWidth) || 0, tl: parseFloat(s.borderTopLeftRadius) || 0, tr: parseFloat(s.borderTopRightRadius) || 0, bl: parseFloat(s.borderBottomLeftRadius) || 0, br: parseFloat(s.borderBottomRightRadius) || 0 }; };
  return { listR: parseFloat(ls.borderTopLeftRadius) || 0, n: vis.length, first: one(vis[0]), last: one(vis[vis.length - 1]), mid: vis.slice(0, -1).map((r) => parseFloat(getComputedStyle(r).borderBottomWidth) || 0) };
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, phone] of [[1280, false], [1024, false], [768, false], [412, true]]) {
      const { context, page, state } = await fx.create(browser, width, { fixtures, collections: [summary], ...(phone ? { mobile: true, touch: true } : {}) });
      await fx.go(page, '/playlist/c1');
      await page.waitForSelector('.plsummary [data-pfilter]', { timeout: 7000 });
      for (const [key, want] of [['', 4], ['library', 2], ['tidal', 2]]) {
        await page.click(`.plsummary [data-pfilter="${key}"]`);
        await page.waitForFunction((n) => document.querySelectorAll('#plsongs .srow:not(.filtered)').length === n, want).catch(() => {}); // (a wrong count is the next check's FAIL)
        const tag = `${width} ${key || 'all'}`;
        let e = await page.evaluate(ends);
        check(e.n === want, `${tag}: ${e.n} rows shown (want ${want})`);
        check(e.last.bb === 0, `${tag}: no line under the last row shown, "${e.last.title}" (${e.last.bb} px; the list's border is there)`);
        check(e.mid.every((b) => b === 1), `${tag}: a line under every other row shown (${e.mid.join(',')})`);
        if (!phone) {
          // Keyboard onto the first and the last row shown: the ring follows the list's corners
          const want0 = Math.max(0, e.listR - 1);
          // The row itself takes the focus (the tagger makes rows keyable in its next frame)
          await page.waitForFunction(() => [...document.querySelectorAll('#plsongs .srow')].every((r) => r.hasAttribute('tabindex')));
          for (const which of ['first', 'last']) {
            await page.evaluate((w) => { const rows = [...document.querySelectorAll('#plsongs .srow:not(.filtered)')]; (w === 'first' ? rows[0] : rows[rows.length - 1]).focus(); }, which);
            // :focus-visible after a script focus needs a keyboard event first
            await page.keyboard.press('Shift'); e = await page.evaluate(ends);
            const fv = await page.evaluate(() => document.activeElement.matches(':focus-visible'));
            const r = e[which]; const got = which === 'first' ? [r.tl, r.tr] : [r.bl, r.br];
            check(fv && got.every((x) => Math.abs(x - want0) < 0.6), `${tag}: ringed ${which} row "${r.title}" has the list's corners (${got.join('/')} px, want ${want0}${fv ? '' : ', not focus-visible'})`);
            if (which === 'last' && key === 'library') await page.screenshot({ path: path.join(SHOTS, `filter-ends-${width}.png`), clip: await page.evaluate(() => { const b = document.querySelector('#plsongs .list').getBoundingClientRect(); return { x: Math.max(0, b.left - 8), y: Math.max(0, b.top - 8), width: Math.min(innerWidth - Math.max(0, b.left - 8), b.width + 16), height: b.height + 16 }; }) });
          }
          await page.evaluate(() => document.activeElement.blur());
        } else if (key === 'library') await page.screenshot({ path: path.join(SHOTS, `filter-ends-${width}.png`), fullPage: false });
      }
      check(!state.errors.length, `${width}: no page errors ${JSON.stringify(state.errors)}`);
      await fx.cleanCopy(page).then(() => check(true, `${width}: clean copy, no overflow`), (er) => check(false, `${width}: ${er.message}`));
      await context.close();
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
