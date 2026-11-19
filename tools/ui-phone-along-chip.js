// Listening along with someone on a phone, the mini player's chip is their name and Leave, so the
// song beside it keeps its title. Real candidate assets on the fixture origin (tools/ui-fixture.js):
// no node, no network. At 360 and 412 px, in a browser and in the Android app, with a files job's
// chip too, without one, and on a listener's account:
//   * the chip draws the name ("Anna") and Leave; "Along with" is not drawn, but a screen reader
//     still hears "Along with Anna" (accessibility tree; the words are clipped, not display: none
//     or visibility: hidden); Leave's name starts with the word it shows
//     ("Leave, stop listening along": a voice command "tap Leave" finds it);
//   * the chip is at most 112 px and the song's title has 75 px or more (at 360 px with a job it
//     had 30 px, "W…", while "Along with Anna" took 159 px);
//   * a long name is cut with … (text-overflow: ellipsis) inside the chip, the title keeps its
//     room, no sideways scroll.
// On a desktop width (1280 px) the chip still reads "Along with Anna" in full.
//   node tools/ui-phone-along-chip.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'phone-along-chip-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const end = Date.now() + ms; for (;;) { if (await fn()) return true; if (Date.now() > end) return false; await sleep(50); } };

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const queue = [{ id: 1, uri: 'track:1', title: 'Wonderful Tonight', artist: 'Eric Clapton', duration_ms: 219000 }];
const np = { item: { ...queue[0], album: 'Eric Clapton Story' }, index: 0, position_ms: 61000, duration_ms: 219000, source_format: fmt, output_format: fmt, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zone = { id: 'linn', name: 'Linn Klimax DS3', output: 'upnp:linn', transport: 'playing', queue, queue_index: 0, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: { percent: 20, muted: false }, now_playing: np };
const job = { id: 'j1', kind: 'copy', state: 'running', total: 25, done: 7, failed: 0, skipped: 0, bytes_total: 750e6, bytes_done: 210e6, errors: [], errors_count: 0, current_folder: '/music', speed_bps: 2.4e6, eta_s: 600 };
// presence.rs's GET /v1/presence: listening along with this person
const along = (name) => ({ key: 'k-anna', name, mine: true, you: false, hosting: false, zone: { id: 'linn', name: 'Linn Klimax DS3' }, your_zone: 'linn' });
const fixtures = (withJob, name) => ({
  '/zones': { body: [zone] }, '/zones/linn/queue': { body: queue },
  '/outputs': { body: [{ id: 'upnp:linn', name: 'Linn', kind: 'upnp', caps: { hw_volume: 'percent' } }] },
  '/pubky/files/jobs': { body: withJob ? { current: job, queued: [], last: null } : { current: null, queued: [], last: null } },
  '/presence': { body: [along(name)] }, '/me/presence': { body: { visible: 'hidden' } }, '/presence/along': { body: zone },
});
const LONG = 'Annabelle-Marie Fitzgerald-Okonkwo';
const RUNS = [
  { job: true, name: 'Anna', what: 'a files job running', file: 'job' },
  { job: false, name: 'Anna', what: 'no job', file: 'nojob' },
  { job: true, listener: true, name: 'Anna', what: "a listener's account", file: 'listener' },
  { job: true, name: LONG, what: 'a files job running, a long name', file: 'long' },
];
const look = (page) => page.evaluate(() => {
  const w = (e) => (e ? e.getBoundingClientRect().width : -1);
  const chip = document.querySelector('#along'); const text = document.querySelector('#alongtext');
  const pre = text.querySelector('.along-pre'); const leave = document.querySelector('#alongleave');
  const title = document.querySelector('#np .np-title');
  const preCs = pre ? getComputedStyle(pre) : null;
  return {
    preShown: !!preCs && preCs.display !== 'none' && preCs.visibility === 'visible', ellipsis: getComputedStyle(text).textOverflow,
    chip: w(chip), pre: w(pre), said: text.textContent.replace(/\s+/g, ' ').trim(), cut: text.scrollWidth > text.clientWidth + 1,
    nameW: w(text), leave: leave.getAttribute('aria-label'), leaveText: leave.textContent.trim(), title: w(title),
    titleCut: title.scrollWidth > title.clientWidth + 1, job: !document.querySelector('#jobchip').hidden && w(document.querySelector('#jobchip')) > 0,
    overflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
  };
});
const open = async (browser, width, R, android, mobile = true) => {
  const { context, page, state } = await fx.create(browser, width, { mobile, fixtures: fixtures(R.job, R.name), ...(R.listener ? { role: 'listener' } : {}) });
  await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones: [zone] })); });
  await context.addInitScript((a) => { localStorage.setItem('pm.zone', 'linn'); if (a) window.pmDesktop = { platform: 'android', version: '0.0.0' }; }, android);
  await fx.go(page, '/');
  await page.waitForFunction(() => /Wonderful Tonight/.test(document.querySelector('#np .np-title')?.textContent || ''));
  return { context, page, state };
};

