// Zones › "Group outputs…" opens Settings › Zones and outputs at its Group outputs form: the form's
// heading is in sight under the top bar, its first speaker is above the mini player, and the
// keyboard focus is on that first speaker (or on the form's heading when there is nothing to pick).
// Before, the tab opened at its top and the form was more than two screens down on a phone (past
// Prefer full tracks, the speaker list, Add a renderer and the zone rows). The form's help is one
// sentence (it was five lines at 360): the kinds that group together are the labelled rows of
// speakers right under it, and with nothing to pick the "No two speakers" line names them.
// Opening the tab any other way (an address, Back and Forward, after leaving Zones' press before
// the tab drew) still starts at its top.
// Checks at 360 (900 and 640 px tall), 360 in the Android app, 412 and 1280 px; no horizontal
// overflow, no page errors.
//   node tools/ui-zones-group-jump.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'zones-group-jump-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// Outputs as in tools/ui-setup.js: two Sonos, a BluOS, squeezelite players, the app's players on a DAC
const rate = (r) => ({ pcm_rates: r, max_pcm_bits: 24, max_channels: 2, dsd: null, hw_volume: null, standby: false, source_switch: false, gapless: true, groupable: false, fixed_latency_ms: null, quality_ceiling: 'bit_perfect' });
const OUTS = [
  { id: 'upnp:RINCON_A', name: 'Sonos Kitchen', protocol: 'upnp', caps: rate([44100, 48000]), group_by: 'sonos' },
  { id: 'upnp:RINCON_B', name: 'Sonos Living', protocol: 'upnp', caps: rate([44100, 48000]), group_by: 'sonos' },
  { id: 'bluos:aa01', name: 'Bluesound Study', protocol: 'bluos', caps: rate([44100, 48000, 96000]), group_by: 'bluos' },
  { id: 'slim:00:04:20:00:00:01', name: 'Pi Bedroom', protocol: 'slim', caps: { ...rate([44100, 96000]), groupable: true } },
  { id: 'slim:00:04:20:00:00:02', name: 'Pi Hall', protocol: 'slim', caps: { ...rate([44100]), groupable: true } },
  { id: 'net:192.168.1.20:7790/alsa:hw:1,0', name: 'Desk · USB DAC', protocol: 'net', caps: { ...rate([44100, 96000]), groupable: true } },
  { id: 'net:192.168.1.21:7790/alsa:hw:0,0', name: 'Studio · DAC', protocol: 'net', caps: { ...rate([44100]), groupable: true } },
  { id: 'upnp:streamer', name: 'Streamer', protocol: 'upnp', caps: rate([44100, 48000, 192000]) },
  { id: 'airplay:phone', name: 'Old phone', protocol: 'airplay', caps: rate([44100]) },
];
const zone = (o) => ({ id: o.id.replace(/[^a-z0-9]/gi, '-'), name: o.name, output: o.id, transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: null });
const ZONES = OUTS.map(zone);
// Only one speaker that groups: nothing to pick
const ONE = OUTS.filter((o) => !o.group_by && !o.caps.groupable).concat(OUTS[0]);

