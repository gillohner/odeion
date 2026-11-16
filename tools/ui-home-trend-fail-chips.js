// A Home Trending row that didn't answer shows only its error line and Try again: its Today / This
// week / This month chips are hidden until it answers (they switch the window of every Trending row
// and asked the whole Home again, a second way to retry under a row with nothing to show). On
// another window a failed row keeps them: the way back to This week (with every row failed, Home had
// no chips left). A row that answered keeps its chips, and they still switch the window. Try again that answers brings the
// chips back. A row's See all page keeps its chips (there they are the page's own control).
// Phone (360 with the Android app, 412) and desktop (1280).
// Real candidate assets, isolated responses: node tools/ui-home-trend-fail-chips.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'home-trend-fail-chips-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const BUSY = { status: 503, body: { error: 'busy' } };
const items = (kind) => Array.from({ length: 3 }, (_, i) => ({ kind, uri: `https://musicbrainz.org/${kind === 'track' ? 'recording' : 'release-group'}/00000000-0000-4000-8000-00000000000${i}`, title: `${kind} ${i + 1}`, artist: 'Sefa' }));
const ALBUMS = { body: { section: 'trending-albums', items: items('album'), tags: [] } };
const SONGS = { body: { section: 'trending-songs', items: items('track'), tags: [] } };
const ANDROID = () => { window.pmDesktop = { platform: 'android' }; };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, mobile, android] of [[360, true, true], [412, true, false], [1280, false, false]]) {
      const at = `${width}${android ? ' Android' : ''}`;
      const { context, page, state } = await fx.create(browser, width, { mobile, fixtures: { '/home/trending-songs': BUSY, '/home/trending-albums': ALBUMS, '/home/trending-artists': BUSY } });
      if (android) await context.addInitScript(ANDROID);
      const asked = (v, q) => state.hits.filter((h) => h.v === v && h.m === 'GET' && (!q || h.q.includes(q))).length;
      const press = (loc) => (mobile ? loc.tap() : loc.click());
      const chipsShown = (name) => page.evaluate((n) => [...document.querySelectorAll(`#h-${n} .section-head [data-win]`)].filter((b) => b.getClientRects().length).length, name);
      try {
        await fx.go(page, '/');
        await page.locator('#h-trending-songs [data-hretry]').waitFor();
        await page.locator('#h-trending-artists [data-hretry]').waitFor();
        await page.locator('#h-trending-albums .tile').first().waitFor();
        await page.locator('#h-trending-songs').scrollIntoViewIfNeeded();
        await page.screenshot({ path: path.join(SHOTS, `home-failed-${width}.png`) });
        check(await chipsShown('trending-songs') === 0, `${at}: Trending songs, not answering, shows no window chips (${await chipsShown('trending-songs')} shown)`);
        check(await chipsShown('trending-artists') === 0, `${at}: Trending artists, not answering, shows no window chips (${await chipsShown('trending-artists')} shown)`);
        check(await chipsShown('trending-albums') === 3, `${at}: Trending albums, answered, keeps its three window chips (${await chipsShown('trending-albums')})`);
        check(await page.locator('#h-trending-songs h3').isVisible() && await page.locator('#h-trending-songs [data-hretry]').isVisible(), `${at}: …the failed row keeps its title and Try again`);
        await fx.cleanCopy(page);

        // Try again that answers brings the row's chips back, on the window it was asked for
        state.fixtures['/home/trending-songs'] = SONGS;
        const s0 = asked('/home/trending-songs');
        await press(page.locator('#h-trending-songs [data-hretry]'));
        await page.locator('#h-trending-songs .srow').first().waitFor().catch(() => {});
        await page.waitForTimeout(150);
        check(asked('/home/trending-songs') === s0 + 1, `${at}: Try again asks Trending songs once (${asked('/home/trending-songs') - s0})`);
        check(await chipsShown('trending-songs') === 3, `${at}: answered, Trending songs shows its window chips again (${await chipsShown('trending-songs')})`);
        check(await page.locator('#h-trending-songs .section-head .chip.on[data-win="7d"]').isVisible(), `${at}: …with This week picked`);
        check(await chipsShown('trending-artists') === 0, `${at}: Trending artists, still not answering, still shows none`);
        await page.screenshot({ path: path.join(SHOTS, `home-retried-${width}.png`) });

        // An answered row's chip still switches the window (every Trending row asked for This month)
        const m0 = asked('/home/trending-albums', 'window=30d');
        await press(page.locator('#h-trending-albums [data-win="30d"]'));
        await page.locator('#h-trending-albums .chip.on[data-win="30d"]').waitFor().catch(() => {});
        await page.locator('#h-trending-artists [data-hretry]').waitFor().catch(() => {});
        check(asked('/home/trending-albums', 'window=30d') === m0 + 1 && await page.locator('#h-trending-albums .chip.on[data-win="30d"]').isVisible(), `${at}: This month on Trending albums asks for this month`);
        check(await chipsShown('trending-artists') === 3, `${at}: …and Trending artists, not answering this month, keeps its chips (the way back to This week; ${await chipsShown('trending-artists')})`);
        // Every Trending row failing on This month (a fresh Home: no answer kept for this month): each
        // keeps its chips, and This week brings Home back
        state.fixtures['/home/trending-songs'] = BUSY;
        await fx.go(page, '/');
        await page.locator('#h-trending-albums .tile').first().waitFor();
        state.fixtures['/home/trending-albums'] = BUSY;
        const f0 = ['songs', 'albums', 'artists'].map((k) => asked(`/home/trending-${k}`, 'window=30d'));
        await press(page.locator('#h-trending-albums [data-win="30d"]'));
        for (const k of ['songs', 'albums', 'artists']) await page.locator(`#h-trending-${k} [data-hretry]`).waitFor().catch(() => {});
        const f1 = ['songs', 'albums', 'artists'].map((k, i) => asked(`/home/trending-${k}`, 'window=30d') - f0[i]);
        const tries = await page.locator('#h-trending-songs [data-hretry], #h-trending-albums [data-hretry], #h-trending-artists [data-hretry]').count();
        const all = [await chipsShown('trending-songs'), await chipsShown('trending-albums'), await chipsShown('trending-artists')];
        check(tries === 3 && f1.every((n) => n === 1), `${at}: all three Trending rows asked for this month, none answering (${f1.join('/')} asked, ${tries} Try again)`);
        check(all.every((n) => n === 3), `${at}: …each keeps its chips (${all.join('/')})`);
        await page.screenshot({ path: path.join(SHOTS, `home-month-failed-${width}.png`) });
        state.fixtures['/home/trending-albums'] = ALBUMS; state.fixtures['/home/trending-songs'] = SONGS;
        await press(page.locator('#h-trending-albums [data-win="7d"]'));
        await page.locator('#h-trending-albums .chip.on[data-win="7d"]').waitFor().catch(() => {});
        await page.locator('#h-trending-albums .tile').first().waitFor().catch(() => {});
        await page.locator('#h-trending-songs .srow').first().waitFor().catch(() => {});
        await page.locator('#h-trending-artists [data-hretry]').waitFor().catch(() => {});
        await page.waitForTimeout(150);
        const wk = [await chipsShown('trending-songs'), await chipsShown('trending-albums'), await chipsShown('trending-artists')];
        check(await page.locator('#h-trending-albums .tile').count() === 3, `${at}: …This week brings back Trending albums`);
        check(wk[0] === 3 && wk[1] === 3 && wk[2] === 0, `${at}: …and on This week the rows that answer have chips, the one still not answering none (${wk.join('/')})`);

        // Home opened again: while a row that failed last time is asked again, its placeholder has no
        // chips either, so failing again moves nothing; a row that answered last time keeps them
        let holdArtists = true; state.delay = (v) => (holdArtists && v === '/home/trending-artists' ? 900 : 0);
        await fx.go(page, '/');
        await page.locator('#h-trending-artists .skeleton').waitFor();
        check(await chipsShown('trending-artists') === 0, `${at}: Home again, Trending artists (failed last time) waits without window chips (${await chipsShown('trending-artists')})`);
        check(await chipsShown('trending-songs') === 3, `${at}: …Trending songs (answered last time) waits with them (${await chipsShown('trending-songs')})`);
        const headH = () => page.evaluate(() => Math.round(document.querySelector('#h-trending-artists .section-head').getBoundingClientRect().height));
        const h0 = await headH();
        await page.locator('#h-trending-artists [data-hretry]').waitFor();
        check(await headH() === h0, `${at}: …failing again, its head keeps its height (${h0} → ${await headH()})`);
        holdArtists = false; state.delay = 0;

        // See all of a row that didn't answer keeps the page's chips
        await fx.go(page, '/home/trending-artists');
        await page.locator('#hsfull [data-hretry]').waitFor().catch(() => {});
        const seeAllChips = await page.evaluate(() => [...document.querySelectorAll('#main [data-win]')].filter((b) => b.getClientRects().length).length);
        check(seeAllChips === 3, `${at}: See all of a row that didn't answer keeps its window chips (${seeAllChips})`);
        await fx.cleanCopy(page);
        check(state.errors.length === 0, `${at}: no page errors (${state.errors.join(' | ')})`);
      } catch (e) { check(false, `${at}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
