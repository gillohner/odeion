// On a phone, Home's Trending heads keep the title and See all on one line and put Today / This week /
// This month on one row of their own under it. Beside the title, the title and the chips both broke
// onto two lines (a 100 px head at 320/360 px; at 412 px See all pushed it over too). Desktop keeps the
// chips beside the title.
// Real candidate assets, isolated responses: node tools/ui-phone-home-trending.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-home-trending-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const SECTIONS = ['trending-songs', 'trending-albums', 'trending-artists'];
// Eight of each, so See all shows, as it does with real trending data
const items = (kind) => Array.from({ length: 8 }, (_, i) => ({ kind, uri: `https://musicbrainz.org/${kind === 'track' ? 'recording' : kind === 'album' ? 'release-group' : 'artist'}/00000000-0000-4000-8000-00000000000${i}`, title: `${kind} ${i + 1}`, artist: 'Sefa' }));
const trending = (n) => ({
  '/home/trending-songs': { body: { section: 'trending-songs', items: items('track').slice(0, n), tags: [] } },
  '/home/trending-albums': { body: { section: 'trending-albums', items: items('album').slice(0, n), tags: [] } },
  '/home/trending-artists': { body: { section: 'trending-artists', items: items('artist').slice(0, n), tags: [] } },
});
const fixtures = trending(8);
// Android's font size as the app's WebView applies it (text zoom)
const ZOOM_ARGS = ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'];
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });

const heads = (page) => page.evaluate((ids) => ids.map((id) => {
  const h = document.querySelector(`#h-${id} .section-head`);
  if (!h) return { id, missing: true };
  h.scrollIntoView({ block: 'center', behavior: 'instant' });
  const box = (el) => { const b = el.getBoundingClientRect(); return { top: b.top, bottom: b.bottom, left: b.left, right: b.right, w: b.width, h: b.height }; };
  const t = h.querySelector('h3'); const cs = getComputedStyle(t);
  const lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.2;
  const see = h.querySelector('[data-seeall]');
  return {
    id, head: box(h), title: box(t), titleLines: Math.round(t.getBoundingClientRect().height / lh),
    chips: [...h.querySelectorAll('[data-win]')].map(box), see: see && !see.hidden ? box(see) : null,
    // A tap in the middle of each chip lands on that chip (See all's 44 px layer stays off the row)
    ownTaps: [...h.querySelectorAll('[data-win]')].every((c) => { const b = c.getBoundingClientRect(); return c.contains(document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2)); }),
    iw: innerWidth, fits: document.documentElement.scrollWidth <= innerWidth,
  };
}), SECTIONS);

const drawn = (page, seeAll) => page.waitForFunction(([ids, seeAll]) => ids.every((id) => { const s = document.querySelector(`#h-${id} [data-seeall]`); return s && document.querySelector(`#h-${id} .hbody .list, #h-${id} .hbody [data-hgrid]`) && s.hidden === !seeAll; }), [SECTIONS, seeAll]);

async function phone(browser, width, android, k = 1, n = 8) {
  const seeAll = n > 6;
  const at = `${android ? 'android' : 'browser'}-${width}${k !== 1 ? ` at ${Math.round(k * 100)} % text zoom` : ''}${seeAll ? '' : ' without See all'}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures: trending(n) });
  try {
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    await fx.go(page, '/');
    await drawn(page, seeAll);
    if (k !== 1) { await textZoom(page, k); await page.waitForTimeout(100); }
    for (const o of await heads(page)) {
      if (o.missing) { check(false, `${at} ${o.id}: the section is on Home`); continue; }
      const [c0] = o.chips; const last = o.chips[o.chips.length - 1];
      check(o.titleLines === 1, `${at} ${o.id}: the title is on one line (${o.titleLines} lines, ${o.title.h.toFixed(0)} px)`);
      check(o.chips.length === 3 && o.chips.every((c) => Math.abs(c.top - c0.top) < 1), `${at} ${o.id}: Today, This week and This month share one row (tops ${o.chips.map((c) => c.top.toFixed(0)).join('/')})`);
      check(c0 && c0.top >= o.title.bottom - 1 && Math.abs(c0.left - o.title.left) < 1, `${at} ${o.id}: the chips sit under the title, from its left edge (chip ${c0 && `${c0.left.toFixed(0)},${c0.top.toFixed(0)}`}, title ${o.title.left.toFixed(0)}..${o.title.bottom.toFixed(0)})`);
      if (seeAll) check(o.see && o.see.top < o.title.bottom && o.see.bottom > o.title.top && o.see.right <= o.iw && o.see.left > o.title.right, `${at} ${o.id}: See all is on the title's line, at its end (${o.see && `${o.see.left.toFixed(0)}..${o.see.right.toFixed(0)}, ${o.see.top.toFixed(0)}`})`);
      check(o.ownTaps, `${at} ${o.id}: a tap in the middle of each chip lands on that chip`);
      check(o.chips.every((c) => c.h >= 40 && c.left >= 0 && c.right <= o.iw), `${at} ${o.id}: each chip is at least 40 px tall and on screen (${o.chips.map((c) => `${c.w.toFixed(0)}×${c.h.toFixed(0)}`).join(', ')})`);
      check(last && o.head.bottom >= last.bottom - 0.5, `${at} ${o.id}: the head holds its chips (${o.head.h.toFixed(0)} px tall)`);
      check(o.fits, `${at}: no sideways overflow`);
    }
    await page.locator('#h-trending-songs').scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(SHOTS, `home-trending-${at.replace(/ /g, '-')}.png`) });
    // One tap on Today picks it, and the heads keep their shape after the redraw
    const today = page.waitForRequest((r) => /\/v1\/home\/trending-songs\?.*window=24h/.test(r.url()), { timeout: 3000 }).catch(() => null);
    await page.locator('#h-trending-songs [data-win="24h"]').tap();
    check(!!(await today), `${at}: one tap on Today asks for today's trending songs`);
    await page.waitForFunction(() => document.querySelector('#h-trending-songs [data-win="24h"]')?.classList.contains('on'));
    await drawn(page, seeAll);
    const after = (await heads(page))[0];
    check(after.titleLines === 1 && after.chips.every((c) => Math.abs(c.top - after.chips[0].top) < 1 && c.top >= after.title.bottom - 1), `${at}: after Today the head keeps its title line and chip row`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

// Desktop and tablet keep the chips beside the title
async function wide(browser, width) {
  const { page, context, state } = await fx.create(browser, width, { fixtures });
  try {
    await fx.go(page, '/');
    await drawn(page, true);
    for (const o of await heads(page)) {
      const mid = (b) => (b.top + b.bottom) / 2;
      check(o.chips.every((c) => c.left > o.title.right && Math.abs(mid(c) - mid(o.title)) < 8), `${width} ${o.id}: the chips stay beside the title (title ${o.title.right.toFixed(0)}, first chip ${o.chips[0].left.toFixed(0)})`);
    }
    check(!state.errors.length, `${width}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch({ args: ZOOM_ARGS });
  try {
    for (const width of [320, 360, 412]) {
      for (const android of [false, true]) await phone(browser, width, android);
      await phone(browser, width, true, 1.3);
    }
    // Fewer than seven of each: no See all (main broke the title at 320 and 360 px here too)
    for (const width of [320, 360, 412]) await phone(browser, width, true, 1, 4);
    for (const width of [768, 1280]) await wide(browser, width);
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
