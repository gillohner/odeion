// Automatic matching in the page (owner, 2026-11-11): "Wrong match?" on a song and on an album
// (the owner's menus only; a listener never sees it), the view it opens (the match now with its
// `auto` mark, the options, a MusicBrainz search, Not matched, Save sends each choice with the
// match the page showed), and Settings › Library's "Match new songs automatically" with the
// `auto` count. Real candidate assets on the fixture origin (tools/ui-fixture.js), at 390 and
// 1280 px. What the page sends is checked; nothing leaves the fixture origin.
//   node tools/ui-wrong-match.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'wrong-match-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 6000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(50); } };

const rec = (i) => `00000000-0000-4000-8000-${String(i).padStart(12, '0')}`;
const REL = '1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e';
const songs = [1, 2, 3].map((i) => ({ no: i, title: `Song ${i}`, artist: 'Band', artist_id: 'lib-31', duration_ms: 200000 + i * 1000, uri: null, track_id: 100 + i, id: `lib-${100 + i}`, page: `lib-${100 + i}` }));
const base = { uri: null, source: 'library', source_id: '7', plays_from: { service: 'library', label: 'Your library' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [], about_pending: false, section: 'library', image: null, tags: [], about: [], about_text: null, image_credit: null };
const album = { ...base, kind: 'album', id: 'lib-7', target: { album_id: 7 }, title: 'Album', artist: { name: 'Band', id: 'lib-31' }, album: null, year: 2001, songs };
const track = (id, no) => ({ id, title: `Song ${no}`, artists: [{ id: 31, name: 'Band' }], track_no: no, disc_no: null, duration_ms: 200000 + no * 1000, file: { path: `/music/Band/Album/0${no} Song ${no}.flac` } });
const opt = (id, trackId, recording, source, score, title) => ({ id, track_id: trackId, recording_mbid: recording, release_mbid: null, title, artist: 'Band', release_title: null, score, source });
const fixPage = {
  albums: [{
    key: '/music/Band/Album', album: 'Album', artist: 'Band', release: null, ready: 0, conflicts: 0,
    songs: [
      { track: track(101, 1), options: [opt(1, 101, rec(2), 'fingerprint', 0.95, 'Song 1 (single)')], status: 'current', pick: null, current: { recording_mbid: rec(1), release_mbid: null, source: 'auto', score: 0.95, auto: true, from_tags: false } },
      { track: track(102, 2), options: [], status: 'current', pick: null, current: { recording_mbid: rec(11), release_mbid: REL, source: 'album_fit', score: 0.97, auto: false, from_tags: false } },
      { track: track(103, 3), options: [], status: 'current', pick: null, current: { recording_mbid: rec(12), release_mbid: null, source: 'tagged_mbid', score: 1, auto: false, from_tags: true } },
    ],
  }],
  next_after: null, songs: 3, total_albums: 1, offset: 0, next: null,
};
const status = (auto) => ({ phase: 'ready', acoustid: true, acoustid_own: false, acoustid_state: 'ok', write_ids: false, auto_match: auto,
  progress: { running: false, total: 40, done: 40, identified: 30, suggested: 2, unknown: 8, errors: 0, started_at: 1793631888, finished_at: 1793633013, settled: { R1: 3, R2: 1 } },
  counts: { total: 120, identified: 110, suggested: 2, suggested_missing: 3, unknown: 8, unchecked: 0, skipped: 0, outdated: 0, by: { tags: 10, album: 70, fingerprint: 20, you: 4, title_artist: 1, auto: 5 } } });

function fixtures(state) {
  const f = {};
  f['/item/album/lib-7'] = { body: album };
  f['/item/album/lib-7/extras'] = { body: { collections: [], related: [], concerts: [], similar: [] } };
  f['/library/artists/31/similar'] = { body: { artists: [] } };
  for (const s of songs) f[`/library/tracks/${s.track_id}`] = { body: { ...fx.track, id: s.track_id, title: s.title } };
  f['/library/identify/fix'] = { body: fixPage };
  f['/library/identify'] = { body: status(state.auto) };
  f['/library/identify/settings'] = { body: status(false) };
  f['/library/tracks/101/match'] = { body: { track: track(101, 1), replaced: rec(1) } };
  f['/library/tracks/102/match'] = { body: { track: track(102, 2), replaced: rec(11) } };
  f['/library/tracks/102/candidates'] = { body: { track: track(102, 2), suggestions: [], needs_contact: false, found: [{ recording: rec(13), release: REL, title: 'The Right Song', artist: 'Band', release_title: 'Album', score: 0.97, source: 'search' }] } };
  return f;
}

// On a phone (2026-11-14): the explanation is folded under the title (as Confirm matches' is), every
// choice's label - the tap target, the radio is 20 px - is at least 44 px, and so is each MusicBrainz
// link, with no target inside another's box; the edges of a short label still choose it.
async function phoneLayout(page, at) {
  const m = await page.evaluate(() => {
    const r = (e) => { const b = e.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, b: b.bottom, r: b.right }; };
    const how = document.querySelector('#main details.idhow');
    const rows = [...document.querySelectorAll('#main .idpick')].map((o) => ({ label: r(o.querySelector('label')), text: o.querySelector('label').textContent.trim().slice(0, 24), mb: o.querySelector('.idmb') ? r(o.querySelector('.idmb')) : null }));
    // what a finger on each edge of a target hits (2 px inside it)
    const hit = (x, y) => { const e = document.elementFromPoint(x, y); return e ? (e.closest('.idmb') ? 'mb' : e.closest('.idpick > label') ? `label:${e.closest('.idpick > label').htmlFor}` : e.tagName) : null; };
    const firstRadio = document.querySelector('#main .idpick input[type=radio]').getBoundingClientRect().y;
    const edges = [...document.querySelectorAll('#main .idpick')].map((o) => {
      o.scrollIntoView({ block: 'center', behavior: 'instant' });
      const l = o.querySelector('label').getBoundingClientRect(); const mb = o.querySelector('.idmb');
      const out = { id: o.querySelector('label').htmlFor, top: hit(l.x + 20, l.y + 2), bottom: hit(l.x + 20, l.bottom - 2) };
      if (mb) { const b = mb.getBoundingClientRect(); out.mbTop = hit(b.x + 10, b.y + 2); out.mbBottom = hit(b.x + 10, b.bottom - 2); }
      return out;
    });
    document.querySelector('#main').scrollTo({ top: 0, behavior: 'instant' });
    return { how: how ? { open: how.open, sum: how.querySelector('summary').getBoundingClientRect().height, introShown: document.querySelector('.idintro').checkVisibility() } : null, rows, edges, firstRadio, over: document.documentElement.scrollWidth - innerWidth };
  });
  check(!!m.how && !m.how.open && !m.how.introShown, `${at}: the explanation is folded under the title (${JSON.stringify(m.how)})`);
  check(!!m.how && m.how.sum >= 44, `${at}: its summary is a 44 px target (${m.how && m.how.sum})`);
  // (main: 362 px at 390, 379 at 360; folded: 304)
  check(m.firstRadio < 340, `${at}: the first choice is high on the first screen (radio at y=${Math.round(m.firstRadio)})`);
  const short = m.rows.filter((o) => o.label.h < 43.5).map((o) => `${o.text} ${Math.round(o.label.h)}`);
  check(m.rows.length >= 6 && !short.length, `${at}: every choice is at least 44 px tall (${m.rows.length} choices; short: ${short.join(' | ') || 'none'})`);
  const mbs = m.rows.filter((o) => o.mb);
  const smallMb = mbs.filter((o) => o.mb.h < 43.5).map((o) => Math.round(o.mb.h));
  check(mbs.length >= 2 && !smallMb.length, `${at}: every MusicBrainz link is at least 44 px tall (${mbs.length} links; short: ${smallMb.join(',') || 'none'})`);
  const lap = mbs.filter((o) => o.mb.y < o.label.b - 0.5).length;
  check(!lap, `${at}: no MusicBrainz link reaches into its choice's label (${lap})`);
  const bad = m.edges.filter((e) => e.top !== `label:${e.id}` || e.bottom !== `label:${e.id}` || ('mbTop' in e && (e.mbTop !== 'mb' || e.mbBottom !== 'mb')));
  check(!bad.length, `${at}: each target's top and bottom edge hit that target (${JSON.stringify(bad[0] || null)})`);
  check(m.over <= 0, `${at}: no sideways scroll (${m.over})`);
}

