// Settings › My accounts, Pubky, signed out: the sign-in row says what signing in keeps ("Sign in
// with the Pubky Ring app to keep your collections, bookmarks and tags…"). A second row, "Saving",
// said it again under it ("Starts once you sign in: then your collections, bookmarks and tags are
// kept on your homeserver"), with nothing to press. Now the group has only the sign-in rows; once
// signed in, the "Your music" row is still there. Checked with no homeserver on the node, with the
// node's own homeserver (Create account here, Use an existing homeserver), and signed in, at a
// computer's, a tablet's and a phone's width.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-accounts-said-once.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/accounts-said-once-shots');
fs.mkdirSync(shots, { recursive: true });

let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };

const KEY = 'o1gg96ewuojmopcjbz8895478wdtxtzzber7aezq6ror5a91j7dy';
// [name, fixtures, the Pubky group's row names, its buttons, signed in]
const CASES = [
  ['signed out', {}, ['Not signed in'], ['Sign in with Pubky'], false],
  ['signed out, the node has its own homeserver', { '/homeserver/bundled': { body: { configured: true, reachable: true } } }, ['Create your Pubky account on this node’s homeserver', 'Use an existing homeserver'], ['Create account here', 'Sign in with Pubky'], false],
  ['signed in', { '/pubky': { body: { user: KEY } } }, ['Signed in', 'Your music'], ['Open Your library'], true],
];
const SIZES = [[1280, {}, '1280'], [768, {}, '768'], [412, { mobile: true }, '412 phone']];

const pubkyGroup = (page) => page.evaluate(() => {
  const g = [...document.querySelectorAll('#main .set-group')].find((x) => (x.querySelector(':scope > .set-h') || {}).textContent === 'Pubky');
  if (!g) return null;
  const rows = [...g.querySelectorAll('.set-row')].filter((r) => r.getClientRects().length);
  return {
    names: rows.map((r) => ((r.querySelector('.set-name') || {}).textContent || '').trim()),
    helps: rows.map((r) => ((r.querySelector('.set-label') || {}).textContent || '').replace((r.querySelector('.set-name') || {}).textContent || '', '').trim()),
    buttons: [...g.querySelectorAll('button')].filter((b) => b.getClientRects().length).map((b) => b.textContent.trim()),
    // a row with words but nothing to press
    bare: rows.filter((r) => !r.querySelector('.set-ctl')).map((r) => ((r.querySelector('.set-name') || {}).textContent || '').trim()),
  };
});

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [w, opts, size] of SIZES) {
      for (const [name, fixtures, names, buttons, signedIn] of CASES) {
        const { page, context, state } = await fx.create(browser, w, { ...opts, fixtures });
        try {
          await fx.go(page, '/settings/accounts');
          await page.waitForFunction(() => [...document.querySelectorAll('#main .set-h')].some((h) => h.textContent === 'Pubky'), null, { timeout: 7000 });
          await page.waitForTimeout(300);
          const g = await pubkyGroup(page);
          check(!!g, `${size}, ${name}: My accounts has its Pubky group`);
          if (!g) continue;
          check(JSON.stringify(g.names) === JSON.stringify(names), `${size}, ${name}: the Pubky rows are ${names.join(' + ')} (${g.names.join(' + ')})`);
          check(!g.names.includes('Saving'), `${size}, ${name}: no "Saving" row`);
          check(buttons.every((b) => g.buttons.includes(b)), `${size}, ${name}: ${buttons.join(', ')} still there (${g.buttons.join(', ')})`);
          // What signing in keeps is said once: no two rows name "collections, bookmarks and tags"
          const keeps = g.helps.filter((h) => /collections, bookmarks(,| and) tags/.test(h));
          check(keeps.length <= 1, `${size}, ${name}: what signing in keeps is said once (${keeps.length})`);
          if (!signedIn) check(g.bare.length === 0, `${size}, ${name}: every row has something to press (bare: ${g.bare.join(', ') || 'none'})`);
          check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${size}, ${name}: no sideways scroll`);
          await page.screenshot({ path: path.join(shots, `accounts-${name.replace(/[^a-z]+/g, '-')}-${size.replace(/ /g, '-')}.png`) });
          check(state.errors.length === 0, `${size}, ${name}: no page errors (${state.errors.join('; ')})`);
        } catch (e) { failed++; console.log(`FAIL ${size}, ${name}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
