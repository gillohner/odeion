// Select on a song list (a playlist here). On a phone: the rows hide the source chip's "finding"
// placeholder as well as the chip, so nothing spins beside the ticks while the sources load, and
// Select stays on when they come; every button on the selection bar is a 44 px tap; the page scrolls
// on past the bar, so the last songs and the end of the page come out from under it and the last
// song can be ticked; Stop selecting puts the page's end back. The desktop page scrolls past its
// bar too. Nothing runs past the screen.
// Real candidate assets, isolated responses: node tools/ui-phone-select-bar.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-select-bar-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Fourteen songs, more than a phone screen holds.
const uri = (n) => `https://musicbrainz.org/recording/00000000-0000-4000-8000-0000000000${String(n).padStart(2, '0')}`;
const tracks = Array.from({ length: 14 }, (_, i) => i + 1).map((id) => ({ ...fx.track, id, mbid: uri(id).split('/').pop(), canonical_uri: uri(id), title: `Song ${id}` }));
const summary = { ...fx.summary, items: tracks.length };
const detail = { summary, playlist: { ...summary, items: tracks.map((t) => ({ uri: t.canonical_uri })) }, tracks, entries: tracks.map((t) => ({ kind: 'track', uri: t.canonical_uri })), local_tracks: [] };
// (where each song plays, under the list's own uris as the node answers them)
const playFrom = { order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: tracks.map((t, index) => ({ index, uri: t.canonical_uri, title: t.title, artist: 'Sefa, Dr. Peacock', track_id: t.id, plays_from: { service: 'library', label: 'Your library' }, sources: [] })) };
const fixtures = { '/playlists/c1': { body: detail }, '/playlists/c1/play-from': { body: playFrom } };

// Scroll the page to its end (at once, not smoothly) and measure the bar and what is under it.
const atEnd = (page) => page.evaluate(() => {
  const m = document.querySelector('#main'); m.scrollTo({ top: m.scrollHeight, behavior: 'instant' });
  const bar = document.querySelector('.selbar'); const top = bar ? bar.getBoundingClientRect().top : Infinity;
  const rows = [...document.querySelectorAll('#plsongs .srow')]; const last = rows[rows.length - 1];
  const box = last.querySelector('.selbox input'); const b = box && box.getBoundingClientRect();
  const hit = b && document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2);
  return { barTop: top, end: m.lastElementChild.getBoundingClientRect().bottom, lastRow: last.getBoundingClientRect().bottom, boxOwn: !!hit && (hit === box || hit.closest('.selbox') === box.closest('.selbox')), pad: parseFloat(getComputedStyle(m).paddingBottom) };
});

async function phone(browser, width, android) {
  const at = `${width}${android ? ' Android' : ''}`;
  const { context, page, state } = await fx.create(browser, width, { mobile: true, fixtures, tracks, collections: [summary] });
  try {
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
    // The services don't answer until the test has looked at the rows in Select.
    let release; const held = new Promise((r) => { release = r; });
    await context.route('**/v1/playlists/c1/play-from', async (r) => { await Promise.race([held, new Promise((res) => setTimeout(res, 10000))]); await r.fallback().catch(() => {}); });
    await fx.go(page, '/playlist/c1');
    await page.waitForSelector('#plsongs .srcchip.wait');
    const pad0 = await page.evaluate(() => parseFloat(getComputedStyle(document.querySelector('#main')).paddingBottom));
    await page.click('#plsongs [data-selstart]');
    await page.waitForSelector('.selbar');
    await page.waitForTimeout(150);
    const spinning = await page.evaluate(() => [...document.querySelectorAll('#plsongs .srow .srcchip.wait')].filter((c) => c.getClientRects().length).length);
    check(spinning === 0, `${at}: in Select, no row shows the "finding" placeholder while the sources load (${spinning} shown)`);
    await page.screenshot({ path: path.join(SHOTS, `select-loading-${width}${android ? '-android' : ''}.png`) });
    // (the sources' answer redraws the page in place: Select stays on, its chips stay hidden)
    release();
    await page.waitForSelector('#plsongs [data-psrc]', { state: 'attached' });
    await page.waitForTimeout(150);
    const kept = await page.evaluate(() => !!document.querySelector('.selbar') && !!document.querySelector('#plsongs .selbox input'));
    check(kept, `${at}: Select stays on when the sources come`);
    const btns = await page.evaluate(() => [...document.querySelectorAll('.selbar button')].map((b) => { const r = b.getBoundingClientRect(); return { name: b.textContent.trim() || b.getAttribute('aria-label'), w: r.width, h: r.height }; }));
    check(btns.length >= 6, `${at}: the bar has its buttons (${btns.length})`);
    for (const b of btns) check(b.w >= 44 && b.h >= 44, `${at}: bar button "${b.name}" is a 44 px tap (${b.w.toFixed(0)}×${b.h.toFixed(0)})`);
    const end = await atEnd(page);
    check(end.end <= end.barTop, `${at}: at the end of the page, the page's end is above the bar (ends ${end.end.toFixed(0)}, bar ${end.barTop.toFixed(0)})`);
    check(end.lastRow <= end.barTop && end.boxOwn, `${at}: the last song is clear of the bar and its tick box takes a tap (row ends ${end.lastRow.toFixed(0)})`);
    await page.screenshot({ path: path.join(SHOTS, `select-end-${width}${android ? '-android' : ''}.png`) });
    const box = await page.locator('#plsongs .srow:last-child .selbox input').boundingBox();
    await page.touchscreen.tap(box.x + box.width / 2, box.y + box.height / 2);
    const count = await page.locator('.selbar .selcount').textContent();
    check(/^1 song selected$/.test(count.trim()), `${at}: a tap ticks the last song (${count.trim()})`);
    const over = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    check(over <= 0, `${at}: nothing runs past the screen (${over})`);
    await page.click('.selbar [data-sel="cancel"]');
    await page.waitForTimeout(100);
    const after = await page.evaluate(() => ({ pad: parseFloat(getComputedStyle(document.querySelector('#main')).paddingBottom), v: document.documentElement.style.getPropertyValue('--selbar'), bar: !!document.querySelector('.selbar') }));
    check(!after.bar && after.pad === pad0 && !after.v, `${at}: Stop selecting puts the page's end back (${pad0} → ${after.pad} px, --selbar "${after.v}")`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [360, 412]) await phone(browser, w, false);
    await phone(browser, 360, true);
    // The desktop page scrolls past its (one-line) bar too.
    const { page, context, state } = await fx.create(browser, 1280, { fixtures, tracks, collections: [summary] });
    try {
      await fx.go(page, '/playlist/c1');
      await page.waitForSelector('#plsongs .srow');
      await page.click('#plsongs [data-selstart]');
      await page.waitForSelector('.selbar');
      await page.waitForTimeout(150);
      const end = await atEnd(page);
      check(end.end <= end.barTop && end.lastRow <= end.barTop, `1280 desktop: at the end of the page, the page's end is above the bar (ends ${end.end.toFixed(0)}, bar ${end.barTop.toFixed(0)})`);
      await page.screenshot({ path: path.join(SHOTS, 'select-end-1280.png') });
      check(!state.errors.length, `1280 desktop: no page errors (${state.errors.join('; ')})`);
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
