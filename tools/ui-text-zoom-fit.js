// Android's larger font size (the WebView's text zoom) on a phone: Songs' head and Collections'
// Smart collections head stay inside the screen, and the filter's placeholder shows whole.
// - Songs at 360 px / 200 %: the row of filter, Play all and + kept its widest size (360 px) inside a
//   332 px head, so + ran past the screen and the page scrolled sideways.
// - Collections at 360-412 px / 150-200 %: "New smart collection" stayed beside "Smart collections"
//   and ran past the screen (the page 470 px wide); it now goes under the title when there is no room.
//   (Only that head: Downloads' heads are left as they are.)
// - The filter on a phone says "Filter" (the heading above names the list); its name stays
//   "Filter songs" / "Filter albums" / "Filter artists", and a desktop's placeholder is unchanged.
// Real candidate assets, isolated responses: node tools/ui-text-zoom-fit.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'text-zoom-fit-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const many = (n) => Array.from({ length: n }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
const albums = Array.from({ length: 20 }, (_, i) => ({ id: i + 1, title: `Album ${i + 1}`, artists: fx.artists, year: 2000 + i, track_count: 3 }));
const artists = Array.from({ length: 20 }, (_, i) => ({ id: i + 1, name: `Artist ${i + 1}`, album_count: 1, track_count: 3 }));
// Android's font size as the app's WebView applies it (text zoom; as ui-phone-library-head.js)
const ZOOM_ARGS = ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'];
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });
const pct = (k) => `${Math.round(k * 100)} %`;

// Nothing slides sideways: neither the page nor #main (which scrolls on its own) is wider than the
// screen, and the named controls lie inside #main's padding
const fit = (page, sels) => page.evaluate((sels) => {
  const m = document.querySelector('#main'); const cs = getComputedStyle(m);
  const left = parseFloat(cs.paddingLeft) - 0.5; const right = innerWidth - parseFloat(cs.paddingRight) + 0.5;
  const out = sels.map((s) => { const e = document.querySelector(s); if (!e) return `${s} missing`; const b = e.getBoundingClientRect(); return b.left >= left && b.right <= right ? null : `${s} ${b.left.toFixed(0)}..${b.right.toFixed(0)}`; }).filter(Boolean);
  return { ok: document.documentElement.scrollWidth <= innerWidth && m.scrollWidth <= m.clientWidth && !out.length, why: `page ${document.documentElement.scrollWidth}, main ${m.scrollWidth}/${m.clientWidth}${out.length ? `, out: ${out.join(', ')}` : ''}` };
}, sels);
// The filter's placeholder, its name, and whether the placeholder's words fit in the box
const filterText = (page) => page.evaluate(() => {
  const f = document.querySelector('#filter'); const cs = getComputedStyle(f);
  const c = document.createElement('canvas').getContext('2d'); c.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
  const need = c.measureText(f.placeholder).width; const room = f.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  return { ph: f.placeholder, name: f.getAttribute('aria-label'), need, room, whole: need <= room };
});

async function songs(browser, width, k) {
  const at = `android-${width} at ${pct(k)}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true, tracks: many(30) });
  try {
    await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    await fx.go(page, '/tracks'); await page.waitForSelector('#tracklist > *'); await page.waitForSelector('#filter');
    if (k !== 1) await textZoom(page, k);
    await page.waitForTimeout(300);
    const fs = await page.evaluate(() => parseFloat(getComputedStyle(document.querySelector('#filter')).fontSize));
    check(Math.abs(fs - 14 * k) < 0.6, `${at} Songs: the text zoom applies (the filter's text ${fs} px)`);
    const f = await fit(page, ['#filter', '#shuffle', '#main .page-head [data-addmusic]']);
    check(f.ok, `${at} Songs: filter, Play all and + inside the screen, nothing scrolls sideways (${f.why})`);
    const t = await filterText(page);
    check(t.ph === 'Filter' && t.name === 'Filter songs', `${at} Songs: the filter shows "${t.ph}", named "${t.name}"`);
    check(t.whole, `${at} Songs: "${t.ph}" fits in the filter (${t.need.toFixed(0)} of ${t.room.toFixed(0)} px)`);
    await page.screenshot({ path: path.join(SHOTS, `songs-${at.replace(' at ', '-').replace(' %', 'pct')}.png`) });
    check(!state.errors.length, `${at} Songs: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

async function collections(browser, width, k) {
  const at = `android-${width} at ${pct(k)}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true });
  try {
    await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    await fx.go(page, '/playlists'); await page.waitForSelector('#smartnew');
    if (k !== 1) await textZoom(page, k);
    await page.waitForTimeout(300);
    await page.evaluate(() => document.querySelector('#smartnew').scrollIntoView({ block: 'center', behavior: 'instant' }));
    const f = await fit(page, ['#smartnew']);
    check(f.ok, `${at} Collections: New smart collection inside the screen, nothing scrolls sideways (${f.why})`);
    const g = await page.evaluate(() => {
      const h = document.querySelector('#smartnew').closest('.section-head').querySelector('h3').getBoundingClientRect(); const b = document.querySelector('#smartnew').getBoundingClientRect();
      return { beside: b.top < h.bottom, titleLines: Math.round(h.height / parseFloat(getComputedStyle(document.querySelector('#smartnew').closest('.section-head').querySelector('h3')).lineHeight)), h: b.height };
    });
    // With room (100 %) the button stays on the title's row; without, it goes under a one-line title
    // With room (100 %, or 412 px at 130 %) the button stays on the title's row; without, it goes under
    // the title, which keeps one line
    if (k === 1 || (width >= 412 && k <= 1.3)) check(g.beside && g.titleLines === 1, `${at} Collections: New smart collection stays beside a one-line Smart collections`);
    else check(!g.beside && g.titleLines === 1, `${at} Collections: New smart collection goes under a one-line Smart collections (beside: ${g.beside}, ${g.titleLines} lines)`);
    check(g.h >= 40, `${at} Collections: New smart collection is still a 40 px tap (${g.h.toFixed(0)} px)`);
    await page.screenshot({ path: path.join(SHOTS, `collections-${at.replace(' at ', '-').replace(' %', 'pct')}.png`) });
    check(!state.errors.length, `${at} Collections: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch({ args: ZOOM_ARGS });
  try {
    for (const width of [360, 412]) for (const k of [1, 1.3, 1.5, 2]) { await songs(browser, width, k); await collections(browser, width, k); }

    // Albums and Artists say "Filter" on a phone too, named for their list
    for (const [route, name, opts] of [['/albums', 'Filter albums', { fixtures: { '/library/albums': { body: albums } } }], ['/artists', 'Filter artists', { fixtures: { '/library/artists': { body: artists } } }]]) {
      const { page, context } = await fx.create(browser, 360, { mobile: true, ...opts });
      try {
        await fx.go(page, route); await page.waitForSelector('#filter'); await page.waitForTimeout(200);
        const t = await filterText(page);
        check(t.ph === 'Filter' && t.name === name, `browser-360 ${route}: the filter shows "${t.ph}", named "${t.name}"`);
      } finally { await context.close(); }
    }
    // A desktop keeps the list's name in the box
    {
      const { page, context } = await fx.create(browser, 1280, { tracks: many(30) });
      try {
        await fx.go(page, '/tracks'); await page.waitForSelector('#filter'); await page.waitForTimeout(200);
        const t = await filterText(page);
        check(t.ph === 'Filter songs' && t.name === 'Filter songs', `desktop-1280 Songs: the filter still shows "${t.ph}"`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
