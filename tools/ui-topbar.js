// The phone top bar keeps every control on screen (#248 review of 5545f546): with Help "?" beside
// the account button, a running scan and the "outside parts" warning can crowd a 320 px bar. Real
// candidate assets on the fixture origin (tools/ui-fixture.js): no node, no network. At 320, 360,
// 375 and 390 px, signed in, each of: nothing showing, a scan running, a part not answering (with
// two more), and both at once:
//   * the page doesn't scroll sideways;
//   * the search, "?", the warning, Install and the account button are each wholly on screen, none under
//     another;
//   * the search keeps its magnifier (44 px);
//   * what gives way stays: a screen reader still reads "Scanning library…", and the warning's dot
//     is drawn on screen.
// On a touch screen (360 and 412 px, a browser with Install and the Android app without it, with and
// without the "not answering" warning, with your picture in the account button), Help, the warning,
// Install and the account button sit 6 px apart (the search 6 px before them): each gap is shared
// half and half (a tap just right of Help doesn't open Install, just right of Install not the
// account menu) with no dead pixel, each reaches 44 px or more wide, the warning (a 25 px line)
// 44 px or more tall inside the bar, the account button keeps 6 px to the screen edge, Help doesn't
// reach down past its box, and the buttons are drawn 44 px as before. The search (drawn 38 px tall)
// takes the bar's height inside the bar and the 14 px to the screen's left edge. With a mouse Help
// has no reach of its own.
// Screenshots of the crowded bar go to the shots folder.
//   node tools/ui-topbar.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'topbar-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const info = (scanning) => ({ body: { name: 'Fixture node', offline: false, scanning, library: { tracks: 1, albums: 1, artists: 2, files: 1, missing_files: 0 } } });
const ok = [{ id: 'homeserver', name: 'Pubky homeserver', state: 'ok' }];
const bad = [
  { id: 'homeserver', name: 'Pubky homeserver', state: 'needs_sign_in', detail: 'approve new folders' },
  { id: 'musicbrainz', name: 'MusicBrainz', state: 'down', detail: 'no answer' },
  { id: 'tidal', name: 'TIDAL', state: 'down', detail: 'no answer' },
];
const STATES = [
  ['nothing showing', false, ok],
  ['a scan running', true, ok],
  ['a part not answering', false, bad],
  ['a scan and a part not answering', true, bad],
];

