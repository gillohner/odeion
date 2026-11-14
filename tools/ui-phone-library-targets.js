// Library's section tabs and Add music entry points have comfortable phone targets.
// Albums' and Artists' A–Z letters too: each reaches 44 px tall and letters sit edge to edge, the
// strip fades at its end while more letters are off to the right, and nothing on the page moves.
// Real candidate assets, isolated responses: node tools/ui-phone-library-targets.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-library-targets-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };
const targets = '#libadd, [data-addmusic], .libtabs .chip';
const fixtures = {
  '/library/folders': { body: [] },
  '/library/webdav': { body: [] },
  '/library/drives': { body: { providers: [], roots: [] } },
  '/library/hidden': { body: [] },
};

async function measure(page, at) {
  const sizes = await page.locator(targets).evaluateAll((els) => els.map((el) => {
    const b = el.getBoundingClientRect();
    return { label: el.textContent.trim(), width: b.width, height: b.height };
  }));
  check(sizes.length > 0 && sizes.every((b) => b.height >= 44 && b.width >= 44), `${at}: targets at least 44 px (${sizes.map((b) => `${b.label}: ${b.width.toFixed(1)}×${b.height.toFixed(1)}`).join(', ')})`);
  await fits(page, at);
}

async function fits(page, at) {
  const layout = await page.evaluate(() => {
    const main = document.querySelector('#main');
    const els = [...document.querySelectorAll('#libadd, [data-addmusic], .libtabs .chip')];
    return {
      fits: document.documentElement.scrollWidth <= innerWidth && main.scrollWidth <= main.clientWidth,
      visible: els.every((el) => { const b = el.getBoundingClientRect(); return b.left >= 0 && b.right <= innerWidth; }),
    };
  });
  check(layout.fits && layout.visible, `${at}: no sideways overflow or clipped controls`);
}

// A real touch near the bottom edge verifies the whole target, not just its label.
async function tapEdge(page, selector) {
  const b = await page.locator(selector).boundingBox();
  await page.touchscreen.tap(b.x + b.width / 2, b.y + b.height - 2);
}

async function chosen(page, tab) {
  await page.waitForSelector(`#libtab-${tab}[aria-selected="true"]`);
  await page.waitForSelector(`#libpanel[aria-labelledby="libtab-${tab}"]`);
}

// A long library (26 letters, 78 albums and artists), so the A–Z strip shows and scrolls sideways.
const NAMES = ['Abba', 'Blur', 'Cream', 'Doors', 'Eels', 'Faust', 'Genesis', 'Hole', 'Iggy', 'Jet', 'Kiss', 'Love', 'Muse', 'Nena', 'Oasis', 'Pixies', 'Queen', 'Rush', 'Sade', 'Toto', 'U2', 'Vangelis', 'Wire', 'Xtc', 'Yes', 'Zappa'];
const longLibrary = {
  '/library/albums': { body: Array.from({ length: 78 }, (_, i) => ({ id: i + 1, title: `${NAMES[i % 26]} ${Math.floor(i / 26) + 1}`, album_artist: NAMES[i % 26], year: 2000, tracks: 1 })) },
  '/library/albums/formats': { body: [] },
  '/library/artists': { body: Array.from({ length: 78 }, (_, i) => ({ id: i + 1, name: `${NAMES[i % 26]} ${Math.floor(i / 26) + 1}`, albums: 1, tracks: 1 })) },
};

