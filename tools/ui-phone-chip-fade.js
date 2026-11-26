// Phone Library: the sticky chip row's right-hand fade ends in the page colour. The page scrolled under
// the row must not show through the fade (it did with a mask, which also made the row's own background
// see-through there), the fade still dims a chip cut by the row's edge, and it goes once the row is at its end.
// Real app assets, isolated API fixtures; no live accounts or devices touched.
// NODE_PATH=/path/to/node_modules node tools/ui-phone-chip-fade.js [repo root] [shots dir]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = process.argv[3];
if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, label) => { console.log(`${ok ? 'ok' : 'FAIL'} ${label}`); if (!ok) failed++; };
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

// The row's last 28 px (the fade), whole height, as RGBA pixels read from a screenshot (CSS px clip).
const strip = async (page) => {
  const box = await page.evaluate(() => { const r = document.querySelector('.main > .libnav').getBoundingClientRect(); return { x: r.right - 28, y: r.top, width: 28, height: r.height }; });
  const png = await page.screenshot({ clip: box, scale: 'css' });
  return page.evaluate(async (b64) => {
    const img = new Image(); img.src = `data:image/png;base64,${b64}`; await img.decode();
    const c = document.createElement('canvas'); c.width = img.width; c.height = img.height;
    const g = c.getContext('2d'); g.drawImage(img, 0, 0);
    return { w: img.width, h: img.height, px: [...g.getImageData(0, 0, img.width, img.height).data] };
  }, png.toString('base64'));
};
const red = (s) => { let n = 0; for (let i = 0; i < s.px.length; i += 4) if (s.px[i] > 180 && s.px[i + 1] < 90 && s.px[i + 2] < 90) n++; return n; };

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) {
      const label = `${width} ${android ? 'Android dark' : 'browser light'}`;
      const { context, page, state } = await fx.create(browser, width, { mobile: true, collections: [fx.summary], favs: [fx.SONG] });
      await context.addInitScript((android) => {
        if (android) window.pmDesktop = { platform: 'android', version: 'fixture' };
        localStorage.setItem('pm.theme', android ? 'dark' : 'light');
      }, android);
      try {
        await fx.go(page, '/library');
        await page.waitForSelector('.main > .libnav [aria-current="page"]');
        await frames(page);
        const row = await page.evaluate(() => { const n = document.querySelector('.main > .libnav'); return { more: n.classList.contains('more-right'), sticky: getComputedStyle(n).position === 'sticky', bg: getComputedStyle(n).backgroundColor }; });
        check(row.more && row.sticky, `${label}: the Library chip row is sticky and has more chips to the right (${JSON.stringify(row)})`);

        // A cut chip under the fade: its far end is dimmed to the row's colour, not drawn at full strength.
        const cut = await page.evaluate(() => {
          const n = document.querySelector('.main > .libnav'), r = n.getBoundingClientRect();
          const c = [...n.children].find((e) => { const b = e.getBoundingClientRect(); return b.left < r.right - 6 && b.right > r.right; });
          return c ? c.dataset.libnav : null;
        });
        const bgRGB = row.bg.match(/\d+/g).slice(0, 3).map(Number);
        let s = await strip(page);
        // The last 2 px column: every pixel within a few levels of the row's background.
        let edgeOff = 0;
        for (let y = 0; y < s.h; y++) for (let x = s.w - 2; x < s.w; x++) { const i = (y * s.w + x) * 4; if (Math.max(...[0, 1, 2].map((k) => Math.abs(s.px[i + k] - bgRGB[k]))) > 12) edgeOff++; }
        check(cut != null, `${label}: a chip (${cut}) is cut by the row's right edge`);
        check(edgeOff === 0, `${label}: the cut chip fades out into the row's colour at the edge (${edgeOff} pixels off)`);

        // Something bright under the row: scroll the page so a red block sits behind the sticky row.
        await page.evaluate(() => {
          const main = document.querySelector('.main'), nav = main.querySelector(':scope > .libnav');
          const block = document.createElement('div'); block.id = 'fade-probe';
          block.style.cssText = 'height:1600px;background:rgb(230,20,20);';
          nav.after(block);
          main.scrollTop = 600;
        });
        await frames(page);
        const stuck = await page.evaluate(() => { const n = document.querySelector('.main > .libnav').getBoundingClientRect(), b = document.getElementById('fade-probe').getBoundingClientRect(); return b.top < n.top && b.bottom > n.bottom; });
        check(stuck, `${label}: the page has scrolled under the sticky row`);
        s = await strip(page);
        const through = red(s);
        check(through === 0, `${label}: nothing of the page shows through the row's fade (${through} red pixels)`);
        if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `fade-${width}-${android ? 'android' : 'browser'}.png`), clip: { x: 0, y: 0, width, height: 160 } });

        // Part way along the row (the overlay sticks to the edge, it is not left at the row's start).
        const mid = await page.evaluate(() => {
          const n = document.querySelector('.main > .libnav'), c = n.children[5];
          n.style.scrollSnapType = 'none'; // a set position, not the nearest snap point
          n.scrollLeft = c.offsetLeft - n.offsetLeft + c.offsetWidth / 2 - n.clientWidth;
          return n.scrollLeft;
        });
        await frames(page);
        s = await strip(page);
        const cutMid = await page.evaluate(() => { const n = document.querySelector('.main > .libnav'), r = n.getBoundingClientRect(); return [...n.children].some((e) => { const b = e.getBoundingClientRect(); return b.left < r.right - 6 && b.right > r.right; }) && n.classList.contains('more-right'); });
        let midOff = 0;
        for (let y = 0; y < s.h; y++) for (let x = s.w - 2; x < s.w; x++) { const i = (y * s.w + x) * 4; if (Math.max(...[0, 1, 2].map((k) => Math.abs(s.px[i + k] - bgRGB[k]))) > 12) midOff++; }
        check(mid > 0 && cutMid && midOff === 0 && red(s) === 0, `${label}: scrolled ${Math.round(mid)} px along, a cut chip still fades into the row's colour with nothing showing through (${midOff} off, ${red(s)} red)`);
        await page.evaluate(() => { document.querySelector('.main > .libnav').style.scrollSnapType = ''; });

        // At the row's end the fade goes, so the last chip is drawn whole.
        await page.evaluate(() => { const n = document.querySelector('.main > .libnav'); n.scrollLeft = n.scrollWidth; });
        await page.waitForFunction(() => !document.querySelector('.main > .libnav').classList.contains('more-right'));
        await frames(page);
        const end = await page.evaluate(() => {
          const n = document.querySelector('.main > .libnav'), last = n.lastElementChild;
          const r = n.getBoundingClientRect(), b = last.getBoundingClientRect();
          return { whole: b.right <= r.right + 1, at: n.scrollLeft + n.clientWidth >= n.scrollWidth - 1, x: b.right - 4, y: b.top + b.height / 2, border: getComputedStyle(last).borderRightColor };
        });
        check(end.whole && end.at, `${label}: scrolled to its end, the row shows its last chip whole`);
        const endStrip = await strip(page);
        // The last chip's right border, 2 px in from its edge, mid-height: drawn, not faded to the background.
        const ex = Math.round(end.x - (await page.evaluate(() => document.querySelector('.main > .libnav').getBoundingClientRect().right - 28)) + 2);
        const ey = Math.round(end.y - (await page.evaluate(() => document.querySelector('.main > .libnav').getBoundingClientRect().top)));
        const shown = (() => { let best = 0; for (let x = Math.max(0, ex - 3); x <= Math.min(endStrip.w - 1, ex + 3); x++) { const i = (ey * endStrip.w + x) * 4; best = Math.max(best, ...[0, 1, 2].map((k) => Math.abs(endStrip.px[i + k] - bgRGB[k]))); } return best; })();
        check(shown > 20, `${label}: no fade over the last chip's end (border ${shown} levels from the background)`);

        check(state.errors.length === 0, `${label}: no page errors ${state.errors.join(' | ')}`);
      } finally {
        await context.close();
      }
    }
  } finally {
    await browser.close();
  }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
