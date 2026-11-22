// A page never says "empty" over songs it lists, nor offers Play all over nothing.
// 1. Songs: when the library's count (stats) and its list disagree because the library was
//    emptied between the two answers, an empty list wins: it shows "No songs yet" (no Play all, no
//    "Songs 6" head).
// 2. A playlist whose songs are all "On this node only" shows only that section: no Songs card
//    saying "This playlist is empty" above them. One with no songs at all still says it is empty,
//    and one with both shows both.
// At 360 (phone, and the Android app's pmDesktop stub), 412 and 1280 px.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-empty-claims.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/empty-claims-shots');
fs.mkdirSync(shots, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const song = (id, title) => ({ ...fx.track, id, title, canonical_uri: `https://musicbrainz.org/recording/0000000${id}-0000-4000-8000-000000000000`, mbid: null });
const LOCAL = [1, 2, 3].map((i) => ({ ...song(10 + i, `Home demo ${i}`), canonical_uri: null }));
const SYNCED = song(1, 'Alpha');
const plOf = (id, name, synced, local) => {
  const summary = { ...fx.summary, id, name, items: synced.length + local.length };
  return { summary, playlist: { ...summary, items: synced.map((t) => ({ uri: t.canonical_uri })) }, tracks: synced, entries: synced.map((t) => ({ kind: 'track', uri: t.canonical_uri })), local_tracks: local };
};
const pfOf = (synced) => ({ order: { order: ['library'], services: [{ id: 'library', name: 'Your library', connected: true }] }, own_order: false, items: synced.map((t, i) => ({ index: i, uri: t.canonical_uri, title: t.title, artist: 'Sefa', track_id: t.id, plays_from: { service: 'library', label: 'Your library' }, sources: [] })) });
const LISTS = { p1: plOf('p1', 'Only here', [], LOCAL), p2: plOf('p2', 'Nothing yet', [], []), p3: plOf('p3', 'Both kinds', [SYNCED], LOCAL) };
const plFixtures = Object.fromEntries(Object.entries(LISTS).flatMap(([id, d]) => [[`/playlists/${id}`, { body: d }], [`/playlists/${id}/play-from`, { body: pfOf(d.tracks) }]]));
plFixtures['/playlists'] = { body: Object.values(LISTS).map((d) => d.summary) };

const shown = (page, sel) => page.evaluate((s) => { const el = document.querySelector(s); return !!(el && el.getClientRects().length && !el.closest('[hidden]')); }, sel);
const visibleText = (page) => page.evaluate(() => document.querySelector('main').innerText);

async function songs(browser, width, label, open) {
  // (a) the count says 6, the list answers none (the library emptied between the two answers)
  {
    const { page, context, state } = await open({ fixtures: { '/library/tracks': { body: { tracks: [], next: null } }, '/library/stats': { body: { tracks: 6, albums: 1, artists: 1 } } } });
    try {
      await fx.go(page, '/tracks');
      await page.waitForFunction(() => document.querySelector('main .page-head h2')?.textContent.startsWith('Songs') && !document.querySelector('main .skeleton'));
      await page.waitForTimeout(150);
      await fx.cleanCopy(page);
      await page.screenshot({ path: path.join(shots, `songs-empty-${label}.png`) });
      const text = await visibleText(page);
      check(/No songs yet/.test(text), `${label} Songs: an empty list says "No songs yet" (text: ${JSON.stringify(text.slice(0, 120))})`);
      check(!(await shown(page, '#shuffle')), `${label} Songs: no Play all over an empty list`);
      check(!(await page.$('.page-head .count')) || !/6/.test(await page.$eval('.page-head .count', (c) => c.textContent)), `${label} Songs: the head doesn't say 6 songs over none`);
      check(state.errors.length === 0, `${label} Songs (empty): no page errors ${state.errors.join('; ')}`);
    } finally { await context.close(); }
  }
}

async function playlists(browser, width, label, open) {
  const { page, context, state } = await open({ fixtures: plFixtures });
  try {
    // Only "on this node only" songs: no Songs card claiming empty
    await fx.go(page, '/playlist/p1');
    await page.waitForFunction(() => document.querySelectorAll('#pllocal .track, #pllocal [data-srow]').length === 3);
    await page.waitForTimeout(200);
    await fx.cleanCopy(page);
    await page.screenshot({ path: path.join(shots, `playlist-local-only-${label}.png`), fullPage: true });
    let text = await visibleText(page);
    check(!/This playlist is empty/.test(text), `${label} playlist with only local songs: no "This playlist is empty"`);
    check(!(await shown(page, '#plsongs')), `${label} playlist with only local songs: no empty Songs card`);
    check(await shown(page, '#pllocal') && /On this node only/.test(text), `${label} playlist with only local songs: "On this node only" lists them`);
    check(/3 songs/.test(await page.$eval('.hero .sub', (e) => e.textContent)), `${label} playlist with only local songs: the head says 3 songs`);
    // No songs at all: it says so
    await fx.go(page, '/playlist/p2');
    await page.waitForFunction(() => document.querySelector('.hero h1')?.textContent === 'Nothing yet');
    await page.waitForTimeout(200);
    text = await visibleText(page);
    check(/This playlist is empty/.test(text) && await shown(page, '#plsongs'), `${label} playlist with no songs: "This playlist is empty"`);
    // Both: both sections, the synced song in Songs
    await fx.go(page, '/playlist/p3');
    await page.waitForFunction(() => document.querySelector('.hero h1')?.textContent === 'Both kinds' && document.querySelectorAll('#pllocal [data-srow], #pllocal .track').length === 3);
    await page.waitForTimeout(200);
    text = await visibleText(page);
    check(await shown(page, '#plsongs') && /Alpha/.test(await page.$eval('#plsongs', (e) => e.innerText)) && !/This playlist is empty/.test(text), `${label} playlist with both: the Songs card lists its song`);
    check(await shown(page, '#pllocal'), `${label} playlist with both: "On this node only" too`);
    check(state.errors.length === 0, `${label} playlists: no page errors ${state.errors.join('; ')}`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
      const label = `${width}${android ? '-android' : ''}`;
      const open = async (opts) => {
        const made = await fx.create(browser, width, { mobile: width < 600, ...opts });
        if (android) await made.context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
        return made;
      };
      await songs(browser, width, label, open);
      await playlists(browser, width, label, open);
    }
  } finally { await browser.close(); }
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exitCode = failed ? 1 : 0;
})().catch((error) => { console.error(error); process.exitCode = 1; });