(async () => {
  const browser = await chromium.launch();
  for (const width of [320, 360, 375, 390]) {
    for (const [name, scanning, health] of STATES) {
      const { context, page, state } = await fx.create(browser, width, { fixtures: { '/info': info(scanning), '/health': { body: health } } });
      await fx.go(page, '/');
      await page.locator('#help button.fb-help').waitFor();
      if (scanning) await page.locator('#status .scanning').waitFor();
      if (health === bad) await page.locator('.topbar #upstreams:not([hidden])').waitFor();
      const m = await page.evaluate(() => {
        const box = (e) => { const r = e.getBoundingClientRect(); return { id: e.id || e.className.split(' ')[0], left: r.left, right: r.right, w: r.width }; };
        const shown = (e) => e && !e.hidden && e.getBoundingClientRect().width > 0 && getComputedStyle(e).display !== 'none';
        const parts = ['.topbar .search', '#status', '#help button.fb-help', '.topbar #upstreams', '#installbtn', '#me'].map((s) => document.querySelector(s)).filter(shown).map(box);
        const dot = document.querySelector('.topbar #upstreams:not([hidden]) .dot');
        return { scroll: document.documentElement.scrollWidth - innerWidth, inner: innerWidth, parts, search: box(document.querySelector('.topbar .search')), dot: dot && box(dot) };
      });
      const where = `${width} px, ${name}`;
      check(m.scroll <= 0, `${where}: no sideways scroll (${m.scroll} px)`);
      const off = m.parts.filter((p) => p.left < -0.5 || p.right > m.inner + 0.5).map((p) => `${p.id} ${p.left.toFixed(0)}–${p.right.toFixed(0)}`);
      check(!off.length, `${where}: every control on screen${off.length ? ': ' + off.join(', ') : ''}`);
      const under = m.parts.slice(1).filter((p, i) => p.left < m.parts[i].right - 0.5).map((p) => p.id);
      check(!under.length, `${where}: none under another${under.length ? ': ' + under.join(', ') : ''}`);
      check(m.search.w >= 43.5, `${where}: the search keeps its magnifier (${m.search.w.toFixed(0)} px)`);
      if (scanning) {
        const said = await page.locator('#status').ariaSnapshot();
        check(said.includes('Scanning library…'), `${where}: a screen reader reads the scan (${JSON.stringify(said)})`);
      }
      if (health === bad) check(!!m.dot && m.dot.w >= 6 && m.dot.left >= 0 && m.dot.right <= m.inner, `${where}: the warning's dot is on screen (${m.dot ? m.dot.w.toFixed(0) + ' px at ' + m.dot.left.toFixed(0) : 'none'})`);
      check(!state.errors.length, `${where}: no page errors ${state.errors.join('; ')}`);
      if (scanning && health === bad) await page.screenshot({ path: path.join(SHOTS, `topbar-${width}.png`), clip: { x: 0, y: 0, width, height: 72 } });
      await context.close();
    }
  }
  // Touch: who takes each tap along the bar's middle line
  for (const width of [360, 412]) {
    for (const android of [false, true]) {
      for (const [name, health, pic] of [['nothing showing', ok, false], ['your picture in the account button', ok, true], ['a part not answering', bad, false]]) {
        const { context, page, state } = await fx.create(browser, width, { mobile: true, fixtures: { '/info': info(false), '/health': { body: health } } });
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
        await fx.go(page, '/');
        await page.locator('#help button.fb-help').waitFor();
        if (!android) await page.locator('#installbtn:not([hidden])').waitFor();
        if (health === bad) await page.locator('.topbar #upstreams:not([hidden])').waitFor();
        // The account button as drawMe draws a signed-in profile's picture (the fixture has no key)
        if (pic) await page.evaluate(() => { const me = document.querySelector('#me'); me.classList.add('has-face'); me.innerHTML = '<img class="face md" alt="" src="data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2264%22 height=%2264%22%3E%3Crect width=%2264%22 height=%2264%22 fill=%22%232a7%22/%3E%3C/svg%3E">'; });
        const m = await page.evaluate(() => {
          const shown = (e) => e && !e.hidden && e.getBoundingClientRect().width > 0 && getComputedStyle(e).display !== 'none';
          const els = ['.topbar .search', '.topbar #upstreams', '#help button.fb-help', '#installbtn', '#me'].map((s) => document.querySelector(s)).filter(shown)
            .sort((a, b) => a.getBoundingClientRect().left - b.getBoundingClientRect().left);
          const id = (e) => (e ? e.id || e.className.split(' ').pop() : 'none');
          const who = (x, y) => { const h = document.elementFromPoint(x, y); return id(h && els.find((e) => e === h || e.contains(h))); };
          const y = (() => { const r = document.querySelector('#me').getBoundingClientRect(); return r.top + r.height / 2; })();
          const gaps = []; const reach = {};
          for (let i = 0; i + 1 < els.length; i++) {
            const A = els[i].getBoundingClientRect(); const B = els[i + 1].getBoundingClientRect();
            // whole pixels (hit testing snaps to the device grid): one more than a pixel from the middle
            // is the nearer button's, the middle pixel either's; in a gap of 12 px or less none is nobody's
            const wrong = [];
            const near = B.left - A.right <= 12;
            for (let x = Math.ceil(A.right); x < B.left; x++) {
              const w = who(x, y); const da = x - A.right; const db = B.left - x;
              if (w === id(els[i + 1]) && da < db - 2) wrong.push(`${x}→${w}`);
              else if (w === id(els[i]) && da > db + 2) wrong.push(`${x}→${w}`);
              else if (near && w === 'none') wrong.push(`${x}→none`);
            }
            gaps.push({ a: id(els[i]), b: id(els[i + 1]), gap: +(B.left - A.right).toFixed(1), wrong, at: A.right.toFixed(2) + '..' + B.left.toFixed(2) });
          }
          for (const e of els) {
            const r = e.getBoundingClientRect(); const me = id(e);
            let l = Math.floor(r.left); let rt = Math.ceil(r.right) - 1;
            while (l > 0 && who(l - 1, y) === me) l--;
            while (rt + 1 < innerWidth && who(rt + 1, y) === me) rt++;
            rt += 1;
            let t = Math.floor(r.top + 1); let bt = Math.ceil(r.bottom) - 1; const cx0 = Math.round(r.left + r.width / 2);
            while (t > 0 && who(cx0, t - 1) === me) t--;
            while (bt + 1 < innerHeight && who(cx0, bt + 1) === me) bt++;
            bt += 1;
            const cx = r.left + r.width / 2;
            reach[me] = { left: l, drawn: +r.width.toFixed(1), drawnH: +r.height.toFixed(1), wide: +(rt - l).toFixed(1), tall: bt - t, top: t, bottom: bt, right: +(rt - r.right).toFixed(1), below: who(cx, r.bottom + 2) === me };
          }
          const bar = document.querySelector('.topbar').getBoundingClientRect();
          return { gaps, reach, inner: innerWidth, bar: { top: bar.top, bottom: bar.bottom }, coarse: matchMedia('(pointer: coarse)').matches };
        });
        const where = `touch ${android ? 'Android app' : 'browser'} ${width} px, ${name}`;
        check(m.coarse, `${where}: a coarse pointer`);
        for (const g of m.gaps) check(!g.wrong.length, `${where}: ${g.a} | ${g.b} share their ${g.gap} px gap${g.wrong.length ? ': ' + g.wrong.join(' ') + ' (' + g.at + ')' : ''}`);
        const want = android ? ['fb-help', 'me'] : ['fb-help', 'installbtn', 'me'];
        check(want.every((k) => m.reach[k]) && (android ? !m.reach.installbtn : true), `${where}: the bar has ${want.join(', ')} (${Object.keys(m.reach).join(', ')})`);
        for (const k of want) {
          const r = m.reach[k]; if (!r) continue;
          check(r.wide >= 44, `${where}: ${k} takes a ${r.wide} px wide tap (≥ 44)`);
          check(Math.abs(r.drawn - 44) < 0.6 && Math.abs(r.drawnH - 44) < 0.6, `${where}: ${k} is drawn 44 × 44 (${r.drawn} × ${r.drawnH})`);
        }
        if (m.reach.me) check(m.reach.me.right >= 6, `${where}: the account button reaches ${m.reach.me.right} px towards the screen edge (≥ 6)`);
        if (health === bad) {
          const u = m.reach.upstreams;
          check(!!u && u.tall >= 44 && u.top >= m.bar.top - 0.5 && u.bottom <= m.bar.bottom + 0.5, `${where}: the warning takes a ${u ? u.tall : 0} px tall tap (≥ 44), inside the bar (${u ? u.top + '–' + u.bottom : '-'} in ${Math.round(m.bar.top)}–${Math.round(m.bar.bottom)})`);
        }
        if (m.reach['fb-help']) check(!m.reach['fb-help'].below, `${where}: Help doesn't reach below its box`);
        // The search is drawn 38 px tall: it takes the bar above and below it and the edge left of it
        const sr = m.reach.search;
        check(!!sr && sr.tall >= 44 && sr.top >= m.bar.top - 0.5 && sr.bottom <= m.bar.bottom + 0.5, `${where}: the search takes a ${sr ? sr.tall : 0} px tall tap (≥ 44), inside the bar (${sr ? sr.top + '–' + sr.bottom : '-'} in ${Math.round(m.bar.top)}–${Math.round(m.bar.bottom)})`);
        check(!!sr && sr.left === 0 && Math.abs(sr.drawnH - 38) < 0.6, `${where}: the search reaches the screen edge (from x ${sr ? sr.left : '-'}) and is drawn ${sr ? sr.drawnH : '-'} px tall as before`);
        // A press 8 px above the drawn box at the screen's left edge goes into the search
        // (a mouse press at the point: a touch there could be snapped onto the box by the browser)
        await page.evaluate(() => document.activeElement && document.activeElement.blur());
        const sb = await page.locator('.topbar .search').boundingBox();
        await page.mouse.click(3, sb.y - 8);
        check(await page.evaluate(() => document.activeElement && document.activeElement.id === 'search'), `${where}: a press 8 px above the search, at the screen's left edge, focuses it`);
        await page.evaluate(() => document.activeElement && document.activeElement.blur());
        check(!state.errors.length, `${where}: no page errors ${state.errors.join('; ')}`);
        if (!android && health === ok) await page.screenshot({ path: path.join(SHOTS, `topbar-touch-${width}${pic ? '-picture' : ''}.png`), clip: { x: 0, y: 0, width, height: 72 } });
        await context.close();
      }
    }
  }
  // A mouse: Help has no reach of its own (the touch rule only)
  {
    const { context, page } = await fx.create(browser, 360, { fixtures: { '/info': info(false), '/health': { body: ok } } });
    await fx.go(page, '/');
    await page.locator('#help button.fb-help').waitFor();
    const c = await page.evaluate(() => ({ fine: !matchMedia('(pointer: coarse)').matches, help: getComputedStyle(document.querySelector('#help button.fb-help'), '::before').content }));
    check(c.fine && c.help === 'none', `mouse 360 px: Help has no reach (${c.fine ? 'fine' : 'coarse'} pointer, ::before ${c.help})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
