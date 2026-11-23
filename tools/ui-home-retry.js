// A Home row that didn't answer (the library's Recently played / added and Forgotten favourites,
// any Pubky row, and a row's See all page) says so with Try again, which asks for that row again and
// draws it in place under a placeholder of the row's height (nothing below moves): no reload of the
// whole page, no other row asked again. A keyboard that pressed Try again is on the new Try again if
// the row fails again, else on the row. A late See all answer for a window since left draws nothing.
// A Pubky row's reason (the node's own words: a request URL, an HTTP body) is the line's tooltip,
// not on screen: "Pubky did not answer." and Try again on one line, as the library's rows (a rate
// limit: "Pubky is busy. Wait a minute.").
// Phone (360, 412) and desktop (1280).
// Real candidate assets, isolated responses: node tools/ui-home-retry.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'home-retry-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const BUSY = { status: 503, body: { error: 'busy' } };
const HOME = { body: { recently_played: [{ id: 7, title: 'Journey', album_artist: 'Sefa', year: 2020 }], recently_added: [{ id: 7, title: 'Journey', album_artist: 'Sefa', year: 2020 }], forgotten: [] } };
// (the node answers a Pubky row that failed with 200 and the Pinakes client's error)
const WHY = 'transport: error sending request for url (https://pinakes.example.org/v1/search?kind=artist&sort=trending&window=7d&limit=12&collapse=group)';
const REAL = { body: { section: 'trending-artists', items: [], tags: [], error: WHY } };
const SLOW = { body: { section: 'following', items: [], tags: [], error: 'rate limited; retry after Some(30s)' } };
const QUIET = { body: { section: 'trending-songs', items: [], tags: [], note: 'Nothing trending this week.' } };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, mobile] of [[360, true], [412, true], [1280, false]]) {
      const at = `${width}`;
      const { context, page, state } = await fx.create(browser, width, { mobile, fixtures: { '/library/home': BUSY, '/home/trending-songs': BUSY, '/home/trending-albums': BUSY, '/home/trending-artists': REAL, '/home/following': SLOW } });
      const asked = (v) => state.hits.filter((h) => h.v === v && h.m === 'GET').length;
      const press = (loc) => (mobile ? loc.tap() : loc.click());
      const hold = new Set(); state.delay = (v) => (hold.has(v) ? 900 : 0);
      try {
        await fx.go(page, '/');
        const played = page.locator('#h-played .hbody');
        await played.getByText('The library did not answer.').waitFor();
        const tries = await page.locator('#h-played [data-hretry], #h-added [data-hretry]').count();
        check(tries === 2, `${at}: Recently played and Recently added say the library did not answer, each with Try again (${tries})`);
        await page.locator('#h-trending-songs [data-hretry]').waitFor().catch(() => {});
        check(await page.locator('#h-trending-songs [data-hretry]').isVisible(), `${at}: a Pubky row that did not answer has Try again`);
        // One line each, the reason only as a tooltip (nothing technical on the main path)
        await page.locator('#h-trending-artists [data-hretry]').waitFor().catch(() => {});
        const line = (k) => page.evaluate((q) => {
          const p = document.querySelector(`#h-${q} .hfail`); if (!p) return null;
          const b = p.querySelector('[data-hretry]').getBoundingClientRect(); const r = p.getBoundingClientRect();
          const t = p.querySelector('[title]');
          return { text: p.innerText.replace(/\s+/g, ' ').trim(), why: (t || {}).title || '', lines: t ? t.getClientRects().length : 0, h: Math.round(r.height), bh: Math.round(b.height) };
        }, k);
        for (const k of ['trending-songs', 'trending-albums', 'trending-artists']) {
          const l = await line(k);
          check(l && l.text === 'Pubky did not answer. Try again', `${at}: ${k} says only "Pubky did not answer." with Try again (${l && l.text})`);
          check(l && l.h <= l.bh + 1 && l.lines === 1, `${at}: …on one line with its Try again (${l && l.h} px for a ${l && l.bh} px button, ${l && l.lines} line)`);
        }
        await page.locator('#h-following [data-hretry]').waitFor().catch(() => {});
        const busy = await line('following');
        check(busy && busy.text === 'Pubky is busy. Wait a minute. Try again' && busy.why === SLOW.body.error && busy.lines === 1, `${at}: a rate-limited row says to wait a minute (${busy && busy.text})`);
        const real = await line('trending-artists');
        check(real && real.why === WHY, `${at}: the node's reason is the line's tooltip (${real && real.why.slice(0, 40)})`);
        check(!(await page.locator('#main').innerText()).includes('http'), `${at}: no request URL on Home`);
        await page.screenshot({ path: path.join(SHOTS, `failed-${at}.png`) });
        await fx.cleanCopy(page);
        // (desktop presses with the keyboard, the phone taps)
        const go = async (loc) => { if (mobile) await loc.tap(); else { await loc.focus(); await page.keyboard.press('Enter'); } };
        const focused = (sel) => page.evaluate((q) => !!document.activeElement && document.activeElement.matches(q), sel);
        // how far a row's header is below the page's top (scrolling doesn't change it)
        const topOf = (sel) => page.evaluate((q) => Math.round((document.querySelector(q).getBoundingClientRect().top - document.querySelector('#main .page-head').getBoundingClientRect().top) * 10) / 10, sel);

        // The library still doesn't answer: while it is asked, nothing below moves; it says so again
        hold.add('/library/home');
        const lib0 = asked('/library/home'); const trend0 = asked('/home/trending-songs'); const alb0 = asked('/home/trending-albums');
        const y0 = await topOf('#h-trending-songs h3');
        await go(page.locator('#h-added [data-hretry]'));
        await page.waitForTimeout(250);
        const y1 = await topOf('#h-trending-songs h3');
        check(Math.abs(y1 - y0) <= 1 && await page.locator('#h-added .skeleton').count() === 1, `${at}: while the library is asked again, the rows below stay where they were (${y0} → ${y1})`);
        await page.locator('#h-added [data-hretry]').waitFor().catch(() => {});
        check(await page.locator('#h-played [data-hretry], #h-added [data-hretry]').count() === 2 && asked('/library/home') === lib0 + 1, `${at}: failing again, both library rows have Try again again (${asked('/library/home') - lib0} asked)`);
        check(Math.abs(await topOf('#h-trending-songs h3') - y0) <= 1, `${at}: …and the rows below are where they were`);
        if (!mobile) check(await focused('#h-added [data-hretry]'), `${at}: failing again, the keyboard is on Recently added's new Try again`);
        hold.delete('/library/home');

        // The library answers now: Try again in one row fills the library rows, in place
        state.fixtures['/library/home'] = HOME;
        const lib1 = asked('/library/home');
        await go(page.locator('#h-played [data-hretry]'));
        await page.locator('#h-played .tile').first().waitFor().catch(() => {});
        check(await page.locator('#h-played .tile').count() === 1 && await page.locator('#h-added .tile').count() === 1, `${at}: Try again draws Recently played and Recently added`);
        check(asked('/library/home') === lib1 + 1, `${at}: …asking the library once (${asked('/library/home') - lib1})`);
        check(await page.locator('#h-played [data-hretry], #h-added [data-hretry]').count() === 0 && await page.locator('#h-forgotten').count() === 0, `${at}: …with no Try again left, and Forgotten bookmarks (none) gone`);
        check(asked('/home/trending-songs') === trend0 && asked('/home/trending-albums') === alb0, `${at}: the Pubky rows are not asked again`);
        if (!mobile) check(await focused('#h-played'), `${at}: answered, the keyboard is on the Recently played row`);

        // A Pubky row: only that row is asked again, and nothing below moves while it is
        state.fixtures['/home/trending-songs'] = QUIET;
        hold.add('/home/trending-songs');
        const lib2 = asked('/library/home');
        const z0 = await topOf('#h-trending-albums h3');
        await go(page.locator('#h-trending-songs [data-hretry]'));
        await page.waitForTimeout(250);
        const z1 = await topOf('#h-trending-albums h3');
        check(Math.abs(z1 - z0) <= 1, `${at}: while Trending songs is asked again, the rows below stay where they were (${z0} → ${z1})`);
        hold.delete('/home/trending-songs');
        await page.locator('#h-trending-songs').getByText('Nothing trending this week.').waitFor().catch(() => {});
        check(await page.locator('#h-trending-songs').getByText('Nothing trending this week.').isVisible(), `${at}: Try again in Trending songs draws its answer`);
        check(asked('/home/trending-songs') === trend0 + 1 && asked('/home/trending-albums') === alb0 && asked('/library/home') === lib2, `${at}: …asking only Trending songs again (${asked('/home/trending-songs') - trend0}; albums ${asked('/home/trending-albums') - alb0}, library ${asked('/library/home') - lib2})`);
        if (!mobile) check(await focused('#h-trending-songs'), `${at}: answered, the keyboard is on the Trending songs row`);
        check(await page.locator('#h-trending-albums [data-hretry]').isVisible(), `${at}: Trending albums, still not answering, keeps its Try again`);
        if (mobile) {
          // (40 px drawn; on a touch screen a small button's reach adds 8 px above and below)
          const box = await page.locator('#h-trending-albums [data-hretry]').boundingBox();
          check(box && box.height >= 40, `${at}: Try again is a full touch (${box && Math.round(box.height)} px tall)`);
        }
        if (!mobile) {
          const alb1 = asked('/home/trending-albums');
          await go(page.locator('#h-trending-albums [data-hretry]'));
          await page.waitForFunction(() => document.activeElement && document.activeElement.matches('#h-trending-albums [data-hretry]'), null, { timeout: 3000 }).catch(() => {});
          check(asked('/home/trending-albums') === alb1 + 1, `${at}: Enter on Try again asks the row again (${asked('/home/trending-albums') - alb1})`);
          check(await focused('#h-trending-albums [data-hretry]'), `${at}: failing again, the keyboard is on the row's new Try again`);
        }
        await page.screenshot({ path: path.join(SHOTS, `retried-${at}.png`) });

        // A row's See all page: Try again draws in place (the page itself stays), and an answer for a
        // window since left draws nothing
        const seeAll = { '7d': BUSY, '30d': BUSY }; let holdWin = null;
        await page.route(/\/v1\/home\/trending-songs/, async (r) => {
          const w = new URL(r.request().url()).searchParams.get('window') || '7d';
          const ans = seeAll[w]; if (holdWin === w) await new Promise((res) => setTimeout(res, 900));
          await r.fulfill({ status: ans.status || 200, contentType: 'application/json', body: JSON.stringify(ans.body) });
        });
        await fx.go(page, '/home/trending-songs');
        await page.locator('#hsfull [data-hretry]').waitFor().catch(() => {});
        check(await page.locator('#hsfull [data-hretry]').isVisible(), `${at}: See all's page that did not answer has Try again`);
        await page.evaluate(() => { window.__head = document.querySelector('#main .page-head h2'); });
        if (!mobile) {
          await go(page.locator('#hsfull [data-hretry]'));
          await page.waitForTimeout(300);
          check(await focused('#hsfull [data-hretry]'), `${at}: See all failing again, the keyboard is on its new Try again`);
        }
        seeAll['7d'] = QUIET;
        await go(page.locator('#hsfull [data-hretry]'));
        await page.locator('#hsfull').getByText('Nothing trending this week.').waitFor().catch(() => {});
        check(await page.locator('#hsfull').getByText('Nothing trending this week.').isVisible() && new URL(page.url()).pathname === '/home/trending-songs', `${at}: …which draws its answer on the same page`);
        check(await page.evaluate(() => !!window.__head && window.__head.isConnected), `${at}: …in place (the page's head is not drawn again)`);
        if (!mobile) check(await focused('#hsfull'), `${at}: answered, the keyboard is on See all's list`);
        // Try again on This week, then This month before This week answers: This month's answer stays
        seeAll['7d'] = BUSY;
        await fx.go(page, '/home/trending-songs');
        await page.locator('#hsfull [data-hretry]').waitFor().catch(() => {});
        seeAll['7d'] = QUIET; seeAll['30d'] = { body: { section: 'trending-songs', items: [], tags: [], note: 'Nothing trending this month.' } }; holdWin = '7d';
        await go(page.locator('#hsfull [data-hretry]'));
        await press(page.locator('[data-win="30d"]'));
        await page.locator('#hsfull').getByText('Nothing trending this month.').waitFor().catch(() => {});
        await page.waitForTimeout(1200);
        check(await page.locator('#hsfull').getByText('Nothing trending this month.').isVisible() && !(await page.locator('#hsfull').getByText('Nothing trending this week.').count()), `${at}: a late answer for This week doesn't draw over This month`);
        holdWin = null;
        await fx.cleanCopy(page);
        check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
      } catch (e) { check(false, `${at}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
