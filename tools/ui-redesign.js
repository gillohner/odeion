// The Android-first redesign, part (b) (TASK-ui-motion-polish-1106 §4; owner: "consistent …
// sizing of elements", "better colors", "optimizing UX and texts"). Real candidate assets on the
// fixture origin (tools/ui-fixture.js) with a phone's viewport rules and the owner's zones:
//   * the palette holds WCAG AA on every pair (tools/palette-contrast.js) and every font-size and
//     radius is on its scale (design-lint rule 5);
//   * on a touch screen every small control's hit area is at least 48 px, without changing its box;
//   * Zones: a device seen two ways (the owner's Linn, AirPlay and UPnP) is one card with a chip per
//     way; a zone with nothing playing shows the zone's mark, not a blank square;
//   * the texts the owner saw: "Group outputs…" is its icon on a phone (named for a screen reader),
//     the chip and the zone button keep more of their words, and no screen says "outside".
//   node tools/ui-redesign.js [repo] [shots]
const { chromium } = require(require.resolve('playwright', { paths: [process.cwd(), __dirname] }));
const fs = require('node:fs'); const path = require('node:path');
const { execFileSync } = require('node:child_process');
const fx = require('./ui-fixture');
const REPO = path.resolve(process.argv[2] || path.join(__dirname, '..'));
const SHOTS = path.resolve(process.argv[3] || path.join(process.env.TMPDIR || '.', 'redesign-shots'));
fs.mkdirSync(SHOTS, { recursive: true });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const fmt = { kind: 'pcm', rate_hz: 44100, bits: 16, channels: 2 };
const NAMES = [['heimkino', 'Heimkino (AirPlay)', 'airplay'], ['linn-air', 'Linn Klimax DS3 (AirPlay)', 'airplay'], ['linn-upnp', 'Linn Klimax DS3:UPnP AV', 'upnp'], ['mbp', 'MacBook Pro ELo (AirPlay)', 'airplay'], ['tv', '[TV] The Frame 65 Lohner', 'upnp']];
const outputs = NAMES.map(([id, name, kind]) => ({ id: `${kind}:${id}`, name, kind, caps: kind === 'upnp' ? { hw_volume: 'percent' } : {} }));
const np = { item: { id: 1, uri: 'track:1', title: 'Come On', artist: 'Dr. Peacock, Sefa', album: 'Frenchcore Worldwide 02', duration_ms: 230000 }, index: 0, position_ms: 61000, duration_ms: 230000, source_format: fmt, output_format: fmt, signal_path: { stages: [] }, format: { codec: 'FLAC', lossless: true, service: 'library', preview: false } };
const zones = NAMES.map(([id, name, kind], i) => ({ id, name, output: `${kind}:${id}`, transport: i === 2 ? 'playing' : 'stopped', queue: i === 2 ? [{ id: 1, uri: 'track:1', title: 'Come On' }] : [], queue_index: i === 2 ? 0 : null, queue_version: 1, repeat: 'off', radio: false, error: null, acked: 1, sync: 'single', crossfade_ms: 0, volume: kind === 'upnp' ? { percent: 20, muted: false } : null, now_playing: i === 2 ? np : null }));
const health = [{ id: 'api-token', name: 'API token', state: 'change', detail: 'change it in the config', ms: null }];
const fixtures = { '/zones': { body: zones }, '/outputs': { body: outputs }, '/health': { body: health } };

