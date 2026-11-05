// Settings: whose TIDAL, Qobuz and Spotify sign-ins people play with (TASK-source-policy-node-or-own).
// The owner picks it on This node › Service apps ("Sources for everyone"); a listener sees which
// sign-ins they play with, and under "Each person chooses" switches between the node's and their own.
// Fixture only (no node): node tools/ui-source-policy.js <repo> <screenshots dir>
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('playwright');
const { create, go, cleanCopy } = require('./ui-fixture');
const OUT = process.argv[3] || '/tmp/ui-source-policy';
fs.mkdirSync(OUT, { recursive: true });
const KEY = 'tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy';
const puts = (state, v) => state.hits.filter((h) => h.v === v && h.m === 'PUT').map((h) => JSON.parse(h.body || '{}'));

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [390, 820, 1280]) {
      // The owner: the three choices, today's ("own") marked, a click saves the node's setting
      {
        const { context, page, state } = await create(browser, width, {
          fixtures: { '/node/settings': { body: { contact: '', discovery: false, token_replaceable: true, sources_policy: 'own' } }, '/node/apps': { body: [] } },
        });
        await go(page, '/settings/apps');
        const chips = page.locator('[data-srcpolicy]');
        await chips.first().waitFor();
        assert.equal(await chips.count(), 3, 'three choices');
        assert.equal(await page.locator('[data-srcpolicy="own"]').getAttribute('aria-checked'), 'true', 'today\'s choice is marked');
        assert.match(await page.locator('.set-group', { hasText: 'Sources for everyone' }).innerText(), /keeps every sign-in/);
        await cleanCopy(page);
        await page.screenshot({ path: path.join(OUT, `owner-${width}.png`), fullPage: false });
        await page.locator('[data-srcpolicy="choice"]').click();
        await page.locator('.toast', { hasText: 'Each person chooses' }).waitFor();
        assert.deepEqual(puts(state, '/node/settings').at(-1), { sources_policy: 'choice' });
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // A listener under "Each person chooses": the line and the switch; a click saves their choice
      {
        const { context, page, state } = await create(browser, width, {
          role: 'listener',
          fixtures: {
            '/me': { body: { role: 'listener', key: KEY, zones: null } },
            '/me/sources': { body: { policy: 'choice', use: 'node', plays_with: 'node', may_choose: true } },
            '/profiles': { body: [] },
          },
        });
        await go(page, '/settings');
        const row = page.locator('.set-row', { hasText: 'Music services' });
        await row.waitFor();
        assert.match(await row.innerText(), /through this node’s sign-ins/);
        assert.equal(await row.locator('[data-mysrc]').count(), 2, 'node’s and own');
        assert.equal(await row.locator('[data-mysrc="node"]').getAttribute('aria-checked'), 'true');
        await cleanCopy(page);
        await page.screenshot({ path: path.join(OUT, `listener-choice-${width}.png`), fullPage: false });
        await row.locator('[data-mysrc="own"]').click();
        await page.locator('.toast', { hasText: 'your own sign-ins' }).waitFor();
        assert.deepEqual(puts(state, '/me/sources').at(-1), { use: 'own' });
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      // A listener under "This node's": the line, no switch
      {
        const { context, page, state } = await create(browser, width, {
          role: 'listener',
          fixtures: {
            '/me': { body: { role: 'listener', key: KEY, zones: null } },
            '/me/sources': { body: { policy: 'node', use: 'node', plays_with: 'node', may_choose: false } },
            '/profiles': { body: [] },
          },
        });
        await go(page, '/settings');
        const row = page.locator('.set-row', { hasText: 'Music services' });
        await row.waitFor();
        assert.match(await row.innerText(), /through this node’s sign-ins/);
        assert.equal(await row.locator('[data-mysrc]').count(), 0, 'no switch when the owner decides');
        await cleanCopy(page);
        await page.screenshot({ path: path.join(OUT, `listener-node-${width}.png`), fullPage: false });
        assert.deepEqual(state.errors, [], 'no page errors');
        await context.close();
      }
      console.log(`ok ${width}`);
    }
  } finally {
    await browser.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });
