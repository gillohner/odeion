// Identify: how a run ends (the card's one line) and album review (explicit choices, conflicts,
// Confirm per album, a stale 409, errors, empty, more albums), on a phone (390), a tablet (834)
// and a desktop (1440). The node serves the page; the identify answers are fixtures, and what
// the page sends is checked. Any request outside the node's own address fails the run.
//   node tools/ui-identify-review.js <api token> <node URL> [screenshot dir]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const SHOTS = process.argv[4] || '';
const ORIGIN = new URL(BASE).origin;
const problems = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 8000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(100); } };

const REL = '1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e';
const rec = (i) => `00000000-0000-4000-8000-${String(i).padStart(12, '0')}`;
const counts = (o) => ({ total: 120, identified: 90, suggested: 0, unknown: 0, unchecked: 0, by: { tags: 10, album: 70, fingerprint: 8, you: 2 }, ...o });
const progress = (o) => ({ running: false, total: 30, done: 30, identified: 20, suggested: 6, unknown: 4, errors: 0, started_at: 1793631888, finished_at: 1793633013, ...o });
const STATUS = {
  needs_review: { phase: 'needs_review', progress: progress({}), counts: counts({ suggested: 6, unknown: 4 }) },
  ready: { phase: 'ready', progress: progress({ suggested: 0 }), counts: counts({ unknown: 4 }) },
  running: { phase: 'running', progress: progress({ running: true, done: 12, finished_at: undefined }), counts: counts({ unchecked: 18 }) },
  failed: { phase: 'failed', progress: progress({ done: 12, stopped: 'the library stayed busy for 10 minutes (a scan writing?)' }), counts: counts({ suggested: 2 }) },
  interrupted: { phase: 'interrupted', progress: progress({ done: 12, interrupted: true, finished_at: undefined }), counts: counts({ unchecked: 18 }) },
  empty: { phase: 'empty', progress: { running: false, total: 0, done: 0, identified: 0, suggested: 0, unknown: 0, errors: 0 }, counts: counts({ total: 0, identified: 0, by: { tags: 0, album: 0, fingerprint: 0, you: 0 } }) },
  new: { phase: 'new', progress: { running: false, total: 0, done: 0, identified: 0, suggested: 0, unknown: 0, errors: 0 }, counts: counts({ unchecked: 30 }) },
};
for (const st of Object.values(STATUS)) Object.assign(st, { acoustid: true, acoustid_own: true, acoustid_state: 'ok', write_ids: false });

