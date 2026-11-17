// Big counts read the same everywhere: "19,800 more", not "19800 more". Library › My music's
// Show more, Save now's "Saved: …" toast and Which song is this?'s "Last run: …" line, at phone and desktop widths, with an en-US browser.
// Real candidate assets, isolated responses: node tools/ui-count-separators.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'count-separators-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

const KEY = 'o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy';
const files = Array.from({ length: 200 }, (_, i) => ({ path: `/priv/pubky.music/files/${i}.flac`, name: `${i}.flac`, size: 30e6, song: { id: i + 1, title: `Song ${i}`, artist: 'Sefa', album: 'Journey' } }));
const fixtures = {
  '/pubky': { body: { user: 'pubky' + KEY, unsynced: 0, needs_resign: false, sync: null } },
  '/storage': { body: { homeserver: { mode: 'external', user: KEY, quota_mb: 'unlimited' }, saving: { state: 'saved', pending: 0 } } },
  '/me/music': { body: { files, count: 20000, bytes: 20000 * 30e6 } },
  '/library/identify': { body: { phase: 'ready', acoustid: true, acoustid_own: true, acoustid_state: 'ok', write_ids: false,
    progress: { running: false, total: 20000, done: 20000, identified: 12840, suggested: 1200, unknown: 3400, errors: 1500, settled: { album: 2000 }, started_at: 1793631888, finished_at: 1793633013 },
    counts: { total: 20000, identified: 12840, suggested: 1200, unknown: 3400, unchecked: 0, by: { tags: 1000, album: 11840, fingerprint: 0, you: 0 } } } },
  '/pubky/sync': { body: { pushed: 1200, pulled: 3400, errors: [], pending: 1500, notes: [] } },
};

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [w, opts] of [[360, { mobile: true }], [1280, {}]]) {
      const { page, context, state } = await fx.create(browser, w, { ...opts, fixtures });
      try {
        await fx.go(page, '/library');
        await page.waitForSelector('#mmlist [data-mmmore]');
        const more = (await page.textContent('#mmlist [data-mmmore]')).replace(/\s+/g, ' ').trim();
        check(/\(19,800 more\)/.test(more), `${w} px My music's Show more says "19,800 more" (${more})`);
        await page.locator('#mmlist [data-mmmore]').scrollIntoViewIfNeeded();
        await page.screenshot({ path: path.join(SHOTS, `my-music-more-${w}.png`) });
        await page.click('#sync');
        // #toast is always in the page (hidden by opacity): wait for the message itself
        await page.waitForFunction(() => /Saved:/.test(document.querySelector('#toast.show')?.textContent || ''), null, { timeout: 4000 }).catch(() => {});
        const toast = await page.textContent('#toast');
        check(/Saved: 1,200 sent to your homeserver, 3,400 received; 1,500 more going out/.test(toast), `${w} px Save now's toast has separators (${toast.trim()})`);
        await page.screenshot({ path: path.join(SHOTS, `save-now-toast-${w}.png`) });
        // Node library's Which song is this? card: the last run's line, after a whole library
        await fx.go(page, '/library/node');
        const last = await page.waitForFunction(() => [...document.querySelectorAll('#main p')].map((p) => p.textContent).find((t) => /^Last run:/.test(t)), null, { timeout: 5000 }).then((h) => h.jsonValue()).catch(() => '');
        check(/^Last run: 12,840 identified.*1,200 to confirm, 3,400 not found, 1,500 errors; 2,000 unsure songs/.test(last), `${w} px Which song is this?'s last run has separators (${last})`);
        await page.screenshot({ path: path.join(SHOTS, `identify-last-run-${w}.png`) });
        check(!state.errors.length, `${w} px no page errors ${state.errors.join('; ')}`);
      } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
