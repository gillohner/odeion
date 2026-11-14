// On a phone the buttons in an action row (a playlist's Play, Shuffle, Play on…, Share and ⋯; the
// Songs page's Play all) are 44 px taps and a playlist's row stays one row at 360 px, as an album's
// does, also at 320 px and with a larger font size; a checkbox's whole line (Import's Roon options) is
// a 44 px tap. A playlist has the album page's compact header (the cover beside its name), so its first
// song shows on the first screen. A queue row's Move up / Move down / Remove are in its ⋯ (no hidden
// buttons a finger could press unseen), and its drag grip is a 44 px tap. Desktop is unchanged.
// Real candidate assets, isolated responses: node tools/ui-phone-action-rows.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-action-rows-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// An album page from the library, to compare with: its row keeps its own 48 px buttons.
const album = { kind: 'album', id: 'lib-7', title: 'Journey', section: 'albums', source: 'Your library', source_id: 'library', target: { album_id: 7 }, uri: null, song_uri: null, link: null,
  songs: [{ id: 'lib-1', track_id: 1, title: 'Journey Through Time', artist: 'Sefa', duration_ms: 240000, no: 1 }], albums: [], versions: [], also_on: [], about: [], open_on: [], connect: [],
  artist: { id: 'lib-2', name: 'Sefa' }, year: 2024, on_pubky: null };
const fixtures = { '/item/album/lib-7': { body: album } };
const ICONS = ['plzone', 'plshare', 'plunshare', 'plcopylink', 'plmore'];

// The visible buttons of the first action row under `scope`: box, words, and whether they take a
// second row (a button starting 20 px or more below the first; heights differ by a few px).
const row = (page, scope) => page.evaluate(([scope, icons]) => {
  const bs = [...document.querySelectorAll(`${scope} .actions > .btn`)].filter((b) => b.getClientRects().length);
  const r = bs.map((b) => { const x = b.getBoundingClientRect(); return { id: b.id, w: x.width, h: x.height, top: x.top, right: x.right, text: b.textContent.trim(), name: b.getAttribute('aria-label') || b.textContent.trim(), icon: icons.includes(b.id), shufIcon: b.id === 'plshuffle' ? getComputedStyle(b.querySelector('.ic')).display : null }; });
  const first = Math.min(...r.map((b) => b.top));
  return { buttons: r, rows: r.some((b) => b.top > first + 20) ? 2 : 1, sw: document.documentElement.scrollWidth, vw: innerWidth };
}, [scope, ICONS]);
const fmt = (r) => r.buttons.map((b) => `${b.id || b.text}:${b.w.toFixed(0)}×${b.h.toFixed(0)}`).join(' ');

