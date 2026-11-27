// On a narrow phone (or with a larger font size) Play and Shuffle can show only their words: their icon is
// left out so ⋯ stays on the row. Pressed, such a button spins while the music starts. Its spinner came
// after the words, so the button grew 26 px: an album's or a collection's Shuffle pushed ⋯ to a line of its
// own at 340-360 px (the page under it jumped a row), and Songs' Play all squeezed the filter. Now the
// spinner stands over the words, which wait unseen: the button and the buttons beside it keep their places.
// A button that keeps its icon is as before (the spinner takes the icon's place, the words stay).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-word-button-spin.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'word-button-spin-shots'));
fs.mkdirSync(SHOTS, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };

const tracks = Array.from({ length: 30 }, (_, k) => ({ ...fx.track, id: k + 1, title: k ? `Song ${k + 1}` : fx.track.title, track_no: k + 1 }));
const FIXTURES = Object.fromEntries(tracks.map((t) => [`/library/tracks/${t.id}`, { body: t }]));
// Android's font size as the app's WebView applies it (text zoom)
const ZOOM_ARGS = ['--blink-settings=textSizeAdjustEnabled=true', '--enable-blink-features=TextSizeAdjust,TextSizeAdjustImprovements'];
const textZoom = (page, k) => page.addStyleTag({ content: `html { -webkit-text-size-adjust: ${Math.round(k * 100)}%; text-size-adjust: ${Math.round(k * 100)}%; }` });

const CASES = [
  ['the album\'s Play', '/album/lib-7', '#iplay'],
  ['the album\'s Shuffle', '/album/lib-7', '#ishuffle'],
  ['the collection\'s Play', '/playlist/c1', '#plplay'],
  ['the collection\'s Shuffle', '/playlist/c1', '#plshuffle'],
  ['Songs\' Play all', '/tracks', '#shuffle'],
];

// The pressed button, its row's buttons (where each one is) and how the button draws its words and spinner
const look = (page, sel) => page.evaluate((sel) => {
  const b = document.querySelector(sel);
  const r = b.getBoundingClientRect();
  const ic = b.querySelector('.ic');
  const after = getComputedStyle(b, '::after');
  const row = [...b.parentElement.children].filter((c) => c.getClientRects().length).map((c) => { const q = c.getBoundingClientRect(); return `${c.id || c.className.split(' ')[0]}@${Math.round(q.left)},${Math.round(q.top)}`; }).join(' ');
  return {
    w: r.width, h: r.height, row,
    icon: !!ic && getComputedStyle(ic).display !== 'none',
    words: getComputedStyle(b).webkitTextFillColor,
    spin: after.content !== 'none' && after.content !== 'normal' ? { pos: after.position, w: parseFloat(after.width), arc: after.borderTopColor } : null,
    bg: getComputedStyle(b).backgroundColor, primary: b.classList.contains('primary'),
    fits: document.documentElement.scrollWidth <= innerWidth,
  };
}, sel);
// Where the spinner draws, its middle against the button's middle (a ::after has no box to read: from its
// computed offsets)
const spinBox = (page, sel) => page.evaluate((sel) => {
  const b = document.querySelector(sel); const r = b.getBoundingClientRect(); const a = getComputedStyle(b, '::after');
  if (a.position !== 'absolute') return null;
  // (content box plus border: `* { box-sizing }` doesn't reach a ::after)
  const w = parseFloat(a.width) + parseFloat(a.borderLeftWidth) + parseFloat(a.borderRightWidth); const h = parseFloat(a.height) + parseFloat(a.borderTopWidth) + parseFloat(a.borderBottomWidth);
  const left = r.left + b.clientLeft + parseFloat(a.left) + parseFloat(a.marginLeft);
  const top = r.top + b.clientTop + parseFloat(a.top) + parseFloat(a.marginTop);
  return { dx: left + w / 2 - (r.left + r.width / 2), dy: top + h / 2 - (r.top + r.height / 2) };
}, sel);

