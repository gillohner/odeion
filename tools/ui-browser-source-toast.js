// A streamed song played in this browser says what happened, as a library song does: its page's
// ⋯ › Play next says "Playing next" (before: "Added to queue"), ⋯ › Add to queue "Added to queue",
// and a song not on your services, whose 30 s preview is picked with nothing queued, starts playing
// with no "Added to queue" toast (before, it said it was only queued while it already played).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-browser-source-toast.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'browser-source-toast-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };

// An album of one TIDAL song (not in the library): its page's ⋯ plays it from TIDAL
const tidal = { service: 'tidal', label: 'TIDAL' };
const albumPage = { kind: 'album', id: 'tidal-9', uri: null, title: 'Elsewhere', artist: { name: 'Sefa', id: null }, album: null, year: 2020, image: null, source: 'tidal', source_id: '9', target: { source: 'tidal', id: '9' }, plays_from: tidal,
  songs: [{ no: 1, title: 'Far Song', artist: 'Sefa', duration_ms: 200000, uri: 'tidal:track:55', track_id: null, id: 'tidal-55', page: null }], total_ms: 200000, albums: [], versions: [], also_on: [], about: [], open_in: [] };
const ELSE = 'https://sefa.bandcamp.com/track/elsewhere';
const DEEZER = 'Deezer plays a 30-second preview of each song without a Deezer partner account';
const fixtures = {
  '/item/album/tidal-9': { body: albumPage },
  '/item/album/tidal-9/extras': { body: { collections: [], related: [], concerts: [], similar: [] } },
  '/favourites': { body: { uris: [ELSE], tracks: [], albums: [], artists: [], other: [ELSE] } },
  '/discover/describe': { body: [{ uri: ELSE, page: ELSE, title: 'Elsewhere', artist: 'Sefa', kind: 'track', site: 'bandcamp', also_on: [], connect: [] }] },
  '/discover/play': { body: { plays_from: null, items: [], connect: [], preview: { service: 'deezer', id: 'track:123' } } },
};
// Every toast said, in order
const logToasts = (page) => page.addInitScript(() => {
  window.__toasts = [];
  document.addEventListener('DOMContentLoaded', () => {
    const t = document.getElementById('toast'); if (!t) return;
    let shown = false; new MutationObserver(() => { const on = t.classList.contains('show'); const s = (t.firstChild && t.firstChild.nodeType === 3 ? t.firstChild.data : t.textContent).trim(); if (s && on && (!shown || s !== window.__toasts[window.__toasts.length - 1])) window.__toasts.push(s); shown = on && !!s; }).observe(t, { childList: true, characterData: true, subtree: true, attributes: true, attributeFilter: ['class'] });
  });
});
// (a toast's text and its show class are two changes: one toast is one entry; the same words said again, after it hid, are two)
const said = (page) => page.evaluate(() => window.__toasts);

async function menu(browser, width, label, want) {
  const tag = `${width} ⋯ › ${label}`;
  const { context, page, state } = await fx.create(browser, width, { fixtures, zone: 'browser' });
  await logToasts(page);
  try {
    await fx.go(page, '/album/tidal-9');
    const more = page.locator('#imore'); await more.waitFor();
    await page.waitForTimeout(300);
    await more.click();
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: label }).first().click();
    await page.waitForFunction(() => window.__toasts.length > 0, null, { timeout: 3000 }).catch(() => {});
    await page.waitForTimeout(200);
    const got = await said(page);
    check(got.join(' | ') === want, `${tag}: the toast says "${want}" (${JSON.stringify(got)})`);
    check(!state.hits.some((h) => /^\/zones\//.test(h.v) && h.m !== 'GET'), `${tag}: no zone is sent anything`);
    await page.screenshot({ path: path.join(SHOTS, `browser-source-toast-${label.replace(/\W+/g, '-').toLowerCase()}-${width}.png`) });
    check(state.errors.length === 0, `${tag}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
}

async function preview(browser, width) {
  const tag = `${width} preview`;
  const { context, page, state } = await fx.create(browser, width, { fixtures, zone: 'browser' });
  await logToasts(page);
  try {
    await fx.go(page, '/favourites');
    const play = page.locator('#favother .list-row [data-dplay]').first();
    await play.waitFor();
    await page.waitForTimeout(300);
    await play.click();
    const pop = page.locator('.popover.menu:not([hidden])'); await pop.waitFor();
    await pop.locator('.pop-item', { hasText: '30 s preview' }).click();
    await page.waitForFunction(() => /\/sources\/deezer\/stream\/123/.test((document.querySelector('audio') || {}).getAttribute?.('src') || ''), null, { timeout: 3000 }).catch(() => {});
    const src = await page.evaluate(() => document.querySelector('audio') && document.querySelector('audio').getAttribute('src'));
    check(/\/sources\/deezer\/stream\/123/.test(src || ''), `${tag}: the preview plays in this browser (${src})`);
    await page.waitForTimeout(500);
    const got = await said(page);
    // (Deezer's one-time note is what it says: before, "Added to queue" took its place at once)
    check(got.join(' | ') === DEEZER, `${tag}: only the Deezer preview note shows, no "Added to queue" (${JSON.stringify(got)})`);
    await page.screenshot({ path: path.join(SHOTS, `browser-source-toast-preview-${width}.png`) });
    check(state.errors.length === 0, `${tag}: no page errors (${state.errors.join(' | ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of (process.env.UI_WIDTHS ? process.env.UI_WIDTHS.split(',').map(Number) : [1280, 1024, 768])) {
      for (const [label, want] of [['Play next', 'Playing next'], ['Add to queue', 'Added to queue']]) {
        try { await menu(browser, width, label, want); } catch (e) { fail++; console.log(`FAIL ${width} ${label}: ${e.message}`); }
      }
      try { await preview(browser, width); } catch (e) { fail++; console.log(`FAIL ${width} preview: ${e.message}`); }
    }
  } finally { await browser.close(); }
  console.log(`${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})();