// Where the Group outputs form is: its heading's box, its first speaker's box, the focus, the bars around
const where = (page) => page.evaluate(() => {
  const name = [...document.querySelectorAll('.set-name')].find((n) => n.textContent.trim() === 'Group outputs');
  const blockEl = name && name.closest('.set-row');
  const first = document.querySelector('#groupouts input');
  const box = (e) => { if (!e) return null; const r = e.getBoundingClientRect(); return { top: r.top, bottom: r.bottom }; };
  const vis = (e) => !!e && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden';
  const tops = [...document.querySelectorAll('.topbar, header')].filter(vis).map((e) => e.getBoundingClientRect().bottom);
  const np = document.querySelector('#np');
  const help = blockEl && blockEl.querySelector('.set-help');
  const none = blockEl && blockEl.querySelector('.set-body > p');
  const lh = help ? parseFloat(getComputedStyle(help).lineHeight) : 0;
  const a = document.activeElement;
  return {
    view: document.body.dataset.view || null, path: location.pathname,
    name: box(name), first: box(first), topBar: tops.length ? Math.max(...tops) : 0,
    np: vis(np) ? np.getBoundingClientRect().top : innerHeight, h: innerHeight,
    help: help ? { text: help.textContent.trim(), lines: Math.round(help.getBoundingClientRect().height / lh) } : null,
    none: none ? none.textContent.trim() : null,
    focus: a === first ? 'first' : a === blockEl ? 'block' : a && blockEl && blockEl.contains(a) ? `in:${a.tagName}` : a ? `${a.tagName}#${a.id}` : null,
  };
});

