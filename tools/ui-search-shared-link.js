// A playlist link pasted into the search box (what a playlist's Copy link gives,
// pubky://<key>/pub/social/v1/posts/<id>) opens that shared playlist, as the Playlists page's
// shared-link box does, instead of searching for the link's text. Phone (360/412 px, the Search tab,
// the Android app) and desktop. A bare key still opens the person; other text still searches.
// Opened from the phone's empty Search tab, its Back says Search (it said Results for “”).
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-search-shared-link.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/search-shared-link-shots');
fs.mkdirSync(shots, { recursive: true });

const KEY = 'zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o';
const ID = '0035WKVKQDHSM';
const LINK = `pubky://${KEY}/pub/social/v1/posts/${ID}`;
const shared = {
  summary: { id: ID, name: 'Night drives', items: 1, link: LINK, description: '' },
  entries: [{ kind: 'track', uri: fx.SONG }],
  play_from: { order: { order: ['library'], services: [] }, own_order: false, items: [{ index: 0, uri: fx.SONG, title: fx.track.title, artist: 'Sefa', track_id: 1, plays_from: { service: 'library', label: 'Your library' }, sources: [] }] },
};
const fixtures = { [`/shared/${KEY}/playlists/${ID}`]: { body: shared }, [`/shared/${KEY}/playlists`]: { body: [] } };

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 3000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };
const where = (page) => new URL(page.url()).pathname;
const searched = (state) => state.hits.filter((h) => h.v === '/search').length;

// Paste (one input event with the whole text), as a long-press Paste or Ctrl+V does
const paste = (page, text) => page.evaluate((t) => { const s = document.getElementById('search'); s.focus(); s.value = t; s.dispatchEvent(new Event('input', { bubbles: true })); }, text);

async function run(browser, width, { phone, android }) {
  const tag = `${width}${android ? ' android' : ''}${phone ? ' phone' : ''}`;
  const { context, page, state } = await fx.create(browser, width, { fixtures, ...(phone ? { mobile: true } : {}) });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  try {
    await fx.go(page, '/tracks'); await page.locator('.tracks .track').first().waitFor();
    if (phone) { await page.click('[data-bnav="search"]'); await until(() => where(page) === '/search'); }
    else await page.click('#search');
    for (const [label, text] of [['pubky:// link', LINK], ['link without pubky://', `${KEY}/pub/social/v1/posts/${ID}`], ['link with pubky prefix and a trailing slash', `pubky${KEY}/pub/social/v1/posts/${ID}/`], ['link with the key in capitals', `pubky://${KEY.toUpperCase()}/pub/social/v1/posts/${ID}`]]) {
      const before = searched(state);
      await paste(page, text);
      check(await until(() => where(page) === `/shared/${KEY}/${ID}`), `${tag}: a pasted ${label} opens the shared playlist (at ${where(page)})`);
      check(await until(async () => (await page.locator('#main h1').textContent().catch(() => '')) === 'Night drives'), `${tag}: its name shows`);
      check(searched(state) === before, `${tag}: nothing is searched for the ${label}`);
      const back = ((await page.locator('#main .back').first().textContent().catch(() => '')) || '').trim();
      check(back === (phone ? 'Search' : 'Songs'), `${tag}: its Back says where it goes back to (${back})`);
      if (label === 'pubky:// link') await page.screenshot({ path: path.join(shots, `${width}${android ? '-android' : ''}${phone ? '-phone' : ''}-pasted-link.png`) });
      await page.goBack(); await until(() => where(page) !== `/shared/${KEY}/${ID}`);
      if (!phone) await page.click('#search');
    }
    // Unchanged: a bare key opens the person, and other text searches
    await paste(page, `pubky://${KEY}`);
    check(await until(() => where(page) === `/person/${KEY}`), `${tag}: a bare key still opens the person (at ${where(page)})`);
    await page.goBack(); await until(() => where(page) !== `/person/${KEY}`);
    for (const [label, bad] of [['a post id too long', `${LINK}X`], ['a post id with a letter no id has (I)', LINK.replace(/M$/, 'I')]]) {
      const n = searched(state);
      await paste(page, bad);
      check(await until(() => searched(state) > n), `${tag}: a link with ${label} is searched as text`);
      check(where(page) === '/search', `${tag}: and stays on Search (at ${where(page)})`);
    }
    check(state.errors.length === 0, `${tag}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    await run(browser, 360, { phone: true });
    await run(browser, 412, { phone: true });
    await run(browser, 360, { phone: true, android: true });
    await run(browser, 1280, {});
  } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} FAIL`);
  if (fail) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
