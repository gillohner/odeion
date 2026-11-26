// Your library › Node library › Identify: its two on/off settings are the Settings switch. Before,
// Match new songs automatically and Write IDs into my files were checkboxes with a sentence and a
// bracketed note as their label, and a third paragraph under them; Last.fm's Send what I play kept
// its new state when the node refused it. Real candidate assets on the fixture origin
// (tools/ui-fixture.js), at 1280, 1024 and 768 with a mouse and 412 on a phone:
//   * each is an `input.switch[role=switch]` at its own row's end (.set-ctl, like the AcoustID key
//     row above it), named by the row, with a help line under the name and no text beside it;
//   * the card has no checkbox line and no separate auto paragraph left; the help says when IDs
//     are written (only after you confirm an auto match);
//   * a press sends the same change (auto_match, write_ids), the switch shows it, and a change the
//     node refuses puts it back;
//   * on a phone, the switch takes a press 20 px off its middle (the 44 px tap layer) and nothing
//     overflows sideways;
//   * with the keyboard, Space changes a switch and the focus stays on it when the card draws again;
//   * the rows' names start where the card's own text and buttons do;
//   * Settings › Accounts: a Last.fm Send what I play the node refuses goes back.
//   node tools/ui-identify-switches.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'identify-switches-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const identify = { phase: 'ready', acoustid: true, acoustid_own: false, acoustid_state: 'ok', write_ids: false, auto_match: true,
  progress: { running: false, total: 40, done: 40, identified: 30, suggested: 2, unknown: 8, errors: 0, started_at: 1793631888, finished_at: 1793633013, settled: {} },
  counts: { total: 120, identified: 110, suggested: 2, suggested_missing: 0, unknown: 8, unchecked: 0, skipped: 0, outdated: 0, by: { tags: 10, album: 90, fingerprint: 10 } } };
const lastfm = { source: 'lastfm', chosen: 'own', in_use: 'own', line: 'Signed in', imports: [], places: [], ways: [],
  details: { lastfm: { configured: true, connected: true, user: 'gil', submit: true, pending: 0, app_from: 'config' } } };
