// The same control inventory must survive different upstream rankings and reloads.
//   node tools/ui-controls-search-check.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const fx = require('./ui-fixture');
const root = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const search = require(path.join(root, 'tools/ui-controls-search'));
const shots = process.argv[3] || '/tmp/control-search-shots';
fs.mkdirSync(shots, { recursive: true });
(async () => {
  const browser = await chromium.launch();
  try {
    const inventories = [];
    for (let shard = 1; shard <= 3; shard++) {
      const { page, context, state } = await fx.create(browser, 1280, {
        searchHits: [{ kind: 'album', id: 'deezer-1007320461', title: shard === 3 ? 'A Day At The Races' : 'A Kind Of Magic' }],
      });
      try {
        await search.install(context);
        let release;
        const gate = new Promise((resolve) => { release = resolve; });
        await context.route(/\/v1\/search\?/, async (route) => { await gate; return route.fallback(); });
        await fx.go(page, '/search?q=queen');
        await page.locator('#scopechips button').first().waitFor();
        try {
          await assert.rejects(search.settle(page, 100), /Search did not finish/, 'an unfinished search cannot become an inventory');
        } finally { release(); }
        await search.settle(page);
        const inventory = () => page.locator('#sresults button').evaluateAll((els) => els.map((b) => (b.title || b.textContent).trim()));
        const first = await inventory();
        assert.ok(first.includes('A Day At The Races'), 'the album missed by the failed CI run stays covered');
        assert.equal(await page.locator('[data-skind="album"] .tile').count(), 12, 'all twelve album controls stay covered');
        assert.ok(await page.locator('[data-skind="artist"] .tile').count() > 0);
        assert.ok(await page.locator('[data-sallsongs]').count() > 0, 'expanded song controls stay covered');
        await page.reload();
        await search.settle(page);
        assert.deepEqual(await inventory(), first, 'reload keeps the inventory used to find each control');
        inventories.push(first);
        await page.locator('[data-scope-in="library"]').click();
        await search.settle(page);
        assert.equal(await page.locator('#sresults .tile').count(), 0, 'scope filtering is preserved');
        await page.locator('[data-scope-in="everywhere"]').click();
        await search.settle(page);
        await page.getByRole('button', { name: /A Day At The Races/ }).click();
        assert.equal(new URL(page.url()).pathname, '/album/deezer-1007320461', 'the retained album control opens its actual item');
        assert.deepEqual(state.errors, []);
        await page.screenshot({ path: path.join(shots, `opened-album-shard-${shard}.png`) });
      } finally { await context.close(); }
    }
    assert.deepEqual(inventories[0], inventories[1]);
    assert.deepEqual(inventories[0], inventories[2]);
    console.log('PASS all three shards and reloads retain the same search controls, including A Day At The Races; pending search refuses inventory');
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
