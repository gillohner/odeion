// An unmatched song's Match shows only while the row is under the mouse or focused. Hidden, it still
// kept its ~60 px beside the title, so a long title was cut that much early on every unmatched row
// (a quarter of the title at 768 px). Now the hidden Match takes no room: the title gets its whole
// cell, and the row under the mouse (after a moment) or the keyboard shows Match right after the
// title, as before (not while the mouse is on the title). A click on the end of a long title opens the
// song, not Match, however long the mouse rested there first.
// A touch screen (no hover) always shows it in its place.
// Real UI in Chromium against fixtures (no node), 1280, 1024 and 768 px.
//   node tools/ui-match-pill-room.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'match-pill-room-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// Row 1: unmatched, long title; row 2: unmatched, short title; row 3: matched (no Match), long title
const LONG = 'Journey Through Time and the Long Road Back Home Again (Extended Live Version)';
const tracks = [
  { ...fx.track, id: 1, title: LONG },
  { ...fx.track, id: 2, title: 'Short one' },
  { ...fx.track, id: 3, title: LONG, canonical: [{ kind: 'recording', mbid: fx.track.mbid }] },
];

// The title cell, its link and its Match of row n (1-based), in px
const measure = (page, n) => page.evaluate((n) => {
  const r = document.querySelectorAll('#tracklist .track')[n - 1]; const t = r.querySelector('.title');
  const a = t.querySelector(':scope > .lnk'); const p = t.querySelector(':scope > .idmark');
  const cs = getComputedStyle(t); const c = t.getBoundingClientRect(); const l = a.getBoundingClientRect();
  const content = c.width - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const pb = p && p.getBoundingClientRect();
  return { content, cellLeft: c.left, cellRight: c.right, link: l.width, linkRight: l.right, full: a.scrollWidth, pill: p ? { left: pb.left, right: pb.right, w: pb.width, op: Number(getComputedStyle(p).opacity) } : null, height: r.getBoundingClientRect().height };
}, n);

// Wait until two frames have drawn the rows (song rows are content-visibility: auto)
const drawn = (page) => page.waitForFunction(() => new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(() => res(document.querySelectorAll('#tracklist .track .title > .lnk').length === 3 && document.querySelector('#tracklist .track .title > .lnk').getBoundingClientRect().width > 0)))));