const FIXTURES = {
  '/library/identify': { body: identify },
  '/sources/lastfm/signin': { body: lastfm },
  // (the fixture answers these paths whatever the method: the PUTs are taken)
  '/library/identify/settings': { body: {} },
};
// control, the row's name, the field it sends and the value a press from the fixture's state sends
const SWITCHES = [
  ['#idauto', 'Match new songs automatically', 'auto_match', false],
  ['#idwrite', 'Write IDs into my files', 'write_ids', true],
];
// A change the node refuses (500 to the PUT): exactly one PUT goes out, and then the switch shows
// what it showed before the press
const refused = async (p, sel, glob, mobile) => {
  let puts = 0;
  await p.route(glob, (r) => { if (r.request().method() !== 'PUT') return r.fallback(); puts++; return r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Not now' }) }); });
  const before = await p.isChecked(sel);
  await press(p, sel, mobile);
  for (let i = 0; i < 50 && !puts; i++) await sleep(100);
  const back = await p.waitForFunction(([q, v]) => { const e = document.querySelector(q); return e && e.checked === v; }, [sel, before], { timeout: 3000 }).then(() => true, () => false);
  await p.unroute(glob);
  return { puts, before, back };
};
const press = async (p, sel, mobile) => { if (mobile) { const b = await p.locator(sel).boundingBox(); await p.touchscreen.tap(b.x + b.width / 2, b.y + b.height / 2); } else await p.click(sel); };

(async () => {
  const browser = await chromium.launch();
  for (const [w, mobile] of [[1280], [1024], [768], [412, true]]) {
    const tag = `[${w}${mobile ? ' phone' : ''}]`;
    const { page: p, state, context } = await fx.create(browser, w, { role: 'owner', fixtures: FIXTURES, mobile });
    const errors = []; p.on('pageerror', (e) => errors.push(e.message));
    await fx.go(p, '/library/node');
    const found = await p.waitForSelector('#idauto', { timeout: 10000 }).then(() => true, () => false);
    check(found, `${tag} Node library shows the Identify card's settings`);
    if (!found) { await context.close(); continue; }
    for (const [sel, name, field, want] of SWITCHES) {
      const s = await p.evaluate((q) => {
        const e = document.querySelector(q); const row = e.closest('.set-row'); const ctl = e.closest('.set-ctl');
        return {
          sw: e.matches('input[type="checkbox"].switch[role="switch"]'), label: e.getAttribute('aria-label') || '',
          name: (row && row.querySelector('.set-name')?.textContent.trim()) || '', help: (row && row.querySelector('.set-help')?.textContent.trim()) || '',
          inCtl: !!ctl, ctlText: ctl ? ctl.textContent.trim() : null, wrapped: !!e.closest('label'), checked: e.checked,
        };
      }, sel);
      check(s.sw, `${tag} ${name} is a switch (input.switch, role=switch)`);
      check(s.inCtl && s.name === name, `${tag} it sits at the end of its own row, "${name}" (row "${s.name}")`);
      check(s.label === name, `${tag} it is named by its row: "${s.label}"`);
      check(s.ctlText === '' && !s.wrapped, `${tag} no words beside it: "${s.ctlText}"`);
      check(!!s.help && !/[()]/.test(s.help), `${tag} a help line under the name, no bracketed note: "${s.help}"`);
      if (sel === '#idwrite') check(/albums, Pubky tags and Pinakes/.test(s.help) && /only after you confirm/.test(s.help), `${tag} Write IDs says an auto match counts at once but is written only after you confirm it`);
      // A press still sends the change, and the switch shows it
      await p.locator(sel).scrollIntoViewIfNeeded();
      await p.evaluate((q) => { document.querySelector(q).dataset.old = '1'; }, sel);
      const n0 = state.hits.length;
      await press(p, sel, mobile);
      let sent = []; for (let i = 0; i < 30 && !sent.length; i++) { await sleep(100); sent = state.hits.slice(n0).filter((h) => h.m === 'PUT' && h.v === '/library/identify/settings').map((h) => JSON.parse(h.body)[field]); }
      check(sent.length === 1 && sent[0] === want, `${tag} a press on ${name} sends ${field} ${JSON.stringify(want)} once (${JSON.stringify(sent)})`);
      // (the card then draws again from the node's answer, which the fixture keeps as it was)
      if (mobile) {
        const reach = await p.evaluate((q) => {
          const e = document.querySelector(q); const r = e.getBoundingClientRect(); const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
          return [[0, -20], [0, 20], [-20, 0], [20, 0]].map(([dx, dy]) => document.elementFromPoint(cx + dx, cy + dy) === e);
        }, sel);
        check(reach.every(Boolean), `${tag} ${name}: a press 20 px off the middle still lands on it (${reach.join(',')})`);
      }
      const redrawn = await p.waitForFunction((q) => { const e = document.querySelector(q); return e && !e.dataset.old; }, sel, { timeout: 3000 }).then(() => true, () => false);
      check(redrawn, `${tag} ${name}: the node takes it and the card draws again from its answer`);
      if (!mobile) {
        // With the keyboard: Space changes it, and the focus stays on the switch through the redraw
        await p.evaluate((q) => { document.querySelector(q).dataset.old = '1'; }, sel);
        await p.focus(sel); await p.keyboard.press('Space');
        const again = await p.waitForFunction((q) => { const e = document.querySelector(q); return e && !e.dataset.old; }, sel, { timeout: 3000 }).then(() => true, () => false);
        const foc = await p.evaluate(() => document.activeElement && document.activeElement.id);
        check(again && foc === sel.slice(1), `${tag} ${name}: Space changes it and the keyboard stays on it after the redraw (on "${foc}")`);
      }
      // A change the node refuses puts the switch back
      const r = await refused(p, sel, '**/v1/library/identify/settings', mobile);
      check(r.puts === 1 && r.back, `${tag} ${name}: a refused change (${r.puts} PUT) puts the switch back to ${r.before}`);
    }
    const card = await p.evaluate(() => {
      const box = document.querySelector('#idbox');
      return {
        plain: [...box.querySelectorAll('input[type="checkbox"]:not(.switch)')].map((e) => e.id || '?'),
        labelRows: box.querySelectorAll('label.row').length, autoHelp: !!document.querySelector('#idautohelp'),
        rows: [...box.querySelectorAll('.set-row')].map((r) => r.id || r.querySelector('.set-name')?.textContent.trim()),
      };
    });
    check(!card.plain.length && !card.labelRows, `${tag} no checkbox line left in the card (${JSON.stringify(card.plain)}, ${card.labelRows} label rows)`);
    check(!card.autoHelp, `${tag} no separate auto paragraph under them`);
    check(card.rows.length === 3, `${tag} the card's settings are three rows alike: ${JSON.stringify(card.rows)}`);
    // The rows' names start where the card's own text and Identify again do, and the switches end at the card's edge
    const al = await p.evaluate(() => {
      const L = (q) => document.querySelector(q).getBoundingClientRect();
      const names = ['#acidrow', '#idauto', '#idwrite'].map((q) => document.querySelector(q).closest('.set-row').querySelector('.set-name').getBoundingClientRect().left);
      return { names, phase: L('#idphase').left, run: L('#idrun').left, sw: ['#idauto', '#idwrite'].map((q) => L(q).right), box: L('#idbox').right };
    });
    check(al.names.every((l) => Math.abs(l - al.phase) <= 1) && Math.abs(al.run - al.phase) <= 1, `${tag} the rows' names start where the card's text and Identify again do (${al.names.map((l) => l.toFixed(1))} / ${al.phase.toFixed(1)} / ${al.run.toFixed(1)})`);
    check(al.sw.every((r) => r <= al.box + 0.5 && al.box - r <= 8), `${tag} the switches end at the card's edge (${al.sw.map((r) => (al.box - r).toFixed(1))} px in)`);
    await fx.cleanCopy(p).catch(() => {});
    await p.locator('#idwrite').scrollIntoViewIfNeeded();
    await p.locator('#idcard').screenshot({ path: path.join(SHOTS, `identify-${w}.png`) });
    if (mobile) { const ov = await p.evaluate(() => [document.documentElement.scrollWidth, innerWidth]); check(ov[0] <= ov[1], `${tag} nothing overflows sideways (${ov[0]} ≤ ${ov[1]})`); }
    // Settings › Accounts: Last.fm's Send what I play goes back when the node refuses it
    await fx.go(p, '/settings/accounts');
    const lf = '[data-lf="submit"]';
    const has = await p.waitForSelector(lf, { timeout: 10000 }).then(() => true, () => false);
    check(has, `${tag} Settings › Accounts shows Last.fm's Send what I play`);
    if (has) {
      await p.locator(lf).scrollIntoViewIfNeeded();
      const r = await refused(p, lf, '**/v1/lastfm', mobile);
      check(r.puts === 1 && r.back, `${tag} Last.fm: a refused change (${r.puts} PUT) puts the switch back to ${r.before}`);
    }
    check(!errors.length, `${tag} no page errors ${errors.join(' | ')}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
