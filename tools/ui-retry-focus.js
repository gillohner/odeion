// Try again by keyboard on a page that could not be read keeps the keyboard in the page. The page
// is drawn again, so the focus on Try again used to fall to the body (Enter did nothing more, and a
// screen reader said nothing of what came back). Real candidate assets on the fixture origin
// (tools/ui-fixture.js); a read answers 500, Try again is pressed with Enter:
//   * still failing: the focus is on the new Try again (Enter again asks again);
//   * once it answers: the focus is on the page's heading;
// for an album page (the page's own notice), Albums (the page's code gave up: #navretry) and the
// queue of a node zone (its own Try again), at 1280, 1024 and 768 px.
//   node tools/ui-retry-focus.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'retry-focus-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const t0 = Date.now(); for (;;) { const v = await fn().catch(() => null); if (v) return v; if (Date.now() - t0 > ms) return v; await sleep(40); } };
const DOWN = { status: 500, body: { error: 'the node is busy' } };

const item = (n) => ({ id: n, uri: `track:${n}`, track_id: n, title: `Song ${n}`, artist: 'Sefa', album: 'Journey', duration_ms: 240000 });
const zone = { id: 'kitchen', name: 'Kitchen', output: 'upnp:kitchen', transport: 'stopped', queue: [item(1), item(2)], queue_index: null, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: null };
const outputs = [{ id: 'upnp:kitchen', name: 'kitchen', kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }];

// Where the focus is: the page's Try again, its heading, or something else
const focusNow = (page) => page.evaluate(() => {
  const a = document.activeElement; const main = document.getElementById('main');
  if (!a || a === document.body) return 'body';
  if (a.matches('[data-failed] [data-retry]') && main.contains(a)) return 'retry';
  if (a.matches('h1, h2') && main.contains(a)) return `heading "${a.textContent.trim().slice(0, 40)}"`;
  return `${a.tagName.toLowerCase()}${a.id ? '#' + a.id : ''}${main.contains(a) ? ' in the page' : ''}`;
});
const notice = (page) => page.evaluate(() => !!document.querySelector('#main [data-failed] [data-retry]'));