async function desktop(browser, width) {
  const { context, page } = await fx.create(browser, width, { tracks });
  const errs = []; page.on('pageerror', (e) => errs.push(e.message));
  try {
    await fx.go(page, '/tracks');
    await page.locator('#tracklist .track .title > .idmark').first().waitFor({ state: 'attached' });
    await drawn(page);
    await page.mouse.move(2, 2);
    await page.waitForTimeout(250);
    const n = (await page.locator('#tracklist .track').count());
    check(n === 3, `${width}: three songs drawn (${n})`);
    check(await page.locator('#tracklist .track').nth(2).locator('.idmark').count() === 0, `${width}: the matched song has no Match`);

    // At rest: the long unmatched title gets its whole cell, as the matched one does
    const long = await measure(page, 1); const matched = await measure(page, 3); const short = await measure(page, 2);
    check(long.pill && long.pill.op === 0, `${width}: Match is hidden at rest`);
    check(long.full > long.content, `${width}: the long title is longer than its cell (${Math.round(long.full)} > ${Math.round(long.content)})`);
    check(Math.abs(long.link - long.content) <= 1, `${width}: the long unmatched title fills its cell at rest (${Math.round(long.link)} of ${Math.round(long.content)} px)`);
    check(Math.abs(long.link - matched.link) <= 1, `${width}: an unmatched title is cut where a matched one is (${Math.round(long.link)} vs ${Math.round(matched.link)} px)`);
    check(long.pill.left >= long.cellLeft - 0.5 && long.pill.right <= long.cellRight + 0.5, `${width}: the hidden Match stays inside the title's cell`);
    check(short.link < short.content - 60, `${width}: the short title keeps its own width`);
    await page.screenshot({ path: path.join(SHOTS, `songs-rest-${width}.png`), clip: { x: 0, y: 0, width, height: 340 } });

    // Under the mouse: Match shows right after the cut title, inside the cell, beside it (not over it)
    await page.locator('#tracklist .track').nth(0).locator('.dur').hover();
    await page.waitForFunction(() => Number(getComputedStyle(document.querySelector('#tracklist .track .idmark')).opacity) === 1);
    const hov = await measure(page, 1);
    check(hov.pill.w > 30, `${width}: under the mouse Match takes its room in the row (${Math.round(hov.pill.w)} px)`);
    check(hov.pill.left >= hov.linkRight - 0.5, `${width}: Match sits after the title, not over it (${Math.round(hov.pill.left)} >= ${Math.round(hov.linkRight)})`);
    check(hov.pill.left - hov.linkRight <= 12, `${width}: Match sits right after the title (${Math.round(hov.pill.left - hov.linkRight)} px gap)`);
    check(hov.pill.right <= hov.cellRight + 0.5, `${width}: Match stays inside the title's cell`);
    check(hov.link < long.link - 40, `${width}: the title makes room for Match while it shows (${Math.round(hov.link)} < ${Math.round(long.link)})`);
    check(Math.abs(hov.height - long.height) < 0.5, `${width}: the row keeps its height`);
    await page.screenshot({ path: path.join(SHOTS, `songs-hover-${width}.png`), clip: { x: 0, y: 0, width, height: 340 } });

    // The short title's Match, under the mouse, sits right after its words, as before
    await page.locator('#tracklist .track').nth(1).locator('.dur').hover();
    await page.waitForFunction(() => Number(getComputedStyle(document.querySelectorAll('#tracklist .track')[1].querySelector('.idmark')).opacity) === 1);
    const sh = await measure(page, 2);
    check(sh.pill.left >= sh.linkRight - 0.5 && sh.pill.left - sh.linkRight <= 12, `${width}: a short title's Match sits right after it`);
    const back = await measure(page, 1);
    check(back.pill.op === 0 && Math.abs(back.link - back.content) <= 1, `${width}: the mouse gone, the first title has its whole cell again`);

    // A click on the visible Match still opens matching (it is a real button in its place)
    await page.locator('#tracklist .track').nth(0).locator('.dur').hover();
    await page.waitForFunction(() => Number(getComputedStyle(document.querySelector('#tracklist .track .idmark')).opacity) === 1);
    const target = await page.locator('#tracklist .track').nth(0).locator('.idmark').evaluate((p) => { const b = p.getBoundingClientRect(); const e = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2); return e === p || p.contains(e); });
    check(target, `${width}: the visible Match is what a click there hits`);

    // Keyboard: Tab onto the row's title link, then to Match; Match shows in its place both times
    await page.mouse.move(2, 2);
    await page.locator('#tracklist .track').nth(0).locator('.title > .lnk').focus();
    await page.keyboard.press('Shift+Tab'); await page.keyboard.press('Tab');
    const kf = await measure(page, 1);
    check(kf.pill.op === 1 && kf.pill.w > 30, `${width}: the row focused by the keyboard shows Match in its place`);
    check(kf.pill.left >= kf.linkRight - 0.5, `${width}: focused, Match is beside the title, not over it`);
    await page.keyboard.press('Tab');
    const onPill = await page.evaluate(() => document.activeElement && document.activeElement.classList.contains('idmark'));
    check(onPill, `${width}: Tab from the title reaches Match`);
    const kp = await measure(page, 1);
    check(kp.pill.op === 1 && kp.pill.w > 30, `${width}: the focused Match is shown in its place`);

    // Shift+Tab in from the next row (the mouse away): back through the first row's controls to its Match
    await page.locator('#tracklist .track').nth(1).locator('.title > .lnk').focus();
    let reached = false;
    for (let i = 0; i < 14 && !reached; i++) {
      await page.keyboard.press('Shift+Tab');
      reached = await page.evaluate(() => { const a = document.activeElement; return !!a && a.classList.contains('idmark') && a.closest('.track') === document.querySelector('#tracklist .track'); });
    }
    check(reached, `${width}: Shift+Tab from the next row reaches the first row's Match`);
    const kb = await measure(page, 1);
    check(kb.pill.op === 1 && kb.pill.w > 30 && kb.pill.left >= kb.linkRight - 0.5, `${width}: reached backwards, Match shows in its place beside the title`);
    check(errs.length === 0, `${width}: no page errors ${errs.join(' | ')}`);
  } finally { await context.close(); }
}

