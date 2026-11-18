// On a phone the library's song lists (Songs, Favourites, a genre's songs, Downloads' All songs and
// Favourites) had a running number (1, 2, 3…) before each title. It counts rows and nothing else,
// and the title had 194 px at 360 (246 at 412). The other phone song rows (search, playlists, item
// pages) have none, and docs/ux/responsive.md draws a phone row as the title, artist, length and ⋯.
// Now the number goes on a phone and the title takes its room: 28 px more (222 / 274 px; it starts
// 12 px in, as the other phone rows do); an album's real track numbers (Downloads › an album) stay. The row's play button stays for the
// keyboard: it shows over the row's start only while the keyboard is on it, and Enter plays from
// there; while it shows, the title and artist move to its right (it hid their first ~24 px). A tap
// (or, in the phone layout, a mouse click) at the title's start still plays the row through the row itself, and a mouse over the row doesn't show the hidden button. Desktop and
// tablet widths keep their numbers, and so does the Phone layout chosen on a window wider than 720 px.
// While the keyboard's play button shows, the row (also the long title's) still fits: no sideways
// scroll, its length and ⋯ inside the row and on screen. Only the keyboard moves the title: not a tap
// (nor a script's focus after one), not desktop/tablet rows, not an album's numbered rows.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-phone-row-numbers.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'phone-row-numbers-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let ok = 0; let fail = 0;
const check = (cond, what) => { if (cond) { ok++; console.log(`ok   ${what}`); } else { fail++; console.log(`FAIL ${what}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const LONG = 'A Remarkably Long Song Title That Keeps Going (Extended Club Remix Version)';
const tracks = [[1, LONG, 3], [2, 'Bright Morning', 1], [3, 'Cold River', 2]].map(([id, title, no]) => ({ ...fx.track, id, title, track_no: no, canonical_uri: `${fx.SONG}?n=${id}` }));
const genre = { genre: { id: 5, name: 'Ambient', albums: 0, tracks: 3 }, albums: [], tracks };
const offline = { offline: true, device_id: 'fixture-device', synced_at: null, bytes: 1, queued: 0, tracks, albums: [{ id: 7, title: 'Journey', track_ids: [2, 3, 1], cover: false }], playlists: [], favourites: { track_ids: [2] } };
// Each row of a list: its number (shown or not, and its text), where its title starts and how wide
const rows = (page, list) => page.$$eval(`[data-list="${list}"] .track`, (rs) => rs.map((r) => {
  const n = r.querySelector('.num'); const box = r.getBoundingClientRect(); const t = r.querySelector('.main-col').getBoundingClientRect();
  return { num: n && n.getClientRects().length ? n.textContent.trim() : null, lead: Math.round(t.left - box.left), title: Math.round(t.width), row: Math.round(box.width) };
}));
// A row's own left padding (12 px on a phone list row; 48 only while the keyboard is on its play button)
const padL = (row) => row.evaluate((r) => getComputedStyle(r).paddingLeft);
// Tab from row i-1's ⋯ on to row i's play button; true once it has the keyboard's focus (focus-visible)
async function tabToPlay(page, rows, i) {
  await rows.nth(i - 1).locator('[data-act="more"]').focus();
  const kb = rows.nth(i).locator('[data-act="play"]');
  for (let k = 0; k < 10; k++) {
    await page.keyboard.press('Tab');
    if (await kb.evaluate((e) => e === document.activeElement)) return kb.evaluate((e) => e.matches(':focus-visible'));
  }
  return false;
}
// While a row's play button shows (the row 48 px in): no sideways scroll, and the row's length and ⋯
// inside the row, and the row's end inside the list and the window (a clipped ⋯ is out of reach too)
const fitsShown = (row) => row.evaluate((r) => {
  const b = r.getBoundingClientRect(); const end = Math.min(b.right, r.parentElement.getBoundingClientRect().right, innerWidth);
  const inside = (e) => { const x = e.getBoundingClientRect(); return x.width > 0 && x.left >= b.left - 0.5 && x.right <= end + 0.5; };
  return { shown: r.querySelector('[data-act="play"]').matches(':focus-visible'), ov: document.documentElement.scrollWidth - innerWidth, pad: getComputedStyle(r).paddingLeft, dur: inside(r.querySelector('.dur')), more: inside(r.querySelector('[data-act="more"]')) };
});
const fitOk = (f) => f.shown && f.pad === '48px' && f.ov <= 0 && f.dur && f.more;
const nowSrc = (page) => page.evaluate(() => (window.pmPlayer && window.pmPlayer.now().src) || '');
const playing = (page, id) => page.waitForFunction((n) => [...document.querySelectorAll('audio, video')].some((a) => a.src.includes(`/library/tracks/${n}/file`)), id, { timeout: 4000 }).then(() => true, () => false);

async function phone(browser, width) {
  const at = `${width}px`;
  const run = await fx.create(browser, width, { mobile: true, tracks, favs: [fx.SONG], fixtures: { '/library/genres/5': { body: genre } } });
  const { page, context, state } = run;
  try {
    for (const [where, url, list] of [['Songs', '/tracks', 'all'], ['Favourites', '/favourites', 'fav'], ['A genre', '/genre/5', 'genre']]) {
      await fx.go(page, url); await page.waitForSelector(`[data-list="${list}"] .track`); await sleep(200);
      const rs = await rows(page, list);
      check(rs.length && rs.every((r) => r.num === null), `${at} ${where}: no running number before the titles (${JSON.stringify(rs.map((r) => r.num))})`);
      // the title starts at the row's own padding, and gets all but the length and ⋯ (38 + 32 + gaps)
      check(rs.every((r) => r.lead >= 10 && r.lead <= 14 && r.title >= r.row - 12 - 10 - 38 - 32 - 16), `${at} ${where}: the title starts ${rs[0].lead} px in and is ${rs[0].title} px of the row's ${rs[0].row}`);
      const ov = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      check(ov <= 0, `${at} ${where}: no horizontal overflow (${ov})`);
      await page.screenshot({ path: path.join(SHOTS, `${where.replace(' ', '-').toLowerCase()}-${width}.png`) });
    }
    // Songs at 360 had 194 px for the title (246 at 412); now 28 more
    await fx.go(page, '/tracks'); await page.waitForSelector('[data-list="all"] .track');
    const [first] = await rows(page, 'all');
    check(Math.abs(first.title - (width === 360 ? 222 : 274)) <= 2, `${at} Songs: the title is ${first.title} px wide (was ${width === 360 ? 194 : 246})`);
    // A tap where the number was, on the title's start: the row plays, not the hidden button
    const r2 = page.locator('[data-list="all"] .track').nth(1); const b2 = await r2.boundingBox();
    const hit = await page.evaluate(([x, y]) => { const e = document.elementFromPoint(x, y); return e ? (e.closest('[data-act]') ? e.closest('[data-act]').dataset.act : e.closest('.main-col') ? 'main-col' : e.className) : null; }, [b2.x + 16, b2.y + b2.height / 2]);
    check(hit === 'main-col', `${at} Songs: the title's start is the row, not its hidden play button (${hit})`);
    await page.touchscreen.tap(b2.x + 16, b2.y + b2.height / 2);
    check(await playing(page, 2), `${at} Songs: a tap at row 2's start plays Bright Morning (${await nowSrc(page)})`);
    // (a tap is not the keyboard: the row keeps its 12 px, the title doesn't move over, even when the
    // play button then gets focus without the keyboard, as a script's focus after a tap)
    const tapPad = await padL(r2);
    const tapFocus = await r2.locator('[data-act="play"]').evaluate((e) => { e.focus(); return { focused: e === document.activeElement, fv: e.matches(':focus-visible'), pad: getComputedStyle(e.closest('.track')).paddingLeft }; });
    await page.evaluate(() => document.activeElement && document.activeElement.blur());
    check(tapPad === '12px' && tapFocus.focused && !tapFocus.fv && tapFocus.pad === '12px', `${at} Songs: after the tap row 2 keeps its 12 px start, also with its play button focused by a script (${tapPad}, ${JSON.stringify(tapFocus)})`);
    // The keyboard's play button: hidden until the keyboard is on it, then over the row's start
    const kb = r2.locator('[data-act="play"]');
    check(await kb.evaluate((e) => getComputedStyle(e).opacity === '0'), `${at} Songs: the row's play button is not shown`);
    const before = await r2.evaluate((row) => ({ h: Math.round(row.getBoundingClientRect().height), next: Math.round(row.nextElementSibling.getBoundingClientRect().top - row.parentElement.getBoundingClientRect().top) }));
    await page.locator('[data-list="all"] .track').nth(0).locator('[data-act="more"]').focus(); await page.keyboard.press('Tab');
    const shown = await kb.evaluate((e) => { const r = e.getBoundingClientRect(); const row = e.closest('.track').getBoundingClientRect(); return { focused: e === document.activeElement, op: getComputedStyle(e).opacity, inRow: r.left >= row.left && r.right <= row.left + 48 && r.top >= row.top && r.bottom <= row.bottom, top: document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2) === e || e.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)) }; });
    check(shown.focused && shown.op === '1' && shown.inRow && shown.top, `${at} Songs: Tab from row 1's ⋯ reaches row 2's play button, shown at the row's start (${JSON.stringify(shown)})`);
    // While it shows, the title and artist (and the now-playing mark before the title) sit right of
    // it and its focus ring, not under it; the row keeps its height and the next row stays put
    const clear = await r2.evaluate((row) => {
      const b = row.querySelector('[data-act="play"]').getBoundingClientRect(); const ring = b.right + 4;
      const t = row.querySelector('.main-col').getBoundingClientRect(); const title = row.querySelector('.title');
      const mark = row.classList.contains('cur') ? Math.round(title.getBoundingClientRect().left - ring) : null; // (the mark is the title's first part)
      return { gap: Math.round(t.left - ring), cur: mark, h: Math.round(row.getBoundingClientRect().height), next: Math.round(row.nextElementSibling.getBoundingClientRect().top - row.parentElement.getBoundingClientRect().top) };
    });
    check(clear.gap >= 4 && clear.gap <= 12 && clear.cur !== null && clear.cur >= 0, `${at} Songs: the shown play button leaves the playing row's mark, title and artist in sight, ${clear.gap} px right of its ring (mark at ${clear.cur})`);
    // ...and the row still fits the screen
    const fits = await fitsShown(r2);
    check(fitOk(fits), `${at} Songs: with the play button shown the row fits: no horizontal overflow, length and ⋯ inside the row and the screen (${JSON.stringify(fits)})`);
    check(clear.h === before.h && clear.next === before.next, `${at} Songs: the row keeps its height (${before.h} → ${clear.h}) and the next row its place (${before.next} → ${clear.next})`);
    await page.screenshot({ path: path.join(SHOTS, `songs-keyboard-play-${width}.png`) });
    await page.keyboard.press('Tab');
    check(await r2.locator('[data-act="more"]').evaluate((e) => e === document.activeElement), `${at} Songs: Tab goes on to its ⋯`);
    check(await kb.evaluate((e) => getComputedStyle(e).opacity === '0'), `${at} Songs: and the play button hides again`);
    const back = (await rows(page, 'all'))[1];
    check(back.lead >= 10 && back.lead <= 14, `${at} Songs: and the title is back at the row's start (${back.lead} px in)`);
    // Enter on the next row's play button (row 2 already plays from the tap): row 3 plays
    await page.keyboard.press('Tab');
    const kb3 = page.locator('[data-list="all"] .track').nth(2).locator('[data-act="play"]');
    check(await kb3.evaluate((e) => e === document.activeElement && getComputedStyle(e).opacity === '1'), `${at} Songs: Tab goes on to row 3's play button, shown`);
    await page.keyboard.press('Enter');
    check(await playing(page, 3), `${at} Songs: Enter on it plays Cold River from there (${await nowSrc(page)})`);
    // The long title's row (row 1, no row before it): Shift+Tab back from its ⋯ to its play button;
    // the title shrinks for it, the length and ⋯ stay in the row and on screen
    const all = page.locator('[data-list="all"] .track'); const kb1 = all.nth(0).locator('[data-act="play"]');
    await all.nth(0).locator('[data-act="more"]').focus();
    for (let k = 0; k < 10 && !(await kb1.evaluate((e) => e === document.activeElement)); k++) await page.keyboard.press('Shift+Tab');
    const fits1 = await fitsShown(all.nth(0));
    check(fitOk(fits1), `${at} Songs: the long title's row fits with its play button shown (${JSON.stringify(fits1)})`);
    await page.screenshot({ path: path.join(SHOTS, `songs-keyboard-long-${width}.png`) });
    check(!state.errors.length, `${at}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
  // Downloads (offline): All songs and Favourites drop the number; an opened album keeps its track numbers
  const off = await fx.create(browser, width, { offline: true, mobile: true, tracks, fixtures: { '/offline': { body: offline } } });
  try {
    await fx.go(off.page, '/downloads'); await off.page.waitForSelector('[data-list="all"] .track', { timeout: 8000 }); await sleep(200);
    for (const list of ['all', 'fav']) { const rs = await rows(off.page, list); check(rs.length && rs.every((r) => r.num === null && r.lead <= 14), `${at} Downloads › ${list}: no running number (${JSON.stringify(rs.map((r) => r.num))})`); }
    await off.page.click('[data-dlalbum="7"]'); await off.page.waitForSelector('[data-list="album"] .track');
    const al = await rows(off.page, 'album');
    check(JSON.stringify(al.map((r) => r.num)) === '["1","2","3"]' && al.every((r) => r.lead >= 30), `${at} Downloads › an album keeps its track numbers (${JSON.stringify(al.map((r) => r.num))}, title ${al[0].lead} px in)`);
    // (its play button shows over the number, not over the title: the row doesn't make room for it)
    const ar = off.page.locator('[data-list="album"] .track'); const aPad = await padL(ar.nth(1));
    const aKb = await tabToPlay(off.page, ar, 1); const aPadKb = await padL(ar.nth(1));
    check(aKb && aPadKb === aPad && aPadKb !== '48px', `${at} Downloads › an album: the keyboard on a row's play button leaves its start as it was (${aPad} → ${aPadKb}, focused ${aKb})`);
    await off.page.screenshot({ path: path.join(SHOTS, `downloads-album-${width}.png`) });
    check(!off.state.errors.length, `${at} Downloads: no page errors (${off.state.errors.join(' | ')})`);
  } finally { await off.context.close(); }
}

