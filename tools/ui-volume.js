// One volume, one source (TASK-volume-one-source): every slider for a zone shows the level the
// node read back from the output, and there is exactly one slider for the current zone in the bar
// area at any width (the bar's, or the volume popover's; never the Sound panel's). Uses the
// developer outputs with a pretend volume (`null:db`, `null:percent`) next to the plain Null output.
//   node tools/ui-volume.js <api token> <node URL>
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const TOKEN = process.argv[2] || 'ci-token';
const BASE = process.argv[3] || 'http://127.0.0.1:7788';
const problems = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) problems.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 8000) => { const end = Date.now() + ms; for (;;) { if (await fn().catch(() => false)) return true; if (Date.now() > end) return false; await sleep(150); } };
const api = async (method, path, body) => { const r = await fetch(BASE + '/v1' + path, { method, headers: { Authorization: 'Bearer ' + TOKEN, 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined }); return r.status === 204 ? null : r.json().catch(() => null); };
const zoneVol = async (id) => (await api('GET', `/zones/${encodeURIComponent(id)}`)).volume;
// Set a range input the way a person does: the value, then input and change
const slide = (p, sel, v) => p.$eval(sel, (el, v) => { el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); }, v);
const visibleRanges = (p, within) => p.$$eval(within, (els) => els.filter((e) => e.type === 'range' && e.id !== 'seek' && e.offsetParent && getComputedStyle(e).visibility !== 'hidden').map((e) => e.id || e.dataset.zvol));

