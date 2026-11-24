// The DSP editor's DSD choice lists only what the node can do. It offered "to_pcm", which the
// engine refuses ("DSD to PCM conversion is not implemented yet", pm-audio pipeline.rs
// negotiate_dsd): a zone saved with it played no DSD file at all. The choices were also the
// setting's raw words (auto, native, to_pcm).
// Real candidate assets on the fixture origin (tools/ui-fixture.js), one zone, Settings > Zones and
// outputs with the zone's DSP editor open:
//   * the choices are Automatic, Native DSD and DoP, saved as auto, native and do_p;
//   * a zone that was saved with to_pcm still shows what it is set to, says that DSD files will
//     not play, and Save with another choice sends that choice;
//   * Save sends the choice unchanged when it was not touched.
//   node tools/ui-dsd-modes.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'dsd-modes-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const zone = { id: 'dac', name: 'Living room DAC', output: 'null', transport: 'stopped', now_playing: null, queue: [], queue_index: 0, queue_version: 1, repeat: 'off', shuffle: false, radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: null };
const dsp = (mode) => ({ headroom_db: null, volume_db: null, max_volume_db: null, dither: true, dsd_mode: mode, resample: true, eq: [], loudness: 'off', convolution: null, crossfeed: null });

async function open(browser, width, mode) {
  const run = await fx.create(browser, width, { mobile: width < 600, zone: zone.id, fixtures: { '/zones': { body: [zone] }, '/zones/dac': { body: zone } } });
  const { context, page } = run; const puts = [];
  await context.route(/\/v1\/zones\/dac\/dsp$/, (r) => { if (r.request().method() === 'PUT') { puts.push(JSON.parse(r.request().postData() || '{}')); return r.fulfill({ contentType: 'application/json', body: r.request().postData() }); } return r.fulfill({ contentType: 'application/json', body: JSON.stringify(dsp(mode)) }); });
  await page.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  await fx.go(page, '/settings/playback'); await page.waitForSelector('[data-dsp="dac"]');
  await page.locator('[data-dsp="dac"]').click(); await page.waitForSelector('#dspbox #d-dsd');
  const read = () => page.evaluate(() => { const s = document.querySelector('#d-dsd'); return { options: [...s.options].map((o) => `${o.value}=${o.textContent.trim()}`), value: s.value, shown: s.selectedOptions[0].textContent.trim(), note: (() => { const n = document.querySelector('#d-dsdnote'); return n && !n.hidden && n.getClientRects().length ? n.textContent : ''; })() }; });
  return { ...run, puts, read };
}

(async () => {
  const browser = await chromium.launch();
  for (const width of [390, 1280]) {
    const at = `${width}px`;
    // 1. A zone on the default
    let t = await open(browser, width, 'auto');
    let a = await t.read();
    check(a.options.join(' | ') === 'auto=Automatic | native=Native DSD | do_p=DoP' && a.value === 'auto' && !a.note, `${at}: the DSD choices are Automatic, Native DSD and DoP, and nothing the node cannot do (${a.options.join(' | ')}; set to ${a.shown})`);
    await t.page.locator('#dspbox').screenshot({ path: path.join(SHOTS, `dsp-editor-${width}.png`) });
    await t.page.locator('#d-save').click(); await t.page.waitForTimeout(400);
    check(t.puts.length === 1 && t.puts[0].dsd_mode === 'auto', `${at}: Save sends the choice as it was (${t.puts.map((p) => p.dsd_mode).join()})`);
    await t.page.locator('#d-dsd').selectOption('do_p'); await t.page.locator('#d-save').click(); await t.page.waitForTimeout(400);
    check(t.puts.length === 2 && t.puts[1].dsd_mode === 'do_p', `${at}: DoP chosen is saved as do_p (${t.puts.map((p) => p.dsd_mode).join()})`);
    check(t.state.errors.length === 0, `${at}: no script errors (${t.state.errors.join(' | ')})`);
    await t.context.close();
    // 2. A zone saved with to_pcm before
    t = await open(browser, width, 'to_pcm');
    a = await t.read();
    check(a.value === 'to_pcm' && /PCM/.test(a.shown) && /not/i.test(a.shown) && a.options.length === 4, `${at}: a zone saved with to_pcm still shows what it is set to (${a.shown}; ${a.options.length} choices)`);
    const warn = await t.page.evaluate(() => { const p = document.createElement('span'); p.style.color = 'var(--warn)'; document.body.appendChild(p); const c = getComputedStyle(p).color; p.remove(); return getComputedStyle(document.querySelector('#d-dsdnote')).color === c; });
    check(/DSD files/.test(a.note) && /won’t play|will not play/.test(a.note) && warn, `${at}: and says what that means, in the warning colour ("${a.note}", ${warn})`);
    // (a screen reader hears it with the choice, and the choice's name is still just DSD)
    const named = await t.page.evaluate(() => { const s = document.querySelector('#d-dsd'); const by = (a) => (s.getAttribute(a) || '').split(/\s+/).filter(Boolean).map((id) => (document.getElementById(id) || {}).textContent || '').join(' ').trim(); return { name: by('aria-labelledby'), described: by('aria-describedby') }; });
    check(named.name === 'DSD' && /DSD files/.test(named.described), `${at}: the choice is named DSD and described by the note (${JSON.stringify(named)})`);
    await t.page.locator('#dspbox').screenshot({ path: path.join(SHOTS, `dsp-editor-to-pcm-${width}.png`) });
    await t.page.locator('#d-dsd').selectOption('auto'); await t.page.waitForTimeout(100);
    const b = await t.read();
    const desc = await t.page.evaluate(() => document.querySelector('#d-dsd').getAttribute('aria-describedby'));
    check(!desc, `${at}: once another is chosen, the choice is no longer described by the hidden note (${desc})`);
    await t.page.locator('#d-save').click(); await t.page.waitForTimeout(400);
    check(!b.note && t.puts.length === 1 && t.puts[0].dsd_mode === 'auto', `${at}: another choice takes the note away and is what Save sends (${t.puts.map((p) => p.dsd_mode).join()}, note "${b.note}")`);
    check(t.state.errors.length === 0, `${at}: no script errors (${t.state.errors.join(' | ')})`);
    await t.context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
