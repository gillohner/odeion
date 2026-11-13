// On a phone every on/off switch, a section's text action (See all, a playlist's Add one), the
// Try again in a playlist's "Couldn't find where these songs play." and forget beside an address added
// by hand (Zones and outputs) take a 44 px tap, while they keep
// their size and place and do not take taps from the controls around them. Try again reads as a link
// (no button box) at every width.
// Real candidate assets, isolated responses: node tools/ui-phone-switch-targets.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-switch-targets-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
let passed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (ok) passed++; else failed++; };

// Two services, so the Sources list shows a switch beside each service's tab.
const sources = ['tidal', 'qobuz'].map((id, i) => ({ id, name: ['TIDAL', 'Qobuz'][i], enabled: true, status: 'signed in', features: ['search'] }));
// Home's Get started card for an owner with no music yet: Hide these steps sits in its head, with
// the Restore from a backup link just below.
const firstSteps = { show: true, hidden: false, steps: [{ id: 'music', done: false, optional: false }, { id: 'play', done: false, optional: false }] };
// Zones and outputs with four speakers added by address, each with its forget: a Linn (address, MAC
// and serial) and a description URL wrap, so at 360 and 412 px some forgets sit under others.
const discovery = { saved_addresses: ['192.168.1.20', '192.168.1.31', '192.168.1.44 00:26:0f:21:3a:9c 1234567', 'http://192.168.1.40:49152/description.xml', '10.0.0.7'], devices: [], renderers: [] };
const fixtures = { '/sources': { body: sources }, '/me/first-steps': { body: firstSteps }, '/outputs/discovery': { body: discovery } };
// A playlist whose services didn't answer: its songs show under "Couldn't find where these songs play. Try again".
const noPlayFrom = { '/playlists/c1/play-from': { status: 500, body: { error: 'services did not answer' } } };
const TARGETS = '.switch, .section-head .link, .pfnote .link, #rendsaved .link';
const SCALED = '.pfnote, #pfretry, #plsongs .section-head, #plsongs .section-head h3, [data-selstart]';
const size = (b) => `${b.width.toFixed(1)}×${b.height.toFixed(1)}`;

// Every target on the page, scrolled to the middle of the screen in turn: its visible box, and the
// span a touch reaches it in through its centre (what the finger can hit). The browser hit-tests whole
// pixels, so half-pixel steps measure an exactly 44 px area as 44 and a 43 px one as 43, at any
// position and at 1x or 2.625x pixels (checked against plain boxes and a centred layer; adding a
// pixel to each sum would let a 43 px area pass).
const measure = (page) => page.evaluate((sel) => [...document.querySelectorAll(sel)].filter((el) => el.getClientRects().length).map((el) => {
  el.scrollIntoView({ block: 'center', behavior: 'instant' });
  const b = el.getBoundingClientRect();
  const cx = b.left + b.width / 2;
  const cy = b.top + b.height / 2;
  const on = (x, y) => { const hit = document.elementFromPoint(x, y); return !!hit && (hit === el || el.contains(hit)); };
  const out = (sx, sy) => { let d = 0; while (d < 60 && on(cx + sx * (d + 0.5), cy + sy * (d + 0.5))) d += 0.5; return d; };
  const cs = getComputedStyle(el);
  return { name: (el.getAttribute('aria-label') || el.textContent).trim(), cls: el.className, width: b.width, height: b.height,
    reachW: out(-1, 0) + out(1, 0), reachH: out(0, -1) + out(0, 1), margin: cs.margin, padding: cs.padding };
}), TARGETS);

// No other control loses taps to a target's larger area: a grid of points over each visible control
// still reaches that control, not a switch or link next to it. The grid includes the points 1 px
// inside each edge, so even a thin overlap (4 px into a 44 px tab) shows.
const stolen = (page) => page.evaluate((sel) => {
  const targets = [...document.querySelectorAll(sel)];
  const out = [];
  // Other switches and links count too, and labels (a whole row can be a switch's label), and a
  // clickable song row.
  for (const el of document.querySelectorAll('button, a[href], input, select, textarea, label, [role=button], [role=tab], summary, .clickable')) {
    if (!el.getClientRects().length) continue;
    const b = el.getBoundingClientRect();
    if (b.width < 4 || b.height < 4 || b.top < 0 || b.bottom > innerHeight) continue;
    const xs = [b.left + 1, b.left + b.width * 0.1, b.left + b.width / 2, b.left + b.width * 0.9, b.right - 1];
    const ys = [b.top + 1, b.top + b.height * 0.1, b.top + b.height / 2, b.top + b.height * 0.9, b.bottom - 1];
    for (const x of xs) for (const y of ys) {
      const hit = document.elementFromPoint(x, y);
      const thief = hit && targets.find((t) => t !== el && !t.contains(el) && !el.contains(t) && (t === hit || t.contains(hit)));
      if (thief) out.push(`${el.tagName.toLowerCase()} "${(el.getAttribute('aria-label') || el.textContent).trim().slice(0, 24)}" → ${(thief.getAttribute('aria-label') || thief.textContent).trim()}`);
    }
  }
  return [...new Set(out)];
}, TARGETS);

