// A progress message goes when its answer comes. "Which song is this?" said "Looking up … on
// MusicBrainz…" and a collection's Play said "Finding where these songs play…"; both stayed their
// full 3.8 s after the answer: behind the next dialog, and over Settings › Discover (the soft
// keyboard's place on a phone) after Add it. A quick answer shows no lookup message at all; an
// error that follows still shows.
// Real UI in Chromium against fixtures (no node), phone widths in the browser and the Android app.
//   node tools/ui-progress-toast.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/progress-toast-shots');
fs.mkdirSync(shots, { recursive: true });

// A song with no match yet: Songs offers "Which song is this?" (the Match chip; a phone's row menu)
const unmatched = { ...fx.track, canonical: null, canonical_uri: undefined, mbid: undefined };
const suggest = { track: unmatched, suggestions: [{ id: 5, title: 'Journey Through Time', artist: 'Sefa', release_title: 'Journey', score: 0.93, recording_mbid: 'b1a9c0e9-d987-4042-ae91-78d6a3267d69' }], needs_contact: false, found: [] };
const contact = { track: unmatched, suggestions: [], needs_contact: true, found: [] };
const LOOKING = /Looking up .* on MusicBrainz/;
const FINDING = /Finding where these songs play/;
// The message showing now ('' when none)
const shown = (page) => page.evaluate(() => { const t = document.querySelector('#toast'); return t.classList.contains('show') ? t.textContent : ''; });
const whichSong = async (page, width) => {
  if (width < 600) { await page.locator('.track [data-act="more"]').first().click(); await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Which song is this?' }).first().click(); }
  else await page.locator('.track .idmark').first().click();
};

(async () => {
  const browser = await chromium.launch();
  let fails = 0;
  const check = async (name, fn) => { try { await fn(); console.log('ok  ', name); } catch (e) { fails++; console.log('FAIL', name, '-', e.message.split('\n')[0]); } };
  const cases = [[360, false], [412, false], [360, true], [412, true], [1280, false]];
  for (const [width, android] of cases) {
    const tag = `${width}${android ? ' android' : ''}`;
    const open = async (opts) => {
      const o = await fx.create(browser, width, { mobile: width < 600, tracks: [unmatched], ...opts });
      if (android) await o.context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
      return o;
    };
    await check(`Which song is this?: the lookup message goes when the choices come (${tag})`, async () => {
      const { page, context, state } = await open({ fixtures: { '/library/tracks/1/candidates': { body: suggest } }, delay: (v) => (v.endsWith('/candidates') ? 600 : 0) });
      try {
        await fx.go(page, '/tracks');
        await whichSong(page, width);
        await page.waitForFunction(() => /Looking up/.test(document.querySelector('#toast.show')?.textContent || ''));
        const dlg = page.locator('dialog.dialog[open]');
        await dlg.waitFor();
        assert.doesNotMatch(await shown(page), LOOKING, 'gone when the dialog opens');
        await page.waitForTimeout(500);
        await page.screenshot({ path: path.join(shots, `which-song-${width}${android ? '-android' : ''}.png`) });
        assert.equal(await shown(page), '', 'nothing behind the dialog');
        await page.keyboard.press('Escape');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Which song is this?: a quick answer opens the dialog with no lookup message first (${tag})`, async () => {
      const { page, context } = await open({ fixtures: { '/library/tracks/1/candidates': { body: suggest } } });
      try {
        await fx.go(page, '/tracks');
        // (every message the page shows, however briefly)
        await page.evaluate(() => { window.__said = []; new MutationObserver(() => { const t = document.querySelector('#toast'); if (t.classList.contains('show')) window.__said.push(t.textContent); }).observe(document.querySelector('#toast'), { attributes: true, childList: true, characterData: true, subtree: true }); });
        await whichSong(page, width);
        await page.locator('dialog.dialog[open]').waitFor();
        await page.waitForTimeout(500);
        const said = await page.evaluate(() => window.__said);
        assert.ok(!said.some((m) => /Looking up/.test(m)), `no flash of the lookup message: ${JSON.stringify(said)}`);
        await page.keyboard.press('Escape');
      } finally { await context.close(); }
    });
    await check(`Which song is this? › Add it: Settings › Discover has no lookup message over it (${tag})`, async () => {
      const { page, context, state } = await open({ fixtures: { '/library/tracks/1/candidates': { body: contact } }, delay: (v) => (v.endsWith('/candidates') ? 300 : 0) });
      try {
        await fx.go(page, '/tracks');
        await whichSong(page, width);
        const dlg = page.locator('dialog.dialog[open]');
        await dlg.waitFor();
        await dlg.locator('.choice').first().click();
        await page.waitForFunction(() => location.pathname === '/settings/discover' && document.activeElement && document.activeElement.id === 'contact');
        await page.waitForTimeout(500);
        await page.screenshot({ path: path.join(shots, `add-it-settings-${width}${android ? '-android' : ''}.png`) });
        assert.doesNotMatch(await shown(page), LOOKING, 'no lookup message on Settings');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Which song is this?: a failed lookup still says why (${tag})`, async () => {
      const { page, context } = await open({ fixtures: { '/library/tracks/1/candidates': { body: { error: 'MusicBrainz is busy, try again' }, status: 503 } }, delay: (v) => (v.endsWith('/candidates') ? 300 : 0) });
      try {
        await fx.go(page, '/tracks');
        await whichSong(page, width);
        await page.waitForFunction(() => /busy/.test(document.querySelector('#toast.show')?.textContent || ''));
        await page.waitForTimeout(500);
        assert.match(await shown(page), /MusicBrainz is busy/, 'the error stays');
      } finally { await context.close(); }
    });
    await check(`A collection's Play: "Finding where these songs play…" goes when they are found (${tag})`, async () => {
      const { page, context, state } = await open({ collections: [fx.summary], delay: (v) => (v === '/playlists/c1/play-from' ? 1500 : 0) });
      try {
        await fx.go(page, '/playlists');
        const play = page.locator('[data-plplay="c1"]').first();
        await play.waitFor();
        const asked = state.hits.length;
        await play.click();
        await page.waitForFunction(() => /Finding where/.test(document.querySelector('#toast.show')?.textContent || ''), null, { timeout: 4000 });
        // the answer comes 1.5 s after the press; the message is gone right after it
        await page.waitForFunction(() => !/Finding where/.test(document.querySelector('#toast.show')?.textContent || ''), null, { timeout: 2500 });
        await page.waitForTimeout(300);
        await page.screenshot({ path: path.join(shots, `collection-play-${width}${android ? '-android' : ''}.png`) });
        assert.doesNotMatch(await shown(page), FINDING, 'gone once found');
        assert.ok(state.hits.slice(asked).some((h) => h.v === '/playlists/c1/play-from'), 'asked where they play');
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  }
  await browser.close();
  if (fails) { console.log(`${fails} FAIL`); process.exit(1); }
  console.log('all passed');
})().catch((e) => { console.error(e); process.exit(1); });
