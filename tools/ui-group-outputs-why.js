// Settings › Zones and outputs › Group outputs: a speaker that can't join the ones picked looks off
// (its chip is dimmed with a not-allowed cursor, not only its small box), and one that is of the
// picked kind but shares no sample rate with them says so in the line under the speakers. Before,
// the chips looked the same as the ones you could pick and the reason was only in a title, which a
// touch screen never shows (a press on the chip did nothing, with no word why).
// Checks at 1280, 1024 and 768 px with the mouse and 412 px on a phone; no horizontal overflow, no
// page errors.
//   node tools/ui-group-outputs-why.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'group-outputs-why-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const rate = (r) => ({ pcm_rates: r, max_pcm_bits: 24, max_channels: 2, dsd: null, hw_volume: null, standby: false, source_switch: false, gapless: true, groupable: false, fixed_latency_ms: null, quality_ceiling: 'bit_perfect' });
// Two Sonos, a BluOS, four squeezelite players: two only at 96 kHz, one only at 44.1, one at both
const OUTS = [
  { id: 'upnp:RINCON_A', name: 'Sonos Kitchen', protocol: 'upnp', caps: rate([44100, 48000]), group_by: 'sonos' },
  { id: 'upnp:RINCON_B', name: 'Sonos Living', protocol: 'upnp', caps: rate([44100, 48000]), group_by: 'sonos' },
  { id: 'bluos:aa01', name: 'Bluesound Study', protocol: 'bluos', caps: rate([44100, 48000, 96000]), group_by: 'bluos' },
  { id: 'slim:00:04:20:00:00:01', name: 'Pi Bedroom', protocol: 'slim', caps: { ...rate([96000]), groupable: true } },
  { id: 'slim:00:04:20:00:00:02', name: 'Pi Hall', protocol: 'slim', caps: { ...rate([44100]), groupable: true } },
  { id: 'slim:00:04:20:00:00:03', name: 'Pi Den', protocol: 'slim', caps: { ...rate([44100, 96000]), groupable: true } },
  { id: 'slim:00:04:20:00:00:04', name: 'Pi Attic', protocol: 'slim', caps: { ...rate([96000]), groupable: true } },
];
const zone = (o) => ({ id: o.id.replace(/[^a-z0-9]/gi, '-'), name: o.name, output: o.id, transport: 'stopped', queue: [], queue_index: null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: null });

// Each speaker chip: its name, whether its box is disabled, the chip's opacity and cursor; the line under them
const read = (page) => page.evaluate(() => ({
  chips: [...document.querySelectorAll('#groupouts label.chip')].map((l) => { const cs = getComputedStyle(l); const i = l.querySelector('input'); return { name: l.textContent.trim(), off: i.disabled, on: i.checked, opacity: Number(cs.opacity), cursor: cs.cursor }; }),
  line: document.querySelector('#groupsync').textContent.trim(),
  lineShown: (() => { const e = document.querySelector('#groupsync'); const r = e.getBoundingClientRect(); return r.height > 0 && getComputedStyle(e).visibility !== 'hidden'; })(),
  overflow: document.documentElement.scrollWidth - innerWidth,
}));
const looksOff = (c) => c.opacity <= 0.6 && c.cursor === 'not-allowed';
const looksOn = (c) => c.opacity === 1 && c.cursor !== 'not-allowed';