(async () => {
  const zones = await api('GET', '/zones');
  const dbZone = zones.find((z) => z.output === 'null:db'); const pcZone = zones.find((z) => z.output === 'null:percent'); const plain = zones.find((z) => z.output === 'null');
  if (!dbZone || !pcZone) { console.log('FAIL the developer volume outputs (null:db, null:percent) are not on this node'); process.exit(1); }
  const devBefore = (await api('GET', '/node/settings')).dev_outputs;
  await api('PUT', '/node/settings', { dev_outputs: true });
  await api('POST', `/zones/${dbZone.id}/volume`, { volume: { db: 0 } });
  await api('POST', `/zones/${pcZone.id}/volume`, { volume: { percent: 100 } });
  await api('POST', `/zones/${pcZone.id}/volume`, { volume: { mute: false } });
  const browser = await chromium.launch();
  try {
    for (const [w, h] of [[1345, 900], [1080, 800], [834, 1194], [768, 1024], [390, 844]]) {
      const ctx = await browser.newContext({ viewport: { width: w, height: h }, hasTouch: w < 1025, isMobile: w < 600 });
      await ctx.addInitScript(([t, z]) => { localStorage.setItem('pm.token', t); localStorage.setItem('pm.zone', z); }, [TOKEN, dbZone.id]);
      const p = await ctx.newPage(); const errors = []; p.on('pageerror', (e) => errors.push(e.message));
      await p.goto(BASE + '/'); await p.waitForSelector('#np', { timeout: 20000 });
      await until(async () => (await p.$eval('#zonename', (e) => e.textContent)) === dbZone.name, 10000);
      const phone = w < 600;
      // Exactly one slider for the zone in the bar area, with the Sound panel open where it can be
      const hasBtn = await p.$eval('#volbtn', (b) => !!b.offsetParent);
      // Counted with the Sound panel open, as the task asks: one slider for the zone, or none and the button
      if (!phone && await p.$eval('#quality', (q) => !!q.offsetParent)) { await p.click('#quality'); await p.waitForSelector('#qpanel:not([hidden])'); }
      const bar = await visibleRanges(p, '.player input');
      const panel = await visibleRanges(p, '#qpanel input');
      check(panel.length === 0, `${w}: the Sound panel has no volume slider`);
      check(bar.length + panel.length === (hasBtn || phone ? 0 : 1), `${w}: with the Sound panel open, ${hasBtn || phone ? 'no slider in the bar area' : 'exactly one slider in the bar area'} (${JSON.stringify([...bar, ...panel])})`);
      await p.keyboard.press('Escape'); await sleep(200);
      if (phone) {
        check(bar.length === 0 && !hasBtn, `${w}: a phone's bar has no volume slider or button (Now playing and Zones have it)`);
      } else if (hasBtn) {
        check(bar.length === 0, `${w}: no room: the bar has the volume button and no slider (${JSON.stringify(bar)})`);
        await p.click('#volbtn'); await p.waitForSelector('#volpop:not([hidden]) #volpopvol', { timeout: 5000 });
        const all = await visibleRanges(p, '.player input, #volpop input, #qpanel input');
        check(all.length === 1 && all[0] === 'volpopvol' && await p.$eval('#qpanel', (q) => q.hidden), `${w}: the volume button opens the popover with the one slider (${JSON.stringify(all)})`);
        check(await p.evaluate(() => document.activeElement && document.activeElement.id === 'volpopvol'), `${w}: focus is on its slider`);
        await slide(p, '#volpopvol', -20);
        check(await until(async () => { const v = await zoneVol(dbZone.id); return v && v.db === -20; }), `${w}: the popover sets the zone's volume (read back -20 dB)`);
        check(await until(async () => /-20 dB/.test(await p.textContent('#volpop .vol-level'))), `${w}: and says the level`);
        await p.click('#volpop [data-volmute]');
        check(await until(async () => { const v = await zoneVol(dbZone.id); return v && v.muted; }) && await until(async () => /Muted/.test(await p.textContent('#volpop .vol-level'))), `${w}: Mute mutes the zone, and says so`);
        await p.click('#volpop [data-volmute]'); await until(async () => { const v = await zoneVol(dbZone.id); return v && !v.muted; });
        await p.mouse.click(w / 2, 40); await sleep(300);
        check(await p.$eval('#volpop', (v) => v.hidden), `${w}: a tap outside closes it`);
      } else {
        check(bar.length === 1 && bar[0] === 'vol' && !hasBtn, `${w}: room: the bar has the one slider and no volume button (${JSON.stringify(bar)})`);
        const r = await p.$eval('#vol', (e) => [e.min, e.max, e.value].join(' '));
        check(r === '-60 0 0', `${w}: the bar's slider is on the output's scale at its level (${r})`);
        await slide(p, '#vol', -20);
        check(await until(async () => { const v = await zoneVol(dbZone.id); return v && v.db === -20; }), `${w}: the bar sets the zone's volume (read back -20 dB)`);
        // A change made elsewhere (the device's own remote, another controller) shows in the bar
        await api('POST', `/zones/${dbZone.id}/volume`, { volume: { db: -35 } });
        check(await until(async () => (await p.$eval('#vol', (e) => e.value)) === '-35', 6000), `${w}: a change made elsewhere shows in the bar`);
        // A slider being dragged isn't overwritten; let go, it catches up
        const box = await p.$eval('#vol', (e) => { const b = e.getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; });
        await p.mouse.move(box[0], box[1]); await p.mouse.down(); await sleep(100);
        await api('POST', `/zones/${dbZone.id}/volume`, { volume: { db: -50 } }); await sleep(1500);
        const held = await p.$eval('#vol', (e) => e.value);
        await p.mouse.up(); await sleep(600);
        const after = await p.$eval('#vol', (e) => e.value);
        // Two browsers differ here: a press may move the thumb to where it was pressed (then
        // letting go sets that), or leave it (then letting go catches up with the zone)
        if (held !== '-35') check(held !== '-50' && after === held && await until(async () => { const v = await zoneVol(dbZone.id); return v && String(v.db) === held; }), `${w}: held, the slider keeps its own value (${held}), not the change from elsewhere; let go, that value is what the zone takes (${after})`);
        else check(after === '-50' && (await zoneVol(dbZone.id)).db === -50, `${w}: held, the slider keeps its value (${held}) while a change from elsewhere arrives; let go without a change, it catches up (${after})`);
      }
      // The Zones page: its slider for the zone agrees, in the zone's own scale (the developer
      // zones list once the page knows they're switched on)
      await p.goto(BASE + '/zones'); await p.waitForSelector(`[data-zvol="${dbZone.id}"]`, { timeout: 15000 });
      await until(async () => (await p.$(`[data-zvol="${pcZone.id}"]`)) !== null, 10000);
      const v = await zoneVol(dbZone.id);
      check(await until(async () => (await p.$eval(`[data-zvol="${dbZone.id}"]`, (e) => Number(e.value))) === v.db), `${w}: the Zones page slider shows the same level (${v.db} dB)`);
      const pc = await p.$eval(`[data-zvol="${pcZone.id}"]`, (e) => [e.min, e.max, e.value].join(' ')).catch(() => null);
      check(pc === '0 100 100', `${w}: a percent output's slider is 0–100 at its level (${pc})`);
      check(await p.$$eval(`[data-zvol="${plain ? plain.id : 'none'}"]`, (els) => els.length) === 0, `${w}: an output without a volume has no slider`);
      if (phone) {
        await slide(p, `[data-zvol="${pcZone.id}"]`, 40);
        check(await until(async () => { const x = await zoneVol(pcZone.id); return x && x.percent === 40; }), `${w}: a percent zone takes percent (40)`);
        check(await until(async () => /40 %/.test(await p.textContent(`[data-vollevel="${pcZone.id}"]`))), `${w}: and says "40 %"`);
      }
      // Switching zones (through Play to): the bar follows the zone's scale, This browser its own
      if (!phone) {
        const pickZone = async (id, name) => { await p.click('#zonebtn'); await p.click(`#zonepop [data-zone="${id}"]`); await until(async () => (await p.$eval('#zonename', (e) => e.textContent)) === name, 8000); };
        await pickZone(pcZone.id, pcZone.name);
        if (!hasBtn) check(await until(async () => (await p.$eval('#vol', (e) => [e.min, e.max].join(' '))) === '0 100'), `${w}: the bar follows the zone's scale after a switch`);
        await pickZone('browser', 'This browser');
        if (!hasBtn) check(await until(async () => (await p.$eval('#vol', (e) => [e.min, e.max, e.value].join(' '))) === '-60 0 0'), `${w}: This browser's own volume, unchanged`);
      }
      check(errors.length === 0, `${w}: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
      await ctx.close();
    }
  } finally {
    await browser.close();
    await api('PUT', '/node/settings', { dev_outputs: !!devBefore }).catch(() => {});
  }
  if (problems.length) { console.log(`${problems.length} failed`); process.exit(1); }
  console.log('volume ok');
})().catch((e) => { console.error(e); process.exit(1); });
