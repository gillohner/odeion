// Home without the node's email or website: every Pubky row (Trending, From people you follow,
// Tags, Concerts, New on Pubky…) has nothing to show and is dropped, leaving "Get started says how".
// With Get started hidden or done, nothing on Home said why the rows were missing or where to fix
// it (a dead end). Now one line takes their place, once, with Add it (Settings › Discover with the
// field ready to type, 1 click). While Get started shows its own contact step, no line.
// Real UI in Chromium against fixtures (no node), at 1280, 1024, 768 and 360 px.
//   node tools/ui-home-contact.js [repo] [screenshot directory]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'home-contact-shots'));
fs.mkdirSync(shots, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const PUBKY = ['trending-songs', 'trending-albums', 'trending-artists', 'following', 'tags', 'concerts', 'podcasts', 'yours', 'new'];
const needs = (name) => ({ section: name, items: [], tags: [], needs: 'contact', note: 'Set the upstream contact in Settings to see what Pubky knows.' });
const steps = (contact) => ({ show: true, hidden: false, steps: [{ id: 'play', done: false, optional: false }, ...(contact ? [{ id: 'contact', done: false, optional: false }] : [])] });
const hidden = { show: false, hidden: true, steps: [{ id: 'play', done: false, optional: false }, { id: 'contact', done: false, optional: false }] };
const fixtures = (fsAnswer) => ({
  '/me/first-steps': { body: fsAnswer },
  '/library/home': { body: { recently_played: [], recently_added: [], forgotten: [] } },
  '/listenbrainz': { body: { connected: false } },
  '/library/folders/suggest': { body: [] },
  ...Object.fromEntries(PUBKY.map((n) => [`/home/${n}`, { body: needs(n) }])),
});
// Home has drawn: no Pubky row is left waiting
const settled = (page) => page.waitForFunction((names) => document.getElementById('h-setup') && names.every((n) => { const s = document.getElementById('h-' + n); return !s || !s.querySelector('.skeleton'); }), PUBKY);
const note = (page) => page.evaluate((names) => {
  // (the line counts once it is on screen: it waits hidden while Get started may say how)
  const all = [...document.querySelectorAll('#h-needcontact')].filter((x) => x.getClientRects().length);
  const n = all[0];
  const order = [...document.querySelectorAll('.main section[id^="h-"], .main #h-setup')].map((s) => s.id);
  return {
    count: all.length,
    text: n ? n.textContent.trim() : '',
    buttons: n ? [...n.querySelectorAll('button')].map((b) => b.textContent.trim()) : [],
    pubkyLeft: names.filter((x) => document.getElementById('h-' + x)).length,
    at: n ? order.indexOf('h-needcontact') : -1,
    order,
    setupContact: !!document.querySelector('#h-setup [data-step="contact"]:not(.done)'),
  };
}, PUBKY);

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768, 360]) {
      // The owner, Get started hidden: one line in the Pubky rows' place, with Add it
      {
        const T = `${width} owner, Get started hidden:`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: fixtures(hidden) });
        try {
          await fx.go(page, '/');
          await settled(page);
          await page.waitForTimeout(300);
          const n = await note(page);
          await page.screenshot({ path: path.join(shots, `owner-hidden-${width}.png`), fullPage: true });
          check(n.pubkyLeft === 0, `${T} the empty Pubky rows are gone (${n.pubkyLeft} left)`);
          check(n.count === 1, `${T} one line says why (${n.count})`);
          check(/email or website/.test(n.text) && /Pubky/.test(n.text), `${T} it says Pubky needs an email or website ("${n.text}")`);
          check(n.buttons.length === 1 && n.buttons[0] === 'Add it', `${T} its one button is Add it (${JSON.stringify(n.buttons)})`);
          // In the Pubky rows' place: after the library's rows, where Trending songs was
          check(n.at > n.order.indexOf('h-added'), `${T} it sits where the Pubky rows were (${n.order.join(' ')})`);
          if (n.count === 1) {
            const b = page.locator('#h-needcontact button');
            const box = await b.boundingBox();
          // (the phone shell's small buttons: 40 px, their touch area reaching past it, app.css)
          check(box && box.height >= (width < 600 ? 40 : 28), `${T} Add it is a full-size button (${box && box.height})`);
            await b.click();
            const ok = await page.waitForFunction(() => location.pathname === '/settings/discover' && document.activeElement && document.activeElement.id === 'contact').then(() => true, () => false);
            check(ok, `${T} Add it opens Settings › Discover with the field ready to type (1 click)`);
          }
          await fx.cleanCopy(page).then(() => check(true, `${T} clean copy, no overflow`), (e) => check(false, `${T} ${e.message.split('\n')[0]}`));
          check(state.errors.length === 0, `${T} no page errors ${state.errors.join(' | ')}`);
        } finally { await context.close(); }
      }
      // The owner, Get started showing its contact step: it says how, so no second line
      {
        const T = `${width} owner, Get started shows the step:`;
        // Get started answers after the Pubky rows (the order that would show and then drop the line)
        const { page, context, state } = await fx.create(browser, width, { fixtures: fixtures(steps(true)), delay: (v) => (v === '/me/first-steps' ? 600 : 0) });
        try {
          await fx.go(page, '/');
          await settled(page);
          await page.waitForSelector('#h-setup [data-step="contact"]');
          await page.waitForTimeout(300);
          const n = await note(page);
          check(n.setupContact, `${T} Get started has the contact step`);
          check(n.count === 0, `${T} no second line (${n.count})`);
          check(n.pubkyLeft === 0, `${T} the empty Pubky rows are gone (${n.pubkyLeft} left)`);
          check(state.errors.length === 0, `${T} no page errors ${state.errors.join(' | ')}`);
        } finally { await context.close(); }
      }
      // The other order: Get started answers first, the Pubky rows after; still no second line
      {
        const T = `${width} owner, the step drawn first:`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: fixtures(steps(true)), delay: (v) => (PUBKY.some((n) => v === `/home/${n}`) ? 600 : 0) });
        try {
          await fx.go(page, '/');
          await settled(page);
          await page.waitForTimeout(300);
          const n = await note(page);
          check(n.setupContact && n.count === 0 && n.pubkyLeft === 0, `${T} the step, no line, no empty rows (${n.setupContact}, ${n.count}, ${n.pubkyLeft})`);
          // Hide these steps: the step goes, so the line says it now (no dead end until the next visit)
          await page.click('#setuphide');
          await page.waitForFunction(() => !document.querySelector('#h-setup [data-step]'));
          const h = await note(page);
          check(h.count === 1 && h.buttons[0] === 'Add it', `${T} after Hide these steps the line shows, with Add it (${h.count}, ${JSON.stringify(h.buttons)})`);
          check(h.at > h.order.indexOf('h-added'), `${T} in the Pubky rows' place (${h.order.join(' ')})`);
          check(state.errors.length === 0, `${T} no page errors ${state.errors.join(' | ')}`);
        } finally { await context.close(); }
      }
      // A slow Get started (its reads can take seconds): the empty Pubky rows still go at once, and
      // the line waits for it rather than showing and going again
      {
        const T = `${width} owner, Get started slow:`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: fixtures(steps(true)), delay: (v) => (v === '/me/first-steps' ? 2500 : 0) });
        try {
          await fx.go(page, '/');
          await page.waitForSelector('#h-setup');
          // (well before Get started answers)
          const early = await page.waitForFunction((names) => names.every((x) => !document.getElementById('h-' + x)), PUBKY, { timeout: 1500 }).then(() => true, () => false);
          check(early, `${T} the empty Pubky rows go before Get started answers`);
          const mid = await note(page);
          check(mid.count === 0, `${T} no line yet (${mid.count})`);
          await page.waitForSelector('#h-setup [data-step="contact"]');
          await page.waitForTimeout(300);
          const n = await note(page);
          check(n.setupContact && n.count === 0, `${T} then the step, still no line (${n.count})`);
          check(state.errors.length === 0, `${T} no page errors ${state.errors.join(' | ')}`);
        } finally { await context.close(); }
      }
      // Home drawn again while the Pubky rows are on their way (Edit Home): the first drawing's
      // answers don't put the line next to the second drawing's step
      {
        const T = `${width} owner, Home drawn again mid-load:`;
        const { page, context, state } = await fx.create(browser, width, { fixtures: fixtures(steps(true)), delay: (v) => (PUBKY.some((n) => v === `/home/${n}`) ? 800 : v === '/me/first-steps' ? 1200 : 0) });
        try {
          await fx.go(page, '/');
          await page.waitForSelector('#h-setup');
          await page.click('#hedit');
          await page.waitForSelector('#h-edit');
          await page.waitForSelector('#h-setup [data-step="contact"]');
          await settled(page);
          await page.waitForTimeout(1500);
          const n = await note(page);
          check(n.setupContact && n.count === 0 && n.pubkyLeft === 0, `${T} the step, no line, no empty rows (${n.setupContact}, ${n.count}, ${n.pubkyLeft})`);
          check(state.errors.length === 0, `${T} no page errors ${state.errors.join(' | ')}`);
        } finally { await context.close(); }
      }
      // With a contact the rows draw as before and there is no line
      {
        const T = `${width} with a contact:`;
        const ok = { ...fixtures(hidden), ...Object.fromEntries(PUBKY.map((n) => [`/home/${n}`, { body: { section: n, items: [], tags: [], note: 'Nothing yet.' } }])) };
        const { page, context } = await fx.create(browser, width, { fixtures: ok });
        try {
          await fx.go(page, '/');
          await settled(page);
          await page.waitForTimeout(300);
          const n = await note(page);
          check(n.count === 0 && n.pubkyLeft === PUBKY.length, `${T} the rows stay and no line (${n.count}, ${n.pubkyLeft})`);
        } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