const track = (id, no, title, folder) => ({ id, title, artists: [{ id: 1, name: 'Band' }], album: { title: folder.split('/').pop(), album_artist: 'Band' }, track_no: no, disc_no: null, duration_ms: (200 + no) * 1000, file: { path: `/music/${folder}/${String(no).padStart(2, '0')} ${title}.flac`, duration_ms: (200 + no) * 1000 } });
const opt = (id, trackId, recording, source, score, release, title) => ({ id, track_id: trackId, recording_mbid: recording, release_mbid: release, title, artist: 'Band', release_title: release === REL ? 'Album' : 'Best Of <b>x</b>', score, source });
const albumA = {
  key: '/music/Band/Album', album: 'Album <i>1</i>', artist: 'Band', release: { mbid: REL, title: 'Album', songs: 3 }, ready: 2, conflicts: 1,
  songs: [
    { track: track(11, 1, 'Song 1', 'Band/Album'), options: [opt(101, 11, rec(1), 'album', 0.95, REL, 'Song 1')], status: 'single', pick: 101 },
    { track: track(12, 2, 'Song 2', 'Band/Album'), options: [opt(102, 12, rec(2), 'fingerprint', 0.93, REL, 'Song 2')], status: 'single', pick: 102 },
    { track: track(13, 3, 'Song 3', 'Band/Album'), options: [opt(104, 13, rec(9), 'fingerprint', 0.96, null, 'Song 3 (Live)'), opt(103, 13, rec(3), 'album', 0.95, REL, 'Song 3')], status: 'conflict', pick: null },
    { track: track(14, 4, 'Song 4', 'Band/Album'), options: [opt(105, 14, rec(4), 'search', 0.6, null, 'Song 4')], status: 'single', pick: null, hold: 'search' },
  ],
};
const albumB = { key: '/music/Other/Single', album: 'Single', artist: 'Other', release: null, ready: 1, conflicts: 0, songs: [{ track: track(21, 1, 'Lone', 'Other/Single'), options: [opt(201, 21, rec(21), 'fingerprint', 0.97, null, 'Lone')], status: 'single', pick: 201 }] };
const albumC = { key: '/music/Third/Later', album: 'Later', artist: 'Third', release: null, ready: 1, conflicts: 0, songs: [{ track: track(31, 1, 'After', 'Third/Later'), options: [opt(301, 31, rec(31), 'album', 0.9, null, 'After')], status: 'single', pick: 301 }] };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [name, w, h] of [['phone', 390, 844], ['tablet', 834, 1194], ['desktop', 1440, 900]]) {
      const ctx = await browser.newContext({ viewport: { width: w, height: h }, reducedMotion: 'reduce', hasTouch: name !== 'desktop' });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', 'browser'); }, TOKEN);
      const outside = [];
      await ctx.route((url) => url.origin !== ORIGIN && !/^(data|blob):/.test(url.protocol), (r) => { outside.push(r.request().url()); return r.abort(); });
      const page = await ctx.newPage();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      const W = `${name}:`;
      let status = STATUS.needs_review;
      await page.route(/\/v1\/library\/identify$/, (r) => (r.request().method() === 'GET' ? r.fulfill({ json: status }) : r.fulfill({ json: status })));
      const sent = []; const albumsAsked = []; let confirmAnswer = null; let pages = null;
      await page.route(/\/v1\/library\/suggestions\/albums(\?.*)?$/, (r) => {
        const u = new URL(r.request().url()); albumsAsked.push(u.search);
        const off = Number(u.searchParams.get('offset') || 0);
        const p = pages ? pages(off) : { albums: [], songs: 0, total_albums: 0, offset: 0, next: null };
        return r.fulfill({ json: p });
      });
      await page.route(/\/v1\/library\/suggestions\/confirm$/, (r) => { sent.push({ confirm: r.request().postDataJSON() }); const a = confirmAnswer; return r.fulfill({ status: a.status, json: a.json }); });
      await page.route(/\/v1\/library\/suggestions\/\d+(\?.*)?$/, (r) => { const u = new URL(r.request().url()); sent.push({ [r.request().method()]: u.pathname.split('/').pop(), query: u.search }); return r.fulfill({ status: 204, body: '' }); });
      const overflow = async () => page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
      const shot = async (what) => { if (SHOTS) await page.screenshot({ path: `${SHOTS}/${name}-${what}.png`, fullPage: false }); };

      // ---- The card: one line says how identification stands ----
      await page.goto(`${BASE}/library/node`);
      const phaseLine = page.locator('#idphase');
      check(await until(async () => (await phaseLine.count()) > 0, 15000), `${W} the Identify card shows its state line`);
      await phaseLine.scrollIntoViewIfNeeded();
      // Review is optional (TASK-identify-less-review-1106): "Finished", the unsure songs a quiet second line
      check(/^Finished$/.test((await phaseLine.textContent()).trim()), `${W} finished: said as "Finished" (${(await phaseLine.textContent()).trim()})`);
      check(/6 songs unsure \(optional\)/.test(await page.locator('#idunsure').textContent()), `${W} the unsure songs are a second line, optional`);
      const review = page.locator('#idbox .actions [data-view="identify"]');
      check(await review.isVisible() && !/primary/.test(await review.getAttribute('class')), `${W} Review is offered, not as the main action`);
      check(!/primary/.test(await page.locator('#idrun').getAttribute('class')), `${W} Identify again is secondary then`);
      await shot('card-needs-review');
      const expect = { ready: /Finished — nothing to review \(4 not found\)/, running: /Working: 12 of 30/, failed: /It stopped after 12 of 30: the library stayed busy/, interrupted: /cut off when the node stopped \(after 12 of 30\)/, empty: /No songs yet/, new: /30 songs not checked yet/ };
      for (const [ph, re] of Object.entries(expect)) {
        status = STATUS[ph];
        await page.evaluate(() => window.dispatchEvent(new Event('popstate')));
        await page.goto(`${BASE}/library/node`);
        await until(async () => (await phaseLine.count()) > 0 && (await phaseLine.getAttribute('data-phase')) === ph, 15000);
        const text = await phaseLine.textContent();
        check(re.test(text), `${W} ${ph}: "${text.trim()}"`);
        if (ph === 'failed' || ph === 'interrupted') check(/warn-text/.test(await phaseLine.getAttribute('class')), `${W} ${ph} is marked as a warning`);
        if (ph === 'empty') check(await page.locator('#idrun').isDisabled(), `${W} nothing to identify: Identify is off`);
        if (ph === 'running') check(await page.locator('#idrun').isDisabled(), `${W} running: Identify is off`);
        check(await page.locator('#idreview').count() === 0, `${W} ${ph}: no Review as the main action`);
        if (ph === 'failed') await shot('card-failed');
      }
      check(await overflow() <= 0, `${W} the card fits the width`);

      // ---- Album review ----
      pages = (off) => (off === 0 ? { albums: [albumA, albumB], songs: 6, total_albums: 3, offset: 0, next: 2 } : { albums: [albumC], songs: 6, total_albums: 3, offset: 2, next: null });
      await page.goto(`${BASE}/identify`);
      const secA = page.locator('section.idalbum').first();
      check(await until(async () => (await secA.count()) > 0, 15000), `${W} albums listed`);
      check(await page.locator('section.idalbum h3 i').count() === 0 && /Album <i>1<\/i>/.test(await secA.locator('h3').textContent()), `${W} album names are text, not markup`);
      check(await page.locator('.idopt .sub b').count() === 0, `${W} release names are text, not markup`);
      check(await secA.locator('input[name="pick-11"][value="101"]').isChecked() && await secA.locator('input[name="pick-12"][value="102"]').isChecked(), `${W} strong single options are offered ticked`);
      check(await secA.locator('input[name="pick-13"][value=""]').isChecked() && !(await secA.locator('input[name="pick-13"][value="103"]').isChecked()) && !(await secA.locator('input[name="pick-13"][value="104"]').isChecked()), `${W} a conflict is left for later, nothing picked for the owner`);
      check(/disagree/.test(await secA.locator('.idrow[data-track="13"]').textContent()), `${W} the conflict says its options disagree`);
      check(/title and artist only/.test(await secA.locator('.idrow[data-track="14"]').textContent()) && await secA.locator('input[name="pick-14"][value=""]').isChecked(), `${W} a search-only option is not ticked, and says why`);
      check(/Nothing is chosen for you/.test(await secA.textContent()), `${W} the album's release is shown as information only`);
      const btnA = secA.locator('[data-confirm]');
      check((await btnA.textContent()).trim() === 'Confirm 2 songs', `${W} Confirm counts the ticked songs`);
      const box = await btnA.boundingBox();
      check(box && box.height >= (name === 'desktop' ? 28 : 32), `${W} Confirm is big enough to tap (${box && Math.round(box.height)} px)`);
      check(await overflow() <= 0, `${W} the review fits the width`);
      await shot('review');
      // The owner picks the conflict's folder option: three songs
      await secA.locator('input[name="pick-13"][value="103"]').check();
      check((await btnA.textContent()).trim() === 'Confirm 3 songs', `${W} picking the conflict adds it`);
      // A stale choice: 409, nothing stored, the song marked and Reload offered
      confirmAnswer = { status: 409, json: { error: 'Some songs changed since the page was loaded, so nothing was confirmed. Reload and check them again.', stale: [{ track_id: 12, suggestion_id: 102, reason: 'identified' }] } };
      await btnA.click();
      check(await until(async () => (await secA.locator('.idrow.idstale[data-track="12"]').count()) === 1), `${W} 409: the stale song is marked`);
      check(/nothing was confirmed/.test(await secA.locator('[data-confirm-out]').textContent()), `${W} 409: says nothing was confirmed`);
      check((await btnA.textContent()).trim() === 'Reload this list', `${W} 409: offers to reload`);
      const body = sent.find((x) => x.confirm);
      check(body && JSON.stringify(body.confirm) === JSON.stringify({ choices: [{ track_id: 11, suggestion_id: 101, recording_mbid: rec(1) }, { track_id: 12, suggestion_id: 102, recording_mbid: rec(2) }, { track_id: 13, suggestion_id: 103, recording_mbid: rec(3) }] }), `${W} the request names each chosen song, suggestion and recording: ${JSON.stringify(body && body.confirm)}`);
      await shot('review-stale');
      // Reload; a failure on the node is said, not hidden
      sent.length = 0;
      await btnA.click();
      await until(async () => (await page.locator('section.idalbum').first().locator('[data-confirm]').textContent()).trim() === 'Confirm 2 songs');
      const secA2 = page.locator('section.idalbum').first();
      confirmAnswer = { status: 500, json: { error: 'the library stayed busy' } };
      await secA2.locator('[data-confirm]').click();
      check(await until(async () => /./.test(await secA2.locator('[data-confirm-out]').textContent())), `${W} a failure is said at the album`);
      check(await secA2.locator('[data-confirm]').isEnabled(), `${W} and Confirm can be tried again`);
      // Success: confirmed, the list loads again
      const asked = albumsAsked.length;
      confirmAnswer = { status: 200, json: { confirmed: [11, 12], count: 2 } };
      await secA2.locator('[data-confirm]').click();
      // The confirmed songs leave the album where it is; the list isn't loaded again
      check(await until(async () => (await secA2.locator('.idrow').count()) === 2), `${W} after Confirm the confirmed songs leave the album`);
      check(albumsAsked.length === asked, `${W} … without loading the list again`);
      // Not this
      sent.length = 0;
      const secB = page.locator('section.idalbum').nth(1);
      await secB.locator('[data-reject="201"]').click();
      check(await until(async () => sent.some((x) => x.DELETE === '201')), `${W} Not this rejects that option only`);
      check(sent.some((x) => x.DELETE === '201' && x.query === `?track_id=21&recording_mbid=${encodeURIComponent(rec(21))}`), `${W} … naming the song and recording it shows`);
      // More albums
      const more = page.locator('#idmore');
      check(await until(async () => (await more.count()) === 1), `${W} Show more albums when there are more`);
      await more.click();
      // Album B's only song had its only option rejected: the album left the page where it was
      check(await page.locator('section.idalbum[data-album="1"]').count() === 0, `${W} an album with nothing left leaves the page`);
      check(await until(async () => albumsAsked.some((q) => /offset=2/.test(q)) && (await page.locator('section.idalbum').count()) === 2), `${W} more albums are added below`);
      check(await page.locator('#idmore').count() === 0, `${W} no more after the last`);
      // Empty
      pages = () => ({ albums: [], songs: 0, total_albums: 0, offset: 0, next: null });
      await page.goto(`${BASE}/identify`);
      check(await until(async () => /Nothing to confirm/.test(await page.locator('#main').textContent())), `${W} nothing to review: said so`);
      await shot('review-empty');

      check(!errors.length, `${W} no page errors ${errors.join(' | ')}`);
      check(!outside.length, `${W} nothing asked outside the node ${outside.slice(0, 3).join(' ')}`);
      await ctx.close();
    }
  } finally {
    await browser.close();
  }
  if (problems.length) { console.log(`\n${problems.length} problem(s)`); process.exit(1); }
  console.log('\nall good');
})().catch((e) => { console.error(e); process.exit(1); });
