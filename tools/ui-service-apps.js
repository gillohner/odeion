// Settings › This node › Service apps (TASK-service-apps-builtin.md): each service says whether
// it is built in or your own, with its limits in plain words; "Use your own app" sets the
// admin's own app, the secret never comes back, and "Use built-in" returns to the build's. Only
// what can be done here is listed (owner, 2026-11-08): no row for an app with nothing to set and
// none in use, no own-app form for Qobuz or TIDAL's device clients (one saved before keeps Remove).
// Run against a disposable fixture node (it changes the node's TIDAL app and sets it back):
//   node tools/ui-service-apps.js <api token> <node URL>
const assert = require('node:assert/strict');
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const get = async () => (await fetch(`${BASE}/v1/node/apps`, { headers: { authorization: `Bearer ${TOKEN}` } })).json();
(async () => {
  const before = (await get()).find((a) => a.id === 'tidal');
  if (before.from === 'config') { console.log('TIDAL is set in node.toml here; nothing to change'); return; }
  const browser = await chromium.launch();
  try {
    for (const width of [1440, 390]) {
      const ctx = await browser.newContext({ viewport: { width, height: width === 390 ? 844 : 1000 }, reducedMotion: 'reduce' });
      await ctx.addInitScript((t) => { localStorage.setItem('pm.token', t); }, TOKEN);
      const page = await ctx.newPage();
      const errors = []; page.on('pageerror', (e) => errors.push(e.message));
      await page.goto(BASE + '/settings/apps');
      const panel = page.locator('#set-apps');
      const tidal = panel.locator('.set-row', { has: page.locator('.set-name', { hasText: /^TIDAL$/ }) });
      await tidal.waitFor();
      assert.match(await tidal.textContent(), /30-second previews/, 'TIDAL says what it limits');
      const spotify = panel.locator('.set-row', { has: page.locator('.set-name', { hasText: /^Spotify$/ }) });
      assert.match(await spotify.textContent(), /25\s+people/, 'Spotify says its 25-person limit');
      // A console link is only under "Use your own app"
      for (const a of await panel.locator('a[href*="developer"], a[href*="console."]').all()) {
        assert.ok(await a.evaluate((el) => !!el.closest('details.app-adv')), `${await a.getAttribute('href')} sits under Use your own app`);
      }
      assert.ok(!/Advanced/.test(await panel.textContent()), 'no "Advanced" wording');
      // Nothing to do, nothing listed: an app with no form and none in use, Qobuz's app (Qobuz hands none out)
      const listed = await panel.locator('.set-row .set-name').allTextContents();
      for (const a of await get()) {
        const doable = a.configured || !!a.page || (!!a.set && !['qobuz', 'tidal-full'].includes(a.id));
        assert.equal(listed.includes(a.name), doable, `${a.name} ${doable ? 'is' : 'is not'} listed`);
      }
      await tidal.locator('summary', { hasText: 'Use your own app' }).click();
      const form = tidal.locator('form[data-appform="tidal"]');
      await form.locator('input[name=client_id]').fill(`ui-check-tidal-${width}`);
      await form.locator('input[name=client_secret]').fill('ui-check-secret');
      assert.equal(await form.locator('input[name=client_secret]').getAttribute('type'), 'password');
      await form.locator('button').click();
      await page.locator('.toast', { hasText: 'your own app is in use' }).waitFor();
      const after = panel.locator('.set-row', { has: page.locator('.set-name', { hasText: /^TIDAL$/ }) });
      await after.locator('.set-help', { hasText: 'Your own app' }).first().waitFor();
      assert.ok(!(await page.content()).includes('ui-check-secret'), 'the secret is never shown again');
      const api = (await get()).find((a) => a.id === 'tidal');
      assert.equal(api.from, 'settings');
      assert.ok(!JSON.stringify(await get()).includes('ui-check-secret'), 'the API never answers the secret');
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      assert.ok(overflow <= 0, `no sideways scroll at ${width} px (${overflow})`);
      // Back to the build's own app (or none)
      await after.locator('[data-appreset="tidal"]').click();
      await page.locator('.toast', { hasText: /built-in app is in use|your app is removed/ }).waitFor();
      assert.equal((await get()).find((a) => a.id === 'tidal').from ?? null, before.from ?? null);
      assert.deepEqual(errors, []);
      await ctx.close();
    }
  } finally {
    await browser.close();
  }
  console.log('service apps: only what can be done, built in or your own, limits, own-app form, secret kept back, reset');
})().catch((e) => { console.error(e); process.exit(1); });
