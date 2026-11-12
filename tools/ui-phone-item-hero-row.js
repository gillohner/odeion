// On a phone an album's or artist's row (Play, Shuffle, and the ♥, Play on… and ⋯ icons) stays one row on a
// narrower phone (320 and 340 px: Android's larger display size) and with a larger font size (Android's 115 %
// and 130 % in the app's WebView): Shuffle drops its icon, then Play's and Shuffle's sides tighten and the
// three icons go from 48 to 44 px, then Play drops its icon, then the gaps and sides tighten once more, so ⋯
// stays on the row. Every button stays a 44 px tap. Android may apply its font size as page zoom instead,
// which lays the page out narrower (130 % on a 360 px phone: 277 px; 115 %: 313 px), so those widths are
// checked at the normal font size too. The app's WebView applies the font size as text zoom: the text grows,
// em does not. text-size-adjust takes the same path in Chromium, so those cases run with it too. 360 and 412 px
// with the normal font size, a song's row and desktop are as before.
// Real candidate assets, isolated responses: node tools/ui-phone-item-hero-row.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-item-hero-row-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// A library album, its artist and one of its songs
const base = { source: 'Your library', source_id: 'library', uri: null, song_uri: null, link: null, albums: [], versions: [], also_on: [], about: [], open_on: [], connect: [], on_pubky: null };
const songs = [{ id: 'lib-1', track_id: 1, title: 'Journey Through Time', artist: 'Sefa', duration_ms: 240000, no: 1 }];
const fixtures = {
  '/item/album/lib-7': { body: { ...base, kind: 'album', id: 'lib-7', title: 'Journey', section: 'albums', target: { album_id: 7 }, songs, artist: { id: 'lib-2', name: 'Sefa' }, year: 2024 } },
  '/item/artist/lib-2': { body: { ...base, kind: 'artist', id: 'lib-2', title: 'Sefa', section: 'artists', target: { artist_id: 2 }, songs } },
  '/item/song/lib-1': { body: { ...base, kind: 'song', id: 'lib-1', title: 'Journey Through Time', section: 'tracks', target: { track_id: 1 }, songs: [], artist: { id: 'lib-2', name: 'Sefa' } } },
};
const PAGES = { album: '/album/lib-7', artist: '/artist/lib-2', song: '/song/lib-1' };
const IDS = ['iplay', 'ishuffle', 'ifav', 'izone', 'imore'];

// The visible buttons of the hero's row, whether they take a second row (one starting 20 px or more below
// the first), the words and icons of Play and Shuffle, and the scaled font size
const row = (page) => page.evaluate((ids) => {
  const bs = ids.map((id) => document.getElementById(id)).filter((b) => b && b.getClientRects().length);
  const r = bs.map((b) => { const x = b.getBoundingClientRect(); const ic = b.querySelector(':scope > .ic'); return { id: b.id, w: x.width, h: x.height, top: x.top, right: x.right, text: b.textContent.trim(), icon: !!ic && getComputedStyle(ic).display !== 'none', pad: parseFloat(getComputedStyle(b).paddingLeft) }; });
  const first = Math.min(...r.map((b) => b.top));
  return { buttons: r, rows: r.some((b) => b.top > first + 20) ? 2 : 1, sw: document.documentElement.scrollWidth, vw: innerWidth, fs: parseFloat(getComputedStyle(document.getElementById('iplay')).fontSize), gap: parseFloat(getComputedStyle(document.querySelector('#main > .hero .actions')).columnGap) };
}, IDS);
const fmt = (r) => r.buttons.map((b) => `${b.id}:${b.w.toFixed(0)}×${b.h.toFixed(0)}`).join(' ');
const by = (r, id) => r.buttons.find((b) => b.id === id) || {};
// Android's text zoom: every font grows by k, em lengths stay (text-size-adjust, the same path in Chromium)
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });
const scale = (page, k) => page.evaluate((k) => { const all = [document.documentElement, ...document.querySelectorAll('body, body *')]; const fs = all.map((e) => parseFloat(getComputedStyle(e).fontSize)); all.forEach((e, i) => { e.style.fontSize = `${fs[i] * k}px`; }); }, k);

// What gives way at each width and font size: [Play's and Shuffle's sides, the icons' size, Play's icon shown, the row's gap]
const AS_IS = [14, 48, true, 4]; const TIGHT = [10, 44, true, 4]; const NO_PLAY_ICON = [10, 44, false, 4]; const TIGHTEST = [6, 44, false, 2];
const WANT = { 277: { 1: TIGHTEST }, 313: { 1: NO_PLAY_ICON }, 320: { 1: NO_PLAY_ICON, 1.15: NO_PLAY_ICON, 1.3: NO_PLAY_ICON }, 340: { 1: TIGHT, 1.15: TIGHT, 1.3: NO_PLAY_ICON }, 360: { 1: AS_IS, 1.15: TIGHT, 1.3: TIGHT }, 412: { 1: AS_IS, 1.15: AS_IS, 1.3: AS_IS } };