(async () => {
  const browser = await chromium.launch();
  for (const width of [1280, 1024, 768, 412]) {
    const phone = width < 600;
    const { page } = await fx.create(browser, width, { mobile: phone, fixtures: { '/outputs': { body: OUTS }, '/zones': { body: OUTS.map(zone) }, '/homeserver/bundled': { body: { configured: false, reachable: false } } } });
    const errors = []; page.on('pageerror', (e) => errors.push(String(e)));
    const pick = async (name, force = false) => { const l = page.locator('#groupouts label.chip', { hasText: name }); await l.scrollIntoViewIfNeeded(); if (phone) await l.tap({ force }); else await l.click({ force }); await page.waitForTimeout(80); };
    const chip = (s, name) => s.chips.find((c) => c.name === name);
    try {
      await fx.go(page, '/settings/playback');
      await page.waitForSelector('#groupouts input', { timeout: 15000 });
      let s = await read(page);
      check(s.chips.length === OUTS.length && s.chips.every((c) => !c.off && looksOn(c)), `${width}: nothing picked, every speaker looks pickable (${s.chips.filter((c) => !looksOn(c)).map((c) => c.name).join(', ') || 'all'})`);
      check(s.line === '', `${width}: nothing picked, the line under the speakers is empty ("${s.line}")`);

      // Another kind: Sonos picked, the rest are off and look it
      await pick('Sonos Kitchen');
      s = await read(page);
      const others = s.chips.filter((c) => !c.name.startsWith('Sonos'));
      check(others.every((c) => c.off), `${width}: Sonos Kitchen picked, the other kinds are disabled`);
      check(others.every(looksOff), `${width}: and their chips look off (opacity ${others.map((c) => c.opacity).join('/')}, cursor ${others[0].cursor})`);
      check(looksOn(chip(s, 'Sonos Kitchen')) && looksOn(chip(s, 'Sonos Living')), `${width}: the picked Sonos and the other Sonos look as before`);
      check(s.line === '', `${width}: one Sonos picked, no sample-rate words ("${s.line}")`);
      await page.locator('#groupouts').screenshot({ path: path.join(SHOTS, `kind-${width}.png`) });
      // A press on a chip that's off changes nothing
      await pick('Bluesound Study', true); // force: the press a person makes, which Playwright would refuse on a disabled control
      s = await read(page);
      check(!chip(s, 'Bluesound Study').on && chip(s, 'Sonos Kitchen').on, `${width}: a press on an off chip picks nothing`);
      if (!phone) {
        // The mouse is on Bluesound Study now: an off chip doesn't light up under it (Pi Hall, off and not under the mouse, is the measure)
        const look = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll('#groupouts label.chip')].map((l) => [l.textContent.trim(), `${getComputedStyle(l).color} ${getComputedStyle(l).borderColor}`])));
        check(look['Bluesound Study'] === look['Pi Hall'], `${width}: a hovered off chip keeps the off look (${look['Bluesound Study']} vs ${look['Pi Hall']})`);
      }
      await pick('Sonos Kitchen');
      s = await read(page);
      check(s.chips.every((c) => !c.off && looksOn(c)) && s.line === '', `${width}: Sonos Kitchen unpicked, everything is pickable again and the line is empty`);

      // Same kind, no sample rate in common: said in words, one speaker
      await pick('Pi Hall');
      s = await read(page);
      check(chip(s, 'Pi Bedroom').off && looksOff(chip(s, 'Pi Bedroom')) && chip(s, 'Pi Attic').off, `${width}: Pi Hall (44.1 kHz) picked, the 96 kHz-only players are off and look it`);
      check(!chip(s, 'Pi Den').off && looksOn(chip(s, 'Pi Den')), `${width}: Pi Den (shares 44.1 kHz) stays pickable`);
      check(s.lineShown && s.line === 'Pi Bedroom and Pi Attic share no sample rate with Pi Hall, so they can’t join.', `${width}: the line says why, in sight ("${s.line}")`);
      await page.locator('#groupouts').locator('xpath=..').screenshot({ path: path.join(SHOTS, `rate-${width}.png`) });
      // Two picked: the in-sync words first, then why
      await pick('Pi Den');
      s = await read(page);
      check(/^In sync: the node keeps them in time sample by sample\. Pi Bedroom and Pi Attic share no sample rate with all of the ones picked, so they can’t join\.$/.test(s.line), `${width}: two picked, the sync words then why ("${s.line}")`);
      // Den first leads alone (it has both rates): nobody is off for a rate
      await pick('Pi Hall');
      s = await read(page);
      check(!chip(s, 'Pi Bedroom').off && !chip(s, 'Pi Hall').off && looksOn(chip(s, 'Pi Bedroom')) && s.line === '', `${width}: only Pi Den picked, every player is pickable and no why ("${s.line}")`);
      // One off for a rate: singular words
      await pick('Pi Bedroom');
      s = await read(page);
      check(chip(s, 'Pi Hall').off && /Pi Hall shares no sample rate with all of the ones picked, so it can’t join\.$/.test(s.line), `${width}: Den and Bedroom (96 kHz) picked, one off: singular ("${s.line}")`);

      check(s.overflow <= 0, `${width}: no horizontal overflow (${s.overflow})`);
    } catch (e) { check(false, `${width}: ${e.message.split('\n')[0]}`); }
    check(errors.length === 0, `${width}: no page errors${errors.length ? ` (${errors[0]})` : ''}`);
    await page.context().close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
