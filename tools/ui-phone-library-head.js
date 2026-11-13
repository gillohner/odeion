// On a phone the Songs and Albums heads keep the filter, Play all and Add music on one row: the filter
// takes the free width and Add music is its + icon (named "Add music"), so the list starts a row higher.
// Desktop keeps the labelled button. Playlists' Import file button wraps at a large font size instead
// of running off a 320 px screen.
// Real candidate assets, isolated responses: node tools/ui-phone-library-head.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-library-head-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const many = (n) => Array.from({ length: n }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
const albums = Array.from({ length: 20 }, (_, i) => ({ id: i + 1, title: `Album ${i + 1}`, artists: fx.artists, year: 2000 + i, track_count: 3 }));
// Android's font size as the app's WebView applies it (text zoom)
const ZOOM_ARGS = ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'];
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });

// The page head's filter and buttons, and where the list under it starts
const head = (page, list) => page.evaluate((list) => {
  const h = document.querySelector('#main .page-head');
  const box = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { top: b.top, bottom: b.bottom, left: b.left, right: b.right, w: b.width, h: b.height }; };
  const add = h.querySelector('[data-addmusic]');
  const label = add && add.querySelector('.btn-label');
  return {
    filter: box(h.querySelector('#filter')),
    all: box(h.querySelector('#shuffle')),
    add: box(add),
    addName: add && (add.getAttribute('aria-label') || add.textContent.trim()),
    labelShown: label ? label.getClientRects().length > 0 : !!add && add.textContent.trim().length > 0,
    list: box(document.querySelector(list)),
    fits: document.documentElement.scrollWidth <= innerWidth,
    iw: innerWidth,
  };
}, list);
const row = (els) => els.filter(Boolean).every((b) => Math.abs((b.top + b.bottom) / 2 - (els[0].top + els[0].bottom) / 2) < 4);
const inside = (els, iw) => els.filter(Boolean).every((b) => b.left >= 0 && b.right <= iw);
const fmt = (o) => ['filter', 'all', 'add'].filter((k) => o[k]).map((k) => `${k} ${o[k].left.toFixed(0)},${o[k].top.toFixed(0)} ${o[k].w.toFixed(0)}×${o[k].h.toFixed(0)}`).join(', ');

