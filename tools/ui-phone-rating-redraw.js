// Rating an album or song redraws the whole top card. The Pubky tags it already showed stay on screen
// through that redraw (asked again quietly, and drawn again only when the answer changed), so the tags
// skeleton never shows and My rating's stars and Add notes don't move: before, at 320/360 px the stars
// dropped ~42 px for a network round trip, then came back, and a quick second press missed.
// Checked at 320, 360 and 412 px (360 as the Android app too) and 1280 px: after a press the stars'
// top stays within 1 px on every frame until the new answer is in, no skeleton is drawn, Pubky is
// asked again, and a changed answer (a tag someone added meanwhile) is shown. A tag you remove isn't
// shown again from an earlier answer (a tag edit asks Pubky afresh), not even one a rating's redraw
// got while the removal was on its way, and another page never gets this page's tags. Two quick saves
// whose answers land out of order: the next redraw stands in the newer answer, not the one that landed last.
// Pubky's answer after a save fails (500): the tags drawn stay as they are (no "didn't load", no skeleton,
// the stars and Add notes don't move), and the next redraw still stands them in. A third redraw that
// starts before the second ask's answer lands stands in the first ask's (newer) answer, not an older one.
// When the newest ask fails and an older redraw's answer was kept meanwhile, that newer kept answer is shown;
// when that kept answer was "no identity" (422), no Pubky tags are shown (not the older ones).
// Real candidate assets, isolated responses: node tools/ui-phone-rating-redraw.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-rating-redraw-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, timeout = 4000) => { const end = Date.now() + timeout; for (;;) { const v = await fn(); if (v) return v; if (Date.now() > end) return v; await sleep(50); } };

const base = { source: 'Your library', source_id: 'library', uri: null, song_uri: null, link: null, plays_from: { service: 'library', label: 'Your library' }, albums: [], versions: [], also_on: [], about: [], open_on: [], connect: [], on_pubky: null };
const songs = [1, 2, 3].map((n) => ({ id: `lib-${n}`, track_id: n, title: `Song ${n}`, artist: 'Sefa', duration_ms: 200000, no: n }));
const fixtures = {
  '/item/album/lib-7': { body: { ...base, kind: 'album', id: 'lib-7', title: 'Journey', section: 'albums', target: { album_id: 7 }, songs, artist: { id: 'lib-2', name: 'Sefa' }, year: 2024 } },
  '/item/album/lib-8': { body: { ...base, kind: 'album', id: 'lib-8', title: 'Other', section: 'albums', target: { album_id: 8 }, songs, artist: { id: 'lib-2', name: 'Sefa' }, year: 2023 } },
  '/item/album/lib-7/extras': { body: {} }, '/item/album/lib-8/extras': { body: {} },
};
const RELEASE = 'https://musicbrainz.org/release/0b8a1a4e-1d9c-4c47-b4e0-6a1b3c1e9d07';
const OTHER = 'https://musicbrainz.org/release/5d3c1f0e-8b6a-4c2e-9a1d-2f7e6b5c4a39';
const LAG = 700; // Pubky's answer, late enough for a frame-by-frame look

