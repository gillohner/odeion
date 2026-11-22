// A keyboard user must see which row or side-menu item they are on. Lists cut what is drawn outside
// them (overflow: hidden, for their round corners; a picker scrolls) and the side menu scrolls, so
// the focus ring, drawn 2 px outside, lost its sides: on Artists, Playlists and Add to playlist only
// a line under the row was left, and the side menu's ring lost its left and right edges.
// The ring is now drawn inside these rows. Checked by geometry (no clipping box cuts the ring) and
// in a real screenshot (the ring's colour on the row's left and right edges).
// Real UI in Chromium against fixtures (no node), 1280, 1024 and 768 px, plus 360/412 px phones
// (square song lists stay square).
//   node tools/ui-list-focus-ring.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'list-focus-ring-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const A = [{ id: 11, name: 'Sefa' }, { id: 22, name: 'Dr. Peacock' }, { id: 33, name: 'Angerfist' }];
const collections = Array.from({ length: 12 }, (_, i) => ({ ...fx.summary, id: `c${i + 1}`, name: `Collection ${i + 1}` }));
const fixtures = { '/library/artists': { body: A.map((a) => ({ ...a, albums: 1, tracks: 4 })) } };

// The focused element's ring against every box above it that cuts what overflows it
const ringReport = () => {
  const e = document.activeElement; const cs = getComputedStyle(e);
  const grow = (parseFloat(cs.outlineWidth) || 0) + (parseFloat(cs.outlineOffset) || 0);
  const r = e.getBoundingClientRect();
  const ring = { t: r.top - grow, b: r.bottom + grow, l: r.left - grow, r: r.right + grow };
  const cut = [];
  for (let a = e.parentElement; a && a !== document.documentElement; a = a.parentElement) {
    const s = getComputedStyle(a);
    if (s.overflowX === 'visible' && s.overflowY === 'visible') continue;
    const c = a.getBoundingClientRect();
    const box = { t: c.top + parseFloat(s.borderTopWidth), b: c.bottom - parseFloat(s.borderBottomWidth), l: c.left + parseFloat(s.borderLeftWidth), r: c.right - parseFloat(s.borderRightWidth) };
    const name = `${a.tagName.toLowerCase()}${a.id ? '#' + a.id : ''}.${[...a.classList].join('.')}`;
    // (only beside a side of the element that itself shows: a row scrolled half out is not a cut ring)
    if (r.left >= box.l - 0.5 && ring.l < box.l - 0.5) cut.push(`${name} left`);
    if (r.right <= box.r + 0.5 && ring.r > box.r + 0.5) cut.push(`${name} right`);
    if (r.top >= box.t - 0.5 && ring.t < box.t - 0.5) cut.push(`${name} top`);
    if (r.bottom <= box.b + 0.5 && ring.b > box.b + 0.5) cut.push(`${name} bottom`);
  }
  return { fv: e.matches(':focus-visible'), style: cs.outlineStyle, width: parseFloat(cs.outlineWidth) || 0, color: cs.outlineColor, ring, cut, label: (e.getAttribute('aria-label') || e.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 40) };
};

// Columns of the ring's colour in a thin strip just inside the row's left and right edges (the
// middle of the row's height, away from round corners)
async function paintedSides(page, rep) {
  const want = rep.color.match(/\d+(\.\d+)?/g).slice(0, 3).map(Number);
  const box = await page.evaluate(() => { const r = document.activeElement.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; });
  const y = Math.floor(box.y + box.h / 2 - 3);
  const strip = async (x) => {
    const buf = await page.screenshot({ clip: { x: Math.max(0, x), y, width: 4, height: 6 } });
    return page.evaluate(async ([b64, want]) => {
      const img = new Image(); img.src = `data:image/png;base64,${b64}`; await img.decode();
      const c = document.createElement('canvas'); c.width = img.width; c.height = img.height;
      const g = c.getContext('2d'); g.drawImage(img, 0, 0);
      const d = g.getImageData(0, 0, c.width, c.height).data; let n = 0;
      for (let i = 0; i < d.length; i += 4) if (Math.abs(d[i] - want[0]) + Math.abs(d[i + 1] - want[1]) + Math.abs(d[i + 2] - want[2]) < 60) n++;
      return n;
    }, [buf.toString('base64'), want]);
  };
  return { left: await strip(Math.floor(box.x)), right: await strip(Math.ceil(box.x + box.w) - 4) };
}

// Focus `sel` the way the keyboard does (Tab from the element before it), so :focus-visible applies
async function keyTo(page, sel) {
  const loc = page.locator(sel).first();
  await loc.waitFor();
  await loc.focus(); await page.keyboard.press('Shift+Tab'); await page.keyboard.press('Tab');
  return page.evaluate((s) => document.activeElement === document.querySelector(s) || document.activeElement.matches(s), sel);
}

// The end rows of a round list take its corners only while ringed (the ring follows them); at rest
// they stay square, so a row moved mid-list (queue drag) shows no round corner
const corners = (page, sel) => page.evaluate((s) => {
  const e = document.querySelector(s); const l = e.closest('.list');
  const r = getComputedStyle(e); const lr = parseFloat(getComputedStyle(l).borderTopLeftRadius) || 0;
  return { list: lr, tl: parseFloat(r.borderTopLeftRadius) || 0, bl: parseFloat(r.borderBottomLeftRadius) || 0 };
}, sel);