(async () => {
  // The palette and the scales, from the files
  {
    let ok = true; let out = '';
    try { out = execFileSync(process.execPath, [path.join(REPO, 'tools/palette-contrast.js'), path.join(REPO, 'crates/pm-api/static/app.css')], { encoding: 'utf8' }); } catch (e) { ok = false; out = String(e.stdout || e.message); }
    check(ok, `palette: every pair at AA${ok ? ` (${(out.match(/(\d+) pairs/) || [])[1]} pairs)` : ': ' + out.split('\n').filter((l) => /FAIL/.test(l)).slice(0, 3).join(' | ')}`);
    let lint = ''; let lintOk = true;
    try { lint = execFileSync(process.execPath, [path.join(REPO, 'tools/design-lint.js'), path.join(REPO, 'crates/pm-api/static')], { encoding: 'utf8' }); } catch (e) { lintOk = false; lint = String(e.stdout || e.message); }
    check(lintOk && !/off the/.test(lint), `every font-size and radius is on its scale (design-lint rule 5)${lintOk ? '' : ': ' + lint.split('\n').filter((l) => /off the/.test(l)).slice(0, 3).join(' | ')}`);
    // (the words in app.js's string literals: a small scan that knows quotes, template strings and
    // their ${} holes, and reads the code inside a hole for strings of its own)
    const js = fs.readFileSync(path.join(REPO, 'crates/pm-api/static/app.js'), 'utf8');
    const strings = [];
    const scan = (code, base) => {
      let q = null; let buf = ''; let line = base; let at = base;
      for (let i = 0; i < code.length; i++) {
        const c = code[i]; if (c === '\n') line++;
        if (!q) {
          if (c === "'" || c === '"' || c === '`') { q = c; buf = ''; at = line; } else if (c === '/' && code[i + 1] === '/') { while (i < code.length && code[i] !== '\n') i++; line++; } else if (c === '/' && code[i + 1] === '*') { const j = code.indexOf('*/', i + 2); line += (code.slice(i, j < 0 ? code.length : j).match(/\n/g) || []).length; i = j < 0 ? code.length : j + 1; } else if (c === '/') {
            // a regex literal (after an operator or an opening bracket): to its closing slash, past classes and escapes
            // (after ++ or -- it is a division; a skip that ends at a newline leaves the newline to the loop)
            let k = i - 1; while (k >= 0 && /\s/.test(code[k])) k--;
            const before = k >= 0 ? code[k] : ''; const upto = code.slice(Math.max(0, k - 6), k + 1);
            const postfix = (before === '+' || before === '-') && code[k - 1] === before;
            if (!postfix && (!before || '(,=:[!&|?{};+-*%<>~^'.includes(before) || /\b(return|typeof|case|do|else|in|of)$/.test(upto))) { let j = i + 1; let cls = false; while (j < code.length && (cls || code[j] !== '/') && code[j] !== '\n') { if (code[j] === '\\') j++; else if (code[j] === '[') cls = true; else if (code[j] === ']') cls = false; j++; } i = code[j] === '\n' ? j - 1 : j; }
          }
          continue;
        }
        if (c === '\\') { buf += code[i + 1]; i++; continue; }
        if (q === '`' && c === '$' && code[i + 1] === '{') {
          // the hole: to its matching brace, counting braces outside strings; scanned as code
          let depth = 1; let j = i + 2; let inner = null;
          while (j < code.length && depth) { const d = code[j]; if (inner) { if (d === '\\') j++; else if (d === inner) inner = null; } else if (d === "'" || d === '"' || d === '`') inner = d; else if (d === '{') depth++; else if (d === '}') depth--; j++; }
          scan(code.slice(i + 2, j - 1), line); line += (code.slice(i, j).match(/\n/g) || []).length; i = j - 1; buf += ' '; continue;
        }
        if (c === q) { strings.push([at, buf]); q = null; continue; }
        buf += c;
      }
    };
    scan(js, 1);
    const said = strings.filter(([, t]) => /\boutside\b/i.test(t)).map(([l, t]) => `${l}: ${t.trim().slice(0, 60)}`);
    check(!said.length, `no screen says "outside" (the owner's rule: external resources)${said.length ? ': ' + said.slice(0, 2).join(' | ') : ''}`);
  }
  const browser = await chromium.launch();
  for (const [w, h] of [[360, 800], [390, 844], [412, 915]]) {
    const { page, context, state } = await fx.create(browser, w, { mobile: true, fixtures });
    await page.setViewportSize({ width: w, height: h });
    await context.routeWebSocket(/\/v1\/events/, (ws) => { ws.onMessage((m) => { let j; try { j = JSON.parse(m); } catch { return; } if (j.type === 'ping') ws.send(JSON.stringify({ type: 'pong', id: j.id })); }); ws.send(JSON.stringify({ type: 'snapshot', seq: 0, boot: 'fixture-boot', library_rev: 0, zones })); });
    await context.addInitScript(() => { localStorage.setItem('pm.zone', 'linn-upnp'); window.pmDesktop = { platform: 'android', version: '0.0.0' }; });
    await fx.go(page, '/zones'); await page.waitForSelector('.zone-card'); await sleep(400);
    const z = await page.evaluate(() => {
      const cards = [...document.querySelectorAll('.zone-card')];
      const linn = cards.find((c) => /Linn Klimax DS3/.test(c.querySelector('h3').textContent));
      const ways = linn ? [...linn.querySelectorAll('.zc-ways .chip')].map((c) => ({ t: c.textContent.trim(), on: c.classList.contains('on'), pressed: c.getAttribute('aria-pressed') })) : [];
      const ph = cards.find((c) => c.querySelector('.zc-ph'));
      // (a point 5 px above the control's box must land on the control itself: its reach, not a neighbour)
      const hit = (sel) => { const el = document.querySelector(sel); if (!el) return null; const cs = getComputedStyle(el, '::before'); const r = el.getBoundingClientRect(); const inset = parseFloat(cs.top) || 0; const above = document.elementFromPoint(r.left + r.width / 2, r.top - 5); return { box: Math.round(Math.min(r.width, r.height)), reach: Math.round(r.height - 2 * inset), content: cs.content, aboveHits: above === el || el.contains(above) }; };
      const g = document.querySelector('#zgroup'); const label = g && g.querySelector('.btn-label');
      return { n: cards.length, titles: cards.map((c) => c.querySelector('h3').textContent.trim()), linnTitle: linn && linn.querySelector('h3').textContent.trim(), ways, phIcon: !!(ph && ph.querySelector('.zc-ph svg')), chip: hit('.zc-ways .chip'), iconBtn: hit('.zone-card .icon-btn'), group: { aria: g && g.getAttribute('aria-label'), labelShown: !!label && getComputedStyle(label).display !== 'none' } };
    });
    check(z.n === 5 && z.linnTitle === 'Linn Klimax DS3' && z.ways.length === 2 && z.ways.map((x) => x.t).join('/') === 'AirPlay/UPnP' && z.ways[1].on && z.ways[1].pressed === 'true', `${w}: the Linn seen two ways is one card, "${z.linnTitle}", with the chips ${z.ways.map((x) => x.t).join(' and ')} (UPnP on) among ${z.n} cards`);
    check(z.phIcon, `${w}: a zone with nothing playing shows the zone's mark`);
    check(z.chip && z.chip.content !== 'none' && z.chip.reach >= 48 && z.chip.aboveHits && z.iconBtn && z.iconBtn.reach >= 48 && z.iconBtn.aboveHits, `${w}: hit areas reach 48 px up and down on a touch screen and a tap 5 px above lands on the control (chip ${z.chip && z.chip.box} → ${z.chip && z.chip.reach}, above ${z.chip && z.chip.aboveHits}; icon button ${z.iconBtn && z.iconBtn.box} → ${z.iconBtn && z.iconBtn.reach}, above ${z.iconBtn && z.iconBtn.aboveHits})`);
    check(z.group.aria === 'Group outputs' && !z.group.labelShown, `${w}: "Group outputs…" is its icon on a phone, named for a screen reader (${z.group.aria})`);
    await page.screenshot({ path: path.join(SHOTS, `zones-${w}.png`) });
    // A way's chip plays to that way: the card follows
    await page.click('.zc-ways .chip:not(.on)'); await sleep(400);
    const sw = await page.evaluate(() => { const c = [...document.querySelectorAll('.zone-card')].find((x) => /Linn Klimax DS3/.test(x.querySelector('h3').textContent)); return { on: [...c.querySelectorAll('.zc-ways .chip')].map((x) => x.classList.contains('on')), zone: localStorage.getItem('pm.zone') }; });
    check(sw.on[0] && !sw.on[1] && sw.zone === 'linn-air', `${w}: the AirPlay chip plays to the Linn over AirPlay (${sw.zone})`);
    // The bar's zone button and the status chip keep more of their words
    await fx.go(page, '/'); await page.waitForSelector('#upstreams:not([hidden])', { timeout: 5000 }).catch(() => {}); await sleep(300);
    const words = await page.evaluate(() => { const zb = document.querySelector('#zonename'); const up = document.querySelector('#upstreams'); const fits = (el) => el && el.scrollWidth <= el.clientWidth + 1; return { zone: zb && zb.textContent, zoneFits: fits(zb), zoneShown: zb && Math.round(zb.getBoundingClientRect().width), zoneMax: zb && getComputedStyle(zb).maxWidth, upMax: up && getComputedStyle(up).maxWidth, vw: innerWidth }; });
    // (the words are cut with an ellipsis where the bar is tight: what shows must still be readable)
    // (a 360 px bar has 36 px for it beside the play button and the quality pill; a 412 px one near 100)
    check(words.zoneMax && parseFloat(words.zoneMax) >= (w < 400 ? 84 : 96) && words.zoneShown >= (w < 400 ? 32 : 44), `${w}: the bar's zone button may keep up to ${words.zoneMax} of its words (was ${w <= 420 ? 64 : 96}px) and shows ${words.zoneShown} px of them`);
    check(words.upMax && (words.upMax === '48%' || parseFloat(words.upMax) >= words.vw * 0.47), `${w}: the status chip may take ${words.upMax} of the bar (was 40%)`);
    check(!state.errors.length, `${w}: no page errors${state.errors.length ? ': ' + state.errors.join('; ') : ''}`);
    await context.close();
  }
  await browser.close();
  console.log(failed ? `${failed} failed` : 'all passed'); console.log(`screenshots: ${SHOTS}`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
