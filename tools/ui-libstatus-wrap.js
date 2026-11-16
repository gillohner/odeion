// Node library's top line ("12,840 songs in the library · last scan 3 h ago · 37 files could not be
// read · Review 64 unsure") at phone widths (and in the Android app) and desktop. When it wraps, no
// row starts with "·" (it did: "· Review 64 unsure" began the second or third row), a part moves
// down whole, the first part of each row lines up with the page's text, the Review button's focus
// ring has room, and big counts read with separators (12,840, not 12840). A count drawn again by
// the Identify card (none left) leaves no stray dot. "N files could not be read" is a link that
// opens the list of those files (Music folders) and goes to it. When the node kept only 20 of them, a
// note under the list says so.
// Real candidate assets, isolated responses: node tools/ui-libstatus-wrap.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'libstatus-wrap-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const now = Math.floor(Date.now() / 1000);
const info = (scan) => ({ body: { name: 'Fixture node', library: { tracks: 12840, albums: 900, artists: 300, files: 12900, missing_files: 0 }, scanning: false, ...scan } });
const identify = (suggested) => ({ body: { phase: suggested ? 'needs_review' : 'ready', progress: { running: false, total: 30, done: 30, identified: 20, suggested, unknown: 4, errors: 0, finished_at: 1 }, counts: { total: 12840, identified: 12000, suggested, unknown: 776, unchecked: 0, by: { tags: 10, album: 60, fingerprint: 8, you: 2, title_artist: 10 } }, acoustid: true, acoustid_own: true, acoustid_state: 'ok', write_ids: false } });
const CASES = {
  errors: { '/info': info({ last_scan: { finished_at: now - 3 * 3600, error_count: 1037, errors: [['/music/A/b.flac', 'bad header'], ...Array.from({ length: 19 }, (_, k) => [`/music/C/${k + 2}.mp3`, 'no audio frames'])] } }), '/library/identify': identify(64), '/library/folders': { body: ['Music', 'Live', 'Vinyl rips', 'Podcasts', 'Old CDs', 'Downloads'].map((f) => [`/srv/media/${f}`, true]) } },
  // Two unreadable files and a folder that could not be read, all listed: no "N of M" note
  few: { '/info': info({ last_scan: { finished_at: now - 3 * 3600, error_count: 3, errors: [['/music/A/b.flac', 'bad header'], ['/music/C/2.mp3', 'no audio frames'], ['/mnt/gone', 'No such file or directory']] } }), '/library/identify': identify(64) },
  scan: { '/info': info({ last_scan: { finished_at: now - 3 * 3600, error_count: 0, errors: [] } }), '/library/identify': identify(64) },
  scanning: { '/info': info({ scanning: true }), '/library/identify': identify(64) },
};
const common = { '/library/webdav': { body: [] }, '/library/drives': { body: { roots: [], providers: [] } }, '/library/hidden': { body: [] }, '/library/identify/settings': { body: {} } };

