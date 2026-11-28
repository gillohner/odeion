// The Sound panel (the player's sound pill, #quality): the words of each part start at one
// line, 10 px in. Before, a note in the DSP part ("This browser plays the file as it is…", "Reading
// this zone's DSP…", or the node's error) started 4 px further in than the Signal path note above it
// and the part titles. The DSP switches and the Source name keep that line too.
// This browser at 1280/1024/768, and a node zone (nothing playing) while its DSP is read, when the
// read fails, and with its switches.
// Real candidate assets, isolated API fixtures; no node or external network required.
//   node tools/ui-sound-panel-align.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs');
const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'sound-panel-align-shots'));
fs.mkdirSync(SHOTS, { recursive: true });

let ok = 0; let fail = 0;
const check = (cond, msg) => { if (cond) { ok++; console.log(`ok   ${msg}`); } else { fail++; console.log(`FAIL ${msg}`); } };

const ZONE = { id: 'study', name: 'Study', output: 'upnp:study', transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: null };
const OUTPUTS = [{ id: 'upnp:study', name: 'Study', kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }];
const DSP = { eq: [], convolution: null, loudness: 'off', crossfeed: null };

// Where the words of each part start (the first letter's left edge), read from the text itself
const lefts = (page) => page.evaluate(() => {
  const p = document.querySelector('#qpanel');
  const at = (el) => {
    if (!el || !el.getClientRects().length) return null;
    // a note that starts with a spinner ("Reading this zone's DSP…") starts where the spinner does
    const lead = el.firstElementChild; if (lead && lead.matches('.spin') && !(el.firstChild.nodeType === 3 && el.firstChild.textContent.trim())) return lead.getBoundingClientRect().left;
    const w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.textContent.trim() ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP) });
    const t = w.nextNode(); if (!t) return null;
    const r = document.createRange(); r.setStart(t, t.textContent.search(/\S/)); r.setEnd(t, t.textContent.search(/\S/) + 1);
    return r.getBoundingClientRect().left;
  };
  const titles = [...p.querySelectorAll(':scope > .pop-title')];
  const named = (re) => titles.find((x) => re.test(x.textContent));
  const after = (title) => { let n = title && title.nextElementSibling; return n; };
  const pathPart = after(named(/^Signal path/));
  return {
    titles: titles.map(at),
    path: pathPart && pathPart.matches('.qp-note') ? at(pathPart) : null,
    src: at(p.querySelector('.qp-src .name')),
    dspNote: at(p.querySelector('.qp-dsp > .qp-note')),
    dspRows: [...p.querySelectorAll('.qp-dsp .qp-row .name')].map(at),
    dspText: (p.querySelector('.qp-dsp') || {}).textContent || '',
  };
});
const same = (a, b) => a != null && b != null && Math.abs(a - b) < 0.5;
const openPanel = async (page) => { await page.locator('#quality').click(); await page.locator('#qpanel').waitFor({ state: 'visible' }); };
const closePanel = async (page) => { await page.keyboard.press('Escape'); await page.locator('#qpanel').waitFor({ state: 'hidden' }); };

async function browserZone(browser, width) {
  const { context, page, state } = await fx.create(browser, width, {});
  try {
    await fx.go(page, '/library');
    await page.locator('#quality').waitFor();
    await page.waitForTimeout(600);
    await openPanel(page);
    const l = await lefts(page);
    const line = l.titles[0];
    check(l.titles.length === 4 && l.titles.every((x) => same(x, line)), `${width} this browser: the four part titles start at one line (${l.titles.map(Math.round)})`);
    check(same(l.path, line), `${width} this browser: the Signal path note starts there (${Math.round(l.path)} vs ${Math.round(line)})`);
    check(/plays the file as it is/.test(l.dspText) && same(l.dspNote, l.path), `${width} this browser: the DSP note starts where the Signal path note does (${l.dspNote != null ? Math.round(l.dspNote) : 'none'} vs ${Math.round(l.path)})`);
    check(same(l.src, line), `${width} this browser: Source's "Nothing playing" starts there too (${Math.round(l.src)})`);
    await page.screenshot({ path: path.join(SHOTS, `browser-${width}.png`) });
    await fx.cleanCopy(page);
    check(state.errors.length === 0, `${width} this browser: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

async function nodeZone(browser, width) {
  let dsp = { status: 200, body: DSP }; let hold = null;
  const { context, page, state } = await fx.create(browser, width, { zone: 'study', fixtures: { '/zones': { body: [ZONE] }, '/outputs': { body: OUTPUTS } } });
  try {
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [ZONE] })); });
    await context.route(/\/v1\/zones\/study\/dsp$/, async (r) => {
      if (hold) await hold;
      return r.fulfill({ status: dsp.status, contentType: 'application/json', body: JSON.stringify(dsp.body) });
    });
    await fx.go(page, '/library');
    await page.locator('#quality').waitFor();
    await page.waitForTimeout(800);

    // 1. While the DSP is read
    let release; hold = new Promise((res) => { release = res; });
    await openPanel(page);
    await page.waitForTimeout(300);
    let l = await lefts(page);
    const line = l.titles[0];
    check(/Study/.test(await page.locator('#qpanel').textContent()), `${width} node zone: the panel is the Study zone's`);
    check(/Reading this zone/.test(l.dspText) && same(l.dspNote, line), `${width} node zone, reading: the DSP note starts at the titles' line (${l.dspNote != null ? Math.round(l.dspNote) : 'none'} vs ${Math.round(line)}; "${l.dspText.trim()}")`);
    check(same(l.path, line), `${width} node zone: the Signal path note ("Nothing playing.") starts there (${Math.round(l.path)})`);
    await page.screenshot({ path: path.join(SHOTS, `node-reading-${width}.png`) });
    release(); hold = null;
    await closePanel(page);

    // 2. The read fails: the node's words, on the same line
    dsp = { status: 500, body: { error: 'The DSP settings could not be read' } };
    await openPanel(page);
    await page.waitForFunction(() => /could not be read/.test(document.querySelector('#qpanel .qp-dsp').textContent), null, { timeout: 4000 });
    l = await lefts(page);
    check(same(l.dspNote, line), `${width} node zone, an error: the DSP note starts at the titles' line (${l.dspNote != null ? Math.round(l.dspNote) : 'none'} vs ${Math.round(line)})`);
    await page.screenshot({ path: path.join(SHOTS, `node-error-${width}.png`) });
    await closePanel(page);

    // 3. Its switches: their names keep the same line (they already did)
    dsp = { status: 200, body: DSP };
    await openPanel(page);
    await page.waitForFunction(() => document.querySelectorAll('#qpanel .qp-dsp .qp-row').length === 4, null, { timeout: 4000 });
    l = await lefts(page);
    check(l.dspRows.length === 4 && l.dspRows.every((x) => same(x, line)), `${width} node zone, switches: each switch's name starts at the titles' line (${l.dspRows.map(Math.round)})`);
    await page.screenshot({ path: path.join(SHOTS, `node-switches-${width}.png`) });
    await fx.cleanCopy(page);
    check(state.errors.length === 0, `${width} node zone: no page errors (${state.errors.join('; ')})`);
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch();
  try {
    for (const width of [1280, 1024, 768]) {
      for (const run of [browserZone, nodeZone]) {
        try { await run(browser, width); } catch (e) { fail++; console.log(`FAIL ${width} ${run.name}: ${e.message.split('\n')[0]}`); }
      }
    }
  } finally { await browser.close(); }
  console.log(`\n${ok} ok, ${fail} failed`);
  process.exit(fail ? 1 : 0);
})();
