// An item page that knows nothing more about its music (a service's album with no catalogue match,
// or the fixture's library pages, which have no facts) has no About section: no "About" heading over
// "Nothing more is known yet." at the foot. Its place stays (hidden), so About still comes at the
// foot when open sources answer later (about_pending), and a page with facts keeps its About.
// At 1280, 1024 and 768 px:
//   * the fixture's library album: no About heading, no "Nothing more is known", #iabout hidden and last;
//   * a service album with about_pending and no facts: no About while pending; when the answer
//     brings a fact, About shows it at the foot; when it brings an article, About goes under the header;
//   * a service album with facts from the start: About with its facts, as before.
//   node tools/ui-item-about-empty.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'item-about-empty-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const songs = Array.from({ length: 6 }, (_, i) => ({ no: i + 1, title: `Track ${i + 1}`, artist: 'Some Band', artist_id: null, duration_ms: 180000 + i * 4000, uri: `https://www.deezer.com/track/90${i}`, track_id: null, id: `deezer-90${i}`, page: null }));
const base = { kind: 'album', uri: 'https://www.deezer.com/album/302127', source: 'Deezer', source_id: 'deezer', plays_from: { service: 'deezer', label: 'Deezer' }, connect: [], reading: false, pending: false, albums: [], versions: [], also_on: [], open_on: [], related: [], concerts: [], section: 'library', image: null, tags: [], target: {}, title: 'Unmatched Album', artist: { name: 'Some Band', id: null }, album: null, year: null, songs, about_text: null, image_credit: null, blurb: null };
const released = { label: 'Released', value: '1997', kind: 'fact' };
const pages = {
  pending: { ...base, id: 'deezer-302127', about: [], about_pending: true },
  facts: { ...base, id: 'deezer-302128', title: 'Known Album', about: [released], about_pending: false },
};
const fixtures = {};
for (const p of Object.values(pages)) { fixtures[`/item/album/${p.id}`] = { body: p }; fixtures[`/item/album/${p.id}/extras`] = { body: { collections: [], related: [], concerts: [], similar: [] } }; }

// What the page shows of About
const about = (page) => page.evaluate(() => {
  const a = document.querySelector('#main #iabout');
  const ids = [...document.querySelectorAll('#main > section, #main > details.section')].map((s) => s.id).filter(Boolean);
  const heads = [...document.querySelectorAll('#main h3')].filter((h) => h.offsetParent).map((h) => h.textContent.trim());
  const songs = document.querySelector('#isongs');
  return { there: !!a, shown: !!(a && a.offsetParent), hidden: !!(a && a.hidden), last: ids[ids.length - 1], heads, text: document.querySelector('#main').innerText, songsTop: songs ? Math.round(songs.getBoundingClientRect().top) : null, over: document.documentElement.scrollWidth - innerWidth };
});

(async () => {
  const browser = await chromium.launch();
  for (const w of [1280, 1024, 768]) {
    // The fixture's library album: no facts at all
    {
      const { page, context, state } = await fx.create(browser, w, { fixtures });
      await fx.go(page, '/album/lib-7'); await page.waitForSelector('#main #isongs'); await page.waitForSelector('#main #iabout', { state: 'attached' });
      const r = await about(page);
      check(r.there && r.hidden && !r.shown, `library album at ${w}: About's place is there, hidden`);
      check(!r.heads.includes('About') && !/Nothing more is known/.test(r.text), `library album at ${w}: no About heading, no "Nothing more is known" (${r.heads.join(', ')})`);
      check(r.last === 'iabout', `library album at ${w}: its place is the page's last section (${r.last})`);
      check(r.over <= 0, `library album at ${w}: no sideways scroll (${r.over})`);
      await page.screenshot({ path: path.join(SHOTS, `album-library-${w}.png`) });
      check(!state.errors.length, `library album at ${w}: no errors (${state.errors.join('; ')})`);
      await context.close();
    }
    // A service album while open sources are asked: nothing, then the fact at the foot
    {
      const { page, context, state } = await fx.create(browser, w, { fixtures: { ...fixtures } });
      await fx.go(page, `/album/${pages.pending.id}`); await page.waitForSelector('#main #isongs'); await page.waitForSelector('#main #iabout', { state: 'attached' });
      const r = await about(page);
      check(r.hidden && !r.heads.includes('About') && !/Nothing more is known/.test(r.text), `pending album at ${w}: no About while asking`);
      await page.screenshot({ path: path.join(SHOTS, `album-pending-${w}.png`) });
      state.fixtures[`/item/album/${pages.pending.id}`] = { body: { ...pages.pending, about_pending: false, about: [released] } };
      const came = await page.waitForSelector('#main #iabout:not([hidden]) dl.kv', { timeout: 9000 }).then(() => true, () => false);
      const r2 = await about(page);
      check(came && r2.shown && r2.heads.includes('About') && /Released/.test(r2.text) && /1997/.test(r2.text), `pending album at ${w}: About comes with the late fact`);
      check(r2.last === 'iabout', `pending album at ${w}: About is at the foot (${r2.last})`);
      await page.screenshot({ path: path.join(SHOTS, `album-pending-answered-${w}.png`), fullPage: true });
      check(!state.errors.length, `pending album at ${w}: no errors (${state.errors.join('; ')})`);
      await context.close();
    }
    // A service album whose late answer brings an article: About under the header, no empty place left
    {
      const { page, context, state } = await fx.create(browser, w, { fixtures: { ...fixtures } });
      await fx.go(page, `/album/${pages.pending.id}`); await page.waitForSelector('#main #isongs'); await page.waitForSelector('#main #iabout', { state: 'attached' });
      state.fixtures[`/item/album/${pages.pending.id}`] = { body: { ...pages.pending, about_pending: false, about: [released], about_text: { text: 'An album by Some Band, recorded in one week.', lang: 'en', source: 'Wikipedia', url: 'https://en.wikipedia.org/wiki/Unmatched_Album' } } };
      const came = await page.waitForSelector('#main #iabout.about-top #iblurb', { timeout: 9000 }).then(() => true, () => false);
      const r = await page.evaluate(() => ({ count: document.querySelectorAll('#main #iabout').length, hidden: document.querySelectorAll('#main #iabout[hidden]').length, next: document.querySelector('#main .hero').nextElementSibling?.id }));
      check(came && r.count === 1 && !r.hidden && r.next === 'iabout', `pending album at ${w}: a late article goes under the header, the hidden place is gone (${JSON.stringify(r)})`);
      check(!state.errors.length, `late article at ${w}: no errors (${state.errors.join('; ')})`);
      await context.close();
    }
    // A service album with a fact from the start: About as before
    {
      const { page, context, state } = await fx.create(browser, w, { fixtures });
      await fx.go(page, `/album/${pages.facts.id}`); await page.waitForSelector('#main #iabout');
      const r = await about(page);
      check(r.shown && !r.hidden && r.heads.includes('About') && /Released/.test(r.text) && r.last === 'iabout', `album with facts at ${w}: About shows them, last`);
      check(!/Nothing more is known/.test(r.text), `album with facts at ${w}: no "Nothing more is known"`);
      check(!state.errors.length, `album with facts at ${w}: no errors (${state.errors.join('; ')})`);
      await context.close();
    }
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