async function phone(browser, width, android) {
  const at = `${android ? 'android' : 'browser'} ${width}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures: { ...fixtures } });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  state.collections = [fx.summary];

  // A playlist: Play, Shuffle, Play on…, Share on Pubky and ⋯, all 44 px, on one row
  await fx.go(page, '/playlist/c1'); await page.waitForSelector('#plsongs .srow'); await page.waitForTimeout(300);
  const pl = await row(page, '#main > .hero');
  check(pl.buttons.length === 5 && pl.buttons.every((b) => b.h >= 44), `${at}: a playlist's five buttons are 44 px tall (${fmt(pl)})`);
  check(pl.buttons.filter((b) => b.icon).length === 3 && pl.buttons.filter((b) => b.icon).every((b) => b.w >= 44), `${at}: its icon buttons (Play on…, Share, ⋯) are 44 px wide`);
  check(pl.rows === 1 && pl.sw <= pl.vw, `${at}: they stay on one row, ⋯ included (${pl.rows} rows, page ${pl.sw} of ${pl.vw})`);
  const shuf = pl.buttons.find((b) => b.id === 'plshuffle');
  check(shuf && shuf.text === 'Shuffle' && shuf.shufIcon === (width < 380 ? 'none' : 'block'), `${at}: Shuffle keeps its word${width < 380 ? ', not its icon (as on an album)' : ' and its icon'} (icon ${shuf && shuf.shufIcon})`);
  // its sharing note keeps some air under the row
  const gap = await page.evaluate(() => { const a = document.querySelector('#main > .hero .actions'); const p = a.nextElementSibling; return p ? p.getBoundingClientRect().top - Math.max(...[...a.children].filter((b) => b.getClientRects().length).map((b) => b.getBoundingClientRect().bottom)) : null; });
  check(gap != null && gap >= 6, `${at}: the sharing note sits ${gap != null ? gap.toFixed(1) : '?'} px under the row (at least 6)`);
  await page.screenshot({ path: path.join(SHOTS, `playlist-${android ? 'android' : 'browser'}-${width}.png`) });
  // The album page's compact header: a 112 px cover beside the kind, name and count, the row under it, and
  // on a 360x640 / 412x732 phone the first song starts on the first screen (it started at y 634 under the
  // 180 px cover on a row of its own, below the visible area at 519 / 611)
  await page.setViewportSize({ width, height: width === 360 ? 640 : 732 }); await page.waitForTimeout(200);
  const hd = await page.evaluate(() => { const b = (q) => { const e = document.querySelector(q); if (!e) return null; const r = e.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, w: r.width }; }; const s = document.querySelector('#plsongs .srow'); return { art: b('#main > .hero > .art'), h1: b('#main > .hero h1'), eyebrow: b('#main > .hero .eyebrow'), sub: b('#main > .hero .sub'), play: b('#plplay'), note: b('#main > .hero .actions + p.muted'), main: b('#main'), song: s && b('#plsongs .srow'), songName: s && s.querySelector('.name').textContent.trim() }; });
  const beside = hd.art && [hd.eyebrow, hd.h1, hd.sub].every((e) => e && e.left >= hd.art.right + 8 && e.top < hd.art.bottom);
  check(hd.art && Math.round(hd.art.w) === 112 && beside, `${at}: a playlist's header is compact, its kind, name and count beside its 112 px cover (cover ${hd.art && hd.art.w.toFixed(0)} px, name at x ${hd.h1 && hd.h1.left.toFixed(0)})`);
  check(hd.play && hd.play.top >= hd.art.bottom && hd.note && Math.abs(hd.note.left - hd.art.left) < 1 && hd.note.top > hd.play.bottom, `${at}: Play's row and the sharing note sit under the cover, full width`);
  check(hd.song && hd.song.top < hd.main.bottom, `${at}: its first song ("${hd.songName}") starts on the first screen (y ${hd.song && hd.song.top.toFixed(0)}, visible to ${hd.main.bottom.toFixed(0)})`);
  await page.screenshot({ path: path.join(SHOTS, `playlist-header-${android ? 'android' : 'browser'}-${width}.png`) });
  await page.setViewportSize({ width, height: 900 }); await page.waitForTimeout(100);
  // still one tap: ⋯ opens the playlist's menu
  await page.locator('#plmore').tap(); await page.waitForTimeout(200);
  check(await page.locator('.popover.menu:not([hidden])').count() === 1, `${at}: one tap on ⋯ opens the playlist's menu`);
  await page.keyboard.press('Escape');

  // A public playlist (no Pubky sign-in here, so no Copy link): Make private takes Share's place, the
  // same size, the row still one row. With Copy link too (six buttons) the row takes two, as on main.
  const pub = { ...fx.summary, visibility: 'public', synced: true };
  state.fixtures['/playlists/c1'] = { body: { ...fx.detail, summary: pub } }; state.collections = [pub];
  await fx.go(page, '/playlists'); await fx.go(page, '/playlist/c1'); await page.waitForSelector('#plunshare'); await page.waitForTimeout(300);
  const pp = await row(page, '#main > .hero');
  check(pp.buttons.length === 5 && pp.buttons.every((b) => b.h >= 44) && pp.rows === 1, `${at}: a public playlist's row too (${fmt(pp)}, ${pp.rows} rows)`);
  delete state.fixtures['/playlists/c1']; state.collections = [fx.summary];

  // An album keeps its 48 px row
  await fx.go(page, '/album/lib-7'); await page.waitForSelector('#iplay'); await page.waitForTimeout(300);
  const al = await row(page, '#main > .hero');
  check(al.buttons.length >= 4 && al.buttons.every((b) => b.h >= 48) && al.rows === 1, `${at}: an album's row keeps its 48 px buttons (${fmt(al)})`);

  // Songs: Play all is 44 px, as Add music beside it
  await fx.go(page, '/tracks'); await page.waitForSelector('#shuffle'); await page.waitForTimeout(300);
  const so = await row(page, '#main .page-head');
  const all = so.buttons.find((b) => b.id === 'shuffle');
  check(all && all.h >= 44, `${at}: Songs' Play all is 44 px tall (${fmt(so)})`);

  // Import › Move from Roon: each option's line is a 44 px tap, and a press 40 px below its top ticks it
  await fx.go(page, '/library/import'); await page.waitForSelector('#roonfav'); await page.waitForTimeout(300);
  const boxes = await page.evaluate(() => [...document.querySelectorAll('#main label.check:has(#roonfav, #roonpl, #roontags)')].map((l) => { l.scrollIntoView({ block: 'center', behavior: 'instant' }); const b = l.getBoundingClientRect(); return { id: l.querySelector('input').id, h: b.height, top: b.top, bottom: b.bottom }; }));
  check(boxes.length === 3 && boxes.every((b) => b.h >= 44), `${at}: Roon's three options are 44 px lines (${boxes.map((b) => `${b.id}:${b.h.toFixed(0)}`).join(' ')})`);
  for (const id of ['roonfav', 'roontags']) {
    const before = await page.locator(`#${id}`).isChecked();
    const b = await page.evaluate((id) => { const l = document.querySelector(`#${id}`).closest('label'); l.scrollIntoView({ block: 'center', behavior: 'instant' }); const r = l.getBoundingClientRect(); return { x: r.left + 120, y: r.top + 40, hit: document.elementFromPoint(r.left + 120, r.top + 40) === l }; }, id);
    await page.mouse.click(b.x, b.y);
    const after = await page.locator(`#${id}`).isChecked();
    check(b.hit && after === !before, `${at}: a press 40 px below the top of "${id}"'s line ticks it (${before} → ${after})`);
  }
  await page.screenshot({ path: path.join(SHOTS, `import-${android ? 'android' : 'browser'}-${width}.png`), fullPage: true });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at}: Import fits the screen`);
  await context.close();
}

// A playlist's row on a narrower phone (320 px: Android's larger display size on a 360 px phone; 277 and
// 313 px: Android's font size as Chrome's page zoom) or with a larger font size (Android's 115 % and 130 %): Shuffle drops its icon, then the gaps and Play's and
// Shuffle's sides tighten, then Play drops its icon, and ⋯ stays on the row. Every button keeps its 44 px;
// at 360 and 412 px with the normal font size the row is as before. The font size is checked two ways:
// every font size scaled inline (em grows with it), and as the app's WebView applies it, as text zoom (the
// text grows, em does not; text-size-adjust takes the same path in Chromium).
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });
// 10em and 10ch in the playlist row's container, in px
const units = (page) => page.evaluate(() => { const h = document.querySelector('#main > .hero:has(> div > .actions > #plplay)'); const w = (v) => { const p = document.createElement('div'); p.style.cssText = `display:block;position:absolute;visibility:hidden;height:0;width:${v}`; h.append(p); const r = p.getBoundingClientRect().width; p.remove(); return r; }; return { em: w('10em'), ch: w('10ch') }; });
const scale = (page, k) => page.evaluate((k) => { const all = [document.documentElement, ...document.querySelectorAll('body, body *')]; const fs = all.map((e) => parseFloat(getComputedStyle(e).fontSize)); all.forEach((e, i) => { e.style.fontSize = `${fs[i] * k}px`; }); }, k);
async function narrow(browser, width, android, k, zoom) {
  const at = `${android ? 'android' : 'browser'} ${width} at ${Math.round(k * 100)} %${zoom ? ' text zoom' : ''}`;
  const { page, context, state } = await fx.create(browser, width, { mobile: true });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  state.collections = [fx.summary];
  await fx.go(page, '/playlist/c1'); await page.waitForSelector('#plsongs .srow'); await page.waitForTimeout(300);
  const units0 = zoom && await units(page);
  if (k !== 1) { await (zoom ? textZoom : scale)(page, k); await page.waitForTimeout(100); }
  // text zoom is what the steps are written for: the text and ch grow with it, em stays as it was (if a later
  // Chromium scaled em too, these cases would no longer test the app's path)
  if (zoom) {
    const u = await units(page);
    check(u.em === units0.em && Math.abs(u.ch / units0.ch - k) < 0.05, `${at}: em stays (10em ${units0.em} → ${u.em} px) and ch grows with the text (10ch ${units0.ch} → ${u.ch} px)`);
  }
  const r = await row(page, '#main > .hero');
  const look = await page.evaluate(() => { const ic = (id) => getComputedStyle(document.querySelector(`#${id} > .ic`)).display !== 'none'; return { fs: parseFloat(getComputedStyle(document.querySelector('#plplay')).fontSize), gap: parseFloat(getComputedStyle(document.querySelector('#main > .hero .actions')).columnGap), play: ic('plplay'), shuffle: ic('plshuffle'), pad: parseFloat(getComputedStyle(document.querySelector('#plplay')).paddingLeft), shufPad: parseFloat(getComputedStyle(document.querySelector('#plshuffle')).paddingLeft) }; });
  check(Math.abs(look.fs - 14 * k) < 0.1, `${at}: Play's words are ${look.fs.toFixed(1)} px (the scaled size held)`);
  check(r.buttons.length === 5 && r.rows === 1 && r.sw <= r.vw, `${at}: a playlist's five buttons stay on one row, ⋯ included (${fmt(r)}, ${r.rows} rows, page ${r.sw} of ${r.vw})`);
  check(r.buttons.every((b) => b.h >= 44 && b.w >= 44), `${at}: every button is still a 44 px tap`);
  check(r.buttons.find((b) => b.id === 'plplay').text === 'Play' && r.buttons.find((b) => b.id === 'plshuffle').text === 'Shuffle', `${at}: Play and Shuffle keep their words`);
  // what gives way, in order: [the row's gap, Play's icon shown, Shuffle's icon shown]; Play's and Shuffle's
  // sides follow the gap. The normal font size at 360/412 px is the row as before.
  const want = { 277: { 1: [2, false] }, 300: { 1.3: [2, false] }, 313: { 1: [4, false] }, 320: { 1: [4, true], 1.15: [4, false], 1.3: [4, false] }, 340: { 1: [4, true], 1.15: [4, true], 1.3: [4, false] }, 360: { 1: [8, true], 1.15: [4, true], 1.3: [4, true] }, 385: { 1.3: [8, true, false] }, 412: { 1: [8, true], 1.15: [8, true], 1.3: [8, true] } }[width][k];
  const shuffle = want[2] ?? width >= 380;
  const pad = { 2: 6, 4: 10, 8: 14 }[want[0]];
  check(look.gap === want[0] && look.play === want[1] && look.shuffle === shuffle && look.pad === pad && look.shufPad === pad, `${at}: gap ${look.gap} px (want ${want[0]}), Play's and Shuffle's sides ${look.pad}/${look.shufPad} px (want ${pad}), Play's icon ${look.play ? 'shown' : 'hidden'} (want ${want[1] ? 'shown' : 'hidden'}), Shuffle's icon ${look.shuffle ? 'shown' : 'hidden'} (want ${shuffle ? 'shown' : 'hidden'})`);
  if (k !== 1.15) await page.locator('#main > .hero').screenshot({ path: path.join(SHOTS, `playlist-row-${android ? 'android' : 'browser'}-${width}-${Math.round(k * 100)}${zoom ? '-textzoom' : ''}.png`) });
  // still one tap: ⋯ opens the playlist's menu
  await page.locator('#plmore').tap(); await page.waitForTimeout(200);
  check(await page.locator('.popover.menu:not([hidden])').count() === 1, `${at}: one tap on ⋯ opens the playlist's menu`);
  await context.close();
}

