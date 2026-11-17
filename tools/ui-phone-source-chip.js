// On a phone a playlist song's source chip (its badge alone: Your library, pinned, Unavailable) is a
// 48 px wide chip beside ⋯, at least a 44 px wide tap of its own (⋯'s reach would otherwise take part
// of it); a touch near either edge opens "Play this song from". ⋯ and the title keep their taps, the
// "finding" placeholder is as wide as the chip (the title doesn't move when the answer comes),
// nothing runs past the screen, and the desktop layout keeps its chip with the service's name.
// Real candidate assets, isolated responses: node tools/ui-phone-source-chip.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-source-chip-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Three songs: one plays from the library, one is pinned to it, one plays from nowhere. The first
// has a long title, so the text runs right up to the chip.
const uri = (n) => `https://musicbrainz.org/recording/00000000-0000-4000-8000-00000000000${n}`;
const tracks = [1, 2, 3].map((id) => ({ ...fx.track, id, mbid: uri(id).split('/').pop(), canonical_uri: uri(id), title: id === 1 ? 'A very long song title that runs right up to the chip beside it' : `Song ${id}` }));
const summary = { ...fx.summary, items: 3 };
const detail = { summary, playlist: { ...summary, items: tracks.map((t) => ({ uri: t.canonical_uri })) }, tracks, entries: tracks.map((t) => ({ kind: 'track', uri: t.canonical_uri })), local_tracks: [] };
const lib = { service: 'library', label: 'Your library' };
const libSource = { service: 'library', name: 'Your library', label: 'Your library', available: true, chosen: true };
const item = (t, k, extra) => ({ index: k, uri: t.canonical_uri, title: t.title, artist: 'Sefa, Dr. Peacock', track_id: t.id, plays_from: lib, sources: [libSource], ...extra });
const playFrom = {
  order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false,
  items: [item(tracks[0], 0), item(tracks[1], 1, { pin: { service: 'library' } }), item(tracks[2], 2, { plays_from: null, sources: [], reason: 'Not on your connected services' })],
};
const fixtures = { '/playlists/c1': { body: detail }, '/playlists/c1/play-from': { body: playFrom } };
const KINDS = ['Your library', 'pinned', 'Unavailable'];

// Every chip's box, how much of a line through its middle reaches it, and what ⋯ and the title get.
const measure = (page) => page.evaluate(() => [...document.querySelectorAll('#plsongs .srow')].map((row) => {
  const chip = row.querySelector('.srcchip'); const b = chip.getBoundingClientRect();
  const y = b.top + b.height / 2; let reach = 0;
  for (let x = Math.floor(b.left) - 30; x <= Math.ceil(b.right) + 30; x++) { const h = document.elementFromPoint(x, y); if (h && (h === chip || chip.contains(h))) reach++; }
  const at = (el) => { const r = el.getBoundingClientRect(); const h = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); return !!h && (h === el || el.contains(h)); };
  const more = row.querySelector('.icon-btn[title="More"]');
  const title = row.querySelector('.name .lnk, .name a, .name');
  const tb = title.getBoundingClientRect();
  const titleEnd = document.elementFromPoint(Math.min(tb.right, b.left - 2) - 2, tb.top + tb.height / 2);
  return { w: b.width, h: b.height, left: b.left, right: b.right, reach, wait: chip.classList.contains('wait'), moreOwn: !!more && at(more), titleOwn: !!titleEnd && (titleEnd === title || title.contains(titleEnd)), titleRight: tb.right };
}));

async function phone(browser, width, android) {
  const at = `${width}${android ? ' Android' : ''}`;
  const { context, page, state } = await fx.create(browser, width, { mobile: true, fixtures, tracks, collections: [summary] });
  try {
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
    // The services answer only once the placeholder is measured (at most 10 s), so the rows first
    // show it (the page waits 1.2 s for them) however slow the first load is.
    let release; const held = new Promise((r) => { release = r; });
    await context.route('**/v1/playlists/c1/play-from', async (r) => { await Promise.race([held, new Promise((res) => setTimeout(res, 10000))]); await r.fulfill({ contentType: 'application/json', body: JSON.stringify(playFrom) }).catch(() => {}); });
    await fx.go(page, '/playlist/c1');
    await page.waitForSelector('#plsongs .srcchip.wait');
    const waiting = await measure(page);
    release();
    await page.waitForSelector('#plsongs [data-psrc]');
    await page.waitForTimeout(150);
    const chips = await measure(page);
    await page.screenshot({ path: path.join(SHOTS, `playlist-${width}${android ? '-android' : ''}.png`) });
    check(chips.length === 3 && waiting.every((c) => c.wait), `${at}: three rows, each first with the "finding" placeholder (${waiting.length})`);
    chips.forEach((c, k) => {
      check(c.w >= 48 && c.h >= 40, `${at} ${KINDS[k]}: the chip is 48 px wide, as wide as ⋯ (${c.w.toFixed(1)}×${c.h.toFixed(1)})`);
      check(c.reach >= 44, `${at} ${KINDS[k]}: 44 px across its middle reach the chip (${c.reach})`);
      check(c.moreOwn, `${at} ${KINDS[k]}: ⋯ beside it keeps its own middle`);
      check(Math.abs(c.w - waiting[k].w) < 0.5 && Math.abs(c.left - waiting[k].left) < 0.5, `${at} ${KINDS[k]}: the placeholder was as wide, in the same place (${waiting[k].w.toFixed(1)} → ${c.w.toFixed(1)})`);
    });
    check(chips[0].titleOwn, `${at}: the long title keeps its taps up to the chip (title ends ${chips[0].titleRight.toFixed(0)}, chip starts ${chips[0].left.toFixed(0)})`);
    // A touch 3 px inside either edge of the chip opens its menu, not ⋯'s or the song.
    for (const [edge, dx] of [['left', 3], ['right', -3]]) {
      const c = chips[0];
      const box = await page.locator('#plsongs [data-psrc="0"]').boundingBox();
      await page.touchscreen.tap(edge === 'left' ? c.left + dx : c.right + dx, box.y + box.height / 2);
      const menu = await page.waitForFunction(() => [...document.querySelectorAll('.pop-item')].some((b) => /Auto \(follow your Settings\)/.test(b.textContent)), null, { timeout: 2000 }).then(() => true, () => false);
      check(menu, `${at}: a touch near the chip's ${edge} edge opens "Play this song from"`);
      await page.keyboard.press('Escape');
      await page.waitForTimeout(150);
    }
    const over = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    check(over <= 0, `${at}: nothing runs past the screen (${over})`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const w of [360, 412]) await phone(browser, w, false);
    await phone(browser, 360, true);
    // The desktop layout keeps its chip with the service's name.
    const { page, context } = await fx.create(browser, 1280, { fixtures, tracks, collections: [summary] });
    try {
      await fx.go(page, '/playlist/c1');
      await page.waitForSelector('#plsongs [data-psrc]');
      const d = await page.locator('#plsongs [data-psrc="0"]').evaluate((b) => ({ w: b.getBoundingClientRect().width, text: b.querySelector('span:not(.src-badge)').getClientRects().length ? b.textContent.trim() : '(no name)', min: getComputedStyle(b).minWidth }));
      check(d.text === 'Library' && d.min === 'auto', `1280 desktop: the chip keeps its name and own width (${d.text}, ${d.w.toFixed(0)} px, min-width ${d.min})`);
      await page.screenshot({ path: path.join(SHOTS, 'playlist-1280.png') });
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
