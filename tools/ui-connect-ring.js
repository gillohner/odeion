// The desktop/Android connect page (crates/pm-desktop/ui) while it waits for Pubky Ring
// (TASK-ring-buttons-contrast-1105; owner, dark Android: the two buttons touched, "Open Pubky Ring"
// looked like Cancel, the waiting button stayed bright purple). The static page, no Tauri (its
// script fails at once; the states are set in the DOM), at 360 and 412 px, dark and light:
//   * "Open Pubky Ring" is the one accent-filled button; the waiting button isn't filled;
//   * the two buttons are at least 10 px apart, with room to the text above and below;
//   * a secondary button's border is at least 3:1 against the page and the card, white on the
//     accent at least 3:1 (WCAG 1.4.11, 1.4.3 for the text);
// and the saved-node, crash and address-form screens at 412 px, for the eye (their borders changed too).
//   node tools/ui-connect-ring.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const REPO = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'connect-ring-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const PAGE = 'file://' + path.join(REPO, 'crates/pm-desktop/ui/index.html');

// The waiting state as connect.js leaves it: the sign-in button disabled and relabelled, the QR
// box shown with a stand-in, Cancel enabled
const WAITING = () => {
  const $ = (id) => document.getElementById(id);
  $('saved').hidden = true; $('crash').hidden = true; $('connect').hidden = false; $('err').hidden = true;
  $('ring').disabled = true; $('ring').textContent = 'Waiting for Pubky Ring…';
  $('ring-out').hidden = false; $('qr').innerHTML = '<svg viewBox="0 0 21 21"><rect width="21" height="21" fill="#fff"/><path d="M0 0h7v7H0zM14 0h7v7h-7zM0 14h7v7H0zM9 2h3v3H9zM9 9h5v2H9zM2 9h3v3H2zM16 9h3v3h-3zM9 16h3v3H9zM16 16h3v3h-3z"/></svg>';
  $('lan-status').textContent = 'None found.';
};
const SCREENS = {
  saved: () => { const $ = (id) => document.getElementById(id); $('connect').hidden = true; $('saved').hidden = false; $('saved-msg').textContent = 'Your node moved to 192.168.1.12:7788.'; $('moved-yes').hidden = false; $('other').hidden = false; },
  crash: () => { const $ = (id) => document.getElementById(id); $('connect').hidden = true; $('crash').hidden = false; $('crash-first').textContent = 'thread main panicked at src/lib.rs:12'; $('crash-text').textContent = 'stack:\n  0: odeion::main\n  1: std::rt::lang_start'; },
  address: () => { const $ = (id) => document.getElementById(id); $('connect').hidden = false; $('manual-box').open = true; $('tok-hint').hidden = false; $('lan-status').textContent = 'None found.'; },
};
const REPORT = () => {
  const $ = (id) => document.getElementById(id);
  const cs = (el) => getComputedStyle(el);
  const lum = (c) => { const m = c.match(/\d+(\.\d+)?/g).map(Number); const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(m[0]) + 0.7152 * f(m[1]) + 0.0722 * f(m[2]); };
  const ratio = (a, b) => { const x = lum(a); const y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const open = $('ring-open'); const cancel = $('ring-cancel'); const ring = $('ring');
  const o = open.getBoundingClientRect(); const c = cancel.getBoundingClientRect();
  // (without the row, as before, the buttons' own neighbours)
  const row = open.closest('.row') || open; const above = row.previousElementSibling.getBoundingClientRect(); const below = (open.closest('.row') ? row : cancel).nextElementSibling.getBoundingClientRect();
  const page = cs(document.body).backgroundColor; const card = cs(cancel).backgroundColor; const line = cs(cancel).borderTopColor;
  return {
    gap: Math.round(c.top - o.bottom), above: Math.round(o.top - above.bottom), below: Math.round(below.top - c.bottom),
    openFill: cs(open).backgroundColor, ringFill: cs(ring).backgroundColor, cancelFill: card, accent: cs(document.documentElement).getPropertyValue('--accent').trim(),
    lineVsPage: ratio(line, page), lineVsCard: ratio(line, card), openText: ratio(cs(open).color, cs(open).backgroundColor), ringText: ratio(cs(ring).color, cs(ring).backgroundColor),
    ringOpacity: cs(ring).opacity, inputLine: ratio(cs($('url')).borderTopColor, page),
  };
};
const hex = (rgb) => '#' + rgb.match(/\d+/g).slice(0, 3).map((n) => Number(n).toString(16).padStart(2, '0')).join('');

(async () => {
  const browser = await chromium.launch();
  for (const scheme of ['dark', 'light']) {
    for (const w of [360, 412]) {
      const ctx = await browser.newContext({ viewport: { width: w, height: 800 }, colorScheme: scheme, hasTouch: true, isMobile: true });
      const page = await ctx.newPage();
      await page.goto(PAGE); await page.evaluate(WAITING); await page.waitForTimeout(100);
      const r = await page.evaluate(REPORT);
      const at = `${scheme} ${w}`;
      check(hex(r.openFill) === r.accent.toLowerCase() && hex(r.ringFill) !== r.accent.toLowerCase() && hex(r.cancelFill) !== r.accent.toLowerCase(), `${at}: "Open Pubky Ring" is the one accent-filled button (open ${hex(r.openFill)}, waiting ${hex(r.ringFill)}, cancel ${hex(r.cancelFill)}, accent ${r.accent})`);
      check(r.gap >= 10 && r.above >= 10 && r.below >= 10, `${at}: the buttons are apart (${r.gap} px), with room above (${r.above}) and below (${r.below})`);
      check(r.lineVsPage >= 3 && r.lineVsCard >= 3 && r.inputLine >= 3, `${at}: borders at least 3:1 against the page and the card (${r.lineVsPage.toFixed(2)}, ${r.lineVsCard.toFixed(2)}; the address field ${r.inputLine.toFixed(2)})`);
      check(r.openText >= 3 && r.ringText >= 4.5 && r.ringOpacity === '1', `${at}: text on the buttons reads (open ${r.openText.toFixed(2)}, waiting ${r.ringText.toFixed(2)} at full opacity)`);
      await page.screenshot({ path: path.join(SHOTS, `waiting-${scheme}-${w}.png`), fullPage: true });
      if (w === 412) for (const [name, fn] of Object.entries(SCREENS)) { await page.goto(PAGE); await page.evaluate(fn); await page.waitForTimeout(100); await page.screenshot({ path: path.join(SHOTS, `${name}-${scheme}.png`), fullPage: true }); }
      await ctx.close();
    }
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