async function azStrip(browser, width, android) {
  const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures: longLibrary });
  try {
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    // below: the list's distance under the letters on main (the reach must not push it down)
    for (const [route, first, below] of [['/albums', '#albumgrid .tile', 28], ['/artists', '#artistlist > *', 29]]) {
      const at = `${android ? 'android' : 'browser'}-${width} ${route} A–Z`;
      await fx.go(page, route);
      await page.locator('.azbar .az').first().waitFor();
      await page.locator(first).first().waitFor();
      await page.waitForFunction(() => document.querySelector('.azbar').classList.contains('more-right'), null, { timeout: 2000 }).catch(() => {});
      const s = await page.evaluate((first) => {
        const bar = document.querySelector('.azbar'), az = [...bar.querySelectorAll('.az')];
        // The finger's reach of each of the first letters, found by what a touch at each point hits
        const reach = az.slice(0, 6).map((x) => {
          const c = x.getBoundingClientRect(), cx = c.left + c.width / 2, cy = c.top + c.height / 2;
          let up = 0, down = 0;
          for (let d = 1; d < 40; d++) { if (document.elementFromPoint(cx, cy - d)?.closest('.az') === x) up = d; if (document.elementFromPoint(cx, cy + d)?.closest('.az') === x) down = d; }
          return up + down + 1;
        });
        // Between two neighbours a touch still lands on a letter, not the strip around them
        const gaps = az.slice(0, 6).map((x) => { const c = x.getBoundingClientRect(); return !!document.elementFromPoint(c.right + 0.5, c.top + c.height / 2)?.closest('.az'); });
        const letters = az.map((x) => x.getBoundingClientRect()), top = Math.min(...letters.map((c) => c.top)), bottom = Math.max(...letters.map((c) => c.bottom));
        const above = [...document.querySelectorAll('.listbar > :not(.azbar)')].map((el) => el.getBoundingClientRect().bottom);
        const box = bar.getBoundingClientRect();
        // The gap above the letters stays the Sort line's (Favourites' own reach reaches 8 px down into it)
        const gapFree = az.slice(0, 12).every((x) => { const c = x.getBoundingClientRect(); for (let y = Math.max(...above) + 0.5; y < c.top - 1; y++) if (document.elementFromPoint(c.left + c.width / 2, y)?.closest('.az')) return false; return true; });
        return { reach, gaps, gapFree, width: Math.min(...letters.slice(0, 6).map((c) => c.width)), fades: bar.classList.contains('more-right'), lines: new Set(letters.map((c) => Math.round(c.top))).size,
          stripTop: box.top, aboveBottom: Math.max(...above), gapAbove: Math.round(top - Math.max(...above)), gapBelow: Math.round(document.querySelector(first).getBoundingClientRect().top - bottom),
          overflow: document.documentElement.scrollWidth - innerWidth };
      }, first);
      check(s.lines === 1 && s.reach.every((h) => h >= 44), `${at}: each letter reaches at least 44 px tall (${s.reach.join(', ')})`);
      check(s.width >= 28 && s.gaps.every(Boolean), `${at}: letters are ${s.width.toFixed(1)} px wide and edge to edge, no dead gap between two`);
      check(s.fades, `${at}: the strip fades at its end while more letters are off to the right`);
      check(s.stripTop >= s.aboveBottom - 0.5 && s.gapFree, `${at}: the letters' reach leaves the Sort line and the gap under it alone (strip ${s.stripTop.toFixed(1)}, line ${s.aboveBottom.toFixed(1)})`);
      // The reach is room taken from margins: the letters and the list sit where they were
      check(s.gapAbove === 8 && s.gapBelow <= below, `${at}: the letters sit 8 px under Sort and the list ${s.gapBelow} px under them, as before`);
      check(s.overflow <= 0, `${at}: nothing wider than the screen (${s.overflow} px)`);
      // A press at the bottom of the reach, at the letter's right edge, jumps to that letter. A mouse
      // press, because a touch there would be snapped onto the letter by the browser even without a reach
      const b = await page.locator('[data-az="B"]').boundingBox();
      await page.mouse.click(b.x + b.width - 1, b.y + b.height + 16);
      await page.locator('[data-az="B"][aria-pressed="true"]').waitFor({ timeout: 2000 }).catch(() => {});
      const jumped = await page.locator(first).first().innerText();
      check(await page.locator('[data-az="B"]').getAttribute('aria-pressed') === 'true' && /Blur 1/.test(jumped), `${at}: a press at the bottom of B's reach jumps to B (${jumped.replace(/\s+/g, ' ').trim()})`);
      await page.locator('.azbar').evaluate((el) => { el.scrollLeft = el.scrollWidth; });
      await page.waitForFunction(() => !document.querySelector('.azbar').classList.contains('more-right'), null, { timeout: 2000 }).catch(() => {});
      check(!await page.locator('.azbar').evaluate((el) => el.classList.contains('more-right')), `${at}: scrolled to Z, the strip no longer fades`);
      if (route === '/albums') await page.screenshot({ path: path.join(SHOTS, `az-${android ? 'android' : 'browser'}-${width}.png`) });
    }
    check(!state.errors.length, `${android ? 'android' : 'browser'}-${width} A–Z: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) await azStrip(browser, width, android);
    for (const width of [360, 412]) for (const android of [false, true]) for (const theme of ['light', 'dark']) {
      const at = `${android ? 'android' : 'browser'}-${width}-${theme}`;
      const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures });
      try {
        await context.addInitScript(({ android, theme }) => {
          localStorage.setItem('pm.theme', theme);
          if (android) window.pmDesktop = { platform: 'android', version: 'fixture' };
        }, { android, theme });
        await fx.go(page, '/library');
        await chosen(page, 'mine');
        await measure(page, `${at} My music`);
        await page.screenshot({ path: path.join(SHOTS, `library-${at}.png`) });
        for (const tab of ['node', 'import', 'mine']) {
          await tapEdge(page, `#libtab-${tab}`);
          await chosen(page, tab);
          check(await page.locator('.libtabs [aria-selected="true"]').count() === 1, `${at}: one edge tap selects ${tab}`);
          await measure(page, `${at} ${tab}`);
        }
        for (const route of ['/library', '/albums', '/tracks']) {
          await fx.go(page, route);
          await page.waitForSelector('#libadd, [data-addmusic]');
          await measure(page, `${at} ${route}`);
          await tapEdge(page, '#libadd, [data-addmusic]');
          await page.waitForSelector('dialog[open] .choice');
          check(await page.locator('dialog[open]').count() === 1 && await page.locator('dialog[open] h2').textContent() === 'Add music', `${at} ${route}: one edge tap opens Add music`);
          check(await page.locator('dialog[open] .choice').count() === 3, `${at} ${route}: upload, folder and import remain available`);
          await page.keyboard.press('Escape');
          await page.waitForSelector('dialog[open]', { state: 'detached' });
        }
        // The enlarged entry control still leads to the existing upload and import flows.
        await tapEdge(page, '#libadd');
        await page.locator('dialog[open] .choice').filter({ hasText: 'Upload files' }).tap();
        await page.waitForSelector('#libupload:not([hidden])');
        check(await page.locator('#libupload').isVisible(), `${at}: Upload files opens the upload panel in two taps`);
        await fx.go(page, '/library');
        await chosen(page, 'mine');
        await tapEdge(page, '#libadd');
        await page.locator('dialog[open] .choice').filter({ hasText: 'Import' }).tap();
        await chosen(page, 'import');
        check(await page.locator('#libpanel').isVisible(), `${at}: import opens its section in two taps`);
        // Keyboard users keep the selected tab and focus as the panel is replaced.
        await page.locator('#libtab-import').focus();
        await page.keyboard.press('ArrowLeft');
        await chosen(page, 'node');
        check(await page.locator('#libtab-node').evaluate((el) => el === document.activeElement), `${at}: arrow navigation keeps focus on the selected tab`);
        // Increase the font tokens actually used by these controls; min-height must allow growth.
        await page.evaluate(() => {
          document.documentElement.style.setProperty('--fs-md', '18.2px');
          document.documentElement.style.setProperty('--fs-sm', '16.9px');
        });
        check(await page.locator('#libtab-node').evaluate((el) => parseFloat(getComputedStyle(el).fontSize) >= 16.9), `${at}: larger text is applied`);
        await measure(page, `${at} larger text`);
        await page.screenshot({ path: path.join(SHOTS, `large-text-${at}.png`) });
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // The listener's sole section gets the same target without exposing owner actions.
    for (const width of [360, 412]) {
      const { page, context } = await fx.create(browser, width, { mobile: true, role: 'listener', fixtures });
      try {
        await fx.go(page, '/library');
        await chosen(page, 'mine');
        await measure(page, `listener ${width}`);
        check(await page.locator('.libtabs .chip').count() === 1 && await page.locator('#libadd').count() === 0, `listener ${width}: only My music, no owner actions`);
      } finally { await context.close(); }
    }
    // These rules belong to the chosen phone layout, including a forced wide phone layout.
    const { page, context } = await fx.create(browser, 599, { fixtures });
    try {
      await fx.go(page, '/library');
      await chosen(page, 'mine');
      await measure(page, '599 px phone');
      await page.setViewportSize({ width: 600, height: 900 });
      await page.waitForFunction(() => document.documentElement.dataset.shell === 'desktop');
      check(await page.locator('#libtab-mine').evaluate((el) => el.getBoundingClientRect().height < 44), '600 px desktop keeps its compact tabs');
      await page.evaluate(() => document.documentElement.dataset.shell = 'phone');
      await measure(page, 'phone layout at 600 px');
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`\n${passed} passed; ${failed} failed`);
  if (failed) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
