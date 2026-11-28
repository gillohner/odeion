// Settings: a group heading names what its rows have in common, so it never repeats the tab's own
// title, nor the name of the first row under it. Privacy put a "Usage insights" heading over its
// "Usage insights" row (then What is recorded, Clear mine); Usage insights said it three times
// (title, heading, row; then Stored); a listener's Settings had a "Usage insights" group over a
// "Usage insights" row. Now Privacy and Usage insights have no group heading (the title names the
// page), and a listener's group is "Privacy", the owner's tab for the same rows. Play from had a
// "Play from" heading under its "Play from" title: now its source order sits right under the title. The usage routes
// answer as a working node's do (usage insights on, shared), so the pages are the real ones.
// Every owner tab is checked, at a computer's, a tablet's and a phone's width (each tab its own
// page there), and a listener's one-page Settings.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-settings-headings.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/settings-headings-shots');
fs.mkdirSync(shots, { recursive: true });

let failed = 0; let passed = 0;
const check = (ok, what) => { if (ok) { passed++; console.log(`ok   ${what}`); } else { failed++; console.log(`FAIL ${what}`); } };
// Each group of a scope (a panel, or a listener's page): its heading and its rows' names. A group
// that folds on a phone (People & access, .set-fold) carries its heading in its summary.
const groupsOf = (page, sel) => page.$$eval(`${sel} .set-group`, (gs) => gs.map((g) => ({
  h: (g.querySelector(':scope > .set-h, :scope > summary > .set-h') || {}).textContent || '',
  fold: g.classList.contains('set-fold'),
  names: [...g.querySelectorAll('.set-name')].map((n) => n.textContent.trim()),
})));
const same = (a, b) => a.trim().toLowerCase() === b.trim().toLowerCase();
const repeats = (title, groups) => groups.filter((g) => g.h && ((title && same(g.h, title)) || (g.names.length && same(g.h, g.names[0]))));
// The node's usage routes (usage.rs), on and shared, the notice already seen (ui-usage.js has the full fake)
const usageRoutes = (context, role) => {
  const limits = { raw_retention_days: 14, aggregate_retention_days: 90 };
  const j = (r, body) => r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  return context.route(/\/v1\/(usage|node\/usage)(\/|$|\?)/, (r) => {
    const p = new URL(r.request().url()).pathname.slice(3); const m = r.request().method();
    if (p === '/usage/me' && m === 'GET') return j(r, { schema: 'pm-usage/1', saved: true, node_on: true, generation: 1, sharing: true, choice: 1, notice_seen: 1, notice_version: 1, role, limits });
    if (p === '/node/usage' && m === 'GET') return j(r, { enabled: true, saved: true, cleanup_pending: false, generation: 1, aliases: 1, waiting: 0, events: 0, bytes: 0, coverage: { owner: 'instrumented', listeners: 'instrumented', guests: 'not instrumented' }, limits });
    if (p === '/usage/sessions' && m === 'POST') return j(r, { alias: 'a000000000000001', idle_s: 1800, expires_in_s: 43200, max_batch: 50 });
    return r.fulfill({ status: 204, body: '' });
  });
};
const fits = (page) => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);

