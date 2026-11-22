// Spacing, layers and font weights stand on the design tokens, and the page is drawn as before.
// Real candidate assets on the fixture origin (tools/ui-fixture.js):
//   * every token the stylesheet uses resolves on the page (a var() of a name that is not defined
//     would leave its declaration without a value);
//   * parts drawn from token values measure what they measured written as plain numbers: a button's
//     weight and padding, the layers in their order (sticky head, scrim, popover, bars, menu, sheet,
//     toast, progress line);
//   * no rule that has a token writes the plain value (tools/refactor/tokens.js --check).
//   node tools/ui-tokens.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const tokens = require('./refactor/tokens.js');
const REPO = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'tokens-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

(async () => {
  const css = fs.readFileSync(path.join(REPO, 'crates', 'pm-api', 'static', 'app.css'), 'utf8');
  const r = tokens.check(css);
  check(!r.problems.length, `the stylesheet is on its tokens (${r.problems.length ? r.problems[0] : `${r.on} values`})`);
  check(r.on > 900, `spacing, layer and weight values on tokens: ${r.on}`);
  const used = [...new Set(css.match(/(?<=var\()--(?:sp|z|fw)-[a-z0-9-]+(?=\))/g) || [])];
  const want = { ...Object.fromEntries(Object.entries(tokens.SPACE).map(([v, n]) => [n, v])), ...Object.fromEntries(Object.entries(tokens.LAYER).map(([v, n]) => [n, String(v)])), ...Object.fromEntries(Object.entries(tokens.WEIGHT).map(([v, n]) => [n, String(v)])) };

  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const { context, page, state } = await fx.create(browser, width, { mobile: width < 600 });
    await fx.go(page, '/tracks'); await page.waitForSelector('#main .track');
    const got = await page.evaluate((names) => { const cs = getComputedStyle(document.documentElement); return Object.fromEntries(names.map((n) => [n, cs.getPropertyValue(n).trim()])); }, Object.keys(want));
    const wrong = Object.keys(want).filter((n) => got[n] !== want[n]);
    check(!wrong.length, `${width}px: all ${Object.keys(want).length} tokens resolve to their values (${wrong.map((n) => `${n}: ${got[n] || 'nothing'}`).join(', ') || 'ok'})`);
    check(used.every((n) => got[n]), `${width}px: every token the stylesheet uses is defined (${used.length} names)`);
    // Parts drawn from tokens measure as they did written out
    const m = await page.evaluate(() => {
      const px = (el, p) => (el ? getComputedStyle(el)[p] : null);
      const probe = (cls) => { const d = document.createElement('div'); d.className = cls; document.body.appendChild(d); const z = getComputedStyle(d).zIndex; d.remove(); return Number(z); };
      const b = document.createElement('button'); b.className = 'btn'; b.textContent = 'x'; document.body.appendChild(b); const btn = { weight: px(b, 'fontWeight'), pad: px(b, 'paddingLeft') }; b.remove();
      return { btn, layers: { sticky: probe('stickyhead'), scrim: probe('scrim'), popover: probe('popover'), bar: probe('selbar'), menu: probe('popover menu'), sheet: probe('popover menu sheet'), progress: probe('navprog') }, toast: px(document.getElementById('toast'), 'zIndex') };
    });
    const L = m.layers;
    // (a phone's popovers are sheets: on the sheet's layer)
    check(L.sticky === 15 && L.scrim === 25 && L.popover === (width < 600 ? 60 : 30) && L.bar === 40 && L.menu === 45 && L.sheet === 60 && L.progress === 70, `${width}px: the layers are in order, at their values (${JSON.stringify(L)})`);
    check(m.toast === '65', `${width}px: a toast is above the sheets (${m.toast})`);
    // (500 and 14 px: what main's plain values gave)
    check(m.btn.weight === '500' && m.btn.pad === '14px', `${width}px: a button's weight and padding are what they were (${JSON.stringify(m.btn)})`);
    await page.screenshot({ path: path.join(SHOTS, `tokens-${width}.png`) });
    check(!state.errors.length, `${width}px: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
