// Room around a card's buttons. A button written right after a line of text, or right after another
// button with only a space between them, sat on the text (0 px) or 4 px from its neighbour:
//   * Home › Get started › "Play on your speakers": Look for speakers | Which speakers work;
//   * Home › Get started › "Create your Pubky account here" (node with its own homeserver):
//     Create account here | Use an existing homeserver;
//   * Your library › My music, not signed in: the sentence, then Sign in;
//   * Favourites (and Playlists) on a listener's homeserver not connected yet: the note, then Connect.
// Each pair now has at least 8 px between them (beside or under each other), at 1280, 1024, 768 and
// 360 px, and nothing is wider than the screen.
//   node tools/ui-action-gaps.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'action-gaps-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const MIN = 8;

const steps = (ids) => ({ show: true, hidden: false, steps: ids.map((id) => ({ id, done: false, optional: id !== 'play' })) });
const noHs = { configured: false, reachable: false };
const hsHere = { configured: true, reachable: true, public_key: 'fixturekey', users: 0 };
const notConnected = { connected: false, lost: false };

// The space between two boxes: across if they share a line, else down from the upper one.
const gapOf = (page, a, b) => page.evaluate(([a, b]) => {
  const ea = typeof a === 'string' ? document.querySelector(a) : null;
  const ra = ea?.getBoundingClientRect(); const eb = ea && document.querySelector(b); const rb = eb?.getBoundingClientRect();
  if (!ra || !rb || !ra.width || !rb.width) return null;
  const sameLine = ra.top < rb.bottom && rb.top < ra.bottom;
  return Math.round(sameLine ? (rb.left >= ra.right ? rb.left - ra.right : ra.left - rb.right) : (rb.top >= ra.bottom ? rb.top - ra.bottom : ra.top - rb.bottom));
}, [a, b]);
const overflow = (page) => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);

const cases = [
  { name: 'home-speakers', path: '/', opts: { fixtures: { '/me/first-steps': { body: steps(['play', 'speakers', 'pubky']) }, '/homeserver/bundled': { body: noHs } } },
    ready: '#setupspeakers', a: '#setupspeakers', b: '[data-step="speakers"] a.lnk', what: 'Look for speakers | Which speakers work' },
  { name: 'home-pubky-here', path: '/', opts: { fixtures: { '/me/first-steps': { body: steps(['play', 'pubky']) }, '/homeserver/bundled': { body: hsHere } } },
    ready: '#setupcreate', a: '#setupcreate', b: '#setupsignin', what: 'Create account here | Use an existing homeserver' },
  { name: 'mymusic-signin', path: '/library', opts: {},
    ready: '[data-goto-accounts]', a: '.card:has(> [data-goto-accounts]) > p', b: '[data-goto-accounts]', what: 'My music: the sentence, then Sign in' },
  { name: 'favourites-hsnote', path: '/favourites', opts: { role: 'listener', fixtures: { '/favourites': { body: { uris: [], tracks: [], albums: [], artists: [], other: [], homeserver: notConnected } } } },
    ready: '#hsnote .btn', a: '#hsnote > p', b: '#hsnote > .btn', what: 'Favourites: the homeserver note, then its button' },
  { name: 'playlists-hsnote', path: '/playlists', opts: { role: 'listener', fixtures: { '/me/homeserver': { body: notConnected } } },
    ready: '#hsnote .btn', a: '#hsnote > p', b: '#hsnote > .btn', what: 'Playlists: the homeserver note, then its button' },
];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 360]) {
      for (const c of cases) {
        const { page, context, state } = await fx.create(browser, width, c.opts);
        try {
          await fx.go(page, c.path);
          const shown = await page.waitForSelector(c.ready, { timeout: 8000 }).then(() => true, () => false);
          check(shown, `${width} ${c.name}: it is on the page (${c.ready})`);
          if (!shown) continue;
          const g = await gapOf(page, c.a, c.b);
          check(g !== null && g >= MIN, `${width} ${c.name}: ${c.what} have room between them (${g} px, want >= ${MIN})`);
          const o = await overflow(page);
          check(o <= 0, `${width} ${c.name}: nothing wider than the screen (${o})`);
          check(!state.errors.length, `${width} ${c.name}: no page errors (${state.errors.join(' | ')})`);
          await page.locator(c.ready).first().scrollIntoViewIfNeeded();
          await page.screenshot({ path: path.join(SHOTS, `${c.name}-${width}.png`) });
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