// One page's answers: your own tags (GET /tags) and Pubky's (GET /discover/uri), both changeable
async function open(browser, width, { android = false } = {}) {
  const { page, context } = await fx.create(browser, width, { mobile: width < 600, fixtures });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  const s = { review: null, mine: [], pub: { 7: [['dreamy', 3], ['jazz', 2], ['live', 1], ['late night', 1]], 8: [['other page', 1]] }, asked: 0, delLag: 0, lags: [], fails: [], landed: [] };
  const album = (u) => Number(new URL(u).searchParams.get('album_id'));
  await page.route(/\/v1\/reviews(\?.*)?$/, async (r) => {
    const m = r.request().method(); await sleep(150);
    if (m === 'PUT') { s.review = { rating: JSON.parse(r.request().postData()).rating, text: '' }; return r.fulfill({ json: {} }); }
    if (m === 'DELETE') { s.review = null; return r.fulfill({ json: {} }); }
    return r.fulfill({ json: { reviews: s.review ? [s.review] : [] } });
  });
  await page.route(/\/v1\/tags(\?.*)?$/, async (r) => {
    const m = r.request().method(); const u = r.request().url();
    if (m === 'DELETE') { await sleep(s.delLag); const l = new URL(u).searchParams.get('label'); s.mine = s.mine.filter((x) => x !== l); s.pub[7] = s.pub[7].filter(([x]) => x !== l); return r.fulfill({ json: {} }); }
    if (!new URL(u).searchParams.get('album_id')) return r.fulfill({ json: { tags: [] } });
    const id = album(u);
    return r.fulfill({ json: { tags: id === 7 ? s.mine.map((label) => ({ label, synced: true })) : [], uri: id === 7 ? RELEASE : OTHER } });
  });
  await page.route(/\/v1\/discover\/uri(\?.*)?$/, async (r) => { const id = album(r.request().url()); s.asked++; const tags = s.pub[id] || []; await sleep(s.lags.length ? s.lags.shift() : LAG); const fail = s.fails.shift(); if (fail) { s.landed.push('failed'); return r.fulfill({ status: fail, json: { error: 'upstream' } }); } s.landed.push(tags.map(([l]) => l)); return r.fulfill({ json: { tags, faces: {} } }); });
  return { page, context, s };
}
const settled = (page) => until(() => page.evaluate(() => !!document.querySelector('#htags .tagitem') && !document.querySelector('#htags .tag-skel') && !!document.querySelector('#stars')));
// Every frame from now: where the stars' top is, whether a skeleton is drawn, which tags show
const watch = (page) => page.evaluate(() => {
  window.__frames = []; const f = () => {
    const s = document.querySelector('#stars'); const n = document.querySelector('#notesbtn');
    window.__frames.push({ h1: (document.querySelector('h1') || {}).textContent || '', star: s ? Math.round(s.getBoundingClientRect().top) : null, notes: n ? Math.round(n.getBoundingClientRect().top) : null, skel: !!document.querySelector('#htags .tag-skel'), pre: !!document.querySelector('#htags[data-pre]'), tags: [...document.querySelectorAll('#htags .tagitem')].map((t) => t.dataset.label), others: [...document.querySelectorAll('#htags .tagitem')].filter((t) => !t.querySelector('[data-htag-del]')).map((t) => t.dataset.label) });
    if (window.__frames.length < 600) requestAnimationFrame(f);
  }; requestAnimationFrame(f);
});
const frames = (page) => page.evaluate(() => window.__frames);
const inApp = (page, to) => page.evaluate((p) => { history.pushState(null, '', p); dispatchEvent(new PopStateEvent('popstate', { state: null })); }, to);
const press = async (page, v) => { const b = await page.evaluate((x) => { const r = document.querySelector(`#stars [data-rate="${x}"]`).getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }, v); await page.mouse.click(b[0], b[1]); };

