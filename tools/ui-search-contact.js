// Search › a source waiting for the node's contact (Pubky without an email or website): its note says
// so in plain words, and for the node's owner one press opens Settings › Discover with the field ready.
// A listener reads who can set it and gets no button; a source that didn't answer keeps its Retry.
// Real UI in Chromium against fixtures (no node).
//   node tools/ui-search-contact.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/search-contact-shots');
fs.mkdirSync(shots, { recursive: true });

const hit = { kind: 'song', id: 'lib-1', title: fx.track.title, subtitle: 'Sefa', track_id: 1, uri: 'track:1', keys: ['track:1'] };
const lines = [
  { source: 'library', status: 'ok', hits: [hit], counts: [] },
  // The node's own words (pm-api search.rs, pubky())
  { source: 'pubky', status: 'error', error: 'Pubky search needs the upstream contact (Settings)', hits: [], counts: [] },
  { source: 'deezer', status: 'error', error: 'deezer: answered 503 Service Unavailable', hits: [], counts: [] },
];
const body = lines.map((l) => JSON.stringify(l)).join('\n') + '\n';

async function open(browser, width, role) {
  const run = await fx.create(browser, width, { role });
  await run.context.route(/\/v1\/search\?/, (route) => route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': 'library,pubky,deezer' }, body }));
  await fx.go(run.page, '/search?q=Journey');
  await run.page.waitForFunction(() => window.pmLastSearch && window.pmLastSearch.q === 'Journey' && document.querySelectorAll('.snote').length === 2);
  return run;
}
const notes = (page) => page.$$eval('.snote', (els) => els.map((e) => ({ text: e.textContent.replace(/\s+/g, ' ').trim(), title: e.getAttribute('title') || '', contact: !!e.querySelector('[data-scontact]'), retry: !!e.querySelector('[data-sretry]') })));

(async () => {
  const browser = await chromium.launch();
  let fails = 0;
  const check = async (name, fn) => { try { await fn(); console.log('ok  ', name); } catch (e) { fails++; console.log('FAIL', name, '-', e.message.split('\n')[0]); } };
  for (const width of [1280, 1024, 768, 360]) {
    await check(`owner: plain words and one press to the field (${width})`, async () => {
      const { page, context, state } = await open(browser, width, 'owner');
      try {
        const [pubky, deezer] = await notes(page);
        await page.locator('.snotes').screenshot({ path: path.join(shots, `owner-note-${width}.png`) });
        assert.match(pubky.text, /^Pubky search needs your email or website/, 'plain words');
        assert.doesNotMatch(pubky.text, /upstream|\(Settings\)/i, 'no node jargon');
        assert.doesNotMatch(pubky.title, /upstream/i, 'no node jargon in the tooltip either');
        assert.ok(pubky.contact && !pubky.retry, 'Add it, not Retry (trying again cannot help)');
        assert.ok(deezer.retry && !deezer.contact, 'a source that did not answer keeps its Retry');
        await fx.cleanCopy(page);
        const add = page.locator('[data-scontact]');
        assert.equal(await add.count(), 1, 'one Add it');
        assert.equal((await add.innerText()).trim(), 'Add it');
        const box = await add.boundingBox(); assert.ok(box.height >= 28, `button height ${box.height}`);
        // Keyboard: Enter on the button
        await add.focus(); await page.keyboard.press('Enter');
        await page.waitForFunction(() => location.pathname === '/settings/discover' && document.activeElement && document.activeElement.id === 'contact');
        await page.screenshot({ path: path.join(shots, `owner-settings-${width}.png`) });
        await fx.cleanCopy(page);
        await page.goBack();
        await page.waitForFunction(() => location.pathname === '/search' && document.querySelector('[data-scontact]'));
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
    await check(`listener: says who can set it, no button (${width})`, async () => {
      const { page, context, state } = await open(browser, width, 'listener');
      try {
        const [pubky] = await notes(page);
        await page.locator('.snotes').screenshot({ path: path.join(shots, `listener-note-${width}.png`) });
        assert.match(pubky.text, /^Pubky search needs an email or website from the node’s owner/, 'plain words for a listener');
        assert.doesNotMatch(pubky.text, /upstream|\(Settings\)/i, 'no node jargon');
        assert.doesNotMatch(pubky.title, /upstream/i, 'no node jargon in the tooltip either');
        assert.ok(!pubky.contact && !pubky.retry, 'nothing to press');
        await fx.cleanCopy(page);
        assert.deepEqual(state.errors, [], 'no page errors');
      } finally { await context.close(); }
    });
  }
  await browser.close();
  if (fails) { console.log(`${fails} FAIL`); process.exit(1); }
  console.log('all passed');
})().catch((e) => { console.error(e); process.exit(1); });
