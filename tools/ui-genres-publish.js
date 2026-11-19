// The Genres page's "Publish my files' genres as tags too" checkbox. It showed to everyone on
// every Genres page: to a listener, whose change the node refuses ("that is the owner's"), and to
// an owner whose library has no songs, above "No genres yet" and its Add music, with nothing to
// publish. Now it shows only to the owner, and only when the library has songs or genres, when the
// setting is already on (so it can be turned off), or when the node didn't say how many songs it
// has. Turning it on or off still sends genre_tags. At 1280, 1024, 768, 412 and 360 px (and in the
// Android app). On a phone its whole line is a 44 px tap, like every other checkbox line: a thumb
// near the line's lower edge still turns it on (the line was 35 px with a 13 px box).
//   node tools/ui-genres-publish.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'genres-publish-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const stats = (tracks) => ({ body: { tracks, albums: 0, artists: 0, files: tracks, missing_files: 0, total_duration_ms: 0 } });
const genres = { body: [{ id: 3, name: 'Jazz', albums: 1, tracks: 4 }] };
const settings = (on) => ({ body: { contact: '', discovery: false, token_replaceable: true, genre_tags: on } });
const CASES = [
  { name: 'owner-genres', fixtures: { '/library/genres': genres }, box: true, toggle: true },
  { name: 'owner-songs-no-genres', fixtures: { '/library/stats': stats(4) }, box: true },
  { name: 'owner-no-songs', fixtures: { '/library/stats': stats(0) }, box: false },
  { name: 'owner-no-songs-on', fixtures: { '/library/stats': stats(0), '/node/settings': settings(true) }, box: true, checked: true, toggle: true },
  { name: 'owner-stats-fail', fixtures: { '/library/stats': { body: { error: 'busy' }, status: 503 } }, box: true },
  { name: 'listener-genres', role: 'listener', fixtures: { '/library/genres': genres }, box: false },
  { name: 'listener-no-songs', role: 'listener', fixtures: { '/library/stats': stats(0) }, box: false },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[1280, false], [1024, false], [768, false], [412, false], [360, false], [360, true]]) {
      const phone = width < 600; const at = `${width}${android ? ' android' : ''}`;
      for (const c of CASES) {
        const { page, context, state } = await fx.create(browser, width, { fixtures: c.fixtures, ...(c.role ? { role: c.role } : {}), ...(phone ? { mobile: true } : {}) });
        try {
          if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
          await fx.go(page, '/genres');
          const drawn = await page.waitForFunction(() => [...document.querySelectorAll('.main h3')].some((h) => h.textContent === 'From your files'), null, { timeout: 8000 }).then(() => true, () => false);
          check(drawn, `${at} ${c.name}: the Genres page is drawn`);
          if (!drawn) continue;
          const g = await page.evaluate(() => {
            const box = document.querySelector('#genretags');
            return { box: !!box && !!box.offsetParent, checked: !!box && box.checked, tags: [...document.querySelectorAll('.main h3')].some((h) => h.textContent === 'Your Pubky tags') };
          });
          check(g.box === c.box, `${at} ${c.name}: the publish checkbox is ${c.box ? 'shown' : 'not shown'} (${g.box ? 'shown' : 'not shown'})`);
          if (c.box) check(g.checked === !!c.checked, `${at} ${c.name}: it is ${c.checked ? 'on' : 'off'}, as the setting says`);
          if (!c.role) check(g.tags, `${at} ${c.name}: Your Pubky tags still shows`);
          const o = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
          check(o <= 0, `${at} ${c.name}: nothing wider than the screen (${o})`);
          await page.screenshot({ path: path.join(SHOTS, `${c.name}-${width}${android ? '-android' : ''}.png`) });
          let row = null;
          if (phone && g.box) {
            row = await page.evaluate(() => { const b = document.querySelector('.genre-pub').getBoundingClientRect(); return { x: b.left + b.width / 2, top: b.top, h: b.height }; });
            check(row.h >= 44, `${at} ${c.name}: the publish line is a 44 px tap (${row.h.toFixed(1)} px)`);
          }
          if (c.toggle && g.box) {
            const want = !c.checked;
            // On a phone: a press exactly 42 px below the line's top (inside a 44 px line, below the old
            // 35 px one). A mouse press, not touchscreen.tap: Chromium's touch adjustment would snap a
            // tap that misses onto the nearby line and hide a short one.
            if (row) await page.mouse.click(row.x, row.top + 42); else await page.locator('#genretags').setChecked(want);
            let sent; for (let i = 0; i < 40 && !sent; i++) { sent = state.hits.find((h) => h.v === '/node/settings' && h.m === 'PUT'); if (!sent) await page.waitForTimeout(100); }
            check(!!sent && JSON.parse(sent.body).genre_tags === want, `${at} ${c.name}: ${row ? 'a tap near the line\'s lower edge turning' : 'turning'} it ${want ? 'on' : 'off'} sends genre_tags ${want} (${sent && sent.body})`);
          }
          check(!state.errors.length, `${at} ${c.name}: no page errors (${state.errors.join(' | ')})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
