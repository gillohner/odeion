// A form that saves, and Enter in a field beside a button: the button shows progress at once and
// the action is sent once. Real candidate assets on the fixture origin (tools/ui-fixture.js), the
// answer delayed:
//   * Node library › Identify, the AcoustID key: Enter twice in the field saves once, and Save is
//     busy meanwhile (the handler returned nothing, so the page's busy mark never saw the save);
//   * Collections, a shared link: Enter in the field presses Open (busy, one request);
//   * tools/design-lint.js keeps it: no form's submit handler in the page returns nothing.
//   node tools/ui-busy-forms.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const { execFileSync } = require('node:child_process');
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const REPO = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'busy-forms-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const t0 = Date.now(); for (;;) { const v = await fn().catch(() => null); if (v) return v; if (Date.now() - t0 > ms) return v; await sleep(40); } };

const KEY = 'o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy';
const identify = { phase: 'ready', acoustid: false, acoustid_own: false, acoustid_state: 'none', write_ids: false, auto_match: true,
  progress: { running: false, total: 200, done: 200, identified: 120, suggested: 20, unknown: 40, errors: 20, settled: {}, started_at: 1793631888, finished_at: 1793633013 },
  counts: { total: 200, identified: 120, suggested: 20, unknown: 40, unchecked: 0, by: { tags: 100, album: 20, fingerprint: 0, you: 0 } } };
const fixtures = {
  '/pubky': { body: { user: 'pubky' + KEY, unsynced: 0, needs_resign: false, sync: null } },
  '/storage': { body: { homeserver: { mode: 'external', user: KEY, quota_mb: 'unlimited' }, saving: { state: 'saved', pending: 0 } } },
  '/library/identify': { body: identify },
  '/library/identify/settings': { body: { ...identify, acoustid: true, acoustid_own: true } },
  '/library/folders': { body: [] }, '/library/webdav': { body: [] }, '/library/drives': { body: { providers: [], roots: [] } }, '/library/hidden': { body: [] }, '/node/settings': { body: { contact: '', discovery: false } },
};
const SLOW = 900;
const busyNow = (page, sel) => page.evaluate((s) => { const b = document.querySelector(s); return b ? { busy: b.getAttribute('aria-busy') === 'true', off: b.getAttribute('aria-disabled') === 'true' || b.disabled } : null; }, sel);

(async () => {
  // the lint that keeps it
  let lint = ''; let lintOk = true;
  try { lint = execFileSync(process.execPath, [path.join(REPO, 'tools', 'design-lint.js'), path.join(REPO, 'crates', 'pm-api', 'static')], { encoding: 'utf8' }); } catch (e) { lintOk = false; lint = String(e.stdout || e.message); }
  check(lintOk && !/submit handler returns nothing/.test(lint), `no form's submit handler returns nothing (${lintOk ? 'design lint ok' : lint.split('\n').filter((l) => /submit handler/.test(l)).length + ' found'})`);

  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const at = `${width}px`;
    {
      const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures, delay: (v, m) => (m === 'PUT' && v === '/library/identify/settings' ? SLOW : 0) });
      await fx.go(page, '/library/node');
      const form = await until(() => page.evaluate(() => !!document.querySelector('#acidform #acidkey')), 8000);
      check(form, `${at} AcoustID key: the form is on Node library`);
      if (form) {
        await page.fill('#acidkey', 'A1b2C3d4E5');
        await page.focus('#acidkey'); await page.keyboard.press('Enter'); await sleep(120);
        const during = await busyNow(page, '#acidsave');
        await page.keyboard.press('Enter'); await sleep(60); await page.keyboard.press('Enter');
        if (width === 1280) await page.screenshot({ path: path.join(SHOTS, 'acoustid-saving-1280.png') });
        check(!!during && during.busy && during.off, `${at} AcoustID key: Save is busy while the key is on its way (${JSON.stringify(during)})`);
        await sleep(SLOW + 500);
        const puts = state.hits.filter((h) => h.m === 'PUT' && h.v === '/library/identify/settings').length;
        check(puts === 1, `${at} AcoustID key: Enter three times saved once (${puts} requests)`);
        const after = await busyNow(page, '#acidsave');
        check(!after || !after.busy, `${at} AcoustID key: and Save is free again after it (${JSON.stringify(after)})`);
      }
      check(!state.errors.length, `${at} AcoustID key: no page error (${state.errors.join(' | ') || 'none'})`);
      await context.close();
    }
    {
      const link = `pubky://${KEY}/pub/pubky.app/collections/0035WKVKQDHSM`;
      const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/shared/resolve': { status: 404, body: { error: 'no such collection' } } }, delay: (v) => (v === '/shared/resolve' ? SLOW : 0) });
      await fx.go(page, '/playlists');
      const field = await until(() => page.evaluate(() => !!document.querySelector('#plshared') && !!document.querySelector('#plopenshared')), 8000);
      check(field, `${at} shared link: the field and Open are on the page`);
      if (field) {
        await page.fill('#plshared', link);
        await page.focus('#plshared'); await page.keyboard.press('Enter'); await sleep(120);
        const during = await busyNow(page, '#plopenshared');
        await page.keyboard.press('Enter');
        check(!!during && during.busy, `${at} shared link: Enter in the field presses Open, busy while the link is looked up (${JSON.stringify(during)})`);
        await sleep(SLOW + 500);
        const gets = state.hits.filter((h) => h.v === '/shared/resolve').length;
        check(gets === 1, `${at} shared link: Enter twice asked once (${gets} requests)`);
      }
      check(!state.errors.length, `${at} shared link: no page error (${state.errors.join(' | ') || 'none'})`);
      await context.close();
    }
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