(async () => {
  const browser = await chromium.launch();
  for (const [width, android, height] of [[360, false], [360, false, 640], [360, true], [412, false], [1280, false]]) {
    const tag = `${width}${height ? 'x' + height : ''}${android ? '-android' : ''}`;
    const phone = width < 600;
    const { context, page, state } = await fx.create(browser, width, { mobile: phone, fixtures: { '/outputs': { body: OUTS }, '/zones': { body: ZONES }, '/homeserver/bundled': { body: { configured: false, reachable: false } } } });
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    if (height) await page.setViewportSize({ width, height });
    const errors = []; page.on('pageerror', (e) => errors.push(String(e)));
    try {
      await fx.go(page, '/zones');
      await page.waitForSelector('#zgroup', { timeout: 15000 });
      await page.screenshot({ path: path.join(SHOTS, `zones-${tag}.png`) });
      if (phone) await page.tap('#zgroup'); else await page.click('#zgroup');
      await page.waitForSelector('#groupouts input', { timeout: 15000 });
      await page.waitForTimeout(400);
      const w = await where(page);
      check(/\/settings\/playback$/.test(w.path), `${tag}: Group outputs… opens Settings › Zones and outputs (${w.path})`);
      check(!!w.name && w.name.top >= w.topBar - 1 && w.name.bottom <= w.np, `${tag}: the form's heading is in sight (top ${w.name && Math.round(w.name.top)}, bar ${Math.round(w.topBar)}, mini player ${Math.round(w.np)})`);
      check(!!w.first && w.first.top >= w.topBar - 1 && w.first.bottom <= w.np, `${tag}: its first speaker is in sight above the mini player (${w.first && Math.round(w.first.top)}-${w.first && Math.round(w.first.bottom)})`);
      check(w.focus === 'first', `${tag}: the focus is on the first speaker (${w.focus})`);
      check(!!w.help && w.help.lines <= 2, `${tag}: the form's help is ${w.help && w.help.lines} line(s), at most 2`);
      check(!!w.help && /^Pick two or more of one kind/.test(w.help.text), `${tag}: the help says to pick two or more of one kind ("${w.help && w.help.text}")`);
      const kinds = await page.$$eval('#groupouts .groupkind > .small', (ls) => ls.map((l) => l.textContent.trim()));
      check(kinds.includes('Sonos') && kinds.includes('Squeezelite players'), `${tag}: the speakers are in rows by kind (${kinds.join(' / ')})`);
      // A screen reader names the focused speaker with its form and its kind ("Group outputs", "Sonos"), not alone
      const groups = await page.evaluate(() => { const out = []; for (let g = document.activeElement?.closest('[role="group"]'); g; g = g.parentElement?.closest('[role="group"]')) out.push(g.getAttribute('aria-label') || document.getElementById(g.getAttribute('aria-labelledby') || '')?.textContent.trim() || ''); return out; });
      check(groups.length === 2 && groups[0] === kinds[0] && groups[1] === 'Group outputs', `${tag}: the focused speaker is read inside its kind and the Group outputs form (${groups.join(' < ') || 'no group'})`);
      const named = await page.getByRole('group', { name: 'Group outputs', exact: true }).getByRole('checkbox').count();
      const all = await page.locator('#groupouts input[type="checkbox"]').count();
      check(all > 1 && named === all && await page.getByRole('group', { name: 'Sonos', exact: true }).getByRole('checkbox').count() > 0, `${tag}: every speaker is in the named Group outputs group (${named} of ${all}), Sonos ones in a Sonos group`);
      await page.screenshot({ path: path.join(SHOTS, `group-${tag}.png`) });
      await fx.cleanCopy(page).then(() => check(true, `${tag}: no undefined text, no horizontal overflow`), (e) => check(false, `${tag}: ${e.message}`));
      // The same tab opened another way starts at its top, focus not taken
      await fx.go(page, '/settings/playback');
      await page.waitForSelector('#groupouts input', { timeout: 15000 });
      await page.waitForTimeout(400);
      const o = await where(page);
      check(!!o.name && o.name.top > o.np - 1 && o.focus !== 'first', `${tag}: opened by address, the tab starts at its top (form at ${o.name && Math.round(o.name.top)}, focus ${o.focus})`);
      // In one page load: Group outputs… pressed, Back before the tab drew (a slow node), then Forward
      // to the tab: that is not the press, so the tab starts at its top
      await fx.go(page, '/zones'); await page.waitForSelector('#zgroup', { timeout: 15000 });
      state.delay = (v) => (v === '/node/settings' ? 1500 : 0);
      if (phone) await page.tap('#zgroup'); else await page.click('#zgroup');
      await page.waitForFunction(() => /\/settings\/playback$/.test(location.pathname));
      await page.goBack(); await page.waitForSelector('#zgroup', { timeout: 15000 });
      await page.waitForTimeout(1800);
      state.delay = 0;
      await page.goForward();
      await page.waitForSelector('#groupouts input', { timeout: 15000 });
      await page.waitForTimeout(400);
      const f = await where(page);
      check(!!f.name && f.name.top > f.np - 1 && f.focus !== 'first', `${tag}: left before it drew, a later visit to the tab starts at its top (form at ${f.name && Math.round(f.name.top)}, focus ${f.focus})`);
      check(!errors.length, `${tag}: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
    } catch (e) { check(false, `${tag}: ${e.message}`); }
    await context.close();
  }
  // Nothing to pick (one speaker that groups): the press lands on the form itself, whose line names the kinds
  for (const width of [360, 1280]) {
    const phone = width < 600;
    const { context, page } = await fx.create(browser, width, { mobile: phone, fixtures: { '/outputs': { body: ONE }, '/zones': { body: ONE.map(zone) }, '/homeserver/bundled': { body: { configured: false, reachable: false } } } });
    const errors = []; page.on('pageerror', (e) => errors.push(String(e)));
    try {
      await fx.go(page, '/zones'); await page.waitForSelector('#zgroup', { timeout: 15000 });
      if (phone) await page.tap('#zgroup'); else await page.click('#zgroup');
      await page.waitForSelector('#groupcard', { timeout: 15000 });
      await page.waitForTimeout(400);
      const w = await where(page);
      check(!!w.name && w.name.top >= w.topBar - 1 && w.name.bottom <= w.np, `${width} none: the form's heading is in sight (top ${w.name && Math.round(w.name.top)})`);
      check(w.focus === 'block', `${width} none: the focus is on the form (${w.focus})`);
      check(!!w.none && /^No two speakers of one kind yet\./.test(w.none) && /Sonos/.test(w.none) && /squeezelite/.test(w.none), `${width} none: it says which speakers group ("${w.none}")`);
      await page.screenshot({ path: path.join(SHOTS, `group-none-${width}.png`) });
      check(!errors.length, `${width} none: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
    } catch (e) { check(false, `${width} none: ${e.message}`); }
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