// The phone layout with a mouse (a narrow window): a hover over the row doesn't show the button,
// and a click at the title's start plays the row
async function mouse(browser) {
  const { page, context } = await fx.create(browser, 390, { tracks });
  try {
    await fx.go(page, '/tracks'); await page.waitForSelector('[data-list="all"] .track');
    const r = page.locator('[data-list="all"] .track').nth(2); const b = await r.boundingBox();
    await page.mouse.move(b.x + 16, b.y + b.height / 2);
    check(await r.locator('[data-act="play"]').evaluate((e) => getComputedStyle(e).opacity === '0'), '390 mouse: a hover over the row leaves its play button hidden');
    await page.mouse.click(b.x + 16, b.y + b.height / 2);
    check(await playing(page, 3), '390 mouse: a click at row 3\'s start plays Cold River');
  } finally { await context.close(); }
}

// Desktop and tablet widths keep the numbers (and the play button over them)
async function wide(browser, width) {
  const { page, context } = await fx.create(browser, width, { tracks });
  try {
    await fx.go(page, '/tracks'); await page.waitForSelector('[data-list="all"] .track');
    const rs = await rows(page, 'all');
    check(JSON.stringify(rs.map((r) => r.num)) === '["1","2","3"]', `${width}px Songs: the rows keep their numbers (${JSON.stringify(rs.map((r) => r.num))})`);
    const r = page.locator('[data-list="all"] .track').nth(1); await r.hover();
    check(await r.locator('[data-act="play"]').evaluate((e) => getComputedStyle(e).opacity === '1'), `${width}px Songs: a hover shows the row's play button`);
    // the keyboard on it: the button sits over the number, so the row's start stays as it was
    await page.mouse.move(0, 0);
    const all = page.locator('[data-list="all"] .track'); const p0 = await padL(r);
    const kb = await tabToPlay(page, all, 1); const p1 = await padL(r);
    check(kb && p1 === p0 && p1 !== '48px', `${width}px Songs: the keyboard on row 2's play button leaves the row's start as it was (${p0} → ${p1}, focused ${kb})`);
  } finally { await context.close(); }
}

