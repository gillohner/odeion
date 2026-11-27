// Home's Get started buttons that open a Settings tab at a control do it once, on that press:
// "Restore it from a backup" (Settings › Backup, the focus on its file picker), "Create account here"
// (Settings › Accounts, starts the Pubky Ring sign-up) and "Use an existing homeserver" / "Sign in
// with Pubky Ring" (starts the Pubky Ring sign-in). Before, the press left a note that only the
// control's own draw cleared, so going Back before Settings drew (a slow node) kept it: a later visit
// to Settings › Accounts started a sign-up or sign-in no one asked for, and a later Settings › Backup
// jumped to its file picker. A note also fired on whichever Settings tab drew next (the controls of
// every tab are in the page), so Back to another tab started the sign-in there, out of sight.
// Checks at 360, 360 in the Android app, 412 and 1280 px; no page errors.
//   node tools/ui-settings-jump-once.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'settings-jump-once-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const steps = (ids) => ({ show: true, hidden: false, steps: ids.map((id) => ({ id, done: false, optional: id !== 'play' && id !== 'music' })) });
const hsHere = { configured: true, reachable: true, public_key: 'fixturekey', users: 0 };
const noHs = { configured: false, reachable: false };
const signUp = '/homeserver/bundled/owner-signup';
const ring = { url: 'pubkyauth:///?relay=https://relay.example&secret=fixture', qr_svg: '<svg width="8" height="8"></svg>' };
// The fixture answers both Pubky Ring starts with a link (GET /pubky/login keeps saying "waiting")
const FIX = (hs) => ({ '/me/first-steps': { body: steps(['music', 'play', 'pubky']) }, '/homeserver/bundled': { body: hs }, [signUp]: { body: ring }, '/pubky/login': { body: { ...ring, status: 'waiting' } } });
const posts = (state, v) => state.hits.filter((h) => h.v === v && h.m === 'POST').length;
const press = async (page, phone, sel) => { await page.waitForSelector(sel, { timeout: 15000 }); if (phone) await page.tap(sel); else await page.click(sel); };
const where = (page) => page.evaluate(() => {
  const a = document.activeElement; const tab = document.querySelector('.set-panel:not([hidden])');
  const card = document.querySelector('#backupcard');
  return { path: location.pathname, focus: a ? (a.id ? `#${a.id}` : a.tagName) : null, tab: tab && tab.dataset.panel, card: card ? Math.round(card.getBoundingClientRect().top) : null };
});

(async () => {
  const browser = await chromium.launch();
  for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
    const tag = `${width}${android ? '-android' : ''}`;
    const phone = width < 600;
    for (const [what, hs, btn, tab, v] of [['Create account here', hsHere, '#setupcreate', 'accounts', signUp], ['Use an existing homeserver', hsHere, '#setupsignin', 'accounts', '/pubky/login'], ['Sign in with Pubky Ring', noHs, '#setupsignin', 'accounts', '/pubky/login'], ['Restore it from a backup', noHs, '#setuprestore', 'backup', null]]) {
      const at = `${tag} ${what}`;
      const { context, page, state } = await fx.create(browser, width, { mobile: phone, fixtures: FIX(hs) });
      if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
      const errors = []; page.on('pageerror', (e) => errors.push(String(e)));
      // The draw the press is for: still does what the press asked
      const done = async () => {
        if (v) { await page.waitForFunction(() => !!document.querySelector('#pkurl .qrbox'), null, { timeout: 15000 }).catch(() => {}); return posts(state, v); }
        await page.waitForSelector('#rsfile', { state: 'attached', timeout: 15000 }); await page.waitForTimeout(400); return (await where(page)).focus === '#rsfile' ? 1 : 0;
      };
      try {
        // 1. The press itself
        await fx.go(page, '/');
        await press(page, phone, btn);
        await page.waitForFunction((t) => location.pathname === `/settings/${t}`, tab);
        check(await done() === 1, `${at}: the press opens Settings › ${tab} and ${v ? `starts Pubky Ring (one POST ${v})` : 'puts the focus on Choose a backup file'}`);
        if (!v) { const w = await where(page); check(w.card !== null && w.card < 200, `${at}: the Restore card is at the top (${w.card})`); }
        await page.screenshot({ path: path.join(SHOTS, `${tag}-${btn.slice(1)}${hs === hsHere ? '-hs' : ''}-press.png`) });
        // 2. Pressed, Back before the tab drew (a slow node), then the same tab again by Forward
        await fx.go(page, '/');
        const before = v ? posts(state, v) : 0;
        state.delay = (p) => (p === '/node/settings' ? 1500 : 0);
        await press(page, phone, btn);
        await page.waitForFunction((t) => location.pathname === `/settings/${t}`, tab);
        await page.goBack(); await page.waitForSelector(btn, { timeout: 15000 });
        await page.waitForTimeout(1800);
        state.delay = 0;
        await page.goForward();
        await page.waitForSelector(v ? '#pklogin' : '#rsfile', { state: 'attached', timeout: 15000 });
        await page.waitForTimeout(800);
        const f = await where(page);
        if (v) check(posts(state, v) === before, `${at}: left before it drew, a later Settings › ${tab} starts nothing by itself (${posts(state, v) - before} POST ${v})`);
        else check(f.focus !== '#rsfile', `${at}: left before it drew, a later Settings › ${tab} doesn't take the focus (${f.focus})`);
        // 3. Pressed, then Back to another Settings tab before it drew: that tab is not the one asked for
        if (v) {
          // (one page load: Settings › Display, Home from the bar, the press; Back twice lands on Settings › Display)
          await fx.go(page, '/settings/appearance');
          await page.waitForSelector('.set-panel:not([hidden])', { timeout: 15000 });
          await press(page, phone, phone ? '[data-bnav="home"]' : '.nav-item[data-view="home"]');
          await page.waitForFunction(() => location.pathname === '/');
          const b2 = posts(state, v);
          state.delay = (p) => (p === '/node/settings' ? 1500 : 0);
          await press(page, phone, btn);
          await page.waitForFunction((t) => location.pathname === `/settings/${t}`, tab);
          await page.goBack(); await page.waitForFunction(() => location.pathname === '/');
          await page.goBack(); await page.waitForFunction(() => location.pathname === '/settings/appearance');
          await page.waitForTimeout(2200);
          state.delay = 0;
          const t = await where(page);
          check(posts(state, v) === b2 && t.tab === 'appearance', `${at}: Back to Settings › Display before it drew starts nothing there (${posts(state, v) - b2} POST ${v}, tab ${t.tab})`);
          const b3 = posts(state, v);
          // and the note is gone for good: Forward twice to Settings › Accounts starts nothing
          await page.goForward(); await page.waitForFunction(() => location.pathname === '/');
          await page.goForward(); await page.waitForFunction((t) => location.pathname === `/settings/${t}`, tab);
          await page.waitForSelector('#pklogin', { state: 'attached', timeout: 15000 }); await page.waitForTimeout(800);
          check(posts(state, v) === b3, `${at}: after that, Settings › ${tab} starts nothing by itself (${posts(state, v) - b3} POST ${v})`);
        }
        await page.screenshot({ path: path.join(SHOTS, `${tag}-${btn.slice(1)}${hs === hsHere ? '-hs' : ''}-later.png`) });
        check(!errors.length, `${at}: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
      } catch (e) { check(false, `${at}: ${e.message}`); }
      await context.close();
    }
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
