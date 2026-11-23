// On a phone, text fields and selects are 44 px taps, as tall as the buttons beside them: Songs'
// Filter next to Play all, Settings › Library's folder field, Settings › Playback's renderer
// address, Add to playlist…'s search, the API token field. A textarea keeps its rows, a tag field
// stays chip-sized, a small button beside a field is as tall as it, and the desktop layout keeps
// its 40 px fields.
// Real candidate assets, isolated responses: node tools/ui-phone-input-targets.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-input-targets-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };
const fixtures = {
  '/library/folders': { body: [] },
  '/library/webdav': { body: [] },
  '/library/drives': { body: { providers: [], roots: [] } },
  '/library/hidden': { body: [] },
};
// (enough songs that Songs offers its Filter)
const tracks = Array.from({ length: 30 }, (_, i) => ({ ...fx.track, id: i + 1, title: `Song ${i + 1}` }));
const box = (page, sel) => page.locator(sel).first().evaluate((e) => { const b = e.getBoundingClientRect(); return { w: b.width, h: b.height, top: b.top }; });

// A textarea, a tag field and a checkbox with the same classes as the app's keep their sizes.
const extras = (page) => page.evaluate(() => {
  const host = document.createElement('div'); host.id = 'fxextras';
  host.innerHTML = '<textarea class="input" rows="1"></textarea><input class="input taginput"><input class="input" type="checkbox">'
    + '<span class="row" style="gap:6px;display:inline-flex"><input class="input" style="width:180px"><button class="btn sm primary" type="button">Save</button></span>';
  document.querySelector('#main').append(host);
  const [ta, tag, cb] = [...host.children].slice(0, 3).map((e) => e.getBoundingClientRect().height);
  const [field, btn] = [...host.querySelectorAll('.row > *')].map((e) => e.getBoundingClientRect().height);
  host.remove();
  return { ta, tag, cb, field, btn };
});

async function phone(browser, width, android) {
  const { context, page, state } = await fx.create(browser, width, { mobile: true, fixtures, tracks, collections: [fx.summary] });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
  const at = `${width}${android ? ' Android' : ''}`;
  await fx.go(page, '/tracks');
  await page.waitForSelector('#tracklist .track');
  const f = await box(page, '#filter');
  const play = await box(page, '#main .btn:has-text("Play all")');
  check(f.h >= 44, `${at} Songs: Filter is a 44 px tap (${f.h.toFixed(1)})`);
  check(Math.abs(f.h - play.h) <= 1 && Math.abs(f.top - play.top) <= 1, `${at} Songs: Filter is as tall as Play all beside it (${f.h.toFixed(1)} / ${play.h.toFixed(1)})`);
  await page.screenshot({ path: path.join(SHOTS, `${width}${android ? '-android' : ''}-songs.png`) });
  // Add to playlist…'s search field, in a dialog
  await page.locator('#tracklist .track:first-child [data-act="more"]').click({ force: true });
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to collection…' }).click();
  await page.waitForSelector('dialog #ppq');
  const q = await box(page, 'dialog #ppq');
  check(q.h >= 44, `${at} Add to collection…: its search field is a 44 px tap (${q.h.toFixed(1)})`);
  await page.screenshot({ path: path.join(SHOTS, `${width}${android ? '-android' : ''}-add-to-playlist.png`) });
  await page.keyboard.press('Escape');
  for (const [route, sel, what] of [['/settings/library', '#folderadd', 'Settings › Library: Folder to add'], ['/settings/playback', '#rendaddr', 'Settings › Playback: Renderer address']]) {
    await fx.go(page, route);
    await page.waitForSelector(sel);
    const b = await box(page, sel);
    check(b.h >= 44, `${at} ${what} is a 44 px tap (${b.h.toFixed(1)})`);
    await page.screenshot({ path: path.join(SHOTS, `${width}${android ? '-android' : ''}${route.replace(/\//g, '-')}.png`) });
  }
  // The API token field (a .field input without .input)
  const tok = await page.evaluate(() => { const e = document.querySelector('#tok'); const p = document.querySelector('#loginpage'); const d = document.querySelector('#logintoken'); p.hidden = false; d.open = true; const h = e.getBoundingClientRect().height; d.open = false; p.hidden = true; return h; });
  check(tok >= 44, `${at} the API token field is a 44 px tap (${tok.toFixed(1)})`);
  const x = await extras(page);
  check(x.ta < 44 && x.tag < 44 && x.cb < 30, `${at} a one-row textarea, a tag field and a checkbox keep their sizes (${x.ta.toFixed(1)}, ${x.tag.toFixed(1)}, ${x.cb.toFixed(1)})`);
  check(x.field >= 44 && Math.abs(x.btn - x.field) <= 1, `${at} a small button beside a field (Home's city Save, AcoustID's Check) is as tall as it (${x.btn.toFixed(1)} / ${x.field.toFixed(1)})`);
  const over = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
  check(over <= 0, `${at} no sideways overflow (${over})`);
  check(!state.errors.length, `${at} no page errors (${state.errors.join(' | ')})`);
  await context.close();
}

async function desktop(browser) {
  const { context, page } = await fx.create(browser, 1280, { fixtures, tracks });
  await fx.go(page, '/tracks');
  await page.waitForSelector('#tracklist .track');
  const f = await box(page, '#filter');
  check(f.h < 44, `1280 Songs: Filter keeps its desktop size (${f.h.toFixed(1)})`);
  await fx.go(page, '/settings/library');
  await page.waitForSelector('#folderadd');
  const b = await box(page, '#folderadd');
  check(b.h < 44, `1280 Settings › Library: Folder to add keeps its desktop size (${b.h.toFixed(1)})`);
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try {
    await phone(browser, 360, true);
    await phone(browser, 412, false);
    await desktop(browser);
  } finally {
    await browser.close();
  }
  console.log(`${passed} ok, ${failed} failed`);
  if (failed) process.exit(1);
})().catch((e) => { console.error(e); process.exit(1); });
