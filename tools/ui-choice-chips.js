// Choice chips say which one is chosen, the same way everywhere: the search source row
// (Everywhere, Library, Pubky, services) and the Albums quality row (All, Hi-res, Lossless...)
// are labelled groups of buttons with a pressed state, like the result filters and Favourites.
// The quality row shows only when a quality narrows the albums: no lone All without format data,
// none when every album has the same quality.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-choice-chips.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const shots = path.resolve(process.argv[3] || '/tmp/choice-chips-shots');
fs.mkdirSync(shots, { recursive: true });
// 700: the narrowest web width, where the source row scrolls sideways instead of wrapping
const WIDTHS = [1280, 1024, 768, 700];
const sources = [['tidal', 'TIDAL'], ['qobuz', 'Qobuz'], ['spotify', 'Spotify'], ['deezer', 'Deezer']].map(([id, name]) => ({ id, name, enabled: true, status: 'signed in', features: ['search'] }));
const albums = Array.from({ length: 6 }, (_, i) => ({ id: i + 1, title: `Album ${i + 1}`, album_artist: 'Band', year: 2000 + i, tracks: 1 }));
// Two hi-res, two CD-quality and two lossy albums: All, Hi-res, CD quality, Lossless and Lossy show
const formats = albums.map((a, i) => ({ album_id: a.id, label: ['24/96', '24/96', 'FLAC', 'FLAC', 'MP3', 'MP3'][i], hi_res: i < 2, lossless: i < 4, dsd: false }));

// What a screen reader is told about a row of chips: its group name and each button's pressed state
async function chipRow(page, selector) {
  return page.locator(selector).evaluate((row) => ({
    role: row.getAttribute('role'), label: row.getAttribute('aria-label'),
    chips: [...row.querySelectorAll('button')].map((b) => ({ role: b.getAttribute('role') || 'button', pressed: b.getAttribute('aria-pressed'), on: b.classList.contains('on'), text: b.textContent.trim() })),
  }));
}
function oneChosen(row, where, label, chosen) {
  assert.equal(row.role, 'group', `${where}: the chips are one group`);
  assert.equal(row.label, label, `${where}: the group says what it chooses`);
  for (const c of row.chips) {
    assert.equal(c.role, 'button', `${where}: ${c.text} is a button, like the other choice chips`);
    assert.equal(c.pressed, String(c.on), `${where}: ${c.text} is announced ${c.on ? 'chosen' : 'not chosen'}, as it looks`);
  }
  const on = row.chips.filter((c) => c.pressed === 'true');
  assert.equal(on.length, 1, `${where}: exactly one chip is chosen`);
  assert.match(on[0].text, chosen, `${where}: the chosen chip is the one in use`);
}

async function searchSources(browser, width) {
  const run = await fx.create(browser, width, { fixtures: { '/sources': { body: sources } } });
  const { page, state } = run;
  const failing = new Set(['qobuz']); const searched = [];
  await run.context.route((u) => u.pathname === '/v1/search', async (route) => {
    const scope = new URL(route.request().url()).searchParams.get('in') || 'everywhere'; searched.push(scope);
    const answers = scope === 'everywhere' ? ['library', 'tidal', 'qobuz'] : [scope];
    await route.fulfill({ contentType: 'application/x-ndjson', headers: { 'x-pm-sources': answers.join(',') },
      body: answers.map((source) => JSON.stringify(failing.has(source)
        ? { source, status: 'error', error: 'timed out', hits: [], counts: [] }
        : { source, status: 'ok', hits: [{ kind: 'song', id: `${source}-1`, title: 'Loop', subtitle: 'Band', keys: [`${source}:1`] }], counts: [] })).join('\n') + '\n' });
  });
  try {
    await fx.go(page, '/search?q=loop');
    await page.waitForFunction(() => window.pmLastSearch?.scope === 'everywhere' && document.querySelector('#scopechips [data-scope-in="qobuz"] .bad'));
    if (width === 1280) await page.screenshot({ path: path.join(shots, `search-sources-${width}.png`) });
    oneChosen(await chipRow(page, '#scopechips'), `search ${width}`, 'Search in', /^Everywhere/);
    // A source that could not answer says so in words, not only a red "!"
    assert.match(await page.locator('#scopechips').ariaSnapshot(), /Qobuz.*couldn.t search/, `search ${width}: a failed source is named as failed`);
    // Choosing a source moves the pressed state with it (one press, as before)
    await page.getByRole('button', { name: /^Library/ }).click();
    await page.waitForFunction(() => window.pmLastSearch?.scope === 'library');
    await page.getByRole('button', { name: /^Library/, pressed: true }).waitFor();
    oneChosen(await chipRow(page, '#scopechips'), `search ${width} after Library`, 'Search in', /^Library/);
    assert.deepEqual(searched, ['everywhere', 'library'], `search ${width}: one search per choice`);
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `search ${width}: no page errors`);
  } finally { await run.context.close(); }
}

