// Settings: every on/off setting is one switch (the one Display, Sources and Last.fm use). Before,
// Prefer full tracks, First steps on Home and Developer outputs were a checkbox with a word ("On",
// "Show") and ListenBrainz's Send what I play a checkbox line, beside switches elsewhere.
// Real candidate assets on the fixture origin (tools/ui-fixture.js), at 1280, 1024 and 768 with a
// mouse and 412 on a phone:
//   * each of the four is an `input.switch[role=switch]` at its row's end (.set-ctl), named by its
//     row's own name, with no "On"/"Show" word beside it;
//   * no plain checkbox is left at a Settings row's end on any tab (all panels are in the page);
//   * a press still sends the same change (prefer_full, first_steps_hidden, dev_outputs, ListenBrainz
//     submit), and the switch shows it; a change ListenBrainz refuses puts its switch back;
//   * ListenBrainz's row lines up with the card's pills and buttons;
//   * on a phone, the switch takes a press 20 px off its middle (the 44 px tap layer) and nothing
//     overflows sideways.
//   node tools/ui-settings-switches.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'settings-switches-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const FIXTURES = {
  '/node/settings': { body: { contact: '', discovery: false, token_replaceable: true, prefer_full: true, dev_outputs: false, first_steps_hidden: false } },
  '/listenbrainz': { body: { connected: true, user: 'gil', submit: true, pending: 0 } },
  '/listenbrainz/playlists': { body: [] },
};
// tab, control, the row's name, the request it sends and the value a press from the fixture's state sends
const SWITCHES = [
  ['playback', '#preferfull', 'Prefer full tracks', '/node/settings', (b) => b.prefer_full, false],
  ['nodehome', '#firststeps', 'First steps on Home', '/node/settings', (b) => b.first_steps_hidden, true],
  ['about', '#devouts', 'Developer outputs', '/node/settings', (b) => b.dev_outputs, true],
  ['accounts', '#lbsubmit', 'Send what I play', '/listenbrainz', (b) => b.submit, false],
];

