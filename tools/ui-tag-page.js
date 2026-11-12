// The tag page's picked tags can always be removed, and a page holds at most 3 of them
// (TASK-tag-page-remove-tags-1112; owner: "allow removing tags being looked for"). Real candidate
// assets on the fixture origin (tools/ui-fixture.js), /v1/home/tag answered here; at 360, 390 and 1440 px:
//   * two tags and an error from Pubky: both chips with their ×, also while the answer is still out;
//     × re-renders with the one tag left, a tap on a chip keeps only that tag;
//   * one tag: no picked row, "Also tagged" adds;
//   * three tags: no "Also tagged" chips, whatever the node lists;
//   * four tags (an old link): nothing is asked, the chips and one line are shown, × leads to a page that asks;
//   * chips wrap: nothing is wider than the screen;
//   * keyboard: after × (Enter) focus is on the next × (the one before for the last), on the heading
//     once one tag is left; after Enter on a picked tag, on the heading; never back on the page.
//   node tools/ui-tag-page.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'tag-page-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (page, fn, arg = null, timeout = 8000) => { const end = Date.now() + timeout; for (;;) { const v = await page.evaluate(fn, arg).catch(() => null); if (v) return v; if (Date.now() > end) return null; await sleep(50); } };

const PINAKES = 'bad request: tags allows at most 5 labels, or 3 with match=all; each is 1–128 bytes';
const item = (t) => ({ kind: 'other', title: t, name: t, description: 'A fixture page', site: 'example.com', uri: `https://example.com/${encodeURIComponent(t)}` });
const CO = [['live', 4], ['a-rather-long-tag-label-that-has-to-wrap', 3], ['track', 2], ['dracula', 2]];

// One page; `answer(labels)` gives { status, body, wait } for a /home/tag request, each of which is counted
const open = async (browser, w, url, answer) => {
  const { page, context, state } = await fx.create(browser, w, { touch: w < 1025 });
  const asked = [];
  await context.route('**/v1/home/tag?*', async (r) => {
    const labels = new URL(r.request().url()).searchParams.get('labels'); asked.push(labels);
    const a = answer(labels.split(','));
    if (a.wait) await sleep(a.wait);
    r.fulfill({ status: a.status || 200, contentType: 'application/json', body: JSON.stringify(a.body) }).catch(() => {});
  });
  await page.setViewportSize({ width: w, height: 900 });
  await fx.go(page, url); await page.waitForSelector('#tagbody');
  return { page, context, state, asked };
};
const read = (page) => page.evaluate(() => {
  const q = (s) => document.querySelector(s); const all = (s) => [...document.querySelectorAll(s)];
  const picked = q('#tagpicked'); const body = q('#tagbody');
  return {
    solo: all('[data-tag-solo]').map((b) => b.dataset.tagSolo), del: all('[data-tag-rm]').map((b) => b.dataset.tagRm), add: all('[data-tag-add]').map((b) => b.dataset.tagAdd),
    delNames: all('[data-tag-rm]').map((b) => b.getAttribute('aria-label')), pickedRow: !!picked, pickedAbove: picked && body ? picked.getBoundingClientRect().bottom <= body.getBoundingClientRect().top + 1 : null,
    body: body ? body.innerText.trim() : null, loading: !!q('#tagbody .skeleton'), head: (q('#main h1, #main .page-head h2') || {}).textContent || '',
    wide: document.documentElement.scrollWidth - innerWidth, right: Math.max(0, ...all('#tagpicked .ptag, #tagcotags .ptag').map((c) => Math.round(c.getBoundingClientRect().right))), vw: innerWidth,
    path: decodeURIComponent(location.pathname),
  };
});
const fits = (r) => r.wide <= 0 && r.right <= r.vw;