async function albumQuality(browser, width) {
  const run = await fx.create(browser, width, { fixtures: { '/library/albums': { body: albums }, '/library/albums/formats': { body: formats } } });
  const { page, state } = run;
  try {
    await fx.go(page, '/albums');
    await page.locator('#albumgrid .tile').first().waitFor();
    const row = '.chips:has([data-focus])';
    oneChosen(await chipRow(page, row), `albums ${width}`, 'Quality', /^All/);
    await page.getByRole('button', { name: /^Lossless/ }).click();
    await page.getByRole('button', { name: /^Lossless/, pressed: true }).waitFor();
    oneChosen(await chipRow(page, row), `albums ${width} after Lossless`, 'Quality', /^Lossless/);
    assert.equal(await page.locator('#albumgrid .tile').count(), 4, `albums ${width}: Lossless shows the four lossless albums`);
    if (width === 1280) await page.screenshot({ path: path.join(shots, `albums-quality-${width}.png`) });
    await fx.cleanCopy(page);
    assert.deepEqual(state.errors, [], `albums ${width}: no page errors`);
  } finally { await run.context.close(); }
}

// No format data, or every album the same quality: no chip would narrow the grid, so no row
async function albumQualityHidden(browser, width) {
  const same = albums.map((a) => ({ album_id: a.id, label: 'FLAC', hi_res: false, lossless: true, dsd: false }));
  for (const [what, list] of [['no format data', []], ['all CD quality', same]]) {
    const run = await fx.create(browser, width, { fixtures: { '/library/albums': { body: albums }, '/library/albums/formats': { body: list } } });
    const { page, state } = run;
    try {
      await fx.go(page, '/albums');
      await page.locator('#albumgrid .tile').first().waitFor();
      assert.equal(await page.locator('#albumgrid .tile').count(), albums.length, `albums ${width} ${what}: every album shows`);
      assert.equal(await page.locator('[data-focus]').count(), 0, `albums ${width} ${what}: no quality row`);
      assert.equal(await page.locator('[role="group"][aria-label="Quality"]').count(), 0, `albums ${width} ${what}: no empty Quality group`);
      assert.equal(await page.locator('[data-lsort]').count(), 1, `albums ${width} ${what}: Sort is still there`);
      if (width === 1280 && !list.length) await page.screenshot({ path: path.join(shots, `albums-no-formats-${width}.png`) });
      assert.deepEqual(state.errors, [], `albums ${width} ${what}: no page errors`);
    } finally { await run.context.close(); }
  }
}

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  try {
    for (const width of WIDTHS) {
      for (const [name, check] of [['search sources', searchSources], ['album quality', albumQuality], ['album quality hidden', albumQualityHidden]]) {
        try { await check(browser, width); console.log(`ok   ${name} ${width}`); } catch (e) { failed++; console.log(`FAIL ${name} ${width}: ${e.message.split('\n')[0]}`); }
      }
    }
  } finally { await browser.close(); }
  if (failed) { console.error(`${failed} choice chip check(s) failed`); process.exit(1); }
  console.log('choice chips: all checks passed');
})().catch((e) => { console.error(e); process.exit(1); });
