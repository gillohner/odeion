// Settings › Zones and outputs › Speakers and outputs: one line saying what the card is, and a
// "Which speakers work" link to the speakers guide (the one Home's setup card links to), before the
// first control. It was a nine-line paragraph on a phone (squeezelite, UPnP/Sonos/…, MPD and Snapcast
// config, the app's own player); all of that is in docs/guides/speakers.md.
// Checks at 360, 360 in the Android app, 412 and 1280 px:
//   * the help above the speaker list is at most 2 lines and names no setup detail (squeezelite -s…);
//   * its link is underlined, on one line, opens docs/guides/speakers.md in a new tab, and on a phone a tap
//     up to 21 px above or below its centre still lands on it (elementFromPoint), and that tap area stays
//     off the speaker list below (no point of the list's top 4 px under the link is the link);
//   * Home's "Which speakers work" goes to the same guide;
//   * no horizontal overflow, no page errors.
//   node tools/ui-speakers-help.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const fx = require('./ui-fixture');
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '/tmp', 'speakers-help-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const GUIDE = /\/docs\/guides\/speakers\.md$/;
const outputs = [{ id: 'upnp:kitchen', name: 'kitchen', kind: 'upnp', caps: { hw_volume: 'percent', max_pcm_bits: 24, pcm_rates: [44100] } }];
const steps = { show: true, hidden: false, steps: ['play', 'speakers'].map((id) => ({ id, done: false, optional: id !== 'play' })) };

(async () => {
  const browser = await chromium.launch();
  for (const [width, android] of [[360, false], [360, true], [412, false], [1280, false]]) {
    const tag = `${width}${android ? '-android' : ''}`;
    const phone = width < 600;
    const { context, page } = await fx.create(browser, width, { mobile: phone, fixtures: { '/me/first-steps': { body: steps }, '/homeserver/bundled': { body: { configured: false, reachable: false } }, '/outputs': { body: outputs } } });
    if (android) await context.addInitScript(() => { window.pmDesktop = { platform: 'android', version: 'fixture' }; });
    const errors = []; page.on('pageerror', (e) => errors.push(String(e)));
    try {
      await fx.go(page, '/settings/playback');
      await page.waitForSelector('#rescan', { timeout: 15000 });
      const h = await page.evaluate(() => {
        const g = [...document.querySelectorAll('.set-group')].find((x) => x.querySelector('.set-h')?.textContent.trim() === 'Speakers and outputs');
        const p = g && g.querySelector('.set-help');
        if (!p) return null;
        const lh = parseFloat(getComputedStyle(p).lineHeight);
        const a = p.querySelector('a');
        g.scrollIntoView({ block: 'center' });
        // The tap area as a finger finds it: how far up and down from the link's centre a tap still lands
        // on the link (the nearer edge, both ways, 1 px steps, so a clipped or covered layer counts as missing)
        let tap = 0, under = null;
        if (a) {
          const r = a.getBoundingClientRect(); const x = r.left + r.width / 2; const y = r.top + r.height / 2;
          const on = (dy) => { const e = document.elementFromPoint(x, y + dy); return !!e && (e === a || a.contains(e)); };
          let up = 0; while (up < 40 && on(-(up + 1))) up++;
          let down = 0; while (down < 40 && on(down + 1)) down++;
          tap = 1 + 2 * Math.min(up, down);
          // The list under the help: its top 4 px, from 8 px left of the link to 8 px right of it
          const body = g.querySelector('.set-body'); const b = body.getBoundingClientRect();
          const hits = [];
          for (let dy = 0.5; dy < 4; dy += 1) for (let px = r.left - 8; px <= r.right + 8; px += 2) { const e = document.elementFromPoint(px, b.top + dy); if (e && (e === a || a.contains(e))) hits.push(`${Math.round(px)},${(b.top + dy).toFixed(1)}`); }
          under = { first: body.querySelector('.set-item') ? 'a speaker row' : body.firstElementChild?.className, gap: b.top - (y + 22), hits };
        }
        return { lines: Math.round(p.getBoundingClientRect().height / lh), text: p.textContent, under, link: a && { text: a.textContent.trim(), href: a.href, target: a.target, rel: a.rel, rects: a.getClientRects().length, underline: getComputedStyle(a).textDecorationLine, tap } };
      });
      check(!!h, `${tag}: Speakers and outputs has a help line`);
      if (h) {
        check(h.lines <= 2, `${tag}: the help is ${h.lines} line(s), at most 2`);
        check(!/squeezelite|Snapcast|BluOS|plays only for this node/.test(h.text), `${tag}: the setup details are in the guide, not the card`);
        check(!!h.link && h.link.text === 'Which speakers work', `${tag}: the help links "Which speakers work"`);
        if (h.link) {
          check(GUIDE.test(h.link.href), `${tag}: the link goes to the speakers guide (${h.link.href})`);
          check(h.link.target === '_blank' && /noopener/.test(h.link.rel), `${tag}: the guide opens in a new tab`);
          check(h.link.rects === 1, `${tag}: the link is on one line`);
          check(/underline/.test(h.link.underline), `${tag}: the link is underlined (it sits in muted text)`);
          if (phone) check(h.link.tap >= 43, `${tag}: the link's tap area is ${h.link.tap} px tall around its centre, at least 43 (±21)`);
          check(h.under?.first === 'a speaker row', `${tag}: the list under the help starts with a speaker (${h.under?.first})`);
          check(h.under && h.under.hits.length === 0, `${tag}: a tap on the speaker list's top edge is not the link (${h.under?.hits.length} points; ${h.under?.gap.toFixed(1)} px between the 44 px layer's foot and the list)`);
        }
      }
      await fx.cleanCopy(page).then(() => check(true, `${tag}: no undefined text, no horizontal overflow`), (e) => check(false, `${tag}: ${e.message}`));
      await page.screenshot({ path: path.join(SHOTS, `after-zones-${tag}.png`) });
      await fx.go(page, '/');
      const home = await page.waitForSelector('[data-step="speakers"] a.lnk', { timeout: 15000 }).then((a) => a.evaluate((x) => x.href)).catch(() => null);
      check(!!home && GUIDE.test(home) && home === h?.link?.href, `${tag}: Home's "Which speakers work" goes to the same guide`);
      check(!errors.length, `${tag}: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
    } catch (e) { check(false, `${tag}: ${e.message}`); }
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} FAIL` : 'all ok');
  process.exit(failed ? 1 : 0);
})();