(async () => {
  const browser = await chromium.launch();
  for (const [w, mobile] of [[1280], [1024], [768], [412, true]]) {
    const tag = `[${w}${mobile ? ' phone' : ''}]`;
    const run = await fx.create(browser, w, { fixtures: FIXTURES, mobile });
    const { page: p, state, context } = run;
    const errors = []; p.on('pageerror', (e) => errors.push(e.message));
    for (const [tab, sel, name, url, read, want] of SWITCHES) {
      await fx.go(p, `/settings/${tab}`);
      const found = await p.waitForSelector(sel, { timeout: 5000 }).then(() => true, () => false);
      check(found, `${tag} Settings › ${tab} has ${name}`);
      if (!found) continue;
      const s = await p.evaluate((q) => {
        const e = document.querySelector(q); const row = e.closest('.set-row'); const ctl = e.closest('.set-ctl');
        return {
          sw: e.matches('input[type="checkbox"].switch[role="switch"]'), label: e.getAttribute('aria-label') || '',
          name: (row && row.querySelector('.set-name')?.textContent.trim()) || '', inCtl: !!ctl,
          ctlText: ctl ? ctl.textContent.trim() : null, wrapped: !!e.closest('label'), checked: e.checked,
        };
      }, sel);
      check(s.sw, `${tag} ${name} is a switch (input.switch, role=switch)`);
      check(s.inCtl && s.name === name, `${tag} it sits at the end of its own row, "${name}" (row "${s.name}")`);
      check(s.label.startsWith(name), `${tag} it is named by its row: "${s.label}"`);
      check(s.ctlText === '' && !s.wrapped, `${tag} no word beside it (was "On"/"Show"): "${s.ctlText}"`);
      // A press still sends the change, and the switch shows it
      await p.locator(sel).scrollIntoViewIfNeeded();
      const n0 = state.hits.length;
      if (mobile) { const b = await p.locator(sel).boundingBox(); await p.touchscreen.tap(b.x + b.width / 2, b.y + b.height / 2); } else await p.click(sel);
      let sent = []; for (let i = 0; i < 30 && !sent.length; i++) { await sleep(100); sent = state.hits.slice(n0).filter((h) => h.m === 'PUT' && h.v === url).map((h) => read(JSON.parse(h.body))); }
      check(sent.length === 1 && sent[0] === want, `${tag} a press on ${name} sends ${url} ${JSON.stringify(want)} once (${JSON.stringify(sent)})`);
      // (First steps draws the page again from the node's answer, which the fixture keeps as it was)
      if (tab !== 'nodehome') { const now = await p.isChecked(sel); check(now === !s.checked, `${tag} and the switch shows it (${s.checked} → ${now})`); }
      if (tab === 'accounts') {
        // A change the node refuses puts the switch back, as the other three do
        await p.route('**/v1/listenbrainz', (r) => (r.request().method() === 'PUT' ? r.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'ListenBrainz is not answering' }) }) : r.fallback()));
        const before = await p.isChecked(sel);
        if (mobile) { const b = await p.locator(sel).boundingBox(); await p.touchscreen.tap(b.x + b.width / 2, b.y + b.height / 2); } else await p.click(sel);
        const back = await p.waitForFunction(([q, v]) => document.querySelector(q).checked === v && !document.querySelector(q).matches('[aria-busy="true"]'), [sel, before], { timeout: 3000 }).then(() => true, () => false);
        check(back, `${tag} ListenBrainz: a refused change puts the switch back (${before} → ${await p.isChecked(sel)})`);
        await p.unroute('**/v1/listenbrainz');
        // Lined up with the card's own pills and buttons
        const al = await p.evaluate(() => {
          const box = document.querySelector('#lbbody'); const L = (q) => { const e = box.querySelector(q); return e ? e.getBoundingClientRect() : null; };
          const pill = L('.pill'), name = L('.set-name'), btn = L('#lbimport'), sw = L('#lbsubmit'), body = box.getBoundingClientRect();
          return { pill: pill.left, name: name ? name.left : NaN, btn: btn.left, swRight: sw.right, bodyRight: body.right };
        });
        check(Math.abs(al.name - al.pill) <= 1 && Math.abs(al.name - al.btn) <= 1, `${tag} ListenBrainz: "Send what I play" starts where the pills and buttons do (${al.name.toFixed(1)} / ${al.pill.toFixed(1)} / ${al.btn.toFixed(1)})`);
        check(al.swRight <= al.bodyRight + 0.5 && al.bodyRight - al.swRight <= 8, `${tag} ListenBrainz: its switch ends at the card's content edge (${(al.bodyRight - al.swRight).toFixed(1)} px in)`);
        await p.locator('#lbcard').screenshot({ path: path.join(SHOTS, `listenbrainz-${w}.png`) });
      }
      if (mobile) {
        // The phone's 44 px tap layer: a press 20 px above or beside the middle still hits the switch
        const reach = await p.evaluate((q) => {
          const e = document.querySelector(q); const r = e.getBoundingClientRect(); const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
          return [[0, -20], [0, 20], [-20, 0], [20, 0]].map(([dx, dy]) => document.elementFromPoint(cx + dx, cy + dy) === e);
        }, sel);
        check(reach.every(Boolean), `${tag} ${name}: a press 20 px off the middle still lands on it (${reach.join(',')})`);
      }
      await p.locator(sel).locator('xpath=ancestor::*[contains(@class,"set-row")][1]').screenshot({ path: path.join(SHOTS, `${tab}-${w}.png`) });
    }
    // No plain checkbox at a Settings row's end on any tab (every panel is in the page)
    await fx.go(p, '/settings/playback'); await p.waitForSelector('#preferfull');
    const plain = await p.evaluate(() => [...document.querySelectorAll('.set-ctl input[type="checkbox"]:not(.switch)')].map((e) => e.id || e.closest('.set-row').querySelector('.set-name')?.textContent || '?'));
    check(!plain.length, `${tag} no plain checkbox at a Settings row's end on any tab (${JSON.stringify(plain)})`);
    if (mobile) { const ov = await p.evaluate(() => [document.documentElement.scrollWidth, innerWidth]); check(ov[0] <= ov[1], `${tag} nothing overflows sideways (${ov[0]} ≤ ${ov[1]})`); }
    check(!errors.length, `${tag} no page errors ${errors.join(' | ')}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