async function hero(browser, kind, width, android, k, zoom) {
  const at = `${kind} ${android ? 'android' : 'browser'} ${width} at ${Math.round(k * 100)} %${zoom ? ' text zoom' : ''}`;
  const { page, context } = await fx.create(browser, width, { mobile: true, fixtures: { ...fixtures } });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  await fx.go(page, PAGES[kind]); await page.waitForSelector('#iplay'); await page.waitForTimeout(300);
  if (k !== 1) { await (zoom ? textZoom : scale)(page, k); await page.waitForTimeout(100); }
  const r = await row(page);
  check(Math.abs(r.fs - 14 * k) < 0.1, `${at}: Play's words are ${r.fs.toFixed(1)} px (the scaled size held)`);
  const n = kind === 'song' ? 4 : 5;
  check(r.buttons.length === n && r.rows === 1 && r.sw <= r.vw, `${at}: the ${n} buttons stay on one row, ⋯ included (${fmt(r)}, ${r.rows} rows, page ${r.sw} of ${r.vw})`);
  check(r.buttons.every((b) => b.h >= 44 && b.w >= 44), `${at}: every button is still a 44 px tap`);
  check(by(r, 'iplay').text === 'Play' && (kind === 'song' || by(r, 'ishuffle').text === 'Shuffle'), `${at}: Play${kind === 'song' ? '' : ' and Shuffle'} keep${kind === 'song' ? 's its word' : ' their words'}`);
  const icons = ['ifav', 'izone', 'imore'].map((id) => by(r, id).w);
  if (kind === 'song') {
    // a song's row has no Shuffle and fits as it is
    check(icons.every((w) => w === 48) && by(r, 'iplay').icon && by(r, 'iplay').pad === 14, `${at}: a song's row is as before (icons ${icons.join('/')} px, Play's icon ${by(r, 'iplay').icon ? 'shown' : 'hidden'})`);
  } else {
    const [pad, size, playIcon, gap] = WANT[width][k];
    const round = ['ifav', 'izone', 'imore'].every((id) => by(r, id).h === by(r, id).w);
    check(icons.every((w) => w === size) && round && by(r, 'iplay').pad === pad && by(r, 'ishuffle').pad === pad && r.gap === gap, `${at}: icons ${icons.join('/')} px and ${round ? 'round' : 'not round'} (want ${size}), Play's and Shuffle's sides ${by(r, 'iplay').pad}/${by(r, 'ishuffle').pad} px (want ${pad}), gap ${r.gap} px (want ${gap})`);
    check(by(r, 'iplay').icon === playIcon && by(r, 'ishuffle').icon === (width >= 380), `${at}: Play's icon ${by(r, 'iplay').icon ? 'shown' : 'hidden'} (want ${playIcon ? 'shown' : 'hidden'}), Shuffle's ${by(r, 'ishuffle').icon ? 'shown' : 'hidden'} (want ${width >= 380 ? 'shown' : 'hidden'})`);
  }
  if (k !== 1.15 && ![313, 340].includes(width)) await page.locator('#main > .hero').screenshot({ path: path.join(SHOTS, `${kind}-row-${android ? 'android' : 'browser'}-${width}-${Math.round(k * 100)}${zoom ? '-textzoom' : ''}.png`) });
  // still one tap: ⋯ opens the page's menu
  await page.locator('#imore').tap(); await page.waitForTimeout(200);
  check(await page.locator('.popover.menu:not([hidden])').count() === 1, `${at}: one tap on ⋯ opens the menu`);
  await context.close();
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const kind of ['album', 'artist']) for (const width of [320, 340, 360, 412]) for (const k of [1, 1.15, 1.3]) for (const android of [false, true]) await hero(browser, kind, width, android, k);
    // Android's font size as page zoom on a 360 px phone: 130 % and 115 %
    for (const kind of ['album', 'artist']) for (const width of [277, 313]) for (const android of [false, true]) await hero(browser, kind, width, android, 1);
    for (const [width, k] of [[277, 1], [320, 1], [320, 1.3], [412, 1], [412, 1.3]]) await hero(browser, 'song', width, true, k);
    // Android's font size as the WebView applies it, as text zoom: the same steps as above
    const zoomed = await chromium.launch({ args: ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'] });
    try {
      for (const kind of ['album', 'artist']) for (const width of [320, 340, 360, 412]) for (const k of [1.15, 1.3]) await hero(zoomed, kind, width, true, k, true);
      await hero(zoomed, 'song', 320, true, 1.3, true);
    } finally { await zoomed.close(); }
    // Desktop with a mouse: the row is as before and the hero is not a size container
    const { page, context } = await fx.create(browser, 1280, { fixtures: { ...fixtures } });
    await fx.go(page, PAGES.album); await page.waitForSelector('#iplay'); await page.waitForTimeout(300);
    const d = await row(page);
    const ct = await page.evaluate(() => getComputedStyle(document.querySelector('#main > .hero')).containerType);
    check(d.rows === 1 && d.buttons.length === 5 && by(d, 'iplay').icon && by(d, 'ishuffle').icon && ct === 'normal', `desktop 1280: an album's row is unchanged (${fmt(d)}, container ${ct})`);
    await context.close();
  } finally { await browser.close(); }
  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
