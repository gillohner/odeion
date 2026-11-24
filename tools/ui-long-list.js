// A long Songs list: rows that are off screen are not styled, laid out or painted until they come
// near, and nothing else changes. Real candidate assets on the fixture origin (tools/ui-fixture.js),
// 1,200 songs:
//   * a song row is content-visibility: auto, and a row far down is skipped while a row on screen
//     is not; the first 24 rows are drawn as they are put in, so a tap at once reaches a row's ⋯;
//   * the list is exactly as long as its rows (the row's height is fixed), so the scrollbar and a
//     jump to a far row are exact, and the far row is drawn when it is reached;
//   * a far row's control takes the focus, and its row is drawn then;
//   * the focus ring of a row's controls fits inside the row, and a row that follows a finger still
//     draws its label in the gap it leaves (a row's box clips what it paints, but for that row);
//   * style and layout for the page cost less than with the rule switched off, in the same run.
//   node tools/ui-long-list.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'long-list-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const N = 1200;
const tracks = Array.from({ length: N }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song number ${i + 1}`, track_no: (i % 12) + 1 }));
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
// What style and layout cost from the page's start until the list is drawn (CDP, seconds)
const cost = async (browser, width, off) => {
  const { context, page } = await fx.create(browser, width, { mobile: width < 600, tracks });
  if (off) await page.addInitScript(() => { document.addEventListener('DOMContentLoaded', () => { const s = document.createElement('style'); s.textContent = '.track { content-visibility: visible !important; }'; document.head.appendChild(s); }); });
  const cdp = await context.newCDPSession(page); await cdp.send('Performance.enable'); await cdp.send('Emulation.setCPUThrottlingRate', { rate: 4 });
  await fx.go(page, '/tracks'); await page.waitForFunction((n) => document.querySelectorAll('#main .track').length >= n, N, { timeout: 60000 }); await frames(page);
  const m = Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map((x) => [x.name, x.value]));
  const cv = await page.evaluate(() => getComputedStyle(document.querySelectorAll('#main .track')[100]).contentVisibility);
  await context.close();
  if ((cv === 'auto') === !!off) throw new Error(`the rule is ${cv} with off=${!!off}`);
  return m.LayoutDuration + m.RecalcStyleDuration;
};

(async () => {
  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const at = `${width}px`;
    const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, tracks });
    await fx.go(page, '/tracks'); await page.waitForFunction((n) => document.querySelectorAll('#main .track').length >= n, N, { timeout: 60000 }); await frames(page);
    const a = await page.evaluate(() => {
      const rows = [...document.querySelectorAll('#main .track')]; const near = rows[1]; const far = rows[900];
      const first = rows.slice(0, 24).every((r) => getComputedStyle(r).contentVisibility === 'visible');
      const seen = (row) => row.firstElementChild.checkVisibility({ contentVisibilityAuto: true });
      const h = near.getBoundingClientRect().height; const box = rows[0].parentElement;
      return { n: rows.length, first, cv: getComputedStyle(rows[100]).contentVisibility, nearSeen: seen(near), farSeen: seen(far), h, span: rows[rows.length - 1].getBoundingClientRect().bottom - rows[0].getBoundingClientRect().top, farTop: far.getBoundingClientRect().top - rows[0].getBoundingClientRect().top, boxH: box.getBoundingClientRect().height };
    });
    check(a.n === N && a.cv === 'auto' && a.first, `${at}: ${a.n} song rows, content-visibility: ${a.cv} but for the first 24, which are drawn as they come (${a.first})`);
    check(a.nearSeen === true && a.farSeen === false, `${at}: a row on screen is drawn, row 901 is skipped until it comes near (${a.nearSeen}, ${a.farSeen})`);
    check(Math.abs(a.span - N * a.h) < 1 && Math.abs(a.farTop - 900 * a.h) < 1, `${at}: the list is exactly as long as its rows (${a.span} = ${N} × ${a.h}; row 901 at ${a.farTop})`);
    // A jump to the far row is exact, and the row is drawn there
    const b = await page.evaluate(async () => {
      const rows = [...document.querySelectorAll('#main .track')]; const far = rows[900];
      far.scrollIntoView({ block: 'center', behavior: 'instant' }); await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const r = far.getBoundingClientRect(); const scroller = [document.getElementById('main'), document.scrollingElement].find((x) => x.scrollTop > 0);
      // (where row 901 must be: 900 rows below the first one, wherever the page was scrolled to)
      const first = rows[0].getBoundingClientRect().top;
      return { centred: r.top >= 0 && r.bottom <= innerHeight && Math.abs(r.top - first - 900 * r.height) < 1, seen: far.firstElementChild.checkVisibility({ contentVisibilityAuto: true }), title: far.innerText.includes('Song number 901'), h: r.height, top: scroller ? scroller.scrollTop : 0 };
    });
    check(b.centred && b.seen && b.title && b.h === a.h, `${at}: a jump to row 901 lands on it, drawn, at its height (${JSON.stringify(b)})`);
    await page.screenshot({ path: path.join(SHOTS, `row-901-${width}.png`) });
    // Scrolled back and forth, the far row is where it was (nothing was re-measured to another size)
    const c = await page.evaluate(async () => { const main = [document.getElementById('main'), document.scrollingElement].find((x) => x.scrollTop > 0); const was = main.scrollTop; const tall = main.scrollHeight; main.scrollTop = 0; await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))); main.scrollTop = was; await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))); const far = document.querySelectorAll('#main .track')[900].getBoundingClientRect(); return { same: main.scrollTop === was, tall: main.scrollHeight === tall, there: far.top >= 0 && far.bottom <= innerHeight }; });
    // (a phone's page moves a little with its bars: the row back on screen is what counts there)
    check((c.same || width < 600) && c.tall && c.there, `${at}: scrolled to the top and back, the page is as long as it was and row 901 is on screen again (${JSON.stringify(c)})`);
    // The keyboard reaches a control in a row that was skipped, and the page goes to it
    const d = await page.evaluate(async () => {
      for (const x of [document.getElementById('main'), document.scrollingElement]) x.scrollTop = 0; await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const row = document.querySelectorAll('#main .track')[700]; const was = row.firstElementChild.checkVisibility({ contentVisibilityAuto: true });
      // (a control a person can reach there: one that is shown)
      const ctl = [...row.querySelectorAll('button, a[href]')].find((x) => getComputedStyle(x).display !== 'none' && getComputedStyle(x).visibility !== 'hidden') || row;
      ctl.focus(); await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const r = row.getBoundingClientRect();
      return { skippedBefore: !was, focused: document.activeElement === ctl, drawn: row.firstElementChild.checkVisibility({ contentVisibilityAuto: true }), inView: r.bottom > 0 && r.top < innerHeight, what: ctl.tagName.toLowerCase() };
    });
    check(d.skippedBefore && d.focused && d.drawn, `${at}: a control in row 701, skipped until then, takes the focus, and its row is drawn (${JSON.stringify(d)})`);
    // The focus ring of a row's own controls is drawn inside the row (what a row paints is clipped to it)
    const e = await page.evaluate(async () => {
      for (const x of [document.getElementById('main'), document.scrollingElement]) x.scrollTop = 0; await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const row = document.querySelectorAll('#main .track')[2]; const box = row.getBoundingClientRect(); const out = [];
      for (const ctl of [...row.querySelectorAll('button, a[href]')].filter((x) => x.getClientRects().length && getComputedStyle(x).visibility !== 'hidden')) {
        ctl.focus(); const cs = getComputedStyle(ctl); const reach = cs.outlineStyle === 'none' ? 0 : parseFloat(cs.outlineWidth) + Math.max(0, parseFloat(cs.outlineOffset)); const r = ctl.getBoundingClientRect();
        out.push({ what: (ctl.getAttribute('aria-label') || ctl.textContent || ctl.tagName).trim().slice(0, 24), inside: r.top - reach >= box.top - 0.5 && r.bottom + reach <= box.bottom + 0.5 && r.left - reach >= box.left - 0.5 && r.right + reach <= box.right + 0.5 });
      }
      return out;
    });
    check(e.length >= 2 && e.every((x) => x.inside), `${at}: the focus ring of each of a row's ${e.length} controls fits inside the row (${e.filter((x) => !x.inside).map((x) => x.what).join(', ') || 'all inside'})`);
    // A row that follows a finger shows its label in the gap it leaves, outside its own box: drawn
    // (the same picture of the gap with and without the label's words must differ)
    if (width < 600) {
      const gap = await page.evaluate(async () => { const row = document.querySelectorAll('#main .track')[30]; if (document.activeElement) document.activeElement.blur(); row.scrollIntoView({ block: 'center', behavior: 'instant' }); await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))); row.classList.add('swiping'); row.dataset.swipeLabel = 'Play next'; row.style.transform = 'translateX(110px)'; const r = row.getBoundingClientRect(); return { cv: getComputedStyle(row).contentVisibility, x: Math.max(0, Math.round(r.left - 104)), y: Math.round(r.top + 8), w: 96, h: Math.round(r.height - 16) }; });
      const withLabel = await page.screenshot({ clip: { x: gap.x, y: gap.y, width: gap.w, height: gap.h } });
      await page.screenshot({ path: path.join(SHOTS, `swipe-label-${width}.png`) });
      await page.evaluate(() => { document.querySelectorAll('#main .track')[30].dataset.swipeLabel = ''; });
      const without = await page.screenshot({ clip: { x: gap.x, y: gap.y, width: gap.w, height: gap.h } });
      await page.evaluate(() => { const row = document.querySelectorAll('#main .track')[30]; row.classList.remove('swiping'); delete row.dataset.swipeLabel; row.style.transform = ''; });
      check(gap.cv === 'visible' && !withLabel.equals(without), `${at}: a row that follows a finger draws its label in the gap it leaves (content-visibility: ${gap.cv}; the gap's picture ${withLabel.equals(without) ? 'is the same without the words' : 'changes with the words'})`);
    }
    // A tap the instant Songs puts its rows in reaches the first row's ⋯ (its insides are drawn then;
    // before, the tap landed on the row itself)
    {
      await page.evaluate(() => document.getElementById('main').scrollTo({ top: 0, behavior: 'instant' })); await frames(page);
      const spot = await page.evaluate(() => { const b = document.querySelector('#main .track [data-act="more"]'); const r = b.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; });
      await page.evaluate(() => { history.pushState({}, '', '/albums'); dispatchEvent(new PopStateEvent('popstate')); });
      await page.waitForFunction(() => !document.querySelector('#main .track'));
      await page.evaluate((p) => { window.__hit = null; const mo = new MutationObserver(() => { const row = document.querySelector('#main .track'); if (!row || window.__hit) return; const el = document.elementFromPoint(p.x, p.y); window.__hit = { more: !!(el && el.closest('[data-act="more"]')), what: el ? `${el.tagName.toLowerCase()}.${el.className && el.className.baseVal == null ? el.className : ''}` : null }; mo.disconnect(); }); mo.observe(document.getElementById('main'), { childList: true, subtree: true }); }, spot);
      await page.evaluate(() => { history.pushState({}, '', '/tracks'); dispatchEvent(new PopStateEvent('popstate')); });
      await page.waitForFunction(() => !!window.__hit, null, { timeout: 8000 }).catch(() => {});
      const hit = await page.evaluate(() => window.__hit);
      check(!!hit && hit.more, `${at}: a tap the instant the rows are put in reaches the first row's ⋯ (${JSON.stringify(hit)})`);
    }
    check(!state.errors.length, `${at}: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  // The cost, with the rule and with it switched off, three times each in turn (the smallest of each)
  for (const width of [390, 1280]) {
    const on = []; const off = [];
    for (let i = 0; i < 3; i++) { on.push(await cost(browser, width, false)); off.push(await cost(browser, width, true)); }
    const a = Math.min(...on); const b = Math.min(...off);
    check(a < b * 0.6, `${width}px: style and layout for ${N} songs cost ${Math.round(a * 1000)} ms, ${Math.round(b * 1000)} ms with the rule switched off (CPU slowed 4×)`);
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
