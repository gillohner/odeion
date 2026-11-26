// A service page's own library (Spotify's Your library, TIDAL's Your collection, SoundCloud's Your
// likes): on a phone its head put the title, the count and "Import as bookmarks" on one row, so the
// title broke onto 2 lines and the count too ("0 songs · 0 / albums"); a tablet beside the sidebar
// (768) did the same with a real library's counts. The button now goes under the title and count when
// they don't fit beside it; a computer's wide page keeps one row. And
// the sections under the page (devices, library, playlists) had no space between them: a section's
// last line ran into the next title. They are spaced like the page's other sections.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-service-library-head.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || path.join(require('node:os').tmpdir(), 'service-library-head-shots'));
fs.mkdirSync(shots, { recursive: true });
let ok = 0;
const failed = [];
const check = (cond, what) => { if (cond) ok++; else { failed.push(what); console.error('FAIL', what); } };
const hit = (id, kind, title) => ({ id, kind, title, creator: 'Fixture Artist', year: 2020 });
// A real library's counts (the page draws the first 20 albums and 50 songs): "1,234 songs · 321 albums"
const many = (n, kind, pre) => Array.from({ length: n }, (_, i) => hit(`${pre}:${kind[0]}${i}`, kind, `${kind} ${i + 1}`));
const services = [
  {
    id: 'spotify', name: 'Spotify', head: 'Your library', btn: '#spotfav',
    fixtures: {
      '/sources/spotify/status': { body: { signed_in: true } },
      '/sources/spotify/signin': { body: { ways: [], in_use: { way: 'app' }, details: { status: { signed_in: true } } } },
      '/sources/spotify/collection': { body: { items: [...many(321, 'album', 'sp'), ...many(1234, 'track', 'sp')] } },
      '/sources/spotify/playlists': { body: [{ id: 'sp:p1', name: 'Road trip', items: 12 }] },
      '/sources/spotify/devices': { body: [] },
    },
    source: { status: 'ready' },
  },
  {
    id: 'tidal', name: 'TIDAL', head: 'Your collection', btn: '#tidalfav',
    fixtures: {
      '/sources/tidal/signin': { body: { ways: [], in_use: { way: 'app' }, details: { status: { signed_in: true } } } },
      '/sources/tidal/collection': { body: { albums: many(321, 'album', 'td'), tracks: many(1234, 'track', 'td') } },
      '/sources/tidal/playlists': { body: [{ id: 'td:p1', name: 'Mornings', items: 8 }] },
    },
    source: { status: 'signed in' },
  },
  {
    id: 'soundcloud', name: 'SoundCloud', head: 'Your likes', btn: '#scfav',
    fixtures: {
      '/sources/soundcloud/likes': { body: { tracks: many(1234, 'track', 'sc'), playlists: many(32, 'item', 'sc') } },
      '/sources/soundcloud/playlists': { body: [{ id: 'sc:p1', name: 'Mixes', items: 3 }] },
      '/sources/soundcloud/unofficial': { body: {} },
    },
    source: { status: 'signed in' },
  },
];
(async () => {
  const browser = await chromium.launch();
  try {
    // [width, phone viewport (a touch screen), the Android app]
    for (const [width, mobile, android] of [[360, true, false], [360, true, true], [412, true, false], [768, true, false], [1280, false, false]]) {
      const tag = `${width}${android ? '-android' : ''}`;
      for (const s of services) {
        const fixtures = { '/sources': { body: [{ id: s.id, name: s.name, description: '', enabled: true, features: ['search'], ...s.source }] }, ...s.fixtures };
        const { page, context, state } = await fx.create(browser, width, { mobile, fixtures });
        if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android' }; });
        const at = `${tag} ${s.name}`;
        try {
          await fx.go(page, `/sources/${s.id}`);
          await page.waitForSelector(`${s.btn}`);
          await page.waitForFunction(() => document.querySelectorAll('#srcmine .section').length >= 2);
          await page.$eval(s.btn, (b) => b.scrollIntoView({ block: 'center' }));
          const m = await page.$eval(s.btn, (b) => {
            const head = b.closest('.section-head'), h = head.querySelector(':scope > h3'), c = head.querySelector(':scope > .muted');
            const lines = (el) => { const r = document.createRange(); r.selectNodeContents(el); return new Set([...r.getClientRects()].filter((x) => x.width > 1).map((x) => Math.round(x.top))).size; };
            const box = (el) => { const r = el.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, left: r.left, right: r.right }; };
            return { title: h.textContent, titleLines: lines(h), countLines: lines(c), h: box(h), c: box(c), b: box(b), head: box(head), sw: document.documentElement.scrollWidth, iw: innerWidth };
          });
          check(m.titleLines === 1, `${at}: "${m.title}" is on one line (${m.titleLines})`);
          check(m.countLines === 1, `${at}: its count is on one line (${m.countLines})`);
          check(m.c.left >= m.h.right && Math.abs(m.c.top - m.h.top) < 12, `${at}: the count is beside the title`);
          // At 360 the three don't fit on one row: the button goes under, at the left; at 1280 they fit and
          // it stays on the title's row, after the count; between, either, as the words fit
          const under = m.b.top >= m.h.bottom - 1 && m.b.left <= m.h.left + 1, beside = m.b.top < m.h.bottom && m.b.left > m.c.right;
          if (width === 360) check(under, `${at}: Import as bookmarks is under the title and count, at the left (button top ${m.b.top.toFixed(1)}, title foot ${m.h.bottom.toFixed(1)}, left ${m.b.left.toFixed(1)} vs ${m.h.left.toFixed(1)})`);
          else if (width === 1280) check(beside, `${at}: Import as bookmarks is on the title's row, after the count`);
          else check(under || beside, `${at}: Import as bookmarks is on the title's row after the count, or under them at the left`);
          check(m.b.right <= m.head.right + 0.5 && m.b.bottom <= m.head.bottom + 0.5, `${at}: the button is inside its head`);
          check(m.sw <= m.iw, `${at}: no sideways scroll (${m.sw} > ${m.iw})`);
          // Each section of the page's own library is spaced from the one above, like the page's sections
          const gaps = await page.$$eval('#srcmine .section', (ss) => ss.slice(1).map((sec, i) => ({ name: sec.querySelector('h3').textContent, gap: sec.getBoundingClientRect().top - ss[i].getBoundingClientRect().bottom })));
          for (const g of gaps) check(g.gap >= 20, `${at}: "${g.name}" has room above it (${g.gap.toFixed(1)} px)`);
          if (width !== 412) await page.screenshot({ path: path.join(shots, `${tag}-${s.id}.png`) });
          check(state.errors.length === 0, `${at}: no page errors (${state.errors})`);
        } catch (e) { check(false, `${at}: ${e.message.split('\n')[0]}`); } finally { await context.close(); }
      }
    }
  } finally { await browser.close(); }
  console.log(`ui-service-library-head: ${ok} ok, ${failed.length} failed`);
  if (failed.length) process.exit(1);
})().catch((e) => { console.error('FAIL', e.message); process.exit(1); });