// Settings › Layout › Phone on a wider window: up to 720 px the row is the phone's four columns and
// the number goes; wider, the row keeps its album and format columns and its number, in one line
async function chosen(browser, width) {
  const { page, context } = await fx.create(browser, width, { tracks });
  try {
    await context.addInitScript(() => { localStorage.setItem('pm.layout', 'phone'); });
    await fx.go(page, '/tracks'); await page.waitForSelector('[data-list="all"] .track'); await sleep(200);
    const shell = await page.evaluate(() => document.documentElement.dataset.shell);
    const rs = await rows(page, 'all');
    const line = await page.$eval('[data-list="all"] .track', (r) => { const kids = [...r.children].filter((c) => c.getClientRects().length && !c.matches('.no') || (c.matches('.no') && getComputedStyle(c).position !== 'absolute')).map((c) => c.getBoundingClientRect()); return kids.every((a, i) => i === 0 || (a.left >= kids[i - 1].right - 1 && Math.abs(a.top + a.height / 2 - (kids[0].top + kids[0].height / 2)) < 12)); });
    const want = width <= 720 ? '[null,null,null]' : '["1","2","3"]';
    check(shell === 'phone' && JSON.stringify(rs.map((r) => r.num)) === want && line, `${width}px Phone layout: numbers ${JSON.stringify(rs.map((r) => r.num))}, the row's parts side by side in one line (${line})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [360, 412]) await phone(browser, w);
    await mouse(browser);
    for (const w of [700, 800]) await chosen(browser, w);
    for (const w of [1280, 768, 600]) await wide(browser, w);
  } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} FAIL`);
  process.exit(fail ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