(async () => {
  const browser = await chromium.launch();
  for (const w of [360, 390, 1440]) {
    // ---- two tags, Pubky answers with an error ----
    {
      const { page, context, state, asked } = await open(browser, w, '/tag/apple-music,dracula', (l) => (l.length > 1 ? { status: 502, body: { error: PINAKES }, wait: 500 } : { body: { items: [item('On dracula')], tags: [] } }));
      const early = await read(page);
      check(early.loading && early.solo.join() === 'apple-music,dracula' && early.del.join() === 'apple-music,dracula', `${w}: two tags, still loading: both chips with × are there (${early.del.join()})`);
      await until(page, () => /Pubky did not answer/.test(document.querySelector('#tagbody').textContent));
      const r = await read(page);
      check(/Pubky did not answer/.test(r.body) && r.pickedRow && r.del.join() === 'apple-music,dracula' && r.pickedAbove, `${w}: two tags and an error: both can be removed, above the message (${r.del.join()})`);
      check(r.delNames.join() === 'Remove apple-music,Remove dracula' && !r.add.length, `${w}: each × is named after its tag (${r.delNames.join()})`);
      check(fits(r), `${w}: nothing wider than the screen with the error (${r.wide})`);
      await page.screenshot({ path: path.join(SHOTS, `two-tags-error-${w}.png`) });
      await page.click('[data-tag-rm="apple-music"]');
      await until(page, () => /On dracula/.test(document.querySelector('#tagbody').textContent));
      const one = await read(page);
      check(one.path === '/tag/dracula' && !one.pickedRow && !one.del.length && /On dracula/.test(one.body) && asked[asked.length - 1] === 'dracula', `${w}: × re-renders with the one tag left (${one.path}, asked ${asked[asked.length - 1]})`);
      check(!state.errors.length, `${w}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
      await context.close();
    }
    // ---- one, two and three tags with co-tags: adding stops at three ----
    {
      const { page, context, asked } = await open(browser, w, '/tag/music', (l) => ({ body: { items: [item('On ' + l.join(' and '))], tags: [...l.map((x) => [x, 9]), ...CO] } }));
      await page.waitForSelector('[data-tag-add]');
      const a = await read(page);
      check(!a.pickedRow && a.add.length === 4 && fits(a), `${w}: one tag: no picked row, Also tagged adds (${a.add.length} chips, page ${a.wide})`);
      await page.click('[data-tag-add="live"]'); await until(page, () => location.pathname.endsWith('music,live') && document.querySelector('[data-tag-add]'));
      const b = await read(page);
      check(b.pickedRow && b.del.join() === 'music,live' && b.add.length === 3 && !b.add.includes('live') && fits(b), `${w}: two tags: both removable, Also tagged still adds (${b.add.length} chips)`);
      await page.click('[data-tag-add="track"]'); await until(page, () => location.pathname.endsWith('music,live,track') && /On music and live and track/.test(document.querySelector('#tagbody').textContent));
      const c = await read(page);
      check(c.del.join() === 'music,live,track' && !c.add.length && /On music and live and track/.test(c.body) && fits(c), `${w}: three tags: all removable, no chips that add a fourth (${c.add.length})`);
      await page.screenshot({ path: path.join(SHOTS, `three-tags-${w}.png`) });
      // A tap on a chip keeps only that tag
      await page.click('[data-tag-solo="live"]'); await until(page, () => location.pathname.endsWith('/tag/live') && document.querySelector('[data-tag-add]'));
      const d = await read(page);
      check(d.path === '/tag/live' && !d.pickedRow && asked[asked.length - 1] === 'live', `${w}: a tap on a picked tag keeps only that one (${d.path})`);
      await context.close();
    }
    // ---- four tags (the owner's link): not asked; one can be removed ----
    {
      const { page, context, state, asked } = await open(browser, w, '/tag/apple-music,dracula,music,track', (l) => (l.length > 3 ? { status: 502, body: { error: PINAKES } } : { body: { items: [item('On three tags')], tags: CO } }));
      await sleep(700);
      const r = await read(page);
      check(asked.length === 0, `${w}: four tags: /home/tag is not asked (${asked.length} requests)`);
      check(r.del.join() === 'apple-music,dracula,music,track' && r.pickedAbove && r.body === 'A tag page matches up to 3 tags.' && !r.loading && !/Pubky did not answer/.test(r.body), `${w}: four tags: all four can be removed, with one line ("${r.body}")`);
      check(fits(r), `${w}: four chips wrap, nothing wider than the screen (page ${r.wide}, chips end at ${r.right} of ${r.vw})`);
      await page.screenshot({ path: path.join(SHOTS, `four-tags-${w}.png`) });
      await page.click('[data-tag-rm="music"]'); await until(page, () => /On three tags/.test(document.querySelector('#tagbody').textContent));
      const t = await read(page);
      check(t.path === '/tag/apple-music,dracula,track' && asked.join('|') === 'apple-music,dracula,track' && t.del.length === 3 && !t.add.length, `${w}: removing one asks for the three left (${asked.join('|')})`);
      check(!state.errors.length, `${w}: no page errors with four tags${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
      await context.close();
    }
    // ---- a label written twice counts once ----
    {
      const { page, context, asked } = await open(browser, w, '/tag/music,music,live,track', (l) => ({ body: { items: [item('On ' + l.join(' and '))], tags: [] } }));
      await until(page, () => /On music and live and track/.test(document.querySelector('#tagbody').textContent));
      const r = await read(page);
      check(asked.join('|') === 'music,live,track' && r.solo.join() === 'music,live,track', `${w}: a repeated label counts once (asked ${asked.join('|')}, chips ${r.solo.join()})`);
      await context.close();
    }
    // ---- keyboard: × and a picked tag leave the focus where the user is ----
    {
      const { page, context, state } = await open(browser, w, '/tag/music,live,track', (l) => ({ body: { items: [item('On ' + l.join(' and '))], tags: [] } }));
      const focus = () => page.evaluate(() => { const a = document.activeElement; return a === document.body ? 'body' : a.dataset.tagRm ? 'x:' + a.dataset.tagRm : a.dataset.tagSolo ? 'tag:' + a.dataset.tagSolo : a.matches('.page-head h2') ? 'heading:' + a.textContent.trim() : a.tagName; });
      const press = async (sel, path) => { await page.focus(sel); await page.keyboard.press('Enter'); await until(page, (p) => decodeURIComponent(location.pathname) === p && !document.querySelector('#tagbody .skeleton'), path); await sleep(100); return focus(); };
      const mid = await press('[data-tag-rm="live"]', '/tag/music,track');
      check(mid === 'x:track', `${w}: Enter on a middle × moves focus to the next × (${mid})`);
      await page.keyboard.press('Shift+Tab');
      const back = await focus();
      check(back === 'tag:track', `${w}: Shift+Tab from there goes to its tag, not the top of the page (${back})`);
      const last = await press('[data-tag-rm="track"]', '/tag/music');
      check(last.startsWith('heading:'), `${w}: Enter on × with two tags left: focus on the heading (${last})`);
      await fx.go(page, '/tag/music,live,track'); await page.waitForSelector('[data-tag-rm="track"]');
      const end = await press('[data-tag-rm="track"]', '/tag/music,live');
      check(end === 'x:live', `${w}: Enter on the last × moves focus to the one before (${end})`);
      const solo = await press('[data-tag-solo="live"]', '/tag/live');
      check(solo.startsWith('heading:') && /#live/.test(solo), `${w}: Enter on a picked tag: focus on the heading of its page (${solo})`);
      // A click on × keeps the same place too
      await fx.go(page, '/tag/music,live,track'); await page.waitForSelector('[data-tag-rm="music"]');
      await page.click('[data-tag-rm="music"]'); await until(page, () => location.pathname.endsWith('/tag/live,track') && !document.querySelector('#tagbody .skeleton'));
      await sleep(100); const click = await focus();
      check(click === 'x:live', `${w}: a click on the first × moves focus to the next × (${click})`);
      check(!state.errors.length, `${w}: no page errors with the keyboard${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
      await context.close();
    }
    // ---- long labels, nothing found ----
    {
      const long = ['a-rather-long-tag-label-that-has-to-wrap', 'another-quite-long-label-for-a-phone', 'third'];
      const { page, context } = await open(browser, w, '/tag/' + long.join(','), () => ({ body: { items: [], tags: [], note: 'Nothing tagged with all of these yet.' } }));
      await until(page, () => !document.querySelector('#tagbody .skeleton'));
      const r = await read(page);
      check(r.del.length === 3 && fits(r), `${w}: long labels and nothing found: chips stay, nothing wider than the screen (page ${r.wide}, chips end at ${r.right} of ${r.vw})`);
      await context.close();
    }
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