(async () => {
  const browser = await chromium.launch();
  for (const width of [1280, 1024, 768]) {
    const at = `${width}px`;
    for (const [name, address, read, healed, drawn, more] of [
      ['an album', '/album/lib-7', '/item/album/lib-7', null, '#main h1, #main .page-head h2'],
      ['Albums', '/albums', '/library/albums', { body: [] }, '#main .page-head h2, #main .empty'],
      ['a node zone’s queue', '/queue', '/zones/kitchen/queue', { body: zone.queue }, '#main [data-qrow]', { zone: 'kitchen', node: true }],
    ]) {
      const fixtures = { [read]: DOWN, ...(more && more.node ? { '/zones': { body: [zone] }, '/outputs': { body: outputs } } : {}) };
      const { context, page, state } = await fx.create(browser, width, { ...(more && more.zone ? { zone: more.zone } : {}), fixtures });
      if (more && more.node) await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
      await fx.go(page, address);
      const n = await until(() => notice(page));
      check(n, `${at} ${name}: the page says it could not be read, with Try again`);
      if (!n) { await context.close(); continue; }
      // Still failing: Enter on Try again asks again, and the focus is on the new Try again
      const asked = state.hits.filter((h) => h.v === read).length;
      await page.focus('#main [data-failed] [data-retry]');
      await page.keyboard.press('Enter');
      await until(async () => state.hits.filter((h) => h.v === read).length > asked && (await notice(page)));
      const f1 = await until(async () => ((await focusNow(page)) === 'retry' ? 'retry' : null), 2000) || await focusNow(page);
      check(f1 === 'retry', `${at} ${name}: failing again, the focus is on the new Try again (${f1})`);
      if (width === 1280 && name === 'an album') await page.screenshot({ path: path.join(SHOTS, `album-failed-again-${width}.png`) });
      // Enter again works from there (the keyboard never left the page)
      const asked2 = state.hits.filter((h) => h.v === read).length;
      await page.keyboard.press('Enter');
      const again = await until(async () => state.hits.filter((h) => h.v === read).length > asked2 && (await notice(page)));
      check(!!again, `${at} ${name}: Enter again asks again (${state.hits.filter((h) => h.v === read).length - asked2} more)`);
      await until(async () => (await focusNow(page)) === 'retry', 2000);
      // Once it answers: the page is drawn and the focus is on its heading
      if (healed) state.fixtures[read] = healed; else delete state.fixtures[read];
      if ((await focusNow(page)) !== 'retry') await page.focus('#main [data-failed] [data-retry]');
      await page.keyboard.press('Enter');
      const ok = await until(() => page.evaluate((r) => !document.querySelector('#main [data-failed]') && !!document.querySelector(r), drawn));
      check(!!ok, `${at} ${name}: once it answers, Try again draws the page`);
      const f2 = await until(async () => { const f = await focusNow(page); return f.startsWith('heading') ? f : null; }, 2000) || await focusNow(page);
      check(f2.startsWith('heading'), `${at} ${name}: once it answers, the focus is on the page's heading (${f2})`);
      if (width === 1280 && name === 'an album') await page.screenshot({ path: path.join(SHOTS, `album-healed-${width}.png`) });
      // The next Tab goes on in the page, not back to the sidebar
      await page.keyboard.press('Tab');
      const next = await page.evaluate(() => { const a = document.activeElement; return { inMain: document.getElementById('main').contains(a), what: a ? `${a.tagName.toLowerCase()} ${(a.getAttribute('aria-label') || a.textContent || '').trim().slice(0, 30)}` : 'none' }; });
      check(next.inMain, `${at} ${name}: the next Tab stays in the page (${next.what})`);
      check(!state.errors.length, `${at} ${name}: no page error (${state.errors.join(' | ') || 'none'})`);
      await context.close();
    }
  }
  // ---- the focus somewhere else: it stays there --------------------------------------------------
  {
    const { context, page, state } = await fx.create(browser, 1280, { fixtures: { '/item/album/lib-7': { ...DOWN } } });
    await fx.go(page, '/album/lib-7');
    await until(() => notice(page));
    // (a press that is a click from elsewhere: the focus is on the search box, Try again pressed by the page's code)
    await page.focus('#search');
    await page.evaluate(() => document.querySelector('#main [data-failed] [data-retry]').click());
    await until(async () => state.hits.filter((h) => h.v === '/item/album/lib-7').length > 1 && (await notice(page)));
    await sleep(300);
    const f = await page.evaluate(() => (document.activeElement ? document.activeElement.id : ''));
    check(f === 'search', `1280px: Try again while the focus is in the search box leaves it there (${f || 'body'})`);
    check(!state.errors.length, `1280px other focus: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  // ---- a slow Try again and a click of theirs meanwhile: the focus isn't taken -----------------
  {
    const { context, page, state } = await fx.create(browser, 1280, { fixtures: { '/item/album/lib-7': { ...DOWN } } });
    await fx.go(page, '/album/lib-7');
    await until(() => notice(page));
    delete state.fixtures['/item/album/lib-7'];
    state.delay = (v) => (v === '/item/album/lib-7' ? 900 : 0);
    await page.focus('#main [data-failed] [data-retry]');
    await page.keyboard.press('Enter');
    await sleep(150);
    // (a click on empty space outside the page that nothing takes the focus for: it falls to the body)
    const box = await page.evaluate(() => {
      const takes = 'button, a[href], input, select, textarea, label, summary, [tabindex], [contenteditable], #main, dialog';
      for (let y = 4; y < innerHeight; y += 12) for (let x = 4; x < innerWidth; x += 12) { const el = document.elementFromPoint(x, y); if (el && !el.closest(takes)) return { x, y, what: el.tagName.toLowerCase() + (el.className ? '.' + String(el.className).split(' ')[0] : '') }; }
      return null;
    });
    check(!!box, `1280px slow Try again: there is empty space to click (${box ? box.what : 'none'})`);
    if (box) await page.mouse.click(box.x, box.y);
    const mid = await focusNow(page);
    check(mid === 'body', `1280px slow Try again: the click left the focus on the body while it is read (${mid})`);
    const drawn = await until(() => page.evaluate(() => !document.querySelector('#main [data-failed]') && !!document.querySelector('#main h1, #main .page-head h2')));
    check(!!drawn, '1280px slow Try again: the album is drawn once it answers');
    await sleep(300);
    const f = await focusNow(page);
    check(!f.startsWith('heading'), `1280px slow Try again: a click meanwhile keeps the focus where it put it, not on the heading (${f})`);
    check(!state.errors.length, `1280px slow Try again: no page error (${state.errors.join(' | ') || 'none'})`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `\n${failed} failed` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