// The mouse moves onto the last letters of a long unmatched title and clicks at once: the song's page
// opens. Match takes its room only after the mouse has been on the row a moment, so it doesn't slide in
// under the pointer just before the click (with reduced motion too: the wait isn't motion). Nor while
// the title is pressed: a press that begins before the wait ends and lets go after it opens the song
// too (else the click would land on the row and play it). Nor while the mouse is on the title at all:
// a mouse that rests on those letters and clicks after 400 ms or 1000 ms still opens the song. Match
// shows once the mouse is on the rest of the row.
async function quickClick(browser, width, motion) {
  const { context, page, state } = await fx.create(browser, width, { tracks });
  const errs = []; page.on('pageerror', (e) => errs.push(e.message));
  const label = `${width} ${motion === 'reduce' ? 'reduced motion' : 'motion on'}`;
  const songs = async () => {
    await fx.go(page, '/tracks');
    await page.locator('#tracklist .track .title > .idmark').first().waitFor({ state: 'attached' });
    await drawn(page);
    await page.evaluate(() => { window.__hit = null; document.addEventListener('click', (e) => { window.__hit = e.target.closest('.idmark') ? 'Match' : e.target.closest('.title > .lnk') ? 'title' : String(e.target.className || e.target.tagName); }, true); });
    await page.mouse.move(2, 2); await page.waitForTimeout(400);
    return page.evaluate(() => { const b = document.querySelector('#tracklist .track .title > .lnk').getBoundingClientRect(); return { x: b.right - 8, y: b.top + b.height / 2 }; });
  };
  const under = (p) => page.evaluate(({ x, y }) => { const e = document.elementFromPoint(x, y); return e && e.closest('.idmark') ? 'Match' : e && e.closest('.title > .lnk') ? 'title' : 'other'; }, p);
  const outcome = async (what) => {
    await page.waitForFunction(() => window.__hit, null, { timeout: 3000 }).catch(() => {});
    const hit = await page.evaluate(() => window.__hit);
    check(hit === 'title', `${label}: ${what} clicks the title, not Match (${hit})`);
    await page.waitForFunction(() => location.pathname.startsWith('/song/'), null, { timeout: 3000 }).catch(() => {});
    const at = await page.evaluate(() => location.pathname);
    check(at.startsWith('/song/'), `${label}: …and the song's page opens (${at})`);
  };
  try {
    await page.emulateMedia({ reducedMotion: motion });
    // At once: the press comes right as the mouse arrives
    let end = await songs();
    await page.mouse.move(end.x, end.y); await page.mouse.down();
    const first = await under(end);
    check(first === 'title', `${label}: right as the mouse arrives, the title's last letters are still under it (${first})`);
    await page.mouse.up();
    await outcome('a quick click on the end of a long title');

    // A press that begins before the wait ends and lets go after it
    end = await songs();
    await page.mouse.move(end.x, end.y); await page.waitForTimeout(150); await page.mouse.down(); await page.waitForTimeout(450);
    const held = await measure(page, 1); const heldOn = await under(end);
    check(held.pill.w < 1 && heldOn === 'title', `${label}: while the title is pressed Match stays out of its way (${Math.round(held.pill.w)} px, ${heldOn} under the mouse)`);
    await page.mouse.up();
    await outcome('a press on the end of a title across the wait');
    const history = state.hits.filter((h) => h.m === 'POST' && /\/history\b/.test(h.v));
    check(history.length === 0, `${label}: neither click plays the song (${history.length} plays)`);

    // The mouse stays on those letters and clicks after a normal point-and-click time (400 ms), or a slow one
    // (1000 ms), past the row's wait: Match never takes the room under it, the song's page opens
    for (const wait of [400, 1000]) {
      end = await songs();
      await page.mouse.move(end.x, end.y);
      const early = await measure(page, 1);
      check(early.pill.w < 1 && Math.abs(early.link - early.content) <= 1, `${label}: right as the mouse arrives, the title still has its whole cell (Match ${Math.round(early.pill.w)} px)`);
      await page.waitForTimeout(wait);
      const still = await measure(page, 1); const stillOn = await under(end);
      check(still.pill.w < 1 && Math.abs(still.link - still.content) <= 1 && stillOn === 'title', `${label}: ${wait} ms on the title's last letters, the title keeps its whole cell (Match ${Math.round(still.pill.w)} px, ${stillOn} under the mouse)`);
      await page.mouse.down(); await page.mouse.up();
      await outcome(`a click ${wait} ms after the mouse came to rest on the end of a long title`);
    }

    // Off the title, on the rest of the row: Match shows beside the title after the wait, and the mouse
    // moved onto it keeps it there (it doesn't fold away under the pointer)
    await songs();
    await page.locator('#tracklist .track').nth(0).locator('.dur').hover();
    await page.waitForFunction(() => Number(getComputedStyle(document.querySelector('#tracklist .track .idmark')).opacity) === 1, null, { timeout: 3000 }).catch(() => {});
    await page.waitForTimeout(400);
    const late = await measure(page, 1);
    check(late.pill.op === 1 && late.pill.w > 30 && late.pill.left >= late.linkRight - 0.5, `${label}: the mouse on the rest of the row, Match shows beside the title (${Math.round(late.pill.w)} px)`);
    const mid = await page.locator('#tracklist .track').nth(0).locator('.idmark').evaluate((p) => { const b = p.getBoundingClientRect(); return { x: b.left + b.width / 2, y: b.top + b.height / 2 }; });
    await page.mouse.move(mid.x, mid.y, { steps: 4 }); await page.waitForTimeout(400);
    const onMatch = await under(mid); const kept = await measure(page, 1);
    check(onMatch === 'Match' && kept.pill.w > 30, `${label}: the mouse on Match, Match stays (${onMatch} under the mouse, ${Math.round(kept.pill.w)} px)`);

    // The keyboard on the row while the mouse rests on the title: Match shows at once all the same
    end = await songs();
    await page.mouse.move(end.x, end.y); await page.waitForTimeout(400);
    await page.locator('#tracklist .track').nth(0).locator('.title > .lnk').focus();
    await page.keyboard.press('Shift+Tab'); await page.keyboard.press('Tab');
    const kOn = await measure(page, 1);
    check(kOn.pill.op === 1 && kOn.pill.w > 30, `${label}: the keyboard on the title, the mouse resting on it, Match shows (${Math.round(kOn.pill.w)} px)`);
    await page.keyboard.press('Tab');
    const kPill = await measure(page, 1);
    check(kPill.pill.w > 30 && await page.evaluate(() => document.activeElement.classList.contains('idmark')), `${label}: …and Tab reaches Match, shown in its place`);
    check(errs.length === 0, `${label}: no page errors ${errs.join(' | ')}`);
  } finally { await context.close(); }
}

