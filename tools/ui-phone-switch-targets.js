// On a phone every on/off switch and a section's text action (See all, a playlist's Add one) take a
// 44 px tap, while they keep their size and place and do not take taps from the controls around them.
// Real candidate assets, isolated responses: node tools/ui-phone-switch-targets.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-switch-targets-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Two services, so the Sources list shows a switch beside each service's tab.
const sources = ['tidal', 'qobuz'].map((id, i) => ({ id, name: ['TIDAL', 'Qobuz'][i], enabled: true, status: 'signed in', features: ['search'] }));
// Home's Get started card for an owner with no music yet: Hide these steps sits in its head, with
// the Restore from a backup link just below.
const firstSteps = { show: true, hidden: false, steps: [{ id: 'music', done: false, optional: false }, { id: 'play', done: false, optional: false }] };
const fixtures = { '/sources': { body: sources }, '/me/first-steps': { body: firstSteps } };
const TARGETS = '.switch, .section-head .link';
const size = (b) => `${b.width.toFixed(1)}×${b.height.toFixed(1)}`;

// Every target on the page, scrolled to the middle of the screen in turn: its visible box, and the
// span a touch reaches it in through its centre (what the finger can hit).
const measure = (page) => page.evaluate((sel) => [...document.querySelectorAll(sel)].filter((el) => el.getClientRects().length).map((el) => {
  el.scrollIntoView({ block: 'center', behavior: 'instant' });
  const b = el.getBoundingClientRect();
  const cx = b.left + b.width / 2;
  const cy = b.top + b.height / 2;
  const on = (x, y) => { const hit = document.elementFromPoint(x, y); return !!hit && (hit === el || el.contains(hit)); };
  const out = (sx, sy) => { let d = 0; while (d < 60 && on(cx + sx * (d + 0.5), cy + sy * (d + 0.5))) d += 0.5; return d; };
  const cs = getComputedStyle(el);
  return { name: (el.getAttribute('aria-label') || el.textContent).trim(), cls: el.className, width: b.width, height: b.height,
    reachW: out(-1, 0) + out(1, 0), reachH: out(0, -1) + out(0, 1), margin: cs.margin, padding: cs.padding };
}), TARGETS);

// No other control loses taps to a target's larger area: a grid of points over each visible control
// still reaches that control, not a switch or link next to it.
const stolen = (page) => page.evaluate((sel) => {
  const targets = [...document.querySelectorAll(sel)];
  const out = [];
  // Other switches and links count too, and labels (a whole row can be a switch's label).
  for (const el of document.querySelectorAll('button, a[href], input, select, textarea, label, [role=button], [role=tab], summary')) {
    if (!el.getClientRects().length) continue;
    const b = el.getBoundingClientRect();
    if (b.width < 4 || b.height < 4 || b.top < 0 || b.bottom > innerHeight) continue;
    for (const px of [0.1, 0.5, 0.9]) for (const py of [0.1, 0.5, 0.9]) {
      const hit = document.elementFromPoint(b.left + b.width * px, b.top + b.height * py);
      const thief = hit && targets.find((t) => t !== el && !t.contains(el) && !el.contains(t) && (t === hit || t.contains(hit)));
      if (thief) out.push(`${el.tagName.toLowerCase()} "${(el.getAttribute('aria-label') || el.textContent).trim().slice(0, 24)}" → ${(thief.getAttribute('aria-label') || thief.textContent).trim()}`);
    }
  }
  return [...new Set(out)];
}, TARGETS);

// A press inside the 44 px area but outside the visible control, 1.5 px inside the area's bottom
// edge: a real touch, and a mouse click (no touch adjustment, so only the area itself catches it).
async function pressBelow(page, selector, how) {
  const el = page.locator(selector).first();
  await el.evaluate((n) => n.scrollIntoView({ block: 'center', behavior: 'instant' }));
  const b = await el.boundingBox();
  const x = b.x + b.width / 2;
  const y = b.y + b.height / 2 + 22 - 1.5;
  if (how === 'touch') await page.touchscreen.tap(x, y); else await page.mouse.click(x, y);
}
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 2000 }).then(() => true, () => false);

