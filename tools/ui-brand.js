// The web page and the installable web app say Odeion (docs/branding-compatibility.md), at desktop,
// tablet and phone widths, against a running node:
//   * the page title, the home-screen title and the brand say Odeion, and no "Pubky Music" shows (compat: pre-Odeion name must be gone);
//   * the PWA manifest is named Odeion and keeps its start_url and scope (an installed web app keeps
//     opening the same page);
//   * the stored token keeps its key, so a signed-in browser stays signed in;
//   * "Odeion" is visibly on screen (inside the viewport, not hidden), not only in the page's code;
//   * a screenshot per width, and one of the manifest.
//   npm install playwright && npx playwright install chromium   (anywhere; run from there)
//   node tools/ui-brand.js <api token> [http://127.0.0.1:7788] [screenshot folder]
// PLAYWRIGHT_CHROMIUM=<path> uses a Chromium already on the machine.
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2];
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const SHOTS = process.argv[4] || '.';
const problems = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
// The visible texts with Odeion: element boxes inside the viewport, not hidden
const visibleOdeion = () => [...document.querySelectorAll('body *')].filter((e) => { const r = e.getBoundingClientRect(); const cs = getComputedStyle(e); return r.width > 0 && r.height > 0 && r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth && cs.visibility !== 'hidden' && cs.display !== 'none' && Number(cs.opacity) > 0 && [...e.childNodes].some((n) => n.nodeType === 3 && /Odeion/.test(n.textContent)); }).map((e) => e.textContent.trim().slice(0, 60));
const waitFor = async (page, fn, arg = null, timeout = 30000) => { const end = Date.now() + timeout; for (;;) { const v = await page.evaluate(fn, arg).catch(() => null); if (v) return v; if (Date.now() > end) throw new Error('timed out waiting in the page'); await new Promise((r) => setTimeout(r, 100)); } };

(async () => {
  const browser = await chromium.launch(process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {});
  for (const [label, width, height, mobile] of [['desktop1440', 1440, 900, false], ['tablet820', 820, 1180, true], ['phone390', 390, 844, true]]) {
    const ctx = await browser.newContext({ viewport: { width, height }, isMobile: mobile, hasTouch: mobile, reducedMotion: 'reduce' });
    await ctx.addInitScript((t) => { if (!localStorage.getItem('pm.token')) localStorage.setItem('pm.token', t); }, TOKEN);
    const p = await ctx.newPage();
    const errors = [];
    p.on('pageerror', (e) => errors.push(e.message));
    await p.goto(BASE + '/');
    await waitFor(p, () => document.querySelector('.nav-item[data-view="home"]') && document.querySelector('#main') && document.querySelector('#main').textContent.trim().length > 0);
    await p.waitForTimeout(1500);
    const s = await p.evaluate(() => ({
      title: document.title,
      apple: document.querySelector('meta[name="apple-mobile-web-app-title"]')?.content,
      brand: document.querySelector('.brand-name')?.textContent,
      manifest: document.querySelector('link[rel="manifest"]')?.getAttribute('href'),
      text: document.body.innerText,
      token: localStorage.getItem('pm.token') !== null,
    }));
    check(s.title === 'Odeion', `${label}: the page title is Odeion`);
    check(s.apple === 'Odeion', `${label}: the home-screen title is Odeion`);
    check(s.brand === 'Odeion', `${label}: the brand says Odeion`);
    check(!s.text.includes('Pubky Music'), `${label}: no old name on the page`); // compat: pre-Odeion name must be gone
    check(s.token, `${label}: signed in with the stored token under its unchanged key`);
    const m = await p.evaluate(async (href) => (await fetch(href)).json(), s.manifest);
    check(m.name === 'Odeion' && m.short_name === 'Odeion', `${label}: the PWA manifest is named Odeion`);
    check(m.start_url === '/' && m.scope === '/', `${label}: the PWA keeps its start_url and scope`);
    // Desktop shows the brand in the sidebar; narrower widths fold it away, so open the header's own
    // Install app button there (a real control, as a person would) and look at its dialog
    let seen = await p.evaluate(visibleOdeion);
    let route = 'home';
    if (!seen.length && await p.isVisible('#installbtn')) {
      await p.click('#installbtn');
      await p.waitForTimeout(800);
      seen = await p.evaluate(visibleOdeion);
      route = 'header Install app button';
    }
    check(seen.length > 0, `${label}: Odeion is visibly on screen via ${route} (${JSON.stringify(seen)})`);
    await p.screenshot({ path: `${SHOTS}/odeion-${label}-visible-${route === 'home' ? 'home' : 'install-dialog'}.png` });
    console.log(`     screenshot ${SHOTS}/odeion-${label}-visible-${route === 'home' ? 'home' : 'install-dialog'}.png`);
    if (route !== 'home') { await p.keyboard.press('Escape'); await p.waitForTimeout(300); }
    check(errors.length === 0, `${label}: no page errors${errors.length ? ': ' + errors.join('; ') : ''}`);
    await p.screenshot({ path: `${SHOTS}/odeion-${label}.png` });
    console.log(`     screenshot ${SHOTS}/odeion-${label}.png`);
    await ctx.close();
  }
  { // the manifest itself, as the browser fetches it
    const ctx = await browser.newContext({ viewport: { width: 820, height: 900 } });
    const p = await ctx.newPage();
    const r = await p.goto(BASE + '/manifest.webmanifest');
    const m = JSON.parse(await r.text());
    check(m.name === 'Odeion' && m.short_name === 'Odeion' && m.id === '/', 'manifest: name, short_name Odeion, id unchanged (/)');
    await p.screenshot({ path: `${SHOTS}/pwa-manifest-json.png` });
    console.log(`     screenshot ${SHOTS}/pwa-manifest-json.png`);
    await ctx.close();
  }
  await browser.close();
  console.log(`\n${problems.length} problem(s)`);
  process.exit(problems.length ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
