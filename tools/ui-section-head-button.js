// A section head's small button (Collections' New smart collection, offline Downloads' Play beside
// Bookmarks and Shuffle beside All songs) has its words in its middle, level with its icon, and on
// the section title's baseline. On a phone or a touch screen it is 40 px tall; its words sat at its
// top, 5.5 px above its middle and its icon. Desktop keeps its compact size.
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
async function buttons(page, tag, sels, tall, touch) {
  for (const sel of sels) {
    const m = await measure(page, sel);
    const reach = await page.$eval(sel, (b) => { const r = b.getBoundingClientRect(), c = getComputedStyle(b, '::before'), z = c.content !== 'none' && getComputedStyle(b).position === 'relative'; return z ? r.height - parseFloat(c.top) - parseFloat(c.bottom) : r.height; });
    const at = `${tag} ${sel}`;
    check(Math.abs(m.words - m.mid) <= 1.5, `${at}: the words are in the button's middle (words ${m.words.toFixed(1)}, middle ${m.mid.toFixed(1)})`);
    check(m.icon != null && Math.abs(m.icon - m.words) <= 1.5, `${at}: the icon is level with the words (icon ${m.icon?.toFixed(1)}, words ${m.words.toFixed(1)})`);
    check(Math.abs(m.base - m.titleBase) <= 1, `${at}: the words are on the title's baseline (${m.base.toFixed(1)} vs ${m.titleBase.toFixed(1)})`);
    if (touch) check(reach >= 44, `${at}: a touch screen's tap reaches 44 px or more (${reach})`);
    if (tall) check(m.height >= 39.5 && m.height <= 41, `${at}: 40 px tall on a phone or a touch screen (${m.height})`);
    else check(m.height < 34, `${at}: compact on a desktop (${m.height})`);
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