// A touch screen has no hover: Match always shows, in its place beside the title
async function touch(browser, width) {
  const { context, page } = await fx.create(browser, width, { tracks, touch: true });
  try {
    const cdp = await context.newCDPSession(page);
    await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'hover', value: 'none' }, { name: 'pointer', value: 'coarse' }] });
    await fx.go(page, '/tracks');
    await page.locator('#tracklist .track .title > .idmark').first().waitFor({ state: 'attached' });
    await drawn(page);
    const hoverNone = await page.evaluate(() => matchMedia('(hover: none)').matches);
    check(hoverNone, `${width} touch: the page sees no hover`);
    const t = await measure(page, 1);
    check(t.pill.op === 1 && t.pill.w > 30, `${width} touch: Match always shows, in its place`);
    check(t.pill.left >= t.linkRight - 0.5 && t.pill.right <= t.cellRight + 0.5, `${width} touch: Match sits after the title, inside its cell`);
    await page.screenshot({ path: path.join(SHOTS, `songs-touch-${width}.png`), clip: { x: 0, y: 0, width, height: 340 } });
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [1280, 1024, 768]) await desktop(browser, w);
    for (const w of [1280, 1024]) for (const m of ['reduce', 'no-preference']) await quickClick(browser, w, m);
    await touch(browser, 1024);
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
