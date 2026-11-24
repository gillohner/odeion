// Where the keyboard focus goes when another page opens:
//   * the first page of a visit leaves it where the browser put it;
//   * a page opened from inside the old one (an album tile, Back) puts it on the new page's heading,
//     not on the page's body (from there the next Tab started at the sidebar again and a screen
//     reader said nothing of the new page); the heading shows no ring, the next Tab goes into the
//     page, and the heading can't be focused by a click afterwards;
//   * a page opened from the sidebar or the phone's tabs leaves the focus on that place, and the
//     page's name (without its count) is said in the status region;
//   * on a phone, a Settings section opened from the list puts it on that section's own title.
// Real candidate assets, isolated API fixtures (tools/ui-fixture.js).
//   node tools/ui-nav-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'nav-focus-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const albums = [{ id: 7, title: 'Journey', album_artist: 'Sefa', year: 2001, tracks: 1 }, ...Array.from({ length: 11 }, (_, i) => ({ id: 20 + i, title: `Other ${i}`, album_artist: `Band ${i}`, year: 1990, tracks: 1 }))];
const artists = [{ id: 11, name: 'Sefa', albums: 1, tracks: 1 }, { id: 22, name: 'Dr. Peacock', albums: 1, tracks: 1 }];
// What has the focus: where it is, what it is, and how it looks
const focus = (page) => page.evaluate(() => {
  const a = document.activeElement; const main = document.getElementById('main');
  const cs = a ? getComputedStyle(a) : null;
  return { tag: a ? a.tagName : null, body: a === document.body, inMain: !!a && main.contains(a) && a !== main, heading: !!a && /^H[12]$/.test(a.tagName) && main.contains(a), text: a ? (a.textContent || '').trim().slice(0, 40) : '', ring: cs ? cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0 : false, tabindex: a ? a.getAttribute('tabindex') : null, view: a && a.dataset ? a.dataset.view || a.dataset.bnav || null : null, top: main.scrollTop };
});
const said = (page) => page.evaluate(() => { const s = document.getElementById('navstatus'); return s ? s.textContent : null; });
const settled = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(r, 150)))));

(async () => {
  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const at = `${width}px`;
    const { context, page, state } = await fx.create(browser, width, { mobile: width < 600, fixtures: { '/library/albums': { body: albums }, '/library/albums/formats': { body: [] }, '/library/artists': { body: artists } } });
    await fx.go(page, '/albums'); await page.waitForSelector('#albumgrid .tile[data-album="7"]'); await settled(page);
    const f0 = await focus(page);
    check(f0.body, `${at}: the first page leaves the focus where the browser put it (${f0.tag})`);

    // An album tile, pressed with the keyboard: the album's page takes the focus on its heading
    await page.focus('#albumgrid .tile[data-album="7"]'); await page.keyboard.press('Enter');
    await page.waitForFunction(() => /Journey/.test((document.querySelector('#main h1, #main .page-head h2') || {}).textContent || '') && !document.querySelector('#albumgrid')); await settled(page);
    const f1 = await focus(page);
    check(f1.heading && /Journey/.test(f1.text), `${at}: an album opened from its tile puts the focus on the album's heading (${f1.tag} "${f1.text}", on the page's body: ${f1.body})`);
    check(!f1.ring && f1.top === 0, `${at}: the heading shows no ring and the page is not scrolled for it (ring ${f1.ring}, scrolled ${f1.top})`);
    await page.screenshot({ path: path.join(SHOTS, `album-heading-${width}.png`) });
    await page.keyboard.press('Tab');
    const f2 = await focus(page);
    check(f2.inMain && !f2.heading, `${at}: the next Tab goes into the page, not back to the sidebar (${f2.tag} "${f2.text}")`);
    const left = await page.evaluate(() => { const h = document.querySelector('#main h1, #main .page-head h2'); return h ? { tabindex: h.getAttribute('tabindex'), mark: h.hasAttribute('data-navfocus') } : null; });
    check(!!left && left.tabindex == null && !left.mark, `${at}: once the focus has left it, the heading can't be focused by a click (${JSON.stringify(left)})`);

    // Back, with the focus on a control of the page that goes: Albums' heading has it
    await page.goBack(); await page.waitForSelector('#albumgrid .tile'); await settled(page);
    const f3 = await focus(page);
    check(f3.heading && /^Albums/.test(f3.text), `${at}: Back puts the focus on the page's heading (${f3.tag} "${f3.text}")`);

    // A place in the navigation: the focus stays on it, and the page's name is said
    const nav = width < 600 ? '#bottomnav [data-bnav="zones"]' : '.sidebar .nav-item[data-view="artists"]';
    const place = await page.$(nav);
    if (place) {
      const want = width < 600 ? 'zones' : 'artists';
      await place.focus(); await page.keyboard.press('Enter');
      await page.waitForFunction(() => { const h = document.querySelector('#main h1, #main .page-head h2'); return !!h && !/^Albums/.test(h.textContent) && !document.querySelector('#albumgrid'); }, null, { timeout: 5000 }).catch(() => {});
      await settled(page);
      const f4 = await focus(page); const words = await said(page);
      check(f4.view === want, `${at}: opened from the ${width < 600 ? 'tabs' : 'sidebar'}, the focus stays on that place (${f4.tag} ${f4.view || f4.text})`);
      const head = await page.evaluate(() => { const h = document.querySelector('#main h1, #main .page-head h2'); return h ? [...h.childNodes].filter((n) => n.nodeType === 3 || !n.matches('.count, [aria-hidden="true"]')).map((n) => n.textContent).join('').trim() : ''; });
      check(!!head && words === head && !/\d$/.test(words), `${at}: and the page's name is said, without its count ("${words}", heading "${head}")`);
    } else check(false, `${at}: a navigation place (${nav}) is there`);
    // A phone's Settings: a section opened from the list is a page of its own; the focus goes to its
    // own title (the page also holds the other sections' titles, hidden)
    if (width < 600) {
      await fx.go(page, '/settings'); await page.waitForSelector('#main .set-tab[data-stab]'); await settled(page);
      const tab = page.locator('#main .set-tab[data-stab]').nth(1);
      const label = (await tab.innerText()).split('\n')[0].trim();
      await tab.focus(); await page.keyboard.press('Enter');
      await page.waitForFunction(() => !!document.querySelector('#main .set-panel:not([hidden]) .set-title'), null, { timeout: 5000 }).catch(() => {});
      await settled(page);
      const f5 = await focus(page);
      const vis = await page.evaluate(() => { const a = document.activeElement; return !!a && a.getClientRects().length > 0 && !a.closest('[hidden]'); });
      check(f5.inMain && vis && /^H2$/.test(f5.tag) && f5.text.startsWith(label), `${at}: a Settings section opened from the list puts the focus on its own title, one that is drawn (${f5.tag} "${f5.text}", "${label}" pressed, drawn ${vis})`);
    }
    check(!state.errors.length, `${at}: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
