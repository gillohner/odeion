// On a narrow phone (or with a larger font size) Play and Shuffle can show only their words: their icon is
// left out so ⋯ stays on the row. Pressed, such a button spins while the music starts. Its spinner came
// after the words, so the button grew 26 px: an album's or a collection's Shuffle pushed ⋯ to a line of its
// own at 340-360 px (the page under it jumped a row), and Songs' Play all squeezed the filter. Now the
// spinner stands over the words, which wait unseen: the button and the buttons beside it keep their places.
// A button that keeps its icon is as before (the spinner takes the icon's place, the words stay).
// Also: the narrow-phone rules alone (a WebView without container queries), and forced colours (Windows High
// Contrast), where the system painted the words under the spinner and drew the spinner's ring in one colour.
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
  const after = getComputedStyle(b, '::after'); const s = getComputedStyle(b);
  const row = [...b.parentElement.children].filter((c) => c.getClientRects().length).map((c) => { const q = c.getBoundingClientRect(); return `${c.id || c.className.split(' ')[0]}@${Math.round(q.left)},${Math.round(q.top)}`; }).join(' ');
  return {
    w: r.width, h: r.height, row,
    icon: !!ic && getComputedStyle(ic).display !== 'none',
    words: getComputedStyle(b).webkitTextFillColor,
    spin: after.content !== 'none' && after.content !== 'normal' ? { pos: after.position, w: parseFloat(after.width), arc: after.borderTopColor, ring: after.borderRightColor, own: after.forcedColorAdjust === 'none' } : null,
    bg: s.backgroundColor, fg: s.color, border: s.borderTopColor, primary: b.classList.contains('primary'),
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

// How much is drawn inside the button, its spinner left out: the pixels unlike its background in a band
// clear of its border and round ends (its words, or nothing while they wait unseen)
const pixels = (page, png) => page.evaluate(async (b64) => { const img = new Image(); img.src = 'data:image/png;base64,' + b64; await img.decode(); const c = document.createElement('canvas'); c.width = img.width; c.height = img.height; const g = c.getContext('2d'); g.drawImage(img, 0, 0); return Array.from(g.getImageData(0, 0, c.width, c.height).data); }, png.toString('base64'));
async function ink(page, sel) {
  const r = await page.locator(sel).first().boundingBox();
  const hide = await page.addStyleTag({ content: `${sel}::after { visibility: hidden !important; }` });
  const png = await page.screenshot({ clip: { x: r.x + 12, y: r.y + 8, width: r.width - 24, height: r.height - 16 }, animations: 'disabled' });
  await hide.evaluate((el) => el.remove());
  const px = await pixels(page, png);
  const count = new Map(); for (let i = 0; i < px.length; i += 4) { const key = `${px[i]},${px[i + 1]},${px[i + 2]}`; count.set(key, (count.get(key) || 0) + 1); }
  const bg = [...count.entries()].sort((a, b) => b[1] - a[1])[0][0].split(',').map(Number); // (the commonest colour)
  let n = 0; for (let i = 0; i < px.length; i += 4) if (Math.abs(px[i] - bg[0]) + Math.abs(px[i + 1] - bg[1]) + Math.abs(px[i + 2] - bg[2]) > 60) n++;
  return n;
}
// (a WebView without container queries: only the narrow-phone rules leave an icon out)
const dropContainerRules = (page) => page.evaluate(() => {
  let n = 0;
  const walk = (list) => { for (let i = list.cssRules.length - 1; i >= 0; i--) { const r = list.cssRules[i]; if (r instanceof CSSContainerRule) { list.deleteRule(i); n++; } else if (r.cssRules) walk(r); } };
  for (const sheet of document.styleSheets) { try { walk(sheet); } catch { /* (another origin's sheet) */ } }
  return n;
});

async function run(browser, width, android, k = 1, { forced = false, noContainer = false } = {}) {
  const at = `${android ? 'android' : 'phone'}-${width}${k !== 1 ? ` at ${Math.round(k * 100)} %` : ''}${forced ? ' forced colours' : ''}${noContainer ? ' no container queries' : ''}`;
  const seen = { off: 0, on: 0, offSels: [] };
  for (const [what, route, sel] of CASES) {
    const { context, page, state } = await fx.create(browser, width, { mobile: true, tracks, collections: [fx.summary], fixtures: FIXTURES });
    try {
      if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
      if (forced) await page.emulateMedia({ forcedColors: 'active' });
      await fx.go(page, route);
      const b = page.locator(sel).first();
      if (!(await b.waitFor({ timeout: 5000 }).then(() => true, () => false))) { check(false, `${at} ${what}: on screen (${route})`); continue; }
      if (k !== 1) await textZoom(page, k);
      if (noContainer) { const n = await dropContainerRules(page); check(n > 0, `${at} ${what}: the container rules are left out (${n})`); }
      await page.waitForTimeout(300);
      const l0 = await look(page, sel);
      const ink0 = forced ? await ink(page, sel) : 0;
      // (the system's own colours, as the page resolves them)
      const sys = forced && await page.evaluate(() => Object.fromEntries(['Highlight', 'GrayText'].map((k) => { const d = document.createElement('div'); d.style.cssText = `forced-color-adjust:none;color:${k}`; document.body.append(d); const c = getComputedStyle(d).color; d.remove(); return [k, c]; })));
      check(!l0.spin, `${at} ${what}: no spinner before the press`);
      await b.click();
      const spun = await page.waitForFunction((s) => { const el = document.querySelector(s); return el && el.classList.contains('starting'); }, sel, { timeout: 2000 }).then(() => true, () => false);
      check(spun, `${at} ${what}: spins while the music starts`);
      if (!spun) continue;
      await page.mouse.move(1, 1); // (a phone has no hover: the button's own colour, not its hover's)
      const l1 = await look(page, sel);
      const kind = l0.icon ? 'with its icon' : 'only its words';
      if (l0.icon) seen.on++; else { seen.off++; seen.offSels.push(sel); }
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
      if (forced) {
        // (the computed colours say the words are unseen either way: what is drawn tells)
        const ink1 = await ink(page, sel);
        check(ink0 > 30, `${at} ${what}: its words are drawn before the press (${ink0} px)`);
        if (l0.icon) check(ink1 > 30, `${at} ${what} (with its icon): its words stay drawn while it spins (${ink1} px)`);
        else check(ink1 <= 2, `${at} ${what} (only its words): the system doesn't paint the words under the spinner (${ink1} px of words drawn)`);
        // (the system paints the whole ring in one colour unless the spinner keeps its own)
        check(l1.spin && l1.spin.own && l1.spin.arc === sys.Highlight && l1.spin.ring === sys.GrayText, `${at} ${what}: the spinner's turning part (Highlight) shows against its ring (GrayText) (${l1.spin && `${l1.spin.own ? 'own colours' : 'the system\'s one colour'}, ${l1.spin.arc} on ${l1.spin.ring}`})`);
        // (at once: with motion on, no fade from the page's own colours to the system's)
        await page.emulateMedia({ reducedMotion: 'no-preference' });
        const fades = await page.evaluate((s) => {
          const el = document.querySelector(s); const had = ['starting', 'is-spinning'].filter((c) => el.classList.contains(c));
          el.classList.remove(...had); getComputedStyle(el).backgroundColor; el.getAnimations().forEach((a) => a.finish());
          el.classList.add(...had); getComputedStyle(el).backgroundColor;
          return el.getAnimations().map((a) => a.transitionProperty).filter((p) => /color|background/.test(p || ''));
        }, sel);
        await page.emulateMedia({ reducedMotion: 'reduce' });
        check(!fades.length, `${at} ${what}: takes the system's colours at once (${fades.join(', ') || 'no fade'})`);
        check(l1.bg === l0.bg && l1.fg === l0.fg && l1.border === l0.border, `${at} ${what}: the button keeps the system's colours while it spins (${l0.bg}/${l0.fg}/${l0.border} → ${l1.bg}/${l1.fg}/${l1.border})`);
        // (the emulated system palette has ButtonText, CanvasText and ButtonBorder all one colour, so the
        // computed colours can't tell them apart: the colours its own forced-colours rule declares do)
        const own = await page.evaluate((s) => {
          const el = document.querySelector(s); const got = {};
          const walk = (rules, forcedOn) => { for (const r of rules) {
            if (r instanceof CSSMediaRule) walk(r.cssRules, forcedOn || [...r.media].some((m) => /^\(forced-colors:\s*active\)$/.test(m)));
            else if (forcedOn && r instanceof CSSStyleRule && r.style.getPropertyValue('forced-color-adjust') === 'none' && el.matches(r.selectorText)) {
              for (const p of ['color', 'background-color', 'border-color', 'outline-color']) { const v = r.style.getPropertyValue(p); if (v) got[p] = v.toLowerCase(); }
            }
          } };
          for (const sh of document.styleSheets) { try { walk(sh.cssRules, false); } catch { /* (another origin's sheet) */ } }
          return got;
        }, sel);
        check(own.color === 'buttontext' && own['background-color'] === 'buttonface' && own['border-color'] === 'buttonborder', `${at} ${what}: its forced-colours rule names the system's button colours (${JSON.stringify(own)})`);
        // its keyboard ring while it spins: Highlight, as the system draws every other focus ring (Tab to the
        // next control and back: the ring there, then on this button)
        await page.keyboard.press('Tab');
        const ringOf = (s) => page.evaluate((s) => { const el = document.activeElement; const a = getComputedStyle(el);
          return { mine: !!s && el === document.querySelector(s) && el.matches('.starting, .is-spinning'), visible: el.matches(':focus-visible'), style: a.outlineStyle, color: a.outlineColor }; }, s);
        const other = await ringOf(null);
        await page.keyboard.press('Shift+Tab');
        const ring = await ringOf(sel);
        ring.visible = ring.visible && ring.mine; ring.plain = other.visible && other.style !== 'none' ? other.color : null;
        await page.evaluate(() => document.activeElement && document.activeElement.blur());
        check(ring.visible && ring.style !== 'none' && ring.color === sys.Highlight && ring.color === ring.plain, `${at} ${what}: its keyboard ring while it spins is Highlight, as the next control's (${ring.visible ? `${ring.style} ${ring.color}; the next control's ${ring.plain}` : 'not focused by the keyboard while it spins'})`);
      }
      if (forced && sel === '#iplay') {
        // (and every other spinner: a song's, a toast's, a chip's, a queue item's)
        const others = await page.evaluate(() => {
          const box = document.createElement('div'); box.style.cssText = 'position:fixed;left:0;top:0;opacity:0';
          box.innerHTML = '<span class="spin"></span><span class="chip scope"><span class="spin"></span></span><span class="srcchip"><span class="spin"></span></span><div class="toast wait">x</div><span class="track starting"></span><button class="np-qitem is-spinning"><span>3:12</span></button><button class="btn primary is-spinning">Save</button>';
          document.body.append(box);
          const at = (el, pseudo) => { const a = getComputedStyle(el, pseudo); return { own: a.forcedColorAdjust === 'none', arc: a.borderTopColor, ring: a.borderRightColor }; };
          const out = [at(box.children[0]), at(box.children[1].firstChild), at(box.children[2].firstChild), at(box.children[3], '::before'), at(box.children[4], '::after'), at(box.children[5].lastChild, '::after'), at(box.children[6], '::after')];
          const len = getComputedStyle(box.children[5].lastChild).color; // (the queue item's length under it)
          box.remove();
          return { out, len };
        });
        check(others.out.every((o) => o.own && o.arc === sys.Highlight && o.ring === sys.GrayText), `${at}: every other spinner's turning part (Highlight) shows against its ring (GrayText) (${others.out.map((o) => `${o.own ? 'own' : 'system'} ${o.arc}/${o.ring}`).join('; ')})`);
        check(others.len === 'rgba(0, 0, 0, 0)', `${at}: a queue item's length waits unseen under its spinner (${others.len})`);
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
    // (the narrow-phone rules alone leave the three Shuffles' and Play all's icons out up to 379 px)
    for (const w of [320, 360]) {
      const s = await run(browser, w, false, 1, { noContainer: true });
      check(['#ishuffle', '#plshuffle', '#shuffle'].every((x) => s.offSels.includes(x)), `phone-${w} no container queries: the narrow-phone rules alone leave Shuffle's and Play all's icons out (${s.offSels.join(' ')})`);
    }
    add(await run(browser, 340, false, 1, { forced: true }));
    add(await run(zoomed, 360, true, 2, { forced: true }));
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