async function expectRing(page, sel, what, shot) {
  const on = await keyTo(page, sel);
  const rep = await page.evaluate(ringReport);
  const paint = await paintedSides(page, rep);
  await page.screenshot({ path: path.join(SHOTS, `${shot}.png`) });
  check(on && rep.fv && rep.style !== 'none' && rep.width >= 2, `${what}: the keyboard puts a ring on it (${rep.label}; ${rep.style} ${rep.width}px${rep.fv ? '' : ', not focus-visible'})`);
  check(rep.cut.length === 0, `${what}: no box cuts the ring${rep.cut.length ? ` (cut: ${rep.cut.join(', ')})` : ''}`);
  check(paint.left > 0 && paint.right > 0, `${what}: the ring shows on the row's left and right edges (${paint.left}/${paint.right} px of its colour)`);
}

(async () => {
  const browser = await chromium.launch();
  for (const w of [1280, 1024, 768]) {
    const { page, context, state } = await fx.create(browser, w, { fixtures, collections });
    await fx.go(page, '/artists');
    await page.waitForSelector('#artistlist .list-row');
    // the side menu: the page's own item (marked) and another
    await expectRing(page, '.nav-item[data-view="artists"]', `${w} side menu, the open page`, `nav-on-${w}`);
    await expectRing(page, '.nav-item[data-view="tracks"]', `${w} side menu, another page`, `nav-${w}`);
    // Artists: the first row (round top corners), a middle row and the last row
    await expectRing(page, '#artistlist .list-row:first-child', `${w} Artists, first row`, `artists-first-${w}`);
    await expectRing(page, '#artistlist .list-row:nth-child(2)', `${w} Artists, a middle row`, `artists-mid-${w}`);
    await expectRing(page, '#artistlist .list-row:last-child', `${w} Artists, last row`, `artists-last-${w}`);
    const lastOn = await corners(page, '#artistlist .list-row:last-child');
    check(lastOn.list > 2 && Math.abs(lastOn.bl - (lastOn.list - 1)) < 0.5, `${w} Artists, last row: while ringed it takes the list's round corners (${lastOn.bl}px in a ${lastOn.list}px list)`);
    await keyTo(page, '#artistlist .list-row:first-child');
    const firstOn = await corners(page, '#artistlist .list-row:first-child');
    check(Math.abs(firstOn.tl - (firstOn.list - 1)) < 0.5, `${w} Artists, first row: while ringed it takes the list's round corners (${firstOn.tl}px)`);
    await keyTo(page, '#artistlist .list-row:nth-child(2)');
    const firstOff = await corners(page, '#artistlist .list-row:first-child');
    check(firstOff.tl === 0, `${w} Artists, first row: square again once the ring moves on (${firstOff.tl}px)`);
    // the rows still open the artist (the ring is the only change)
    await keyTo(page, '#artistlist .list-row:first-child');
    await page.keyboard.press('Enter');
    await page.waitForURL(/\/artist\//, { timeout: 5000 }).catch(() => {});
    check(/\/artist\//.test(page.url()), `${w} Artists: Enter on the ringed row still opens the artist (${new URL(page.url()).pathname})`);
    // Playlists: a row of the list
    await fx.go(page, '/playlists');
    await page.waitForSelector('.main .list .list-row.clickable');
    await expectRing(page, '.main .list .list-row.clickable', `${w} Collections, first row`, `playlists-${w}`);
    // Add to playlist (a scrolling picker): New playlist, then a playlist
    await fx.go(page, '/tracks');
    await page.click('#tracklist .track:first-child [data-act="more"]', { force: true });
    await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Add to collection…' }).click();
    await page.waitForSelector('dialog #pplist [data-ppi="1"]');
    await expectRing(page, 'dialog #pplist [data-ppnew]', `${w} Add to collection, New collection`, `picker-new-${w}`);
    await expectRing(page, 'dialog #pplist [data-ppi="1"]', `${w} Add to collection, a collection`, `picker-row-${w}`);
    check(state.errors.length === 0, `${w} no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
    await context.close();
  }
  // Phone: song lists are square, edge to edge; their end rows stay square, ringed or not (a round
  // row there would show the page's colour in its corners)
  for (const w of [360, 412]) {
    const { page, context, state } = await fx.create(browser, w, { mobile: true, collections });
    await fx.go(page, '/playlist/c1');
    await page.waitForSelector('.list > .srow');
    const sel = '.list:has(> .srow) > .srow:first-child';
    const off = await corners(page, sel);
    await page.evaluate((s) => { const e = document.querySelector(s); if (e.tabIndex < 0) e.tabIndex = 0; }, sel);
    await keyTo(page, sel);
    const on = await corners(page, sel);
    const fv = await page.evaluate((s) => document.querySelector(s).matches(':focus-visible'), sel);
    await page.screenshot({ path: path.join(SHOTS, `phone-songs-${w}.png`) });
    check(off.list === 0 && off.tl === 0 && on.tl === 0 && on.bl === 0 && fv, `phone ${w}: a song list stays square, its first row too, with the ring on it (list ${off.list}px, row ${off.tl}/${on.tl}px${fv ? '' : ', not focus-visible'})`);
    check(state.errors.length === 0, `phone ${w} no page errors${state.errors.length ? ': ' + state.errors.join(' | ') : ''}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
