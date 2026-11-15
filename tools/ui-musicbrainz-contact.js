// MusicBrainz waiting for the node's email or website: "Which song is this?" (a song's Match) and
// Wrong match?'s MusicBrainz search both offer Add it, which opens Settings › Discover with the
// field ready to type, as Search's note does. Before, both pointed at Get started on Home, a card
// the owner can hide (then Home had no field at all) and that never took the keyboard.
// Real UI in Chromium against fixtures (no node).
//   node tools/ui-musicbrainz-contact.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/musicbrainz-contact-shots');
fs.mkdirSync(shots, { recursive: true });

// A song with no match yet (Songs shows its Match chip), and the node's answer without a contact
const unmatched = { ...fx.track, canonical: null, canonical_uri: undefined, mbid: undefined };
const needsContact = (t) => ({ track: t, suggestions: [], needs_contact: true, found: [] });
// Wrong match? for an album's song (the shapes of tools/ui-wrong-match.js)
const song = { no: 1, title: 'Song 1', artist: 'Band', artist_id: 'lib-31', duration_ms: 201000, uri: null, track_id: 101, id: 'lib-101', page: 'lib-101' };
const album = { kind: 'album', id: 'lib-7', target: { album_id: 7 }, title: 'Album', artist: { name: 'Band', id: 'lib-31' }, album: null, year: 2001, songs: [song], uri: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library', image: null, tags: [], about: [], about_text: null, image_credit: null };
const fixTrack = { id: 101, title: 'Song 1', artists: [{ id: 31, name: 'Band' }], track_no: 1, disc_no: null, duration_ms: 201000, file: { path: '/music/Band/Album/01 Song 1.flac' } };
const fixPage = { albums: [{ key: '/music/Band/Album', album: 'Album', artist: 'Band', release: null, ready: 0, conflicts: 0, songs: [{ track: fixTrack, options: [], status: 'current', pick: null, current: { recording_mbid: '00000000-0000-4000-8000-000000000001', release_mbid: null, source: 'auto', score: 0.95, auto: true, from_tags: false } }] }], next_after: null, songs: 1, total_albums: 1, offset: 0, next: null };
const fixtures = {
  '/library/tracks/1/candidates': { body: needsContact(unmatched) },
  '/item/album/lib-7': { body: album },
  '/item/album/lib-7/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
  '/library/artists/31/similar': { body: { artists: [] } },
  '/library/tracks/101': { body: { ...fx.track, id: 101, title: 'Song 1' } },
  '/library/identify/fix': { body: fixPage },
  '/library/identify': { body: { phase: 'ready', write_ids: false, auto_match: true } },
  '/library/tracks/101/candidates': { body: needsContact(fixTrack) },
};
// Settings › Discover is open, its field has the keyboard
const atContact = (page) => page.waitForFunction(() => location.pathname === '/settings/discover' && document.activeElement && document.activeElement.id === 'contact');

(async () => {
  const browser = await chromium.launch();
  let fails = 0;
  const check = async (name, fn) => { try { await fn(); console.log('ok  ', name); } catch (e) { fails++; console.log('FAIL', name, '-', e.message.split('\n')[0]); } };
  for (const width of [1280, 1024, 768, 360]) {
    await check(`Which song is this?: Add it opens the field, ready to type (${width})`, async () => {
      const { page, context, state } = await fx.create(browser, width, { tracks: [unmatched], fixtures });
      try {
        await fx.go(page, '/tracks');
        // (phones hide the Match chip for room: the row's menu offers "Which song is this?" there)
        if (width < 600) { await page.locator('.track [data-act="more"]').first().click(); await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Which song is this?' }).first().click(); }
        else await page.locator('.track .idmark').first().click();
        const dlg = page.locator('dialog.dialog[open]');
        await dlg.waitFor();
        const text = (await dlg.innerText()).replace(/\s+/g, ' ');
        await dlg.screenshot({ path: path.join(shots, `which-song-${width}.png`) });
        assert.match(text, /needs your email or website\./, 'says what it needs');
        assert.doesNotMatch(text, /Get started/, 'not a card on Home that may be hidden');
        const add = dlg.locator('.choice').first();
        assert.equal((await add.innerText()).trim(), 'Add it', 'the same words as Search');
        assert.ok(await add.evaluate((b) => b === document.activeElement), 'Add it has the keyboard');
        await page.keyboard.press('Enter');
        await atContact(page);
        await page.screenshot({ path: path.join(shots, `which-song-settings-${width}.png`) });
        await fx.cleanCopy(page);
        await page.goBack();
        await page.waitForFunction(() => location.pathname === '/tracks' && !document.querySelector('dialog.dialog[open]'));
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`Wrong match?: MusicBrainz's search says what it needs, Add it opens the field (${width})`, async () => {
      const { page, context, state } = await fx.create(browser, width, { fixtures });
      try {
        await fx.go(page, '/album/lib-7');
        await page.click('#imore');
        await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Wrong match?' }).first().click();
        const find = page.locator('.idrow[data-track="101"] form[data-find]');
        await find.locator('input').fill('song band');
        // (by keyboard: Search pressed with Enter, its button briefly disabled, then the message's button has the focus)
        await find.locator('button').focus(); await page.keyboard.press('Enter');
        const toast = page.locator('#toast.show');
        await toast.waitFor();
        await page.screenshot({ path: path.join(shots, `wrong-match-toast-${width}.png`) });
        assert.match(await toast.innerText(), /MusicBrainz needs your email or website first/, 'says what it needs');
        assert.doesNotMatch(await toast.innerText(), /Get started/, 'not a card on Home that may be hidden');
        assert.ok(await find.locator('button').isEnabled(), 'Search can be pressed again');
        const add = toast.locator('button.toast-act');
        assert.equal((await add.innerText()).trim(), 'Add it', 'the same words as Search');
        await page.waitForFunction(() => document.activeElement && document.activeElement.matches('#toast .toast-act'));
        await page.keyboard.press('Enter');
        await atContact(page);
        await page.screenshot({ path: path.join(shots, `wrong-match-settings-${width}.png`) });
        await fx.cleanCopy(page);
        await page.goBack();
        await page.waitForFunction(() => document.querySelector('.idrow[data-track="101"]'));
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  }
  await browser.close();
  if (fails) { console.log(`${fails} FAIL`); process.exit(1); }
  console.log('all passed');
})().catch((e) => { console.error(e); process.exit(1); });
