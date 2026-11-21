// The fixture's default /v1/item answer (tools/ui-fixture.js itemPage) draws song, album and artist
// pages as a node's would (1280, 1024 and 768 px): the title, the breadcrumb's artist and album, the
// page's own songs (an album's, an artist's), Play starting them, and the "On the album" tile. Before,
// it lacked the page's target, so every item page in a fixture audit said "Cannot convert undefined
// or null to object" with a Try again, and audits could not look at those pages at all.
// Isolated API fixtures; no node or external network required.
//   node tools/ui-fixture-item-pages.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/fixture-item-pages-shots');
fs.mkdirSync(shots, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };
// Two albums by two artists: a page lists only its own songs
const sefa = { id: 11, name: 'Sefa' }; const peacock = { id: 22, name: 'Dr. Peacock' };
const tracks = [
  { ...fx.track, id: 1, title: 'Journey Through Time', artists: [sefa, peacock], album: { id: 7, title: 'Journey' }, duration_ms: 240000 },
  { ...fx.track, id: 2, title: 'Second Step', artists: [sefa], album: { id: 7, title: 'Journey' }, duration_ms: 180000 },
  { ...fx.track, id: 3, title: 'Feathers', artists: [peacock], album: { id: 8, title: 'Plumage' }, duration_ms: 200000 },
];
// (the fixture answers /library/tracks/1 only; Play reads each song it starts)
const fixtures = Object.fromEntries(tracks.map((t) => [`/library/tracks/${t.id}`, { body: t }]));
const text = (page, sel) => page.locator(sel).first().textContent({ timeout: 3000 }).then((t) => t.trim(), () => null);
const rows = (page) => page.locator('#isongs .list-row .name').evaluateAll((ns) => ns.map((n) => n.textContent.trim()));
const crumbs = (page) => page.locator('.crumbs [data-crumb-kind]').evaluateAll((bs) => bs.map((b) => `${b.dataset.crumbKind}:${b.textContent.trim()}`));

async function run(browser, width) {
  const { context, page, state } = await fx.create(browser, width, { tracks, fixtures });
  const at = `${width}`;
  try {
    // A song page: its title, its artists and album in the breadcrumb, its album's tile
    await fx.go(page, '/song/lib-1');
    await page.locator('#main h1').first().waitFor({ timeout: 5000 }).catch(() => {});
    check(await text(page, '#main h1') === 'Journey Through Time', `${at} song: the title is the song's (${await text(page, '#main h1')})`);
    check(!/Cannot convert|Try again/.test(await page.locator('#main').innerText()), `${at} song: no error and Try again in place of the page`);
    if (width >= 900) check(JSON.stringify(await crumbs(page)) === JSON.stringify(['artist:Sefa, Dr. Peacock', 'album:Journey']), `${at} song: the breadcrumb names its artists and album (${await crumbs(page)})`);
    check(await page.locator('.tile', { hasText: 'Journey' }).count() > 0, `${at} song: On the album shows Journey`);
    await fx.cleanCopy(page).then(() => check(true, `${at} song: no undefined, no overflow`), (e) => check(false, `${at} song: ${e.message}`));
    await page.screenshot({ path: path.join(shots, `after-song-${width}.png`) });

    // An album page: only that album's songs, in order, and Play starts the first
    await fx.go(page, '/album/lib-7');
    await page.locator('#isongs .list-row').first().waitFor({ timeout: 5000 }).catch(() => {});
    check(await text(page, '#main h1') === 'Journey', `${at} album: the title is the album's`);
    check(JSON.stringify(await rows(page)) === JSON.stringify(['Journey Through Time', 'Second Step']), `${at} album: its two songs, not Plumage's (${await rows(page)})`);
    check(/2 songs · 7 min/.test(await text(page, '#isongs .section-head') || ''), `${at} album: "2 songs · 7 min" over the list (${await text(page, '#isongs .section-head')})`);
    await page.screenshot({ path: path.join(shots, `after-album-${width}.png`) });
    await page.locator('#iplay').click({ timeout: 3000 }).catch(() => {});
    const queued = () => page.evaluate(() => JSON.parse(localStorage.getItem('pm.browserQueue') || '[]').map((q) => q.title));
    const want = JSON.stringify(['Journey Through Time', 'Second Step']);
    for (let i = 0; i < 30 && JSON.stringify(await queued()) !== want; i++) await page.waitForTimeout(100);
    check(JSON.stringify(await queued()) === want, `${at} album: Play queues the album's songs (${await queued()})`);

    // An artist page: the songs with that artist and the albums they are on
    await fx.go(page, '/artist/lib-22');
    await page.locator('#isongs .list-row').first().waitFor({ timeout: 5000 }).catch(() => {});
    check(await text(page, '#main h1') === 'Dr. Peacock', `${at} artist: the title is the artist's`);
    check(JSON.stringify(await rows(page)) === JSON.stringify(['Journey Through Time', 'Feathers']), `${at} artist: the songs with Dr. Peacock (${await rows(page)})`);
    const tiles = await page.locator('#ialbums .tile .t').evaluateAll((ts) => ts.map((t) => t.textContent.trim())).catch(() => []);
    check(tiles.includes('Journey') && tiles.includes('Plumage'), `${at} artist: the albums they are on, Journey and Plumage (${tiles})`);
    // An id the library doesn't have is a 404, as the node answers: no other page drawn in its place
    await fx.go(page, '/album/lib-99');
    await page.locator('#main .notice, #main h1').first().waitFor({ timeout: 5000 }).catch(() => {});
    check(await text(page, '#main h1') !== 'Journey' && /no album 99/.test(await page.locator('#main').innerText()), `${at} a missing album says so (${(await page.locator('#main').innerText()).slice(0, 60)})`);
    await fx.go(page, '/artist/lib-22');
    await page.locator('#isongs .list-row').first().waitFor({ timeout: 5000 }).catch(() => {});
    await fx.cleanCopy(page).then(() => check(true, `${at} artist: no undefined, no overflow`), (e) => check(false, `${at} artist: ${e.message}`));
    await page.screenshot({ path: path.join(shots, `after-artist-${width}.png`) });

    check(state.hits.some((h) => h.v === '/item/artist/lib-22/extras'), `${at} the artist page asked for its extras (Similar, Concerts)`);
    check(state.errors.length === 0, `${at} no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try { for (const w of [1280, 1024, 768]) await run(browser, w); } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} FAIL`);
  process.exit(fail ? 1 : 0);
})();
