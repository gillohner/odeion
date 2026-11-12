// On a phone a page's plain buttons (Settings › Library's Add folder and Scan now, Import's Check
// and Import, an empty page's Open settings) are 44 px taps, like a dialog's; an icon button stays
// square, Back keeps its height so the page doesn't move, and desktop keeps its compact buttons.
// Real candidate assets, isolated responses: node tools/ui-phone-page-buttons.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-page-buttons-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// The pages and the buttons each one must show (all of them measure 38 px on a phone without the rule).
const pages = [
  ['/library', ['Sign in']],
  ['/albums', ['Open settings']],
  ['/settings/library', ['Add folder', 'Scan now', 'Add share']],
  ['/library/import', ['Choose a playlist file', 'Check', 'Import', 'Open Sources']],
  ['/zones', ['Group outputs']],
];

// Every plain button shown in the page (not a small one, not Back), with its box and whether a touch
// 42 px below its top still lands on it (the old 38 px button ends before that).
const buttons = (page) => page.evaluate(() => [...document.querySelectorAll('#main .btn')]
  .filter((el) => el.getClientRects().length && !el.matches('.sm, .back, .set-back') && !el.closest('[hidden]'))
  .map((el) => {
    el.scrollIntoView({ block: 'center', behavior: 'instant' });
    const b = el.getBoundingClientRect();
    const hit = document.elementFromPoint(b.left + b.width / 2, b.top + 42);
    const name = (el.getAttribute('aria-label') || el.textContent || '').trim().replace(/\s+/g, ' ').replace(/…$/, '');
    return { name, width: b.width, height: b.height, left: b.left, right: b.right, iconic: el.classList.contains('iconic'), lower: !!hit && (hit === el || el.contains(hit)) };
  }));
const list = (bs) => bs.map((b) => `${b.name} ${b.width.toFixed(0)}×${b.height.toFixed(0)}`).join(', ');

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      const { page, context, state } = await fx.create(browser, width, { mobile: true });
      state.collections = [fx.summary];
      try {
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
        for (const [route, want] of pages) {
          await fx.go(page, route);
          await page.waitForFunction((names) => {
            const shown = [...document.querySelectorAll('#main .btn')].filter((el) => el.getClientRects().length).map((el) => (el.getAttribute('aria-label') || el.textContent || '').trim());
            return names.every((n) => shown.some((t) => t.startsWith(n)));
          }, want, { timeout: 5000 }).catch(() => {});
          const bs = await buttons(page);
          const names = bs.map((b) => b.name);
          for (const w of want) check(names.some((n) => n.startsWith(w)), `${at} ${route}: "${w}" is shown`);
          check(bs.length > 0 && bs.every((b) => b.height >= 44), `${at} ${route}: every button is at least 44 px tall (${list(bs)})`);
          check(bs.every((b) => b.lower), `${at} ${route}: a touch 42 px below a button's top still presses it`);
          check(bs.filter((b) => b.iconic).every((b) => b.width >= 44 && Math.abs(b.width - b.height) < 1), `${at} ${route}: icon buttons stay square at 44 px (${list(bs.filter((b) => b.iconic))})`);
          check(bs.every((b) => b.left >= 0 && b.right <= width), `${at} ${route}: buttons inside the screen`);
          check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at} ${route}: no sideways overflow`);
          await page.evaluate(() => document.querySelector('#main').scrollTo(0, 0));
          await page.screenshot({ path: path.join(SHOTS, `${at}-${route.replace(/[^a-z0-9]+/gi, '_')}.png`) });
        }

        // One tap still does the job: Scan now starts a scan.
        await fx.go(page, '/settings/library');
        await page.waitForSelector('#scan');
        const scans = () => state.hits.filter((h) => h.v === '/library/scan' && h.m === 'POST').length;
        const before = scans();
        await page.locator('#scan').scrollIntoViewIfNeeded();
        // 42 px below its top: past the bottom of the old 38 px button. A mouse press, since a touch there
        // is snapped onto the nearest button anyway.
        const b = await page.locator('#scan').boundingBox();
        await page.mouse.click(b.x + b.width / 2, b.y + 42);
        for (let i = 0; i < 30 && scans() === before; i++) await page.waitForTimeout(50);
        check(scans() === before + 1, `${at}: one press near the bottom of Scan now starts a scan`);

        // The Playlists page's toolbar: the name box, Create and Import line up at 44 px.
        await fx.go(page, '/playlists');
        await page.waitForSelector('#plcreate');
        const bar = await page.evaluate(() => [...document.querySelectorAll('#plname, #plcreate, label.file-btn')]
          .filter((el) => el.getClientRects().length).map((el) => Math.round(el.getBoundingClientRect().height)));
        check(bar.length >= 2 && bar.every((h) => h === 44), `${at} /playlists: the toolbar's controls are all 44 px (${bar.join(', ')})`);

        // A playlist's action row keeps its own sizes: its icon buttons stay square.
        await fx.go(page, '/playlist/c1');
        await page.waitForSelector('.hero .actions #plmore');
        const row = await page.evaluate(() => [...document.querySelectorAll('.hero .actions > :is(.hero-zone, #plmore)')]
          .filter((el) => el.getClientRects().length).map((el) => el.getBoundingClientRect()).map((r) => [r.width, r.height]));
        check(row.length >= 2 && row.every(([w, h]) => Math.abs(w - h) < 1), `${at} /playlist/c1: the action row's icon buttons stay square (${row.map(([w, h]) => `${w.toFixed(0)}×${h.toFixed(0)}`).join(', ')})`);

        // Back keeps its height, so the panel under it stays where it was.
        await fx.go(page, '/settings/appearance');
        await page.waitForSelector('#set-appearance .set-back');
        const back = await page.locator('#set-appearance .set-back').evaluate((el) => el.getBoundingClientRect().height);
        check(back < 40, `${at}: Settings Back keeps its height (${back.toFixed(1)} px)`);
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // The desktop layout keeps its compact buttons.
    const { page, context } = await fx.create(browser, 1024, {});
    try {
      await fx.go(page, '/settings/library');
      await page.waitForSelector('#scan');
      const h = await page.locator('#scan').evaluate((el) => el.getBoundingClientRect().height);
      check(h < 44, `1024 px desktop keeps its compact Scan now (${h.toFixed(1)} px)`);
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
