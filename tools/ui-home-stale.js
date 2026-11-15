// Home's rows show the answer of the Home on screen, not of an earlier drawing of it. Home draws
// itself again when you pick Today / This week / This month on a Trending row, or open Edit Home;
// each drawing asks for its rows, and an answer found its row page-wide. So This week → This month
// (slow) → This week again (kept from a minute ago, at once) ended with This month's songs under
// "This week": the month's late answer filled the newer drawing's row. Now each drawing fills only
// its own rows; a newer drawing's rows wait for their own answer.
// Real UI in Chromium against fixtures (no node), at 1280, 1024 and 768 px.
//   node tools/ui-home-stale.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'home-stale-shots'));
fs.mkdirSync(shots, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const TRENDING = ['trending-songs', 'trending-albums', 'trending-artists'];
const KIND = { 'trending-songs': 'track', 'trending-albums': 'album', 'trending-artists': 'artist' };
const PATH = { track: 'recording', album: 'release-group', artist: 'artist' };
const WORD = { '24h': 'Today', '7d': 'Week', '30d': 'Month' };
// Each window its own names: "Week track 1", "Month album 3"…
const items = (kind, win) => Array.from({ length: 4 }, (_, i) => ({ kind, uri: `https://musicbrainz.org/${PATH[kind]}/00000000-0000-4000-8000-${win === '30d' ? '1' : '0'}0000000000${i}`, title: `${WORD[win]} ${kind} ${i + 1}`, artist: 'Sefa' }));
const album = (id, title) => ({ id, title, album_artist: 'Sefa' });
const fixtures = {
  '/library/home': { body: { recently_played: [], recently_added: [album(7, 'Journey'), album(8, 'Loop')], forgotten: [] } },
  '/listenbrainz': { body: { connected: false } },
  '/me/first-steps': { body: { show: false, hidden: true, steps: [] } },
  '/library/folders/suggest': { body: [] },
  ...Object.fromEntries(['following', 'tags', 'concerts', 'podcasts', 'yours', 'new'].map((n) => [`/home/${n}`, { body: { section: n, items: [], tags: [] } }])),
};
// What a Trending row shows, and which window chip is on
const row = (page, name) => page.evaluate((n) => {
  const s = document.getElementById('h-' + n);
  if (!s) return null;
  return {
    on: [...s.querySelectorAll('[data-win].on')].map((b) => b.dataset.win),
    text: (s.querySelector('.hbody') || s).textContent,
    waiting: !!s.querySelector('.hbody .skeleton'),
    count: document.querySelectorAll('#h-' + n).length,
  };
}, name);

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      const T = `${width}:`;
      const { page, context, state } = await fx.create(browser, width, { fixtures });
      // Trending answers per window; This month's is slow, as a first ask of a window is on a real node
      let slow = 0; let landed = 0;
      await context.route(/\/v1\/home\/trending-/, async (r) => {
        const u = new URL(r.request().url()); const name = u.pathname.split('/').pop(); const win = u.searchParams.get('window') || '7d';
        if (win === '30d') { slow++; await new Promise((res) => setTimeout(res, 1500)); }
        await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ section: name, items: items(KIND[name], win), tags: [] }) });
        if (win === '30d') landed++;
      });
      try {
        await fx.go(page, '/');
        await page.waitForFunction((ns) => ns.every((n) => { const s = document.getElementById('h-' + n); return s && !s.querySelector('.hbody .skeleton'); }), TRENDING);
        const first = await row(page, 'trending-songs');
        check(first && first.on.join() === '7d' && /Week track 1/.test(first.text), `${T} Home opens on This week, with this week's songs (${first && first.on})`);

        // This month (slow), then This week again before it lands (kept, so at once)
        await page.click('#h-trending-songs [data-win="30d"]');
        await page.waitForTimeout(150);
        const mid = await row(page, 'trending-songs');
        check(mid && mid.on.join() === '30d' && mid.waiting, `${T} This month is on and its row waits for its answer`);
        await page.click('#h-trending-songs [data-win="7d"]');
        const back = await row(page, 'trending-songs');
        check(back && back.on.join() === '7d' && /Week track 1/.test(back.text), `${T} This week again shows this week's songs at once`);
        // Let This month's late answers land (all three, however slow the machine), then the page take them
        for (let t = Date.now(); landed < 3 && Date.now() - t < 15000;) await page.waitForTimeout(50);
        check(landed === 3, `${T} This month's late answers have come (${landed})`);
        await page.waitForTimeout(300);
        check(slow === 3, `${T} This month's three rows were asked once each (${slow})`);
        for (const name of TRENDING) {
          const r = await row(page, name);
          const kind = KIND[name];
          check(r && r.count === 1, `${T} ${name}: one row (${r && r.count})`);
          check(r && r.on.join() === '7d', `${T} ${name}: This week is still the chosen window (${r && r.on})`);
          check(r && new RegExp(`Week ${kind} 1`).test(r.text) && !/Month/.test(r.text), `${T} ${name}: still this week's, not this month's late answer ("${r && r.text.replace(/\s+/g, ' ').trim().slice(0, 60)}")`);
        }
        await page.screenshot({ path: path.join(shots, `week-after-late-month-${width}.png`) });

        // Edit Home draws Home again: its rows still fill, from the new drawing's own answers
        await page.click('#hedit');
        await page.waitForSelector('#h-edit');
        await page.waitForFunction(() => { const s = document.getElementById('h-added'); return s && !s.hidden && /Journey/.test(s.textContent); }).then(() => check(true, `${T} after Edit Home, Recently added is filled`), () => check(false, `${T} after Edit Home, Recently added is filled`));
        await page.waitForFunction(() => { const s = document.getElementById('h-trending-albums'); return s && /Week album 1/.test(s.textContent); }).then(() => check(true, `${T} after Edit Home, Trending albums is filled`), () => check(false, `${T} after Edit Home, Trending albums is filled`));
        await page.click('#hdone');
        await page.waitForFunction(() => !document.getElementById('h-edit') && /Week track 1/.test((document.getElementById('h-trending-songs') || {}).textContent || '')).then(() => check(true, `${T} Done: Home without the edit panel, rows filled`), () => check(false, `${T} Done: Home without the edit panel, rows filled`));
        await fx.cleanCopy(page).then(() => check(true, `${T} clean copy, no overflow`), (e) => check(false, `${T} ${e.message.split('\n')[0]}`));
        check(state.errors.length === 0, `${T} no page errors ${state.errors.join(' | ')}`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