// The Queue page: a row shows what a finger can press (the grip, Play from here, ⋯) and nothing else; Move
// up / Move down / Remove are in the row's ⋯ (they were invisible until hovered, yet tappable, and Remove
// had no visible way at all). The grip and ⋯ are 44 px taps, and the row keeps its height.
const QUEUE = ['Journey Through Time', 'Rambling On My Mind', 'The Beginning', 'Last One Standing'].map((title, i) => ({ uri: `file:///music/${i}.flac`, title, artist: 'Sefa', album: 'Journey', duration_ms: 240000 + i * 1000 }));
// what a tap at each x along the row's middle line presses (grip, play, more or -), as runs
const qrow = (page, n) => page.evaluate((n) => {
  const row = document.querySelectorAll('.list.queue .list-row')[n]; const rb = row.getBoundingClientRect(); const y = rb.top + rb.height / 2;
  const who = (x, yy) => { const e = document.elementFromPoint(x, yy); const c = e && row.contains(e) && e.closest('[data-qgrip], [data-qi], [data-qmore], .q-actions button'); return !c ? '-' : c.dataset.qgrip != null ? 'grip' : c.dataset.qi != null ? 'play' : c.dataset.qmore != null ? 'more' : 'hidden'; };
  const runs = {}; for (let x = Math.ceil(rb.left); x < rb.right; x++) { const w = who(x, y); runs[w] = (runs[w] || 0) + 1; }
  const tall = (q) => { const b = row.querySelector(q).getBoundingClientRect(); let h = 0; for (let yy = Math.floor(rb.top); yy < rb.bottom; yy++) if (who(b.left + b.width / 2, yy) === (q === '[data-qgrip]' ? 'grip' : 'more')) h++; return h; };
  const acts = row.querySelector('.q-actions');
  return { h: rb.height, runs, gripH: tall('[data-qgrip]'), moreH: row.querySelector('[data-qmore]')?.getClientRects().length ? tall('[data-qmore]') : 0,
    acts: acts.getClientRects().length > 0, title: row.querySelector('.grow').getBoundingClientRect().width,
    moreName: row.querySelector('[data-qmore]')?.getAttribute('aria-label'), sw: document.documentElement.scrollWidth, vw: innerWidth };
}, n);
const names = (page) => page.locator('.list.queue .name').allInnerTexts();
const sheet = async (page, n) => { await page.locator('.list.queue .list-row').nth(n).locator('[data-qmore]').tap(); await page.locator('.popover.menu:not([hidden]) .pop-item').first().waitFor(); return (await page.locator('.popover.menu:not([hidden]) .pop-item').allInnerTexts()).map((t) => t.trim()); };
async function queue(browser, width, android) {
  const at = `${android ? 'android' : 'browser'} ${width} queue`;
  const { page, context } = await fx.create(browser, width, { mobile: true });
  await context.addInitScript((q) => { localStorage.setItem('pm.browserQueue', JSON.stringify(q)); }, QUEUE);
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  await fx.go(page, '/queue'); await page.waitForSelector('.list.queue .list-row'); await page.waitForTimeout(300);
  for (const n of [0, 2]) {
    const r = await qrow(page, n);
    check(!r.acts && !r.runs.hidden, `${at}: row ${n + 1} has no hidden Move/Remove buttons a finger could press unseen (${JSON.stringify(r.runs)})`);
    check(r.runs.grip >= 44 && r.gripH >= 44, `${at}: row ${n + 1}'s drag grip is a 44 px tap (${r.runs.grip}×${r.gripH})`);
    check(r.runs.more >= 44 && r.moreH >= 44, `${at}: row ${n + 1}'s ⋯ is a 44 px tap (${r.runs.more || 0}×${r.moreH}), named "${r.moreName}"`);
    check(r.runs.play >= 44, `${at}: row ${n + 1}'s Play from here is a 44 px tap (${r.runs.play || 0} wide)`);
    check(r.h <= 52 && r.title >= (width === 360 ? 130 : 180) && r.sw <= r.vw, `${at}: row ${n + 1} keeps its height (${r.h.toFixed(1)} px) and its title has more room (${r.title.toFixed(0)} px); page ${r.sw} of ${r.vw}`);
  }
  await page.screenshot({ path: path.join(SHOTS, `queue-${android ? 'android' : 'browser'}-${width}.png`) });
  if (!(await page.locator('.list.queue [data-qmore]').first().isVisible())) { check(false, `${at}: a row has a ⋯ to open`); await context.close(); return; }
  // ⋯ offers only what applies: the first row can't move up, the last can't move down
  const first = await sheet(page, 0); await page.keyboard.press('Escape'); await page.waitForTimeout(150);
  const last = await sheet(page, 3); await page.keyboard.press('Escape'); await page.waitForTimeout(150);
  check(first.join('|') === 'Move down|Remove from queue' && last.join('|') === 'Move up|Remove from queue', `${at}: ⋯ on the first row offers ${first.join(', ')}; on the last ${last.join(', ')}`);
  // two taps: ⋯ → Remove from queue
  const mid = await sheet(page, 1);
  check(mid.join('|') === 'Move up|Move down|Remove from queue', `${at}: ⋯ on a middle row offers ${mid.join(', ')}`);
  await page.screenshot({ path: path.join(SHOTS, `queue-menu-${android ? 'android' : 'browser'}-${width}.png`) });
  await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Remove from queue' }).tap(); await page.waitForFunction((n) => document.querySelectorAll('.list.queue .list-row').length === n, QUEUE.length - 1, { timeout: 5000 }).catch(() => {});
  let got = await names(page);
  check(got.join('|') === [QUEUE[0], QUEUE[2], QUEUE[3]].map((x) => x.title).join('|'), `${at}: ⋯ → Remove from queue takes that song out (${got.join(' / ')})`);
  await sheet(page, 0); await page.locator('.popover.menu:not([hidden]) .pop-item', { hasText: 'Move down' }).tap(); await page.waitForFunction((t) => document.querySelector('.list.queue .name')?.textContent.trim() !== t, QUEUE[0].title, { timeout: 5000 }).catch(() => {});
  got = await names(page);
  check(got.join('|') === [QUEUE[2], QUEUE[0], QUEUE[3]].map((x) => x.title).join('|'), `${at}: ⋯ → Move down moves it one place (${got.join(' / ')})`);
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) await phone(browser, width, android);
    for (const width of [360, 412]) for (const android of [false, true]) await queue(browser, width, android);
    for (const width of [320, 340, 360, 412]) for (const k of [1, 1.15, 1.3]) for (const android of [false, true]) await narrow(browser, width, android, k);
    // Android's font size as page zoom (Chrome) on a 360 px phone lays the page out narrower: 130 % 277 px, 115 % 313 px
    for (const width of [277, 313]) for (const android of [false, true]) await narrow(browser, width, android, 1);
    // Android's font size as the app's WebView applies it, as text zoom: the same steps as above
    const zoomed = await chromium.launch({ args: ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'] });
    try {
      for (const width of [320, 340, 360, 412]) for (const k of [1.15, 1.3]) await narrow(zoomed, width, true, k, true);
      // a wide phone at 130 %: only Shuffle's icon gives way
      await narrow(zoomed, 385, true, 1.3, true);
      // a narrow phone (300 px) at 130 %: the fourth step, as for the 277 px page zoom above
      await narrow(zoomed, 300, true, 1.3, true);
    } finally { await zoomed.close(); }
    // Desktop with a mouse: the same rows as before (no 44 px rule, Roon options stay one line of text)
    const { page, context, state } = await fx.create(browser, 1280, {});
    state.collections = [fx.summary];
    await fx.go(page, '/playlist/c1'); await page.waitForSelector('#plsongs .srow'); await page.waitForTimeout(300);
    const d = await row(page, '#main > .hero');
    check(d.buttons.length >= 5 && d.buttons.every((b) => b.h < 44) && d.rows === 1, `desktop 1280: a playlist's row is unchanged (${fmt(d)})`);
    const dart = await page.evaluate(() => { const a = document.querySelector('#main > .hero > .art').getBoundingClientRect(); const h = document.querySelector('#main > .hero h1').getBoundingClientRect(); return { w: a.width, beside: h.left > a.right }; });
    check(dart.w === 220 && dart.beside, `desktop 1280: a playlist's header keeps its 220 px cover beside the words (${dart.w} px)`);
    await fx.go(page, '/library/import'); await page.waitForSelector('#roonfav'); await page.waitForTimeout(300);
    const dh = await page.evaluate(() => [...document.querySelectorAll('#main label.check:has(#roonfav, #roonpl, #roontags)')].map((l) => l.getBoundingClientRect().height));
    check(dh.length === 3 && dh.every((h) => h < 30), `desktop 1280: Roon's options stay lines of text (${dh.map((h) => h.toFixed(0)).join(' ')})`);
    await context.close();
    // Desktop: a queue row keeps its Move up / Move down / Remove (shown on hover), and has no ⋯
    const dq = await fx.create(browser, 1280, {});
    await dq.context.addInitScript((q) => { localStorage.setItem('pm.browserQueue', JSON.stringify(q)); }, QUEUE);
    await fx.go(dq.page, '/queue'); await dq.page.waitForSelector('.list.queue .list-row'); await dq.page.waitForTimeout(300);
    await dq.page.locator('.list.queue .list-row').nth(1).hover(); await dq.page.waitForTimeout(250);
    const dqr = await dq.page.evaluate(() => { const row = document.querySelectorAll('.list.queue .list-row')[1]; const a = row.querySelector('.q-actions'); return { acts: a.getClientRects().length > 0 && getComputedStyle(a).opacity === '1', buttons: a.querySelectorAll('button').length, more: row.querySelector('[data-qmore]')?.getClientRects().length ?? 0 }; });
    check(dqr.acts && dqr.buttons === 3 && dqr.more === 0, `desktop 1280: a hovered queue row shows Move up / Move down / Remove and no ⋯ (${JSON.stringify(dqr)})`);
    await dq.context.close();
  } finally { await browser.close(); }
  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