// A press inside the 44 px area but outside the visible control, 1.5 px inside the area's bottom
// edge: a real touch, and a mouse click (no touch adjustment, so only the area itself catches it).
async function pressBelow(page, selector, how) {
  const el = page.locator(selector).first();
  await el.evaluate((n) => n.scrollIntoView({ block: 'center', behavior: 'instant' }));
  const b = await el.boundingBox();
  const x = b.x + b.width / 2;
  const y = b.y + b.height / 2 + 22 - 1.5;
  if (how === 'touch') await page.touchscreen.tap(x, y); else await page.mouse.click(x, y);
}
// Try again sits in a sentence: no border or button fill, underlined, in the accent colour.
const looksLikeLink = (page) => page.locator('#pfretry').evaluate((n) => {
  const c = getComputedStyle(n);
  const probe = document.createElement('span'); probe.style.color = 'var(--accent-strong)'; document.body.append(probe);
  const accent = getComputedStyle(probe).color; probe.remove();
  return c.borderTopWidth === '0px' && c.backgroundColor === 'rgba(0, 0, 0, 0)' && c.textDecorationLine === 'underline' && c.color === accent;
});
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 2000 }).then(() => true, () => false);

// Each page with its targets checked against the sizes they have everywhere else.
async function page1(page, at, route, wantMin, scrollTo) {
  await fx.go(page, route);
  await page.waitForSelector(scrollTo);
  await page.waitForTimeout(300);
  const items = await measure(page);
  check(items.length >= wantMin, `${at} ${route}: ${items.length} switches/links shown (at least ${wantMin})`);
  for (const t of items) {
    check(t.reachW >= 44 && t.reachH >= 44, `${at} ${route}: “${t.name}” takes a 44 px tap (${t.reachW.toFixed(1)}×${t.reachH.toFixed(1)})`);
    if (/\bswitch\b/.test(t.cls)) check(t.width === 34 && t.height === 20 && t.margin === '0px 4px', `${at} ${route}: “${t.name}” keeps its 34×20 look and margins (${size(t)}, ${t.margin})`);
    else {
      const pad = /\brelall\b/.test(t.cls) ? '6px 2px' : '4px 0px';
      check(t.padding === pad && t.margin === '0px', `${at} ${route}: “${t.name}” keeps its padding and margins (${t.padding}, ${t.margin})`);
    }
  }
  for (const t of items) {
    await page.evaluate(([sel, name]) => [...document.querySelectorAll(sel)].find((el) => (el.getAttribute('aria-label') || el.textContent).trim() === name)?.scrollIntoView({ block: 'center', behavior: 'instant' }), [TARGETS, t.name]);
    const s = await stolen(page);
    check(!s.length, `${at} ${route}: around “${t.name}” every other control keeps its taps${s.length ? ` (${s.join('; ')})` : ''}`);
  }
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${at} ${route}: no sideways overflow`);
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [360, 412]) for (const android of [false, true]) {
      const at = `${android ? 'android' : 'browser'}-${width}`;
      const { page, context, state } = await fx.create(browser, width, { mobile: true, fixtures });
      state.collections = [fx.summary];
      try {
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });

        // Display: Dense lists and the two colour switches.
        await page1(page, at, '/settings/appearance', 3, '#densepick');
        for (const how of ['touch', 'mouse']) {
          const was = await page.locator('#densepick').isChecked();
          await pressBelow(page, '#densepick', how);
          check(await until(page, (w) => document.querySelector('#densepick').checked !== w, was), `${at}: a ${how} press just below Dense lists turns it ${was ? 'off' : 'on'}`);
        }
        await page.locator('#densepick').evaluate((n) => n.scrollIntoView({ block: 'center', behavior: 'instant' }));
        await page.screenshot({ path: path.join(SHOTS, `display-${at}.png`) });

        // Home: the Get started card's Hide these steps, with a link just below its head.
        await page1(page, at, '/', 1, '#setuphide');
        check(await page.locator('#setuprestore').isVisible(), `${at}: Restore from a backup shows just below the card's head`);
        await page.locator('#setuphide').evaluate((n) => n.scrollIntoView({ block: 'center', behavior: 'instant' }));
        await page.screenshot({ path: path.join(SHOTS, `home-${at}.png`) });

        // Sources: a switch beside each service's tab; the tab keeps its taps (checked in page1).
        await page1(page, at, '/sources', 2, '[data-src-on="tidal"]');
        await page.screenshot({ path: path.join(SHOTS, `sources-${at}.png`) });

        // Zones and outputs: forget beside each address added by hand, with the address box and Add
        // renderer just above. Each address has a row of its own with its forget on it, so forgets
        // stack under each other and each keeps its own taps (page1).
        await page1(page, at, '/settings/playback', 5, '[data-rendrm]');
        const rows = await page.evaluate(() => [...document.querySelectorAll('[data-rendrm]')].map((b) => {
          const code = b.previousElementSibling; const bb = b.getBoundingClientRect(); const cb = code.getBoundingClientRect();
          const others = [...document.querySelectorAll('#rendsaved code')].filter((c) => c !== code && c.getBoundingClientRect().bottom > bb.top && c.getBoundingClientRect().top < bb.bottom);
          return { own: code.textContent === b.dataset.rendrm, sameRow: bb.top < cb.bottom && bb.bottom > cb.top, alone: !others.length };
        }));
        check(rows.length === discovery.saved_addresses.length && rows.every((r) => r.own && r.sameRow && r.alone), `${at}: each address is on a row of its own with its forget beside it (${JSON.stringify(rows.filter((r) => !(r.own && r.sameRow && r.alone)))})`);
        const unnamed = await page.evaluate(() => [...document.querySelectorAll('[data-rendrm]')].filter((b) => b.getAttribute('aria-label') !== `Forget ${b.dataset.rendrm}`).map((b) => b.getAttribute('aria-label')));
        check(!unnamed.length, `${at}: each forget is named by its address for a screen reader ("Forget 192.168.1.20")${unnamed.length ? ` (${unnamed.join(', ')})` : ''}`);
        check(await page.evaluate(() => [...document.querySelectorAll('#rendsaved .rendsep')].every((s) => !s.getClientRects().length)), `${at}: no · between the address rows`);
        await page.locator('[data-rendrm]').first().evaluate((n) => n.scrollIntoView({ block: 'center', behavior: 'instant' }));
        await page.screenshot({ path: path.join(SHOTS, `outputs-forget-${at}.png`) });
        for (const how of ['touch', 'mouse']) {
          const was = state.hits.filter((h) => h.m === 'DELETE' && h.v.startsWith('/outputs/upnp/')).length;
          await pressBelow(page, '[data-rendrm]', how);
          let gone = false;
          for (let i = 0; i < 20 && !gone; i++) { await page.waitForTimeout(100); gone = state.hits.filter((h) => h.m === 'DELETE' && h.v === '/outputs/upnp/192.168.1.20').length > was; }
          check(gone, `${at}: a ${how} press just below forget forgets that address`);
          await page.waitForSelector('[data-rendrm]');
        }


        // A playlist's description: Add one.
        await page1(page, at, '/playlist/c1', 1, '#pldesc');
        await page.screenshot({ path: path.join(SHOTS, `playlist-${at}.png`) });
        for (const how of ['touch', 'mouse']) {
          await pressBelow(page, '#pldesc', how);
          check(await until(page, () => !!document.querySelector('dialog.dialog[open] [data-field]')), `${at}: a ${how} press just below Add one opens the description`);
          await page.keyboard.press('Escape');
          await until(page, () => !document.querySelector('dialog.dialog[open]'));
        }

        // The same playlist when its services don't answer: Try again under the Songs head.
        state.fixtures = { ...fixtures, ...noPlayFrom };
        await page1(page, at, '/playlist/c1', 2, '#pfretry');
        check(await looksLikeLink(page), `${at}: Try again reads as a link, not a grey button box`);
        // A larger Android font size moves Try again under Select: Select keeps its taps, and Try
        // again's middle stays its own. Every text around it grows: the note, Try again, the Songs
        // heading and Select.
        for (const scale of [1.15, 1.3]) {
          // Measured on screen: how wide each one's words are drawn, before and after (a stylesheet size
          // with !important would keep them as they were).
          const grown = await page.evaluate(([k, sel]) => {
            const els = [...document.querySelectorAll(sel)];
            const drawn = (el) => {
              const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT); let w = 0;
              for (let n = walk.nextNode(); n; n = walk.nextNode()) if (n.textContent.trim()) { const r = document.createRange(); r.selectNodeContents(n); for (const b of r.getClientRects()) w += b.width; }
              return w;
            };
            for (const el of els) el.style.fontSize = '';
            const was = els.map((el) => parseFloat(getComputedStyle(el).fontSize));
            const before = els.map(drawn);
            els.forEach((el, i) => { el.style.fontSize = `${was[i] * k}px`; });
            document.querySelector('#pfretry').scrollIntoView({ block: 'center', behavior: 'instant' });
            const after = els.map(drawn);
            return { all: els.length, grew: els.filter((el, i) => before[i] > 0 && after[i] >= before[i] * (1 + (k - 1) * 0.6)).length, sizes: els.map((el, i) => `${before[i].toFixed(1)}→${after[i].toFixed(1)}`).join(', ') };
          }, [scale, SCALED]);
          check(grown.all >= 5 && grown.grew === grown.all, `${at}: with text at ${Math.round(scale * 100)} %, the note, Try again, the Songs heading and Select all grow on screen (${grown.grew} of ${grown.all}: ${grown.sizes})`);
          const s = await stolen(page);
          check(!s.length, `${at}: with text at ${Math.round(scale * 100)} %, Select and the songs keep their taps around Try again${s.length ? ` (${s.join('; ')})` : ''}`);
          const mid = await page.locator('#pfretry').evaluate((n) => { const b = n.getBoundingClientRect(); const h = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2); return h === n || n.contains(h) ? 'Try again' : `${h && h.tagName.toLowerCase()} “${h && h.textContent.trim().slice(0, 20)}”`; });
          check(mid === 'Try again', `${at}: with text at ${Math.round(scale * 100)} %, a tap on Try again's middle reaches it (${mid})`);
        }
        await page.evaluate((sel) => { for (const el of document.querySelectorAll(sel)) el.style.fontSize = ''; }, SCALED);
        await page.locator('#pfretry').evaluate((n) => n.scrollIntoView({ block: 'center', behavior: 'instant' }));
        await page.screenshot({ path: path.join(SHOTS, `playlist-retry-${at}.png`) });
        const asks = () => state.hits.filter((h) => h.v === '/playlists/c1/play-from').length;
        for (const how of ['touch', 'mouse']) {
          const was = asks();
          await pressBelow(page, '#pfretry', how);
          let again = false;
          for (let i = 0; i < 20 && !again; i++) { await page.waitForTimeout(100); again = asks() > was; }
          check(again, `${at}: a ${how} press just below Try again asks the services again`);
          await page.waitForSelector('#pfretry');
        }
        state.fixtures = fixtures;
        check(!state.errors.length, `${at}: no page errors (${state.errors.join('; ')})`);
      } finally { await context.close(); }
    }
    // These rules belong to the phone layout: the desktop layout keeps its compact switches and links.
    const { page, context, state } = await fx.create(browser, 1024, { fixtures });
    try {
      await fx.go(page, '/settings/appearance');
      await page.waitForSelector('#densepick');
      const d = (await measure(page)).find((t) => t.name === 'Dense lists');
      check(d && d.reachH < 44, `1024 px desktop keeps its compact switch (${d && `${d.reachW}×${d.reachH}`})`);
      state.fixtures = { ...fixtures, ...noPlayFrom };
      await fx.go(page, '/playlist/c1');
      await page.waitForSelector('#pfretry');
      const r = (await measure(page)).find((t) => t.name === 'Try again');
      check(await looksLikeLink(page), `1024 px desktop: Try again reads as a link (${await page.locator('#pfretry').evaluate((n) => { const c = getComputedStyle(n); return `${c.borderTopWidth} border, ${c.backgroundColor}, ${c.textDecorationLine}`; })})`);
      check(r && r.reachH < 44, `1024 px desktop keeps Try again's compact tap (${r && `${r.reachW}×${r.reachH}`})`);
      state.fixtures = fixtures;
      await fx.go(page, '/settings/playback');
      await page.waitForSelector('[data-rendrm]');
      const inline = await page.evaluate(() => ({ text: document.querySelector('#rendsaved').textContent, rows: [...document.querySelectorAll('#rendsaved .rendrow')].map((r) => getComputedStyle(r).display) }));
      check(inline.text.startsWith('Added by address: 192.168.1.20 forget · 192.168.1.31 forget · ') && inline.rows.every((d) => d === 'inline'), `1024 px desktop keeps the addresses in one sentence with · between them (${inline.text.slice(0, 60)}…, ${[...new Set(inline.rows)]})`);
    } finally { await context.close(); }
  } finally { await browser.close(); }
  console.log(`${passed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