(async () => {
  const browser = await chromium.launch();
  for (const width of [360, 412]) {
    for (const android of [false, true]) {
      for (const R of RUNS) {
        const where = `${width} px, ${android ? 'Android app' : 'browser'}, ${R.what}`;
        const { context, page, state } = await open(browser, width, R, android);
        check(await until(() => page.isVisible('#along')), `${where}: the along chip shows`);
        if (R.job && !R.listener) check(await until(() => page.isVisible('#jobchip')), `${where}: the files job's chip shows`);
        await sleep(300);
        const m = await look(page);
        const heard = (await page.locator('#along').ariaSnapshot()).replace(/\s+/g, ' ');
        check(m.pre >= 0 && m.pre <= 1, `${where}: "Along with" is not drawn (${m.pre < 0 ? 'no separate words' : Math.round(m.pre) + ' px'})`);
        check(m.said === `Along with ${R.name}`, `${where}: the chip's text is "Along with ${R.name}" ("${m.said}")`);
        check(heard.includes(`Along with ${R.name}`), `${where}: a screen reader hears "Along with ${R.name}" (${heard})`);
        check(m.preShown, `${where}: "Along with" is clipped, not display: none or visibility: hidden`);
        check(m.leave === 'Leave, stop listening along' && m.leaveText === 'Leave', `${where}: Leave says "${m.leaveText}" / "${m.leave}"`);
        check(m.chip > 0 && m.chip <= 112.5, `${where}: the chip takes ${Math.round(m.chip)} px (<= 112)`);
        check(m.title >= 75, `${where}: the song's title has ${Math.round(m.title)} px (>= 75)`);
        if (R.name === LONG) check(m.cut && m.nameW > 20 && m.ellipsis === 'ellipsis', `${where}: the long name is cut with … (${Math.round(m.nameW)} px shown, text-overflow ${m.ellipsis})`);
        check(m.overflow <= 0, `${where}: no sideways scroll (${m.overflow})`);
        check(!state.errors.length, `${where}: no page errors (${state.errors.join('; ') || 'none'})`);
        await page.screenshot({ path: path.join(SHOTS, `along-${width}-${android ? 'app' : 'web'}-${R.file}.png`) });
        await context.close();
      }
    }
  }
  {
    const R = RUNS[0]; const where = '1280 px, browser';
    const { context, page } = await open(browser, 1280, R, false, false);
    check(await until(() => page.isVisible('#along')), `${where}: the along chip shows`);
    const m = await look(page);
    check(m.said === 'Along with Anna' && m.pre > 20, `${where}: the chip draws "Along with Anna" in full ("${m.said}", the words ${Math.round(m.pre)} px)`);
    await page.screenshot({ path: path.join(SHOTS, 'along-1280-web.png') });
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