(async () => {
  const browser = await chromium.launch();
  for (const [width, android] of [[320, false], [360, false], [360, true], [412, false], [1280, false]]) {
    const at = `${width}${android ? ' Android' : ''}`;
    const { page, context, s } = await open(browser, width, { android });
    await fx.go(page, '/album/lib-7');
    check(await settled(page), `${at}: the album's tags are in`);
    const top0 = await page.evaluate(() => Math.round(document.querySelector('#stars').getBoundingClientRect().top));
    const asked0 = s.asked;
    // Someone else tags it meanwhile: the answer after the save has one more tag
    s.pub[7] = [...s.pub[7], ['vinyl', 1]];
    await watch(page);
    await press(page, 8);
    if (width === 360 && !android) { await sleep(450); await page.screenshot({ path: path.join(SHOTS, 'during-rating-redraw-360.png') }); }
    await until(() => s.review && s.review.rating === 8);
    await until(async () => (await page.evaluate(() => [...document.querySelectorAll('#htags .tagitem')].map((t) => t.dataset.label))).includes('vinyl'), LAG + 3000);
    await sleep(200);
    const fr = await frames(page);
    const moved = fr.filter((x) => x.star != null && Math.abs(x.star - top0) > 1);
    check(fr.length > 20, `${at}: watched ${fr.length} frames`);
    check(!moved.length, `${at}: the stars stay at ${top0} px through the save's redraw${moved.length ? ` (moved to ${[...new Set(moved.map((x) => x.star))].join(', ')} px on ${moved.length} frames)` : ''}`);
    const notesTops = [...new Set(fr.map((x) => x.notes).filter((x) => x != null))];
    check(notesTops.length === 1, `${at}: Add notes stays put (${notesTops.join(', ')} px)`);
    check(!fr.some((x) => x.skel), `${at}: no tags skeleton is drawn during the redraw`);
    check(!fr.some((x) => x.tags.length === 0), `${at}: the tags never all disappear`);
    check(s.asked > asked0, `${at}: Pubky is asked again after the save (${s.asked - asked0} more)`);
    check(fr[fr.length - 1].tags.includes('vinyl'), `${at}: the newer answer's tag is shown`);
    check(await page.evaluate(() => document.querySelector('#stars [data-rate="8"]').getAttribute('aria-pressed') === 'true'), `${at}: the rating shows 4 stars`);
    if (width === 360) await page.screenshot({ path: path.join(SHOTS, `after-rating-redraw-${at.replace(' ', '-').toLowerCase()}.png`) });
    // A second quick press lands where the first did
    const before2 = s.review.rating;
    await press(page, 6); await sleep(80);
    check(await until(() => s.review && s.review.rating === 6) && before2 === 8, `${at}: a second press right after sets 3 stars`);
    await settled(page); await sleep(LAG + 300);
    // Two quick saves whose Pubky answers land out of order (the first one's late, without "fresh"):
    // the next redraw shows the newer answer at once, never the older one that landed last
    const old7 = s.pub[7];
    s.lags = [1800]; const a1 = s.asked; const l0 = s.landed.length;
    await press(page, 10);
    await until(() => s.asked > a1);
    s.pub[7] = [...old7, ['fresh', 1]];
    s.lags = [150]; const a2 = s.asked;
    await press(page, 2);
    await until(() => s.asked > a2);
    await sleep(2200);
    // (the race really happened: the second save's answer, with "fresh", landed first)
    const order = s.landed.slice(l0).map((t) => (t.includes('fresh') ? 'newer' : 'older'));
    check(order.join(',') === 'newer,older', `${at}: the older answer landed last (${order.join(', ')})`);
    const both = await page.evaluate(() => [...document.querySelectorAll('#htags .tagitem')].map((t) => t.dataset.label));
    check(both.includes('fresh'), `${at}: after both answers are in, the newer one is shown (${both.join(', ')})`);
    s.lags = [1500];
    // (the tags box before the press is marked: the redraw draws a new one)
    await page.evaluate(() => { document.querySelector('#htags').dataset.pre = '1'; });
    await watch(page);
    await press(page, 8);
    await until(() => s.review && s.review.rating === 8);
    await sleep(450);
    if (width === 360 && !android) await page.screenshot({ path: path.join(SHOTS, 'redraw-after-two-quick-saves-360.png') });
    await sleep(450);
    const fz = await frames(page);
    // Every frame with the page drawn shows the newer answer: none without "fresh" (the older answer, a
    // skeleton or no tags until the redraw's own answer lands)
    const drawn = fz.filter((x) => /Journey/.test(x.h1));
    const older = drawn.filter((x) => x.skel || !x.tags.includes('fresh')).length;
    const redrawn = drawn.filter((x) => !x.pre).length;
    check(drawn.length > 20 && redrawn > 10 && !older, `${at}: the next redraw stands in the newer answer, not the one that landed last (${older} of ${drawn.length} frames without "fresh" or with a skeleton; ${redrawn} after the redraw)`);
    await sleep(900);
    check(await settled(page) && (await page.evaluate(() => [...document.querySelectorAll('#htags .tagitem')].map((t) => t.dataset.label))).includes('fresh'), `${at}: the tags are in after that redraw, still with "fresh"`);
    // Pubky fails after a save (500): what was drawn stays, and the next redraw still stands it in
    const tagsNow = () => page.evaluate(() => [...document.querySelectorAll('#htags .tagitem')].map((t) => t.dataset.label));
    const want = await tagsNow();
    const top1 = await page.evaluate(() => Math.round(document.querySelector('#stars').getBoundingClientRect().top));
    s.fails = [500]; const a3 = s.asked; const l3 = s.landed.length;
    await watch(page);
    await press(page, 6);
    await until(() => s.review && s.review.rating === 6);
    await until(() => s.landed.length > l3, LAG + 3000); await sleep(300);
    check(s.asked > a3 && s.landed[l3] === 'failed', `${at}: Pubky's answer after the save failed (500)`);
    const ff = (await frames(page)).filter((x) => /Journey/.test(x.h1));
    const off = ff.filter((x) => x.skel || x.tags.join() !== want.join()).length;
    const jumped = ff.filter((x) => (x.star != null && Math.abs(x.star - top1) > 1) || x.notes !== ff[0].notes).length;
    check(ff.length > 20 && !off, `${at}: after a failed answer the tags stay as drawn (${off} of ${ff.length} frames changed or with a skeleton)`);
    check(!jumped, `${at}: after a failed answer the stars and Add notes don't move (${jumped} frames moved)`);
    check(!(await page.evaluate(() => /didn.t load/.test(document.querySelector('#htags')?.textContent || ''))), `${at}: no "Pubky tags didn't load" over tags that are there`);
    if (width === 360 && !android) await page.screenshot({ path: path.join(SHOTS, 'after-failed-answer-360.png') });
    // The next redraw (its answer slow) stands in the kept tags, not a skeleton
    s.lags = [1500];
    await watch(page);
    await press(page, 8);
    await until(() => s.review && s.review.rating === 8); await sleep(600);
    const fn = (await frames(page)).filter((x) => /Journey/.test(x.h1));
    const lost = fn.filter((x) => x.skel || x.tags.join() !== want.join()).length;
    check(fn.length > 20 && !lost, `${at}: the redraw after a failed answer still stands in the kept tags (${lost} of ${fn.length} frames without them or with a skeleton)`);
    await sleep(1500); await settled(page);
    // Three quick redraws: the first's answer (with "third") lands before the second's; the third redraw,
    // drawn before the second's answer lands, stands in the first's, not the one kept from before
    s.pub[7] = [...old7, ['fresh', 1], ['third', 1]];
    s.lags = [1400, 3500]; const a4 = s.asked; const l4 = s.landed.length;
    await press(page, 4);
    await until(() => s.asked > a4);
    s.pub[7] = [...old7, ['fresh', 1]];
    await until(() => s.review && s.review.rating === 4);
    await press(page, 2);
    await until(() => s.asked > a4 + 1);
    await until(() => s.landed.length > l4, 3000);
    check(s.landed.length === l4 + 1 && s.landed[l4].includes('third'), `${at}: the first ask's answer landed before the second's (${s.landed.slice(l4).map((t) => t.join('/')).join('; ')})`);
    await page.evaluate(() => { document.querySelector('#htags').dataset.pre = '1'; });
    await watch(page);
    s.lags = [3500];
    await press(page, 6);
    await until(() => s.review && s.review.rating === 6); await sleep(500);
    const ft = (await frames(page)).filter((x) => /Journey/.test(x.h1) && !x.pre);
    const no3 = ft.filter((x) => x.skel || !x.tags.includes('third')).length;
    check(ft.length > 10 && !no3, `${at}: a third redraw stands in the first ask's answer that landed (${no3} of ${ft.length} frames without "third")`);
    if (width === 360 && !android) await page.screenshot({ path: path.join(SHOTS, 'third-redraw-360.png') });
    await sleep(4000); await settled(page);
    // Two quick redraws: the first's answer (with "fourth") lands, the second's (the one on screen) fails:
    // the screen shows the first's, kept meanwhile, not the older tags it stood in
    s.pub[7] = [...old7, ['fresh', 1], ['fourth', 1]];
    s.lags = [500, 1800]; s.fails = [0, 500]; const a5 = s.asked; const l5 = s.landed.length;
    await press(page, 10);
    await until(() => s.asked > a5);
    s.pub[7] = [...old7, ['fresh', 1]];
    await until(() => s.review && s.review.rating === 10);
    await press(page, 8);
    await until(() => s.asked > a5 + 1);
    await until(() => s.landed.length > l5 + 1, 4000); await sleep(300);
    const shownNow = await tagsNow();
    check(s.landed[l5 + 1] === 'failed' && shownNow.includes('fourth') && !(await page.evaluate(() => /didn.t load/.test(document.querySelector('#htags')?.textContent || ''))), `${at}: a failed answer shows a newer one kept meanwhile (${s.landed.slice(l5).map((t) => (Array.isArray(t) ? t.join('/') : t)).join('; ')} -> ${shownNow.join(', ')})`);
    s.fails = []; s.lags = [];
    await sleep(2500); await settled(page);
    // The same with the first's answer "no identity" (422): the failed one on screen draws that answer,
    // no Pubky tags, not the older tags it stood in
    const a6 = s.asked; const l6 = s.landed.length;
    s.lags = [500, 1800]; s.fails = [422, 500];
    await press(page, 4);
    await until(() => s.asked > a6);
    await until(() => s.review && s.review.rating === 4);
    await press(page, 2);
    await until(() => s.asked > a6 + 1);
    await until(() => s.landed.length > l6 + 1, 4000); await sleep(300);
    const shown422 = await tagsNow();
    check(s.landed.length === l6 + 2 && !shown422.some((t) => old7.some(([l]) => l === t)), `${at}: a failed answer after a newer "no identity" shows no Pubky tags, not the older ones (${shown422.join(', ') || 'none'})`);
    if (width === 360 && !android) await page.screenshot({ path: path.join(SHOTS, 'failed-after-no-identity-360.png') });
    s.fails = []; s.lags = [];
    s.pub[7] = old7;
    // Another album: never this one's tags
    await watch(page);
    await inApp(page, '/album/lib-8');
    await until(() => page.evaluate(() => /Other/.test(document.querySelector('h1')?.textContent || '')));
    await settled(page);
    const fo = (await frames(page)).filter((x) => /Other/.test(x.h1));
    const leaked = fo.filter((x) => x.tags.includes('dreamy')).length;
    const shown = await page.evaluate(() => [...document.querySelectorAll('#htags .tagitem')].map((t) => t.dataset.label).join(','));
    check(fo.length > 0 && !leaked && shown === 'other page', `${at}: another album shows only its own tags (${shown}; ${leaked} frames with this album's)`);
    // Back on the first album, now with your own tag "live": removing it doesn't draw the earlier answer
    // (which had it) again
    s.mine = ['live'];
    await inApp(page, '/album/lib-7');
    await until(() => page.evaluate(() => /Journey/.test(document.querySelector('h1')?.textContent || '')));
    await settled(page); await sleep(LAG + 300);
    await watch(page);
    const del = await page.$('#htags [data-htag-del][data-label="live"]');
    check(!!del, `${at}: your tag "live" can be removed`);
    if (del) {
      // The removal takes a while to land, and a rating is saved meanwhile: that redraw's Pubky answer
      // (asked before the removal landed) still has "live", and isn't what the page shows after it
      s.delLag = 1200;
      await del.click();
      await sleep(100); await press(page, 4);
      await until(() => !s.mine.includes('live'));
      await sleep(LAG + 1200);
      const fd = await frames(page);
      // (yours, it has a remove button; drawn from the earlier answer, it would be someone else's "live")
      const stale = fd.filter((x) => x.others.includes('live')).length;
      check(fd.length > 20 && !stale && !fd[fd.length - 1].tags.includes('live'), `${at}: a removed tag isn't drawn again from the earlier answer (${stale} frames)`);
    }
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
    check(overflow <= 0, `${at}: nothing wider than the screen (${overflow} px over)`);
    await context.close();
  }
  await browser.close();
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
