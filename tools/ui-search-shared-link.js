// A playlist link pasted into the search box (what a playlist's Copy link gives,
// pubky://<key>/pub/social/v1/posts/<id>) opens that shared playlist, as the Playlists page's
// shared-link box does, instead of searching for the link's text. Phone (360/412 px, the Search tab,
// the Android app) and desktop. A bare key still opens the person; other text still searches.
// Opened from the phone's empty Search tab, its Back says Search (it said Results for “”).
// A link to a post that is no playlist (a comment, an ordinary post) is searched as text instead of
// an error page; a playlist made private again, or one whose homeserver doesn't answer, still says so.
// Pasted over a search with a query, that search's entry takes it (Back and Escape go where Search was
// opened from); a post read as no playlist is asked for again after 10 minutes. The box edited while
// that Back lands: what was typed is searched there (the old query is not put back over it), also when
// the Back lands after 3 s or never comes; Escape then goes Home on a phone; a second post typed then
// takes its search in its own entry.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-search-shared-link.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/search-shared-link-shots');
fs.mkdirSync(shots, { recursive: true });

const KEY = 'zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o';
const ID = '0035WKVKQDHSM';
const LINK = `pubky://${KEY}/pub/social/v1/posts/${ID}`;
const shared = {
  summary: { id: ID, name: 'Night drives', items: 1, link: LINK, description: '' },
  entries: [{ kind: 'track', uri: fx.SONG }],
  play_from: { order: { order: ['library'], services: [] }, own_order: false, items: [{ index: 0, uri: fx.SONG, title: fx.track.title, artist: 'Sefa', track_id: 1, plays_from: { service: 'library', label: 'Your library' }, sources: [] }] },
};
// A post that is no playlist (a comment, an ordinary post) and a playlist made private again
const NOTPL = '0035WKVKQDHSN'; const GONE = '0035WKVKQDHSP'; const SLOW = '0035WKVKQDHSQ';
// (another post that is no playlist, pasted over a search with a query; a homeserver that doesn't answer)
const OVER = '0035WKVKQDHSR'; const DOWN = '0035WKVKQDHSS';
// (one more, the box edited while its Back to the search under it lands)
const EDITS = ['0035WKVKQDHST', '0035WKVKQDHSV', '0035WKVKQDHSW', '0035WKVKQDHSX', '0035WKVKQDHSY', '0035WKVKQDHSZ', '0035WKVKQDHT0', '0035WKVKQDHT1', '0035WKVKQDHT2', '0035WKVKQDHT3', '0035WKVKQDHT4', '0035WKVKQDHT5'];
const NOTPL_LINK = `pubky://${KEY}/pub/social/v1/posts/${NOTPL}`;
const fixtures = {
  [`/shared/${KEY}/playlists/${ID}`]: { body: shared }, [`/shared/${KEY}/playlists`]: { body: [] },
  [`/shared/${KEY}/playlists/${NOTPL}`]: { status: 422, body: { error: 'that post is not a playlist this node can read' } },
  [`/shared/${KEY}/playlists/${GONE}`]: { status: 404, body: { error: 'no shared playlist with that id (it may be private again)' } },
  [`/shared/${KEY}/playlists/${SLOW}`]: { status: 422, body: { error: 'that post is not a playlist this node can read' } },
  [`/shared/${KEY}/playlists/${OVER}`]: { status: 422, body: { error: 'that post is not a playlist this node can read' } },
  ...Object.fromEntries(EDITS.map((e) => [`/shared/${KEY}/playlists/${e}`, { status: 422, body: { error: 'that post is not a playlist this node can read' } }])),
  [`/shared/${KEY}/playlists/${DOWN}`]: { status: 502, body: { error: 'the homeserver doesn’t answer' } },
};
// (the post that is no playlist and answers late: the box emptied meanwhile)
const delay = (v) => (v === `/shared/${KEY}/playlists/${SLOW}` ? 1200 : 0);

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 3000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await fn()) return true; await sleep(50); } return false; };
const where = (page) => new URL(page.url()).pathname;
const searched = (state) => state.hits.filter((h) => h.v === '/search').length;
const searchedFor = (state, q) => state.hits.filter((h) => h.v === '/search' && new URLSearchParams(h.q).get('q') === q).length;

