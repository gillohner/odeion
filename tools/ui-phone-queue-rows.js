// On a phone a Queue row is two lines, as a Songs row is: the artist · album line stays on one line
// and ends in "…" when it is long (it wrapped to 3-4 lines, so a 360 × 900 screen showed 7 whole rows
// instead of 11), and the row's gaps are 8 px, not 14, so the title has about 20 px more room.
// The drag grip, Play from here and ⋯ stay 44 px taps along the row's middle and don't take each
// other's taps. Desktop keeps its rows (14 px gaps, the line may wrap).
// Real candidate assets, isolated responses: node tools/ui-phone-queue-rows.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-queue-rows-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Twelve songs; the even ones have a long artist and album (their line is wider than any phone row),
// the odd ones a short one.
const QUEUE = Array.from({ length: 12 }, (_, i) => ({
  uri: `file:///music/${i}.flac`, title: `Song number ${i + 1} with a longer title`, duration_ms: 180000 + i * 1000,
  artist: i % 2 ? 'Sefa' : 'Sefa, Dr. Peacock, Angerfist, Miss K8', album: i % 2 ? 'Journey' : 'Journey Through Time (Deluxe Edition)',
}));

const rows = (page) => page.evaluate(() => [...document.querySelectorAll('.list.queue .list-row')].map((row) => {
  const rb = row.getBoundingClientRect(); const y = rb.top + rb.height / 2;
  const sub = row.querySelector('.sub'); const sb = sub.getBoundingClientRect(); const cs = getComputedStyle(sub);
  // what a tap at each x along the row's middle line presses, as runs
  const who = (x) => { const e = document.elementFromPoint(x, y); const c = e && row.contains(e) && e.closest('[data-qgrip], [data-qi], [data-qmore]'); return !c ? '-' : c.dataset.qgrip != null ? 'grip' : c.dataset.qi != null ? 'play' : 'more'; };
  const seq = []; for (let x = Math.ceil(rb.left); x < rb.right; x++) { const w = who(x); if (!seq.length || seq[seq.length - 1][0] !== w) seq.push([w, 0]); seq[seq.length - 1][1]++; }
  return { h: rb.height, top: rb.top, bottom: rb.bottom, subLines: Math.round(sb.height / parseFloat(cs.lineHeight)), subCut: sub.scrollWidth > sub.clientWidth, ellipsis: cs.textOverflow, wrap: cs.whiteSpace,
    title: row.querySelector('.grow').getBoundingClientRect().width, gap: parseFloat(getComputedStyle(row).columnGap), seq };
}));

async function phone(browser, width, android) {
  const at = `${android ? 'android' : 'browser'} ${width}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true });
  await context.addInitScript((q) => { localStorage.setItem('pm.browserQueue', JSON.stringify(q)); }, QUEUE);
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  await fx.go(page, '/queue'); await page.waitForSelector('.list.queue .list-row'); await page.waitForTimeout(300);
  const r = await rows(page);
  const long = r.filter((_, i) => i % 2 === 0); const short = r.filter((_, i) => i % 2);
  check(r.length === 12 && r.every((x) => x.subLines === 1), `${at}: every row's artist · album is one line (${r.map((x) => x.subLines).join(' ')})`);
  check(long.every((x) => x.subCut && x.ellipsis === 'ellipsis'), `${at}: a long artist · album line ends in "…" (cut ${long.map((x) => x.subCut).join(' ')}, ${long[0].ellipsis})`);
  const hs = r.map((x) => x.h); const tall = Math.max(...hs);
  check(tall - Math.min(...hs) <= 1.5 && tall <= 52, `${at}: a long line's row is as tall as a short one's, at most 52 px (${[...new Set(hs.map((h) => h.toFixed(1)))].join(' / ')} px)`);
  check(r.every((x) => x.gap === 8), `${at}: the row's gaps are 8 px (${[...new Set(r.map((x) => x.gap))].join(' / ')})`);
  const room = width === 360 ? 150 : 200;
  check(r.every((x) => x.title >= room), `${at}: the title has ${r[0].title.toFixed(0)} px (at least ${room})`);
  // grip, play, -, more: each a 44 px run, in that order, and nothing else tappable in between
  for (const [n, x] of [[0, r[0]], [1, r[1]]]) {
    const taps = x.seq.filter(([w]) => w !== '-');
    const len = (w) => (taps.find(([v]) => v === w) || [w, 0])[1];
    check(taps.map(([w]) => w).join(',') === 'grip,play,more' && len('grip') >= 44 && len('play') >= 44 && len('more') >= 44,
      `${at}: row ${n + 1}'s grip, Play from here and ⋯ are each a 44 px tap in a row (${x.seq.map(([w, k]) => `${w} ${k}`).join(', ')})`);
  }
  // On a 900 px tall screen, more whole rows show above the mini player than with three-line rows
  const bar = await page.evaluate(() => { const p = document.querySelector('.player'); return p ? p.getBoundingClientRect().top : innerHeight; });
  const whole = r.filter((x) => x.bottom <= bar).length;
  check(whole >= 10, `${at}: ${whole} whole rows show above the player (at least 10)`);
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at}: no sideways scroll`);
  // A tap on a row's Play from here still plays that song; its ⋯ still opens its menu
  await page.locator('.list.queue .list-row').nth(2).locator('[data-qi]').tap(); await page.waitForTimeout(300);
  check(await page.evaluate(() => document.querySelectorAll('.list.queue .list-row')[2].classList.contains('cur')), `${at}: Play from here on row 3 plays it`);
  await page.locator('.list.queue .list-row').nth(1).locator('[data-qmore]').tap();
  check(await page.locator('.popover.menu:not([hidden]) .pop-item').first().isVisible().catch(() => false), `${at}: a row's ⋯ opens its menu`);
  await page.keyboard.press('Escape');
  await page.screenshot({ path: path.join(SHOTS, `queue-${android ? 'android' : 'browser'}-${width}.png`) });
  check(!state.errors.length, `${at}: no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
  await context.close();
}

async function desktop(browser, width) {
  const { page, context } = await fx.create(browser, width);
  await context.addInitScript((q) => { localStorage.setItem('pm.browserQueue', JSON.stringify(q)); }, QUEUE);
  await fx.go(page, '/queue'); await page.waitForSelector('.list.queue .list-row'); await page.waitForTimeout(300);
  const r = await rows(page);
  check(r.every((x) => x.gap === 14) && r.every((x) => x.ellipsis !== 'ellipsis' && x.wrap === 'normal'), `desktop ${width}: rows keep their 14 px gaps and the line may wrap (${r[0].gap} px, ${r[0].wrap}, ${r[0].ellipsis})`);
  await page.screenshot({ path: path.join(SHOTS, `queue-desktop-${width}.png`) });
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) await phone(browser, width, android);
    await desktop(browser, 1024);
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