// [width, fixture options, name]
const SIZES = [[1280, {}, '1280'], [768, {}, '768'], [412, { mobile: true }, '412 phone']];

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [w, opts, name] of SIZES) {
      const { page, context, state } = await fx.create(browser, w, opts);
      try {
        await usageRoutes(context, 'owner');
        await fx.go(page, '/settings');
        await page.waitForSelector('#main .set-tab');
        const tabs = await page.$$eval('#main .set-tab', (ts) => ts.map((t) => t.dataset.stab));
        check(['privacy', 'usage', 'playfrom'].every((t) => tabs.includes(t)), `${name}: Settings has Privacy, Usage insights and Play from (${tabs.join(' ')})`);
        for (const t of tabs) {
          await fx.go(page, `/settings/${t}`);
          const sel = `#main .set-panel[data-panel="${t}"]`;
          await page.waitForSelector(`${sel}:not([hidden]) .set-title`);
          // Usage insights draws its rows once the node answers (usage.js; a Loading… row before)
          if (t === 'privacy') await page.waitForSelector('#usageme [data-usage="stop"]');
          if (t === 'usage') await page.waitForSelector('#usagenode [data-usage-node]');
          // Play from draws its order once the node answers; People & access its groups
          const drawn = { playfrom: '#gorder .porder-row', people: '#peoplebox .set-group' }[t];
          if (drawn) check(await page.waitForSelector(drawn, { timeout: 8000 }).then(() => true, () => false), `${name} ${t}: its rows are drawn (${drawn})`);
          const title = await page.$eval(`${sel} .set-title`, (e) => e.textContent.trim());
          const groups = await groupsOf(page, sel);
          const bad = repeats(title, groups);
          const what = `${name} ${t} "${title}": no group heading repeats the title or its first row (${JSON.stringify(groups.map((g) => [g.h, g.names.slice(0, 2)]))})`;
          check(!bad.length, what);
          // A phone folds People & access into groups whose heading is in the summary: those are read too
          if (t === 'people' && opts.mobile) check(groups.some((g) => g.fold && g.h), `${name} people: its folding groups' headings are read (${JSON.stringify(groups.filter((g) => g.fold).map((g) => g.h))})`);
          if (t === 'privacy' || t === 'usage' || t === 'playfrom') {
            const g = await page.evaluate((s) => {
              const p = document.querySelector(s); const head = p.querySelector('.set-title').getBoundingClientRect(); const card = p.querySelector('.set-rows').getBoundingClientRect();
              return { hs: p.querySelectorAll('.set-h').length, gap: Math.round(card.top - head.bottom), row: (p.querySelector('.set-rows .set-name') || {}).textContent, rows: p.querySelectorAll('.set-rows .set-name').length, order: !!p.querySelector('.set-rows #gorder .porder-row') };
            }, sel);
            if (t === 'playfrom') check(g.hs === 0 && g.row === 'Where songs play from' && g.order, `${name} ${t}: the source order right under the title, no "Play from" heading between (${JSON.stringify(g)})`);
            else check(g.hs === 0 && g.row === 'Usage insights' && g.rows >= 2, `${name} ${t}: the Usage insights rows right under the title, no heading between (${JSON.stringify(g)})`);
            check(g.gap >= 4 && g.gap <= 32, `${name} ${t}: the row sits a heading's gap under the title, not glued to it or a heading's room away (${g.gap} px)`);
            await page.screenshot({ path: path.join(shots, `after-settings-${t}-${w}.png`) });
          }
          check(await fits(page), `${name} ${t}: nothing wider than the window`);
        }
        await fx.cleanCopy(page);
        check(!state.errors.length, `${name}: no page errors ${state.errors.join('; ')}`);
      } catch (e) { failed++; console.log(`FAIL ${name}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
    }
    // A listener: one page of groups; Usage insights sits in "Privacy", as on the owner's tab
    for (const [w, opts, name] of [[1280, {}, 'listener 1280'], [412, { mobile: true }, 'listener 412 phone']]) {
      const { page, context, state } = await fx.create(browser, w, { ...opts, role: 'listener', token: 'pml_fixture-listener' });
      try {
        await usageRoutes(context, 'listener');
        await fx.go(page, '/settings');
        await page.waitForSelector('#main #usageme [data-usage="stop"]');
        const groups = await groupsOf(page, '#main');
        const bad = repeats('', groups);
        check(!bad.length, `${name}: no group heading repeats its first row (${JSON.stringify(bad)})`);
        const usage = await page.$eval('#usageme', (e) => (e.closest('.set-group').querySelector('.set-h') || {}).textContent);
        check(usage === 'Privacy', `${name}: Usage insights is in the Privacy group ("${usage}")`);
        check(await fits(page), `${name}: nothing wider than the window`);
        await page.locator('#usageme').scrollIntoViewIfNeeded();
        await page.screenshot({ path: path.join(shots, `after-listener-${w}.png`) });
        await fx.cleanCopy(page);
        check(!state.errors.length, `${name}: no page errors ${state.errors.join('; ')}`);
      } catch (e) { failed++; console.log(`FAIL ${name}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
    }
  } finally { await browser.close(); }
  console.log(`\n${passed} ok, ${failed} FAIL`);
  process.exit(failed ? 1 : 0);
})();