// Paste (one input event with the whole text), as a long-press Paste or Ctrl+V does
const paste = (page, text) => page.evaluate((t) => { const s = document.getElementById('search'); s.focus(); s.value = t; s.dispatchEvent(new Event('input', { bubbles: true })); }, text);

async function run(browser, width, { phone, android }) {
  const tag = `${width}${android ? ' android' : ''}${phone ? ' phone' : ''}`;
  const { context, page, state } = await fx.create(browser, width, { fixtures, delay, ...(phone ? { mobile: true } : {}) });
  if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
  try {
    await fx.go(page, '/tracks'); await page.locator('.tracks .track').first().waitFor();
    if (phone) { await page.click('[data-bnav="search"]'); await until(() => where(page) === '/search'); }
    else await page.click('#search');
    for (const [label, text] of [['pubky:// link', LINK], ['link without pubky://', `${KEY}/pub/social/v1/posts/${ID}`], ['link with pubky prefix and a trailing slash', `pubky${KEY}/pub/social/v1/posts/${ID}/`], ['link with the key in capitals', `pubky://${KEY.toUpperCase()}/pub/social/v1/posts/${ID}`]]) {
      const before = searched(state);
      await paste(page, text);
      check(await until(() => where(page) === `/shared/${KEY}/${ID}`), `${tag}: a pasted ${label} opens the shared playlist (at ${where(page)})`);
      check(await until(async () => (await page.locator('#main h1').textContent().catch(() => '')) === 'Night drives'), `${tag}: its name shows`);
      check(searched(state) === before, `${tag}: nothing is searched for the ${label}`);
      const back = ((await page.locator('#main .back').first().textContent().catch(() => '')) || '').trim();
      check(back === (phone ? 'Search' : 'Songs'), `${tag}: its Back says where it goes back to (${back})`);
      if (label === 'pubky:// link') await page.screenshot({ path: path.join(shots, `${width}${android ? '-android' : ''}${phone ? '-phone' : ''}-pasted-link.png`) });
      await page.goBack(); await until(() => where(page) !== `/shared/${KEY}/${ID}`);
      if (!phone) await page.click('#search');
    }
    // Unchanged: a bare key opens the person, and other text searches
    await paste(page, `pubky://${KEY}`);
    check(await until(() => where(page) === `/person/${KEY}`), `${tag}: a bare key still opens the person (at ${where(page)})`);
    await page.goBack(); await until(() => where(page) !== `/person/${KEY}`);
    for (const [label, bad] of [['a post id too long', `${LINK}X`], ['a post id with a letter no id has (I)', LINK.replace(/M$/, 'I')]]) {
      const n = searched(state);
      await paste(page, bad);
      check(await until(() => searched(state) > n), `${tag}: a link with ${label} is searched as text`);
      check(await until(() => where(page) === '/search'), `${tag}: and stays on Search (at ${where(page)})`);
    }
    // A link to a post that is no playlist is searched as text, in the shared page's place:
    // Back goes where search was opened from (the phone's Search tab, desktop Songs)
    const depth = () => page.evaluate(() => (history.state && history.state.depth) || 0);
    const fresh = async () => {
      await fx.go(page, '/tracks'); await page.locator('.tracks .track').first().waitFor();
      if (phone) { await page.click('[data-bnav="search"]'); await until(() => where(page) === '/search'); } else await page.click('#search');
    };
    await fresh();
    {
      const n = searched(state); const d0 = await depth();
      await paste(page, NOTPL_LINK);
      check(await until(() => state.hits.some((h) => h.v === `/shared/${KEY}/playlists/${NOTPL}`)), `${tag}: a post link that is no playlist is asked for first`);
      check(await until(() => searched(state) > n && where(page) === '/search'), `${tag}: then it is searched as text (at ${where(page)})`);
      const q = new URL(page.url()).searchParams.get('q');
      check(q === NOTPL_LINK, `${tag}: the search is for the link (${q})`);
      check(await page.locator('#search').inputValue() === NOTPL_LINK, `${tag}: the box still holds the link`);
      await sleep(300); await page.screenshot({ path: path.join(shots, `${width}${android ? '-android' : ''}${phone ? '-phone' : ''}-pasted-post-link.png`) });
      check(!(await page.locator('#main .notice').count()), `${tag}: no "not a playlist" error is left on screen`);
      check(await depth() === d0 + 1, `${tag}: the shared page left no entry of its own (${d0} → ${await depth()})`);
      // Enter there (or the link again): the search, in the same entry, the post not asked again
      const asked = state.hits.filter((h) => h.v === `/shared/${KEY}/playlists/${NOTPL}`).length;
      await page.locator('#search').press('Enter');
      await sleep(500);
      check(new URL(page.url()).searchParams.get('q') === NOTPL_LINK, `${tag}: Enter on those results keeps them (q ${new URL(page.url()).searchParams.get('q')})`);
      check(await depth() === d0 + 1 && where(page) === '/search', `${tag}: in the same entry (depth ${await depth()}, at ${where(page)})`);
      check(state.hits.filter((h) => h.v === `/shared/${KEY}/playlists/${NOTPL}`).length === asked, `${tag}: the post known to be no playlist is not asked for again`);
      await page.goBack();
      const home = phone ? '/search' : '/tracks';
      check(await until(() => where(page) === home && !new URL(page.url()).search), `${tag}: Back goes where search was opened from (at ${where(page)}${new URL(page.url()).search})`);
      // Pasted again: searched at once; 10 minutes later the post is asked for again (the node may read it by then)
      await paste(page, NOTPL_LINK);
      check(await until(() => where(page) === '/search' && new URL(page.url()).searchParams.get('q') === NOTPL_LINK), `${tag}: pasted again, it is searched at once (at ${where(page)})`);
      await sleep(300);
      check(state.hits.filter((h) => h.v === `/shared/${KEY}/playlists/${NOTPL}`).length === asked, `${tag}: without asking for the post`);
      await page.evaluate(() => { const real = Date.now; Date.now = () => real.call(Date) + 601000; });
      await page.locator('#search').fill('');
      await paste(page, NOTPL_LINK);
      check(await until(() => state.hits.filter((h) => h.v === `/shared/${KEY}/playlists/${NOTPL}`).length > asked), `${tag}: 10 minutes later the post is asked for again`);
      check(await until(() => where(page) === '/search' && new URL(page.url()).searchParams.get('q') === NOTPL_LINK), `${tag}: and searched as text again (at ${where(page)})`);
    }
    // A reload of those results: searched again in the reload's own entry (no second one to go Back to)
    {
      const n = searched(state);
      await fx.go(page, `/search?q=${encodeURIComponent(NOTPL_LINK)}`);
      check(await until(() => searched(state) > n && where(page) === '/search' && new URL(page.url()).searchParams.get('q') === NOTPL_LINK), `${tag}: a reload of the results searches the link again (at ${where(page)})`);
      await sleep(300);
      check(await depth() === 0, `${tag}: in the reload's own entry (depth ${await depth()})`);
      // and a reload of a playlist link's search opens the playlist in its place
      await fx.go(page, `/search?q=${encodeURIComponent(LINK)}`);
      check(await until(() => where(page) === `/shared/${KEY}/${ID}`), `${tag}: a reload of a playlist link's search opens the playlist (at ${where(page)})`);
      check(await depth() === 0, `${tag}: in the reload's own entry (depth ${await depth()})`);
    }
    // The box emptied while such a post is still being read: the link is not searched
    await fresh();
    {
      const n = searched(state);
      await paste(page, `pubky://${KEY}/pub/social/v1/posts/${SLOW}`);
      await until(() => where(page) === `/shared/${KEY}/${SLOW}`);
      await page.locator('#search').fill('');
      await sleep(1800);
      check(searched(state) === n, `${tag}: the box emptied before the post answered: nothing searched`);
      if (phone) check(where(page) === '/search' && !new URL(page.url()).search, `${tag}: and the phone shows the Search tab's empty page (at ${where(page)}${new URL(page.url()).search})`);
      else check(where(page) === `/shared/${KEY}/${SLOW}` && /not a playlist/.test((await page.locator('#main .notice').textContent().catch(() => '')) || ''), `${tag}: and the page says the post is no playlist (at ${where(page)})`);
    }
    // A playlist made private again still says so on its page (not searched)
    await fresh();
    {
      const n = searched(state);
      await paste(page, `pubky://${KEY}/pub/social/v1/posts/${GONE}`);
      check(await until(() => where(page) === `/shared/${KEY}/${GONE}`), `${tag}: a playlist made private again opens its page (at ${where(page)})`);
      check(await until(async () => /private again/.test((await page.locator('#main .notice').textContent().catch(() => '')) || '')), `${tag}: which says it is gone`);
      await sleep(400);
      check(searched(state) === n && where(page) === `/shared/${KEY}/${GONE}`, `${tag}: and nothing is searched`);
      await page.goBack(); await until(() => where(page) !== `/shared/${KEY}/${GONE}`);
    }
    // A homeserver that doesn't answer (502) still says so on the playlist's page (not searched)
    await fresh();
    {
      const n = searched(state);
      await paste(page, `pubky://${KEY}/pub/social/v1/posts/${DOWN}`);
      check(await until(() => where(page) === `/shared/${KEY}/${DOWN}`), `${tag}: a playlist whose homeserver doesn't answer opens its page (at ${where(page)})`);
      check(await until(async () => /doesn’t answer/.test((await page.locator('#main .notice').textContent().catch(() => '')) || '')), `${tag}: which says the homeserver doesn't answer`);
      await sleep(400);
      check(searched(state) === n && where(page) === `/shared/${KEY}/${DOWN}`, `${tag}: and nothing is searched (at ${where(page)})`);
    }
    // A post that is no playlist pasted over a search with a query: its search takes that search's
    // entry (as a new query there does), so Back goes where Search was opened from, not to the old query
    await fresh();
    {
      await paste(page, 'Night');
      await until(() => new URL(page.url()).searchParams.get('q') === 'Night');
      const d0 = await depth(); const n = searched(state);
      const overLink = `pubky://${KEY}/pub/social/v1/posts/${OVER}`;
      await paste(page, overLink);
      check(await until(() => state.hits.some((h) => h.v === `/shared/${KEY}/playlists/${OVER}`)), `${tag}: over a search, the post link is asked for first`);
      check(await until(() => searched(state) > n && where(page) === '/search' && new URL(page.url()).searchParams.get('q') === overLink), `${tag}: then it is searched as text (at ${where(page)}?q=${new URL(page.url()).searchParams.get('q')})`);
      await sleep(300);
      check(await depth() === d0, `${tag}: in the old query's entry (depth ${d0} → ${await depth()})`);
      check(await page.locator('#search').inputValue() === overLink, `${tag}: the box holds the link`);
      await page.screenshot({ path: path.join(shots, `${width}${android ? '-android' : ''}${phone ? '-phone' : ''}-post-link-over-search.png`) });
      // Forward onto the post's page left behind: it says the post is no playlist, and stays (no Back again)
      const nf = searched(state);
      await page.goForward();
      check(await until(async () => where(page) === `/shared/${KEY}/${OVER}` && /not a playlist/.test((await page.locator('#main .notice').textContent().catch(() => '')) || '')), `${tag}: Forward onto the post's page says it is no playlist (at ${where(page)})`);
      await sleep(800);
      check(where(page) === `/shared/${KEY}/${OVER}` && searched(state) === nf, `${tag}: and stays there, nothing searched (at ${where(page)})`);
      await page.goBack();
      check(await until(() => where(page) === '/search' && new URL(page.url()).searchParams.get('q') === overLink), `${tag}: Back from it: the link's search (at ${where(page)})`);
      const home = '/tracks'; // (the phone's Search tab took the query's entry)
      if (!phone) {
        await page.locator('#search').focus(); await page.locator('#search').press('Escape');
        check(await until(() => where(page) === home && !new URL(page.url()).search), `${tag}: Escape goes where Search was opened from (at ${where(page)}${new URL(page.url()).search})`);
        await page.goForward(); await until(() => where(page) === '/search');
      }
      await page.goBack();
      check(await until(() => where(page) === home && !new URL(page.url()).search), `${tag}: Back goes where Search was opened from, not to “Night” (at ${where(page)}${new URL(page.url()).search})`);
    }
    // The box edited while that Back to the search under it lands: what is in it then decides, in that
    // entry (before, the old query was put back over the typing): a new search, a key's person page, or,
    // emptied, the phone's empty Search tab and on desktop the page Search was opened from
    // (back: when the page's own Back lands, ms after it went Back; null: never, as in a sandboxed history.
    // escape: Escape pressed with the box emptied)
    const at = () => where(page) + new URL(page.url()).search;
    const over = async (i, { label, typed, escape, back = 0 }) => {
      await fresh();
      await paste(page, 'Night');
      await until(() => new URL(page.url()).searchParams.get('q') === 'Night');
      const d0 = await depth(); const n = searched(state); const nSefa = searchedFor(state, 'Sefa');
      // (typed the moment the page goes Back: before the search's entry is shown again)
      await page.evaluate(([t, esc, ms]) => {
        const real = history.back;
        history.back = function () {
          history.back = real; const s = document.getElementById('search'); s.value = t; s.dispatchEvent(new Event('input', { bubbles: true }));
          if (esc) s.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
          if (ms === null) return undefined; if (ms) { setTimeout(() => real.call(history), ms); return undefined; } return real.call(history);
        };
      }, [typed, !!escape, back]);
      await paste(page, `pubky://${KEY}/pub/social/v1/posts/${EDITS[i]}`);
      check(await until(() => state.hits.some((h) => h.v === `/shared/${KEY}/playlists/${EDITS[i]}`)), `${tag}: ${label}: the post link is asked for first`);
      const want = typed === 'Sefa' ? (p) => p === '/search?q=Sefa' : typed === LINK ? (p) => p === `/shared/${KEY}/${ID}` : typed ? (p) => p === `/person/${KEY}`
        : escape && phone ? (p) => p === '/' : phone ? (p) => p === '/search' : back === null ? (p) => p === '/' : (p) => p === '/tracks';
      check(await until(() => want(at()), back !== 0 ? 4500 : 3000), `${tag}: ${label}: that is what shows (at ${at()})`);
      // (a Back landing late: once it did)
      await sleep(back ? Math.max(600, back - 2400) : back === null ? 1000 : 600);
      check(want(at()), `${tag}: ${label}: and stays (at ${at()})`);
      if (back === null) await page.screenshot({ path: path.join(shots, `${width}${android ? '-android' : ''}${phone ? '-phone' : ''}-no-back-${typed ? 'typed' : 'emptied'}.png`) });
      check(!(await page.locator('#main .skeleton').count()), `${tag}: ${label}: no page is left on its loading skeleton`);
      check(await page.locator('#search').inputValue() !== 'Night', `${tag}: ${label}: the old query is not put back in the box (${await page.locator('#search').inputValue()})`);
      if (typed === 'Sefa') check(await page.locator('#search').inputValue() === 'Sefa', `${tag}: ${label}: the box keeps what was typed`);
      // (the typing's own search waiting 250 ms is not run as well: once for the new text, nothing for a link, key or empty box)
      check(searched(state) === n + (typed === 'Sefa' ? 1 : 0) && (typed !== 'Sefa' || searchedFor(state, 'Sefa') === nSefa + 1), `${tag}: ${label}: searched once for new text, never for a link, key or empty box (${searched(state) - n})`);
      if (typed === LINK) check(await until(async () => (await page.locator('#main h1').textContent().catch(() => '')) === 'Night drives'), `${tag}: ${label}: the playlist shows`);
      if (escape && phone) return;
      if (back === null) {
        // (no Back came: the post's own entry took it, the search under it is still there; a Back pressed
        // 2 s after that is the person's own: it shows that search and stays, no entry taken over)
        check(await depth() === d0 + 1, `${tag}: ${label}: in the post's entry, none pushed over it (depth ${d0} → ${await depth()})`);
        await sleep(1500);
        await page.goBack();
        check(await until(() => at() === '/search?q=Night'), `${tag}: ${label}: Back goes to the search under it (at ${at()})`);
        await sleep(800);
        check(at() === '/search?q=Night' && await depth() === d0 && await page.locator('#search').inputValue() === 'Night', `${tag}: ${label}: and stays there, its query in the box (at ${at()}, depth ${await depth()}, box ${await page.locator('#search').inputValue()})`);
        await page.goForward();
        check(await until(() => want(at())), `${tag}: ${label}: Forward goes back to it (at ${at()})`);
        return;
      }
      if (where(page) !== '/tracks') {
        check(await depth() === d0, `${tag}: ${label}: in the old query's entry (depth ${d0} → ${await depth()})`);
        await page.goBack();
        check(await until(() => where(page) === '/tracks' && !new URL(page.url()).search), `${tag}: ${label}: Back goes where Search was opened from (at ${at()})`);
      }
    };
    const cases = [
      { label: 'a new search during Back', typed: 'Sefa' }, { label: 'a Pubky key during Back', typed: `pubky://${KEY}` }, { label: 'the box emptied during Back', typed: '' },
      { label: 'a playlist link during Back', typed: LINK }, { label: 'Escape during Back', typed: '', escape: true },
      { label: 'a new search, Back after 3.5 s', typed: 'Sefa', back: 3500 }, { label: 'the box emptied, Back after 3.5 s', typed: '', back: 3500 },
      { label: 'a new search, no Back', typed: 'Sefa', back: null }, { label: 'the box emptied, no Back', typed: '', back: null },
    ];
    for (const [i, c] of cases.entries()) await over(i, c);
    // A search opened over another search (a menu's Search “…”), then a post that is no playlist pasted,
    // and another one typed while its Back lands: that one takes its search in its own entry, the first
    // search's entry stays under it
    await fresh();
    {
      await paste(page, 'Night');
      await until(() => new URL(page.url()).searchParams.get('q') === 'Night');
      const d0 = await depth();
      // (what a row's ⋯ menu leaves: its entry over the search, then its Search “Sefa” in that entry's place)
      await page.evaluate(() => { history.pushState({ ...history.state, transient: true, layer: false }, '', location.pathname + location.search); const s = document.getElementById('search'); s.value = 'Sefa'; s.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); });
      check(await until(() => at() === '/search?q=Sefa'), `${tag}: a menu's search over a search (at ${at()})`);
      check(await depth() === d0 + 1, `${tag}: in an entry of its own (depth ${d0} → ${await depth()})`);
      const second = `pubky://${KEY}/pub/social/v1/posts/${EDITS[cases.length + 1]}`;
      await page.evaluate((t) => { const real = history.back; history.back = function () { history.back = real; const s = document.getElementById('search'); s.value = t; s.dispatchEvent(new Event('input', { bubbles: true })); return real.call(history); }; }, second);
      await paste(page, `pubky://${KEY}/pub/social/v1/posts/${EDITS[cases.length]}`);
      check(await until(() => state.hits.some((h) => h.v === `/shared/${KEY}/playlists/${EDITS[cases.length + 1]}`)), `${tag}: a second post typed during Back is asked for`);
      check(await until(() => new URL(page.url()).searchParams.get('q') === second), `${tag}: and searched as text (at ${at()})`);
      await sleep(800);
      check(new URL(page.url()).searchParams.get('q') === second && await depth() === d0 + 1, `${tag}: in the menu's search's entry, not the first search's (depth ${d0 + 1} → ${await depth()})`);
      await page.goBack();
      check(await until(() => at() === '/search?q=Night'), `${tag}: Back goes to the first search (at ${at()})`);
    }
    // Phone: Back to the Search tab from an opened playlist, then emptying the box: no navigation,
    // the focus stays in the box to type again
    if (phone) {
      await fresh();
      await paste(page, LINK);
      await until(() => where(page) === `/shared/${KEY}/${ID}`);
      await page.goBack();
      check(await until(() => where(page) === '/search' && !new URL(page.url()).search), `${tag}: Back from the playlist goes to the Search tab (at ${where(page)})`);
      const d1 = await depth(); const at = page.url();
      check(await page.locator('#search').inputValue() === LINK, `${tag}: the box there still holds the link (${await page.locator('#search').inputValue()})`);
      await page.locator('#search').focus();
      await page.locator('#search').fill('');
      await sleep(500);
      check(page.url() === at && await depth() === d1, `${tag}: emptying the box there goes nowhere (at ${where(page)})`);
      check(await page.evaluate(() => document.activeElement && document.activeElement.id === 'search'), `${tag}: and the focus stays in the box`);
      check(await page.locator('[data-bnav="search"]').getAttribute('aria-current') === 'page' || await page.locator('[data-bnav="search"].on').count() > 0, `${tag}: on the Search tab`);
    }
    check(state.errors.length === 0, `${tag}: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    await run(browser, 360, { phone: true });
    await run(browser, 412, { phone: true });
    await run(browser, 360, { phone: true, android: true });
    await run(browser, 1280, {});
  } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} FAIL`);
  if (fail) process.exitCode = 1;
})().catch((e) => { console.error(e); process.exitCode = 1; });
