// A section head's small button (Collections' New smart collection, offline Downloads' Play beside
// Bookmarks and Shuffle beside All songs) has its words in its middle, level with its icon, and on
// the section title's baseline. On a phone or a touch screen it is 40 px tall; its words sat at its
// top, 5.5 px above its middle and its icon. Desktop keeps its compact size. Busy (a press that
// waits), its spinner takes the icon's place in the button's middle, not 3-4 px above it, and nothing
// moves. Also Home's Done (words only) and a service's Import as bookmarks.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-section-head-button.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || path.join(require('node:os').tmpdir(), 'section-head-button-shots'));
fs.mkdirSync(shots, { recursive: true });
const tracks = [1, 2].map((id) => ({ ...fx.track, id, title: `Song ${id}` }));
const offline = { offline: true, device_id: 'fixture-device', synced_at: null, bytes: 1, queued: 0, tracks, albums: [{ id: 7, title: 'Journey', track_ids: [1, 2], cover: false }], playlists: [], favourites: { track_ids: [2] } };
let ok = 0;
const failed = [];
const check = (cond, what) => { if (cond) ok++; else { failed.push(what); console.error('FAIL', what); } };
// The button's words (its text, not its icon), its icon and its section's title, in page px
const measure = (page, sel) => page.$eval(sel, (b) => {
  const h = b.closest('.section-head').querySelector(':scope > h3');
  const words = (el) => { const r = document.createRange(); const t = [...el.childNodes].filter((n) => n.nodeType === 3 && n.textContent.trim()); r.setStart(t[0], 0); r.setEnd(t.at(-1), t.at(-1).length); return r.getBoundingClientRect(); };
  // A line's baseline: its text box's foot less its font's descent
  const ctx = document.createElement('canvas').getContext('2d');
  const baseline = (el) => { const cs = getComputedStyle(el); ctx.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`; return words(el).bottom - ctx.measureText('Hg').fontBoundingBoxDescent; };
  const br = b.getBoundingClientRect(), wr = words(b), ic = b.querySelector('.ic')?.getBoundingClientRect();
  return { height: br.height, mid: (br.top + br.bottom) / 2, words: (wr.top + wr.bottom) / 2, icon: ic ? (ic.top + ic.bottom) / 2 : null, base: baseline(b), titleBase: baseline(h) };
});
// A screenshot's pixels, read by the page (no PNG library needed)
const pixels = (page, png) => page.evaluate(async (b64) => { const img = new Image(); img.src = 'data:image/png;base64,' + b64; await img.decode(); const c = document.createElement('canvas'); c.width = img.width; c.height = img.height; const g = c.getContext('2d'); g.drawImage(img, 0, 0); return { w: img.width, h: img.height, d: Array.from(g.getImageData(0, 0, c.width, c.height).data) }; }, png.toString('base64'));
// Busy as busy() makes it (.is-busy.is-spinning: the icon hidden, a spinner after the words); the spinner is
// a ::after, so it is found by its pixels: right of the words, 5 px in from the button's top and foot (off
// its round ends), the pixels unlike the strip's most common colour (the button's own). Its middle is its
// foot less half its width: its accent top arc doesn't show on a primary button (Done)
async function busy(page, at, sel, baseline) {
  await page.$eval(sel, (b) => b.scrollIntoView({ block: 'center' }));
  const before = await measure(page, sel);
  await page.$eval(sel, (b) => b.classList.add('is-busy', 'is-spinning'));
  await page.waitForTimeout(50);
  const m = await measure(page, sel);
  const box = await page.$eval(sel, (b) => { const r = b.getBoundingClientRect(); const t = [...b.childNodes].filter((n) => n.nodeType === 3 && n.textContent.trim()); const g = document.createRange(); g.setStart(t[0], 0); g.setEnd(t.at(-1), t.at(-1).length); return { x: r.left, y: r.top, w: r.width, h: r.height, words: g.getBoundingClientRect().right }; });
  const clip = { x: box.words + 1, y: box.y + 5, width: Math.min(24, box.x + box.w - box.words - 4), height: box.h - 10 };
  const dpr = await page.evaluate(() => devicePixelRatio);
  const { w, h, d } = await pixels(page, await page.screenshot({ clip, animations: 'disabled' }));
  const count = {};
  for (let i = 0; i < d.length; i += 4) { const k = `${d[i]},${d[i + 1]},${d[i + 2]}`; count[k] = (count[k] || 0) + 1; }
  const bg = Object.entries(count).sort((a, b) => b[1] - a[1])[0][0].split(',').map(Number);
  let foot = -1, left = w, right = -1;
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const i = (y * w + x) * 4; if (Math.abs(d[i] - bg[0]) + Math.abs(d[i + 1] - bg[1]) + Math.abs(d[i + 2] - bg[2]) > 60) { foot = y; left = Math.min(left, x); right = Math.max(right, x); } }
  await page.$eval(sel, (b) => b.classList.remove('is-busy', 'is-spinning'));
  const size = (right - left + 1) / dpr, mid = clip.y + (foot + 1) / dpr - size / 2;
  check(foot >= 0 && size >= 14 && size <= 18, `${at}: busy, a spinner shows after the words (${foot < 0 ? 'none' : size.toFixed(1) + ' px wide'})`);
  if (foot >= 0) check(Math.abs(mid - m.mid) <= 1.5, `${at}: busy, the spinner is in the button's middle (spinner ${mid.toFixed(1)}, middle ${m.mid.toFixed(1)})`);
  check(Math.abs(m.height - before.height) <= 0.5 && Math.abs(m.words - before.words) <= 0.5 && (!baseline || Math.abs(m.base - m.titleBase) <= 1), `${at}: busy, the button and its words don't move (height ${before.height} → ${m.height}, words ${before.words.toFixed(1)} → ${m.words.toFixed(1)}, ${(m.base - m.titleBase).toFixed(1)} px off the title's baseline)`);
}
async function buttons(page, tag, sels, tall, touch, baseline = true) {
  for (const sel of sels) {
    const m = await measure(page, sel);
    const iconed = await page.$eval(sel, (b) => !!b.querySelector('.ic'));
    const reach = await page.$eval(sel, (b) => { const r = b.getBoundingClientRect(), c = getComputedStyle(b, '::before'), z = c.content !== 'none' && getComputedStyle(b).position === 'relative'; return z ? r.height - parseFloat(c.top) - parseFloat(c.bottom) : r.height; });
    const at = `${tag} ${sel}`;
    check(Math.abs(m.words - m.mid) <= 1.5, `${at}: the words are in the button's middle (words ${m.words.toFixed(1)}, middle ${m.mid.toFixed(1)})`);
    if (iconed) check(m.icon != null && Math.abs(m.icon - m.words) <= 1.5, `${at}: the icon is level with the words (icon ${m.icon?.toFixed(1)}, words ${m.words.toFixed(1)})`);
    if (baseline) check(Math.abs(m.base - m.titleBase) <= 1, `${at}: the words are on the title's baseline (${m.base.toFixed(1)} vs ${m.titleBase.toFixed(1)})`);
    if (touch) check(reach >= 44, `${at}: a touch screen's tap reaches 44 px or more (${reach})`);
    if (tall) check(m.height >= 39.5 && m.height <= 41, `${at}: 40 px tall on a phone or a touch screen (${m.height})`);
    else check(m.height < 34, `${at}: compact on a desktop (${m.height})`);
    await busy(page, at, sel, baseline);
  }
}
(async () => {
  const browser = await chromium.launch();
  try {
    // [width, phone viewport (a touch screen), the Android app]; 360 with a mouse is the phone layout
    // without a touch screen's sizes, 768 a touch screen without the phone layout
    for (const [width, mobile, android] of [[360, true, false], [360, true, true], [360, false, false], [412, true, false], [768, true, false], [1280, false, false]]) {
      const tag = `${width}${android ? '-android' : ''}${mobile ? '' : width < 600 ? '-mouse' : ''}`;
      const tall = mobile || width < 600;
      {
        const { page, context, state } = await fx.create(browser, width, { mobile });
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
        try {
          await fx.go(page, '/playlists');
          await page.waitForSelector('#smartnew');
          if (mobile) check(await page.evaluate(() => matchMedia('(pointer: coarse)').matches), `${tag}: a touch screen`);
          await buttons(page, `${tag} Collections`, ['#smartnew'], tall, mobile);
          await page.screenshot({ path: path.join(shots, `${tag}-collections.png`) });
          await fx.cleanCopy(page);
          check(state.errors.length === 0, `${tag} Collections: no page errors (${state.errors})`);
        } finally { await context.close(); }
      }
      {
        // Home › Edit Home: the "Rows on Home" head's Done (words only); Spotify's Import as bookmarks
        const spotify = { id: 'spotify', name: 'Spotify', description: '', status: 'ready', enabled: true };
        const { page, context, state } = await fx.create(browser, width, { mobile, fixtures: { '/sources': { body: [spotify] }, '/sources/spotify/status': { body: { signed_in: true } }, '/sources/spotify/signin': { body: { ways: [], in_use: { way: 'app' }, details: { status: { signed_in: true } } } }, '/sources/spotify/collection': { body: { items: [] } }, '/sources/spotify/playlists': { body: [] }, '/sources/spotify/devices': { body: [] } } });
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
        try {
          await fx.go(page, '/');
          await page.locator('#hedit').click();
          await page.waitForSelector('#hdone');
          await buttons(page, `${tag} Home`, ['#hdone'], tall, mobile);
          await fx.go(page, '/sources/spotify');
          await page.waitForSelector('#spotfav');
          // At 360 this head is crowded (its title wraps beside the count and the button), so its words
          // aren't on the title's last line; the rest holds
          await buttons(page, `${tag} Spotify`, ['#spotfav'], tall, mobile, width > 360);
          await page.screenshot({ path: path.join(shots, `${tag}-spotify.png`) });
          await fx.cleanCopy(page);
          check(state.errors.length === 0, `${tag} Home and Spotify: no page errors (${state.errors})`);
        } finally { await context.close(); }
      }
      {
        const { page, context, state } = await fx.create(browser, width, { mobile, offline: true, tracks, fixtures: { '/offline': { body: offline } } });
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
        try {
          await fx.go(page, '/downloads');
          await page.waitForSelector('[data-dlplay="all"]');
          await buttons(page, `${tag} Downloads`, ['[data-dlplay="fav"]', '[data-dlplay="all"]'], tall, mobile);
          await page.screenshot({ path: path.join(shots, `${tag}-downloads.png`) });
          await fx.cleanCopy(page);
          check(state.errors.length === 0, `${tag} Downloads: no page errors (${state.errors})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`ui-section-head-button: ${ok} ok, ${failed.length} failed`);
  if (failed.length) process.exit(1);
})().catch((e) => { console.error('FAIL', e.message); process.exit(1); });