async function run(browser, width, android, k = 1) {
  const at = `${android ? 'android' : 'phone'}-${width}${k !== 1 ? ` at ${Math.round(k * 100)} %` : ''}`;
  const seen = { off: 0, on: 0 };
  for (const [what, route, sel] of CASES) {
    const { context, page, state } = await fx.create(browser, width, { mobile: true, tracks, collections: [fx.summary], fixtures: FIXTURES });
    try {
      if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
      await fx.go(page, route);
      const b = page.locator(sel).first();
      if (!(await b.waitFor({ timeout: 5000 }).then(() => true, () => false))) { check(false, `${at} ${what}: on screen (${route})`); continue; }
      if (k !== 1) await textZoom(page, k);
      await page.waitForTimeout(300);
      const l0 = await look(page, sel);
      check(!l0.spin, `${at} ${what}: no spinner before the press`);
      await b.click();
      const spun = await page.waitForFunction((s) => { const el = document.querySelector(s); return el && el.classList.contains('starting'); }, sel, { timeout: 2000 }).then(() => true, () => false);
      check(spun, `${at} ${what}: spins while the music starts`);
      if (!spun) continue;
      await page.mouse.move(1, 1); // (a phone has no hover: the button's own colour, not its hover's)
      const l1 = await look(page, sel);
      const kind = l0.icon ? 'with its icon' : 'only its words';
      if (l0.icon) seen.on++; else seen.off++;
      check(Math.abs(l1.w - l0.w) < 1 && Math.abs(l1.h - l0.h) < 1, `${at} ${what} (${kind}): keeps its size while it spins (${l0.w.toFixed(1)}×${l0.h.toFixed(1)} → ${l1.w.toFixed(1)}×${l1.h.toFixed(1)} px)`);
      check(l1.row === l0.row, `${at} ${what} (${kind}): the buttons beside it stay where they were${l1.row === l0.row ? '' : ` (${l0.row} → ${l1.row})`}`);
      check(l1.fits, `${at} ${what}: no sideways overflow while it spins`);
      check(l1.spin && l1.spin.w >= 14, `${at} ${what}: the spinner shows (${l1.spin ? `${l1.spin.w} px` : 'none'})`);
      if (l1.primary) check(l1.spin && l1.spin.arc !== l1.bg, `${at} ${what}: on the filled button the spinner's turning part shows (${l1.spin && l1.spin.arc} on ${l1.bg})`);
      if (l0.icon) {
        check(l1.spin && l1.spin.pos !== 'absolute' && l1.words !== 'rgba(0, 0, 0, 0)', `${at} ${what} (with its icon): the spinner takes the icon's place, the words stay (${l1.spin && l1.spin.pos}, ${l1.words})`);
      } else {
        check(l1.words === 'rgba(0, 0, 0, 0)', `${at} ${what} (only its words): the words wait unseen under the spinner (${l1.words})`);
        const c = await spinBox(page, sel);
        check(c && Math.abs(c.dx) < 1.5 && Math.abs(c.dy) < 1.5, `${at} ${what} (only its words): the spinner is in the button's middle (${c ? `${c.dx.toFixed(1)}, ${c.dy.toFixed(1)}` : 'not over the words'})`);
      }
      await page.screenshot({ path: path.join(SHOTS, `${at.replace(/\W+/g, '-')}-${sel.slice(1)}.png`) });
      // and once the music has started (or failed), the words come back
      await page.evaluate((s) => document.querySelector(s).classList.remove('starting', 'is-spinning'), sel);
      const l2 = await look(page, sel);
      check(l2.words === l0.words && Math.abs(l2.w - l0.w) < 1, `${at} ${what}: as before once it stops spinning`);
      check(!state.errors.length, `${at} ${what}: no page errors ${state.errors.join(' | ')}`);
    } finally { await context.close(); }
  }
  return seen;
}

(async () => {
  const browser = await chromium.launch();
  const zoomed = await chromium.launch({ args: ZOOM_ARGS });
  const all = { off: 0, on: 0 };
  const add = (s) => { all.off += s.off; all.on += s.on; };
  try {
    for (const w of [320, 340, 360, 412]) add(await run(browser, w, false));
    add(await run(zoomed, 360, true, 1.3));
    add(await run(zoomed, 360, true, 2));
    add(await run(zoomed, 412, true, 2));
  } finally { await browser.close(); await zoomed.close(); }
  // (the cases reach both kinds: a narrow phone leaves Shuffle's and Play all's icons out, 412 px keeps them,
  // and a larger font size leaves Play's out too)
  check(all.off >= 10 && all.on >= 8, `both kinds pressed (${all.off} with only words, ${all.on} with an icon)`);
  console.log(`\n${ok} ok, ${fail} FAIL`);
  process.exit(fail ? 1 : 0);
})();