// Each page with its targets checked against the sizes they have everywhere else.
async function page1(page, at, route, wantMin, scrollTo) {
  await fx.go(page, route);
  await page.waitForSelector(scrollTo);
  await page.waitForTimeout(300);
  const items = await measure(page);
  check(items.length >= wantMin, `${at} ${route}: ${items.length} switches/links shown (at least ${wantMin})`);
  for (const t of items) {
    check(t.reachW >= 44 && t.reachH >= 44, `${at} ${route}: “${t.name}” takes a 44 px tap (${t.reachW.toFixed(1)}×${t.reachH.toFixed(1)})`);
    if (/\bswitch\b/.test(t.cls)) check(t.width === 34 && t.height === 20 && t.margin === '0px 4px', `${at} ${route}: “${t.name}” keeps its 34×20 look and margins (${size(t)}, ${t.margin})`);
    else {
      const pad = /\brelall\b/.test(t.cls) ? '6px 2px' : '4px 0px';
      check(t.padding === pad && t.margin === '0px', `${at} ${route}: “${t.name}” keeps its padding and margins (${t.padding}, ${t.margin})`);
    }
  }
  for (const t of items) {
    await page.evaluate(([sel, name]) => [...document.querySelectorAll(sel)].find((el) => (el.getAttribute('aria-label') || el.textContent).trim() === name)?.scrollIntoView({ block: 'center', behavior: 'instant' }), [TARGETS, t.name]);
    const s = await stolen(page);
    check(!s.length, `${at} ${route}: around “${t.name}” every other control keeps its taps${s.length ? ` (${s.join('; ')})` : ''}`);
  }
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at} ${route}: no sideways overflow`);
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures });
      state.collections = [fx.summary];
      try {
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });

        // Display: Dense lists and the two colour switches.
        await page1(page, at, '/settings/appearance', 3, '#densepick');
        for (const how of ['touch', 'mouse']) {
          const was = await page.locator('#densepick').isChecked();
          await pressBelow(page, '#densepick', how);
          check(await until(page, (w) => document.querySelector('#densepick').checked !== w, was), `${at}: a ${how} press just below Dense lists turns it ${was ? 'off' : 'on'}`);
        }
        await page.locator('#densepick').evaluate((n) => n.scrollIntoView({ block: 'center', behavior: 'instant' }));
        await page.screenshot({ path: path.join(SHOTS, `display-${at}.png`) });

        // Home: the Get started card's Hide these steps, with a link just below its head.
        await page1(page, at, '/', 1, '#setuphide');
        check(await page.locator('#setuprestore').isVisible(), `${at}: Restore from a backup shows just below the card's head`);
        await page.locator('#setuphide').evaluate((n) => n.scrollIntoView({ block: 'center', behavior: 'instant' }));
        await page.screenshot({ path: path.join(SHOTS, `home-${at}.png`) });

        // Sources: a switch beside each service's tab; the tab keeps its taps (checked in page1).
        await page1(page, at, '/sources', 2, '[data-src-on="tidal"]');
        await page.screenshot({ path: path.join(SHOTS, `sources-${at}.png`) });

        // A playlist's description: Add one.
        await page1(page, at, '/playlist/c1', 1, '#pldesc');
        await page.screenshot({ path: path.join(SHOTS, `playlist-${at}.png`) });
        for (const how of ['touch', 'mouse']) {
          await pressBelow(page, '#pldesc', how);
          check(await until(page, () => !!document.querySelector('dialog.dialog[open] [data-field]')), `${at}: a ${how} press just below Add one opens the description`);
          await page.keyboard.press('Escape');
          await until(page, () => !document.querySelector('dialog.dialog[open]'));
        }
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // These rules belong to the phone layout: the desktop layout keeps its compact switches and links.
    const { page, context } = await fx.create(browser, 1024, { fixtures });
    try {
      await fx.go(page, '/settings/appearance');
      await page.waitForSelector('#densepick');
      const d = (await measure(page)).find((t) => t.name === 'Dense lists');
      check(d && d.reachH < 44, `1024 px desktop keeps its compact switch (${d && `${d.reachW}×${d.reachH}`})`);
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