const menuLabels = async (page) => page.$$eval('.popover.menu:not([hidden]) .pop-item', (els) => els.map((e) => e.textContent.trim()));

(async () => {
  const browser = await chromium.launch();
  for (const w of [390, 1280]) {
    for (const role of ['owner', 'listener']) {
      const st = { auto: true };
      const { page, context, state } = await fx.create(browser, w, { role, fixtures: fixtures(st), touch: w < 1025 });
      const at = `${role} at ${w}`;
      await fx.go(page, '/album/lib-7');
      await page.waitForSelector('#imore');
      // The album's menu
      await page.click('#imore');
      await until(async () => (await menuLabels(page)).length > 0);
      let labels = await menuLabels(page);
      check(labels.some((l) => /Wrong match/.test(l)) === (role === 'owner'), `${at}: the album menu ${role === 'owner' ? 'offers' : 'never offers'} Wrong match? (${labels.join(' | ')})`);
      await page.keyboard.press('Escape'); await sleep(200);
      // A song's menu
      const more = page.locator('[data-smore]').first();
      await more.click();
      await until(async () => (await menuLabels(page)).length > 0);
      labels = await menuLabels(page);
      check(labels.some((l) => /Wrong match/.test(l)) === (role === 'owner'), `${at}: a song's menu ${role === 'owner' ? 'offers' : 'never offers'} Wrong match? (${labels.join(' | ')})`);
      if (role === 'owner') {
        await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Wrong match?' }).first().click();
        check(await until(async () => (await page.locator('.idrow').count()) === 3), `${at}: Wrong match? opens the songs' matches`);
        const asked = state.hits.find((h) => h.v === '/library/identify/fix');
        check(!!asked && /tracks=101/.test(asked.q), `${at}: for the song (${asked && asked.q})`);
        await fx.go(page, '/album/lib-7'); await page.waitForSelector('#imore');
        await page.click('#imore'); await until(async () => (await menuLabels(page)).length > 0);
        await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Wrong match?' }).first().click();
        await until(async () => (await page.locator('.idrow').count()) === 3);
        const all = state.hits.filter((h) => h.v === '/library/identify/fix').pop();
        check(/tracks=101%2C102%2C103|tracks=101,102,103/.test(all.q), `${at}: and for the album's songs (${all.q})`);
        check(await page.locator('.idrow[data-track="101"] [data-auto]').count() === 1, `${at}: the automatic pick says auto`);
        check(/not written into your file/.test(await page.locator('.idrow[data-track="101"]').textContent()), `${at}: and that its IDs wait for the owner`);
        check(/or correct the tags/.test(await page.locator('.idrow[data-track="103"]').textContent()) && await page.locator('.idrow[data-track="103"] input').count() === 0, `${at}: a song matched by its tags is corrected in the file`);
        check(/stay on it/.test(await page.locator('.idintro').textContent()), `${at}: Pubky tags on the old match are said to stay`);
        if (w < 600) await phoneLayout(page, at);
        else check(await page.locator('#main details.idhow').count() === 0 && await page.locator('#main p.idintro').isVisible(), `${at}: a wide screen keeps the explanation open`);
        // Another option for song 1, a search and Not matched for song 2
        await page.locator(`.idrow[data-track="101"] input[value="${rec(2)}"]`).check();
        const find = page.locator('.idrow[data-track="102"] form[data-find]');
        await find.locator('input').fill('right song band');
        await find.locator('button').click();
        check(await until(async () => (await page.locator(`.idrow[data-track="102"] input[value="${rec(13)}"]`).count()) === 1), `${at}: the search's recording is offered`);
        const q = state.hits.filter((h) => h.v === '/library/tracks/102/candidates').pop();
        check(!!q && q.q === '?q=right%20song%20band', `${at}: with the owner's words (${q && q.q})`);
        check(await page.locator(`.idrow[data-track="101"] input[value="${rec(2)}"]`).isChecked(), `${at}: a choice made before the search stays`);
        await page.locator('.idrow[data-track="102"] input[value="none"]').check();
        await fx.cleanCopy(page);
        if (w === 1280) await page.screenshot({ path: path.join(SHOTS, `wrong-match-${w}.png`), fullPage: true });
        await page.locator('[data-fixsave="0"]').click();
        await until(async () => state.hits.filter((h) => /\/match$/.test(h.v)).length === 2);
        const sent = state.hits.filter((h) => /\/match$/.test(h.v)).map((h) => [h.v, JSON.parse(h.body)]);
        check(sent.length === 2 && sent[0][0] === '/library/tracks/101/match' && sent[0][1].recording_mbid === rec(2) && sent[0][1].expect === rec(1), `${at}: Save sends the new recording with the match the page showed (${JSON.stringify(sent[0])})`);
        check(sent[1] && sent[1][0] === '/library/tracks/102/match' && sent[1][1].none === true && sent[1][1].expect === rec(11), `${at}: and Not matched (${JSON.stringify(sent[1])})`);
        check(!state.hits.some((h) => /\/library\/tracks\/103\/match$/.test(h.v)), `${at}: an untouched song is not sent`);
        // Settings › Library: the toggle and the auto count
        await fx.go(page, '/library/node');
        check(await until(async () => (await page.locator('#idauto').count()) === 1, 10000), `${at}: Settings shows Match new songs automatically`);
        check(await page.locator('#idauto').isChecked(), `${at}: on, as the node says`);
        check(await page.locator('#idby [data-auto]').count() === 1 && /5 picked automatically/.test(await page.locator('#idby').textContent()), `${at}: the automatic picks are counted with their mark`);
        check(/4 unsure songs picked automatically/.test(await page.locator('#idbox').textContent()), `${at}: the last run says what it settled`);
        check(/only after you confirm them/.test(await page.locator('#idautohelp').textContent()), `${at}: the help line says when IDs are written`);
        check(/^3 more unsure songs’ files are out of reach/.test((await page.locator('#idmissing').textContent()).trim()), `${at}: unsure songs whose files are out of reach are said apart`);
        await page.locator('#idauto').uncheck();
        check(await until(async () => state.hits.some((h) => h.v === '/library/identify/settings' && h.m === 'PUT')), `${at}: turning it off is sent`);
        const put = state.hits.filter((h) => h.v === '/library/identify/settings').pop();
        check(JSON.parse(put.body).auto_match === false, `${at}: as auto_match false (${put.body})`);
        if (w === 1280) await page.screenshot({ path: path.join(SHOTS, `settings-auto-${w}.png`) });
      }
      check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      await context.close();
    }
  }
  // The phone widths, in a browser and in the Android app, opened from the album's menu
  for (const w of [360, 412]) {
    for (const android of [false, true]) {
      const { page, context, state } = await fx.create(browser, w, { role: 'owner', fixtures: fixtures({ auto: true }), mobile: true });
      if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
      const at = `phone ${w}${android ? ' Android' : ''}`;
      await fx.go(page, '/album/lib-7'); await page.waitForSelector('#imore');
      await page.click('#imore'); await until(async () => (await menuLabels(page)).length > 0);
      await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Wrong match?' }).first().click();
      check(await until(async () => (await page.locator('.idrow').count()) === 3), `${at}: Wrong match? opens`);
      await phoneLayout(page, at);
      // a finger on the bottom edge of "No change" chooses it, on the top edge of "Not matched" too
      const tapEdge = async (sel, edge) => { const b = await page.locator(sel).boundingBox(); await page.mouse.click(b.x + 30, edge === 'top' ? b.y + 2 : b.y + b.height - 2); };
      await tapEdge('label[for="fx-101-none"]', 'top');
      check(await page.locator('#fx-101-none').isChecked(), `${at}: the top edge of Not matched chooses it`);
      await tapEdge('label[for="fx-101-same"]', 'bottom');
      check(await page.locator('#fx-101-same').isChecked(), `${at}: the bottom edge of No change chooses it`);
      const folded = await page.locator('#main details.idhow > summary').count() === 1;
      if (folded) await page.locator('#main details.idhow > summary').click();
      check(folded && await page.locator('#main .idintro').isVisible() && /stay on it/.test(await page.locator('#main .idintro').textContent()), `${at}: the folded explanation opens with a tap`);
      // a MusicBrainz search redraws the page: the opened explanation stays open
      const find = page.locator('.idrow[data-track="102"] form[data-find]');
      await find.locator('input').fill('right song band'); await find.locator('button').click();
      await until(async () => (await page.locator(`.idrow[data-track="102"] input[value="${rec(13)}"]`).count()) === 1);
      check(await page.locator(`.idrow[data-track="102"] input[value="${rec(13)}"]`).count() === 1 && await page.locator('#main details.idhow[open]').count() === 1, `${at}: after a MusicBrainz search the opened explanation is still open`);
      await fx.cleanCopy(page);
      if (folded) await page.locator('#main details.idhow > summary').click();
      await page.screenshot({ path: path.join(SHOTS, `wrong-match-phone-${w}${android ? '-android' : ''}.png`) });
      check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      await context.close();
    }
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
