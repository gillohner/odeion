// The app's connect page (crates/pm-desktop/ui) and phone mode, the app without a node. The real
// page with a stand-in for the app's commands (window.__TAURI__), at 360 px:
//   * with no node saved, "Use without a node" shows where the build has phone mode, and not
//     where it hasn't;
//   * its press shows the loader at once, starts phone mode's Pubky Ring sign-in once (a second
//     press does nothing), and the Ring code then shows on the page;
//   * phone mode chosen before opens again by itself, without the connect form;
//   * a saved node still comes first (phone mode isn't asked about).
//   node tools/ui-connect-phone.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const REPO = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'connect-phone-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const PAGE = 'file://' + path.join(REPO, 'crates/pm-desktop/ui/index.html');

// The app's commands: answers by name; `phone_ring` and `open_phone` never end (Ring waits, the
// app navigates away). Every call is recorded.
const STUB = (answers) => {
  window.__calls = [];
  const listeners = {};
  window.__emit = (name, payload) => (listeners[name] || []).forEach((f) => f({ payload }));
  window.__TAURI__ = {
    core: {
      invoke: (cmd, args) => {
        window.__calls.push(cmd);
        if (cmd === 'phone_ring' || cmd === 'open_phone' || cmd === 'open_saved') return new Promise(() => {});
        if (cmd === 'discover_nodes') return Promise.resolve([]);
        return Promise.resolve(cmd in answers ? answers[cmd] : null);
      },
    },
    event: { listen: (name, f) => { (listeners[name] = listeners[name] || []).push(f); return Promise.resolve(() => {}); } },
  };
};

async function open(browser, answers) {
  const ctx = await browser.newContext({ viewport: { width: 360, height: 800 }, hasTouch: true, isMobile: true });
  const page = await ctx.newPage();
  await page.addInitScript(STUB, answers);
  await page.goto(PAGE);
  await page.waitForTimeout(300);
  return { ctx, page };
}

const calls = (page) => page.evaluate(() => window.__calls);
const visible = (page, sel) => page.evaluate((s) => { const e = document.querySelector(s); return !!e && !e.closest('[hidden]') && e.getBoundingClientRect().height > 0; }, sel);

(async () => {
  const browser = await chromium.launch();
  {
    const { ctx, page } = await open(browser, { phone_available: true });
    check(await visible(page, '#phone-go'), 'no node saved: "Use without a node" shows');
    check((await page.textContent('#phone-go')).trim() === 'Use without a node', 'its label');
    await page.screenshot({ path: path.join(SHOTS, 'connect-phone.png'), fullPage: true });
    await page.tap('#phone-go');
    const at = await page.evaluate(() => ({ text: document.getElementById('phone-go').textContent, disabled: document.getElementById('phone-go').disabled, ring: document.getElementById('ring').disabled }));
    check(at.disabled && at.text === 'Starting…', `the press shows the loader at once (${JSON.stringify(at.text)}, disabled ${at.disabled})`);
    check(at.ring, 'the node sign-in waits meanwhile (one Ring sign-in at a time)');
    await page.evaluate(() => document.getElementById('phone-go').click());
    check((await calls(page)).filter((c) => c === 'phone_ring').length === 1, 'phone mode\'s Ring sign-in starts once');
    await page.evaluate(() => window.__emit('ring-url', { url: 'pubkyauth:///?x', qr_svg: '<svg id="qrfix"></svg>' }));
    check(await visible(page, '#ring-out'), 'the Ring code shows');
    check((await page.textContent('#phone-go')).trim() === 'Waiting for Pubky Ring…', 'the button says it waits for Ring');
    check((await page.textContent('#ring')).trim() === 'Sign in with Pubky Ring', 'the node sign-in button keeps its label');
    await page.screenshot({ path: path.join(SHOTS, 'connect-phone-ring.png'), fullPage: true });
    await ctx.close();
  }
  {
    const { ctx, page } = await open(browser, { phone_available: false });
    check(!(await visible(page, '#phone-go')), 'a build without phone mode doesn\'t offer it');
    await ctx.close();
  }
  {
    const { ctx, page } = await open(browser, { phone_available: true, phone_saved: { pubky: 'o1gg96ewuojmopcjbz8895478wdtxtzzuxnfjjz8o8e77csa1ngo' } });
    const c = await calls(page);
    check(c.includes('open_phone'), `phone mode chosen before opens again (${c.join(', ')})`);
    check(!(await visible(page, '#connect')), 'without the connect form');
    check((await page.textContent('#saved-msg')).includes('this phone'), 'and says so');
    await ctx.close();
  }
  {
    const { ctx, page } = await open(browser, { phone_available: true, saved: { node_url: 'http://192.168.1.10:7788' }, phone_saved: { pubky: 'x' } });
    const c = await calls(page);
    check(c.includes('open_saved') && !c.includes('open_phone') && !c.includes('phone_saved'), `a saved node comes first (${c.join(', ')})`);
    await ctx.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