async function phone(browser, width, android, k = 1) {
  const at = `${android ? 'android' : 'browser'}-${width}${k !== 1 ? ` at ${Math.round(k * 100)} % text zoom` : ''}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true, tracks: many(30), fixtures: { '/library/albums': { body: albums } } });
  try {
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    // Songs: filter, Play all and Add music on one row, the list straight under it
    await fx.go(page, '/tracks'); await page.waitForSelector('#tracklist .srow, #tracklist > *'); await page.waitForSelector('#filter');
    if (k !== 1) await textZoom(page, k);
    await page.waitForTimeout(300);
    const so = await head(page, '#tracklist');
    check(so.filter && so.all && so.add && row([so.filter, so.all, so.add]), `${at} Songs: filter, Play all and Add music share one row (${fmt(so)})`);
    check(so.add && so.add.w >= 44 && so.add.h >= 44 && Math.abs(so.add.w - so.add.h) < 1, `${at} Songs: Add music is a 44 px square icon (${so.add && `${so.add.w.toFixed(0)}×${so.add.h.toFixed(0)}`})`);
    check(so.addName === 'Add music' && !so.labelShown, `${at} Songs: the icon is named "${so.addName}", its label hidden`);
    check(so.filter && so.filter.w >= 120, `${at} Songs: the filter keeps room to type (${so.filter && so.filter.w.toFixed(0)} px)`);
    check(so.list && so.add && so.list.top - so.add.bottom < 40, `${at} Songs: the list starts right under the row (${so.list && (so.list.top - so.add.bottom).toFixed(0)} px gap)`);
    check(so.fits && inside([so.filter, so.all, so.add], so.iw), `${at} Songs: no sideways overflow, everything on screen`);
    await page.screenshot({ path: path.join(SHOTS, `songs-${at.replace(/ /g, '-')}.png`) });
    // One tap on the icon opens Add music
    await page.locator('#main .page-head [data-addmusic]').tap();
    await page.waitForSelector('dialog[open] h2', { timeout: 3000 }).catch(() => {});
    check(await page.locator('dialog[open] h2').first().textContent().catch(() => '') === 'Add music', `${at} Songs: one tap on + opens Add music`);
    await page.keyboard.press('Escape');

    // Albums: the filter and Add music on one row
    await fx.go(page, '/albums'); await page.waitForSelector('#albumgrid > *'); await page.waitForSelector('#filter');
    if (k !== 1) await textZoom(page, k);
    await page.waitForTimeout(300);
    const al = await head(page, '#albumgrid');
    check(al.filter && al.add && row([al.filter, al.add]) && al.add.w >= 44 && al.add.h >= 44, `${at} Albums: filter and Add music share one row (${fmt(al)})`);
    check(al.fits && inside([al.filter, al.add], al.iw), `${at} Albums: no sideways overflow, everything on screen`);
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

// Playlists' Import file button at 320 px with Android's 130 % font size: inside the screen, still a
// 44 px tap, its label whole (on main it was 324 px wide and its right end cut off)
async function importButton(browser, width, k) {
  const at = `android-${width} at ${Math.round(k * 100)} % text zoom`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true });
  try {
    await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    await fx.go(page, '/playlists'); await page.waitForSelector('#plimport', { state: 'attached' });
    await textZoom(page, k); await page.waitForTimeout(300);
    const b = await page.evaluate(() => {
      const el = document.querySelector('#main label.file-btn:has(#plimport)'); el.scrollIntoView({ block: 'center', behavior: 'instant' });
      const r = el.getBoundingClientRect(); const cs = getComputedStyle(document.querySelector('#main'));
      return { left: r.left, right: r.right, h: r.height, w: r.width, edge: innerWidth - parseFloat(cs.paddingRight), whole: el.scrollWidth <= el.clientWidth + 1, fits: document.documentElement.scrollWidth <= innerWidth };
    });
    check(b.left >= 0 && b.right <= b.edge + 0.5 && b.whole && b.fits, `${at}: Import stays inside the screen with its whole label (${b.left.toFixed(0)}..${b.right.toFixed(0)}, edge ${b.edge.toFixed(0)})`);
    check(b.h >= 44, `${at}: Import is still a 44 px tap (${b.w.toFixed(0)}×${b.h.toFixed(0)})`);
    await page.screenshot({ path: path.join(SHOTS, `import-${at.replace(/ /g, '-')}.png`) });
    check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  const zoomed = await chromium.launch({ args: ZOOM_ARGS });
  try {
    for (const width of [320, 360, 412]) for (const android of [false, true]) await phone(browser, width, android);
    await phone(zoomed, 320, true, 1.3);
    await importButton(zoomed, 320, 1.3);
    await importButton(zoomed, 360, 1.3);

    // A listener: no Add music, the filter and Play all still share a row
    {
      const { page, context } = await fx.create(browser, 360, { mobile: true, role: 'listener', tracks: many(30) });
      try {
        await fx.go(page, '/tracks'); await page.waitForSelector('#filter'); await page.waitForTimeout(300);
        const li = await head(page, '#tracklist');
        check(!li.add && li.filter && li.all && row([li.filter, li.all]) && li.fits, `listener-360 Songs: no Add music, filter and Play all on one row (${fmt(li)})`);
      } finally { await context.close(); }
    }
    // Artists' filter has no button beside it: it stays beside the heading at 360/412 px, 200 px wide
    for (const width of [360, 412]) {
      const artists = Array.from({ length: 20 }, (_, i) => ({ id: i + 1, name: `Artist ${i + 1}`, album_count: 1, track_count: 3 }));
      const { page, context } = await fx.create(browser, width, { mobile: true, fixtures: { '/library/artists': { body: artists } } });
      try {
        await fx.go(page, '/artists'); await page.waitForSelector('#filter'); await page.waitForTimeout(300);
        const a = await page.evaluate(() => { const f = document.querySelector('#filter').getBoundingClientRect(); const h = document.querySelector('#main .page-head h2').getBoundingClientRect(); return { w: f.width, beside: f.top < h.bottom, fits: document.documentElement.scrollWidth <= innerWidth }; });
        check(a.beside && Math.abs(a.w - 200) < 1 && a.fits, `browser-${width} Artists: the filter stays beside the heading, 200 px wide (${a.w.toFixed(0)} px, beside: ${a.beside})`);
      } finally { await context.close(); }
    }
    // A wide phone layout (599 px): Songs' filter, Play all and + stay on the heading's row
    {
      const { page, context } = await fx.create(browser, 599, { mobile: true, tracks: many(30) });
      try {
        await fx.go(page, '/tracks'); await page.waitForSelector('#filter'); await page.waitForTimeout(300);
        const w = await head(page, '#tracklist');
        const h2 = await page.locator('#main .page-head h2').boundingBox();
        check(w.filter && w.add && row([w.filter, w.all, w.add]) && w.filter.top < h2.y + h2.height && w.filter.w <= 360 && w.fits, `browser-599 Songs: filter, Play all and + stay beside the heading (${fmt(w)})`);
      } finally { await context.close(); }
    }
    // A short list (no filter): Play all and + sit beside the heading
    {
      const { page, context } = await fx.create(browser, 360, { mobile: true, tracks: many(5) });
      try {
        await fx.go(page, '/tracks'); await page.waitForSelector('#shuffle'); await page.waitForTimeout(300);
        const sh = await head(page, '#tracklist');
        const h2 = await page.locator('#main .page-head h2').boundingBox();
        check(!sh.filter && sh.all && sh.add && row([sh.all, sh.add]) && sh.add.w >= 44 && sh.all.top < h2.y + h2.height && sh.fits, `browser-360 Songs with 5 songs: Play all and + beside the heading (${fmt(sh)})`);
      } finally { await context.close(); }
    }
    // Desktop keeps the labelled Add music button
    {
      const { page, context } = await fx.create(browser, 1024, { tracks: many(30) });
      try {
        await fx.go(page, '/tracks'); await page.waitForSelector('#main .page-head [data-addmusic]'); await page.waitForTimeout(300);
        const d = await head(page, '#tracklist');
        const text = await page.locator('#main .page-head [data-addmusic]').innerText();
        check(d.labelShown && text.trim() === 'Add music' && d.filter && d.filter.w < 230, `desktop-1024 Songs: Add music keeps its label and the filter its size (filter ${d.filter && d.filter.w.toFixed(0)} px, "${text.trim()}")`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); await zoomed.close(); }
  console.log(`\n${passed} passed; ${failed} failed`);
  if (failed) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