// The line's rows as drawn: the visible characters (not clipped by the line) grouped by row, and
// each row's first visible character's left edge
const rows = (page) => page.evaluate(() => {
  const line = document.querySelector('.libstatus');
  const cs = getComputedStyle(line); const lr = line.getBoundingClientRect();
  const clipLeft = cs.overflowX === 'clip' || cs.overflowX === 'hidden' ? lr.left + parseFloat(cs.borderLeftWidth) : -Infinity;
  const out = {};
  const add = (c, r) => {
    if (!r.width || r.left + r.width / 2 < clipLeft) return; // nothing drawn, or clipped off
    const top = Math.round(r.top + r.height / 2);
    const key = Object.keys(out).find((t) => Math.abs(t - top) < 8) ?? top;
    (out[key] ||= []).push({ c, left: r.left, right: r.right });
  };
  const walk = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
  for (let n = walk.nextNode(); n; n = walk.nextNode()) {
    for (let i = 0; i < n.data.length; i++) {
      if (!n.data[i].trim()) continue;
      const rg = document.createRange(); rg.setStart(n, i); rg.setEnd(n, i + 1);
      add(n.data[i], rg.getBoundingClientRect());
    }
  }
  line.querySelectorAll('.spin').forEach((e) => add('◌', e.getBoundingClientRect())); // the scan's spinner starts its row
  const text = document.querySelector('.libstatus + .muted.small'); const textLeft = text ? text.getBoundingClientRect().left : lr.left;
  const parts = [...line.querySelectorAll('.lspart')].map((p) => { const t = p.lastElementChild.getBoundingClientRect(); return { h: t.height, lh: parseFloat(getComputedStyle(p).lineHeight) || 18, text: p.lastElementChild.textContent }; });
  const rv = document.querySelector('#libreview'); const rvr = rv && rv.getBoundingClientRect();
  const clipRight = clipLeft === -Infinity ? Infinity : lr.right - parseFloat(cs.borderRightWidth);
  return { clipLeft, clipRight, textLeft, parts, review: rvr ? { left: rvr.left, right: rvr.right } : null, rows: Object.keys(out).sort((a, b) => a - b).map((k) => { const cs2 = out[k].sort((a, b) => a.left - b.left); return { text: cs2.map((x, i) => (i && x.left - cs2[i - 1].right > 2 ? ' ' : '') + x.c).join(''), first: cs2[0].c, left: cs2[0].left }; }) };
});

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, true], [412, false], [1280, false]]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      for (const [name, fixtures0] of Object.entries(CASES)) {
        const fixtures = { ...common, ...fixtures0 };
        const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures });
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
        try {
          await fx.go(page, '/library/node');
          const drawn = await page.waitForFunction(() => document.querySelector('.libstatus #libreview'), null, { timeout: 6000 }).then(() => true, () => false);
          check(drawn, `${at} ${name}: the top line offers Review 64 unsure`);
          if (!drawn) continue;
          const g = await rows(page);
          const all = g.rows.map((r) => r.text).join(' / ');
          check(g.rows.every((r) => r.first !== '·'), `${at} ${name}: no row starts with "·" (${all})`);
          check(g.rows.every((r) => Math.abs(r.left - g.textLeft) < 4), `${at} ${name}: each row starts where the page's text does (${g.rows.map((r) => Math.round(r.left)).join(', ')} vs ${Math.round(g.textLeft)})`);
          check(/12,840 songs/.test(all) && !/12840/.test(all), `${at} ${name}: the song count reads 12,840`);
          if (name === 'errors') check(/1,037 files could not be read/.test(all) && /last scan 3 h ago/.test(all), `${at} ${name}: every part is there`);
          if (width < 600) check(g.parts.length > 0 && g.parts.every((p) => p.h < p.lh * 1.6), `${at} ${name}: a part moves down whole, not split over two rows (${g.parts.map((p) => Math.round(p.h)).join(', ')} px)`);
          if (width >= 1280) check(g.rows.length === 1, `${at} ${name}: one row on a desktop (${g.rows.length})`);
          // The Review button's focus ring (2px, 2px off) has room, wherever the button lands
          // (and its tap reach on a phone, 6px out: #libreview's ::before on a phone)
          check(g.review && g.review.left - 6 >= g.clipLeft - 0.5 && g.review.right + 6 <= g.clipRight + 0.5, `${at} ${name}: Review's focus ring and tap reach aren't cut (${g.review && Math.round(g.review.left - g.clipLeft)} px left, ${g.review && Math.round(g.clipRight - g.review.right)} px right)`);
          if (name === 'scan' && width === 360) {
            await page.locator('#libreview').focus();
            await page.keyboard.press('Shift+Tab'); await page.keyboard.press('Tab');
            await page.screenshot({ path: path.join(SHOTS, `${at}-${name}-focus.png`), clip: { x: 0, y: 260, width, height: 140 } });
          }
          await page.screenshot({ path: path.join(SHOTS, `${at}-${name}.png`) });
          await fx.cleanCopy(page).then(() => check(true, `${at} ${name}: clean copy, no horizontal overflow`), (e) => check(false, `${at} ${name}: ${e.message}`));
          if (name === 'errors') {
            // "1,037 files could not be read" on the line is a link to the list of them (in Music folders,
            // below six folders here): one press opens the list and goes to it
            const le = await page.evaluate(() => {
              const b = document.querySelector('.libstatus #liberrs'); if (!b) return null; const r = b.getBoundingClientRect(); const d = document.querySelector('#scanerrs');
              // A phone's tap 12px under the words still lands on it (44px reach, as Review's)
              const below = document.elementFromPoint(r.left + r.width / 2, r.bottom + 12);
              return { tag: b.tagName, text: b.textContent, left: r.left, right: r.right, reach: !!(below && b.contains(below)), controls: b.getAttribute('aria-controls'), listOpen: d ? d.open : null, listBelow: d ? d.getBoundingClientRect().top > innerHeight : null, title: d && d.querySelector('summary').textContent };
            });
            check(le && le.tag === 'BUTTON' && le.text === '1,037 files could not be read' && le.controls === 'scanerrs', `${at} ${name}: "1,037 files could not be read" is a button for the list of them (${le && le.tag} "${le && le.text}")`);
            if (le) {
              check(le.left - 6 >= g.clipLeft - 0.5 && le.right + 6 <= g.clipRight + 0.5, `${at} ${name}: its focus ring and tap reach aren't cut (${Math.round(le.left - g.clipLeft)} px left, ${Math.round(g.clipRight - le.right)} px right)`);
              if (width < 600) check(le.reach, `${at} ${name}: a tap 12px under it still opens the list (44px reach)`);
              check(le.title === '1,037 files could not be read in the last scan', `${at} ${name}: the list's title writes the count as the line does (${le.title})`);
              if (width < 600) check(le.listBelow === true && le.listOpen === false, `${at} ${name}: the list starts closed, out of sight below the folders`);
              await page.locator('#liberrs').click();
              const after = await page.evaluate(() => {
                const d = document.querySelector('#scanerrs'); const sm = d && d.querySelector('summary'); const r = sm && sm.getBoundingClientRect();
                const hit = r && document.elementFromPoint(r.left + Math.min(r.width / 2, 40), r.top + r.height / 2);
                const li = d && d.querySelector('li'); const lr = li && li.getBoundingClientRect();
                const liHit = lr && document.elementFromPoint(lr.left + 10, lr.top + lr.height / 2);
                return { open: d && d.open, focused: document.activeElement === sm, seen: !!(hit && sm.contains(hit)), item: !!(liHit && li.contains(liHit)), itemText: li && li.textContent };
              });
              check(after.open && after.focused, `${at} ${name}: one press opens the list and puts the keyboard on it (open ${after.open}, focus ${after.focused})`);
              check(after.seen && after.item && /A\/b\.flac: bad header/.test(after.itemText || ''), `${at} ${name}: …and the list is in sight, not under the player or the top bar (${after.itemText})`);
              // The node keeps only the first 20: the list says so, and where the rest are
              const more = await page.evaluate(() => { const n = document.querySelector('#scanerrs > ul + .scanmore'); if (!n) return null; n.scrollIntoView({ block: 'center', behavior: 'instant' }); const r = n.getBoundingClientRect(); const hit = document.elementFromPoint(r.left + 10, r.top + r.height / 2); return { text: n.textContent, seen: !!(hit && n.contains(hit)), items: document.querySelectorAll('#scanerrs li').length }; });
              check(more && more.items === 20 && more.text === '20 of 1,037 are listed here; the node’s log has the rest.', `${at} ${name}: under the 20 listed, a note says 20 of 1,037 are listed (${more && more.text})`);
              check(more && more.seen, `${at} ${name}: …and it can be scrolled into sight, not under the player`);
              await page.screenshot({ path: path.join(SHOTS, `${at}-${name}-list.png`) });
              await page.evaluate(() => { document.querySelector('#scanerrs').open = false; document.querySelector('.main').scrollTo(0, 0); window.scrollTo(0, 0); });
            }
            // The Identify card drawn again with none unsure: the review goes and leaves no dot behind
            fixtures['/library/identify'] = identify(0);
            await page.evaluate(() => { document.querySelector('#idphase').dataset.old = '1'; const a = document.querySelector('#idauto'); a.checked = !a.checked; a.dispatchEvent(new Event('change')); });
            await page.waitForFunction(() => !document.querySelector('#libreview') && document.querySelector('#idphase') && !document.querySelector('#idphase').dataset.old, null, { timeout: 6000 }).catch(() => {});
            const g0 = await rows(page);
            const t0 = g0.rows.map((r) => r.text).join(' / ');
            check(!/unsure/.test(t0) && !/·\s*$/.test(t0) && g0.rows.every((r) => r.first !== '·'), `${at} ${name}: with none unsure left the line drops the review and no dot stays (${t0})`);
            fixtures['/library/identify'] = identify(5);
            await page.evaluate(() => { document.querySelector('#idphase').dataset.old = '1'; const a = document.querySelector('#idauto'); a.checked = !a.checked; a.dispatchEvent(new Event('change')); });
            await page.waitForFunction(() => document.querySelector('#libreview'), null, { timeout: 6000 }).catch(() => {});
            const g5 = await rows(page);
            const t5 = g5.rows.map((r) => r.text).join(' / ');
            check(/(^|· |\/ )Review 5 unsure$/.test(t5) && g5.rows.every((r) => r.first !== '·'), `${at} ${name}: …and back with 5 it reads as before (${t5})`);
          }
          if (name === 'few') {
            await page.locator('#liberrs').click();
            const few = await page.evaluate(() => ({ open: document.querySelector('#scanerrs').open, items: document.querySelectorAll('#scanerrs li').length, note: !!document.querySelector('#scanerrs .scanmore'), title: document.querySelector('#scanerrs summary').textContent }));
            check(few.open && few.items === 3 && !few.note && few.title === '3 files could not be read in the last scan', `${at} ${name}: with all listed (3 of 3, one a folder) there is no "N of M" note (${few.items} listed, note ${few.note})`);
          }
          check(state.errors.length === 0, `${at} ${name}: no page errors (${state.errors.join('; ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
